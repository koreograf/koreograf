//! Process-local counters and the Prometheus text scrape.
//!
//! Counters reset when the process starts. Cache gauges are read from disk at
//! scrape time. Ansible role names come from the default callback
//! (`TASK [role : task]`), not from cache-copy lines in the job log.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use crate::cacheinfo::{self, CacheReport};
use crate::db;
use crate::Shared;

const LABEL_PLAYBOOK: usize = 120;
const LABEL_ROLE: usize = 80;
const LABEL_TEAM: usize = 80;
const LABEL_TEMPLATE: usize = 80;
const LABEL_TOOL: usize = 32;

pub(crate) struct JobMeta<'a> {
    pub job_id: &'a str,
    pub team_id: &'a str,
    pub tool: &'a str,
    pub playbook: &'a str,
    pub template: &'a str,
}

#[derive(Clone, Debug)]
pub(crate) struct CacheGauge {
    pub team_id: String,
    pub team: String,
    pub collections: u64,
    pub roles: u64,
    pub collections_bytes: u64,
    pub roles_bytes: u64,
    pub tofu_bytes: u64,
}

struct OpenJob {
    tool: String,
    playbook: String,
    team_id: String,
    template: String,
    started: Instant,
    observer: PlayObserver,
}

struct Book {
    started: BTreeMap<(String, String), u64>,
    finished: BTreeMap<(String, String, String), u64>,
    roles: BTreeMap<(String, String, &'static str), u64>,
    open: HashMap<String, OpenJob>,
}

pub(crate) struct Metrics {
    inner: Mutex<Book>,
}

struct RoleSummary {
    role: String,
    ok: u64,
    changed: u64,
    failed: u64,
    unreachable: u64,
    skipped: u64,
}

enum Step {
    Host { role: String, result: &'static str },
    Done(RoleSummary),
}

enum LogEv {
    Host {
        playbook: String,
        role: String,
        result: &'static str,
    },
    Done {
        playbook: String,
        summary: RoleSummary,
    },
}

struct ClosedJob {
    tool: String,
    playbook: String,
    team_id: String,
    template: String,
    status: String,
    duration_ms: u64,
    summary: Option<RoleSummary>,
}

#[derive(Default)]
struct PlayObserver {
    role: Option<String>,
    ok: u64,
    changed: u64,
    failed: u64,
    unreachable: u64,
    skipped: u64,
    saw_play: bool,
    saw_host: bool,
    saw_work: bool,
}

impl Metrics {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(Book {
                started: BTreeMap::new(),
                finished: BTreeMap::new(),
                roles: BTreeMap::new(),
                open: HashMap::new(),
            }),
        }
    }

    pub(crate) fn begin(&self, meta: &JobMeta<'_>) {
        let Some(open) = self.record_begin(meta) else {
            return;
        };
        tracing::info!(
            target: "koreograf::job",
            job_id = meta.job_id,
            team_id = %open.team_id,
            tool = %open.tool,
            playbook = %open.playbook,
            template = %open.template,
            "job started"
        );
    }

    pub(crate) fn observe(&self, job_id: &str, line: &str) {
        let Some(events) = self.record_line(job_id, line) else {
            return;
        };
        emit(job_id, events);
    }

    /// A tag filter reached hosts and never ran a task other than Gathering Facts.
    pub(crate) fn tag_filter_matched_nothing(&self, job_id: &str) -> bool {
        self.inner
            .lock()
            .ok()
            .and_then(|book| {
                let open = book.open.get(job_id)?;
                (open.tool == "ansible").then_some(open.observer.matched_nothing())
            })
            .unwrap_or(false)
    }

    pub(crate) fn close(&self, job_id: &str, status: &str) {
        let Some(closed) = self.record_close(job_id, status) else {
            return;
        };
        if let Some(summary) = closed.summary {
            log_role_finished(job_id, &closed.playbook, &summary);
        }
        tracing::info!(
            target: "koreograf::job",
            job_id,
            team_id = %closed.team_id,
            tool = %closed.tool,
            playbook = %closed.playbook,
            template = %closed.template,
            status = %closed.status,
            duration_ms = closed.duration_ms,
            "job finished"
        );
    }

    pub(crate) fn render(&self, caches: &[CacheGauge]) -> String {
        let Ok(book) = self.inner.lock() else {
            return String::new();
        };
        let mut out = String::new();
        family(
            &mut out,
            "koreograf_jobs_started_total",
            "counter",
            "Jobs that began execution.",
        );
        for ((tool, playbook), value) in &book.started {
            sample(
                &mut out,
                "koreograf_jobs_started_total",
                &[("tool", tool), ("playbook", playbook)],
                *value,
            );
        }
        family(
            &mut out,
            "koreograf_jobs_finished_total",
            "counter",
            "Jobs that reached a terminal status.",
        );
        for ((tool, playbook, status), value) in &book.finished {
            sample(
                &mut out,
                "koreograf_jobs_finished_total",
                &[("tool", tool), ("playbook", playbook), ("status", status)],
                *value,
            );
        }
        family(
            &mut out,
            "koreograf_role_results_total",
            "counter",
            "Ansible role task results seen in playbook output.",
        );
        for ((playbook, role, result), value) in &book.roles {
            sample(
                &mut out,
                "koreograf_role_results_total",
                &[("playbook", playbook), ("role", role), ("result", result)],
                *value,
            );
        }
        family(
            &mut out,
            "koreograf_cache_collections",
            "gauge",
            "Ansible collections stored in the team cache.",
        );
        for gauge in caches {
            sample(
                &mut out,
                "koreograf_cache_collections",
                &[("team_id", &gauge.team_id), ("team", &gauge.team)],
                gauge.collections,
            );
        }
        family(
            &mut out,
            "koreograf_cache_roles",
            "gauge",
            "Ansible roles stored in the team cache.",
        );
        for gauge in caches {
            sample(
                &mut out,
                "koreograf_cache_roles",
                &[("team_id", &gauge.team_id), ("team", &gauge.team)],
                gauge.roles,
            );
        }
        family(
            &mut out,
            "koreograf_cache_bytes",
            "gauge",
            "Cache size in bytes.",
        );
        for gauge in caches {
            let labels = [
                ("team_id", gauge.team_id.as_str()),
                ("team", gauge.team.as_str()),
            ];
            sample(
                &mut out,
                "koreograf_cache_bytes",
                &[labels[0], labels[1], ("kind", "collections")],
                gauge.collections_bytes,
            );
            sample(
                &mut out,
                "koreograf_cache_bytes",
                &[labels[0], labels[1], ("kind", "roles")],
                gauge.roles_bytes,
            );
            sample(
                &mut out,
                "koreograf_cache_bytes",
                &[labels[0], labels[1], ("kind", "tofu")],
                gauge.tofu_bytes,
            );
        }
        out
    }

    fn record_begin(&self, meta: &JobMeta<'_>) -> Option<OpenJob> {
        let mut book = self.inner.lock().ok()?;
        if book.open.contains_key(meta.job_id) {
            return None;
        }
        let open = OpenJob {
            tool: sanitize_label(meta.tool, LABEL_TOOL),
            playbook: sanitize_label(meta.playbook, LABEL_PLAYBOOK),
            team_id: sanitize_label(meta.team_id, LABEL_TEAM),
            template: sanitize_label(meta.template, LABEL_TEMPLATE),
            started: Instant::now(),
            observer: PlayObserver::default(),
        };
        *book
            .started
            .entry((open.tool.clone(), open.playbook.clone()))
            .or_default() += 1;
        book.open.insert(meta.job_id.to_string(), clone_open(&open));
        Some(open)
    }

    fn record_line(&self, job_id: &str, line: &str) -> Option<Vec<LogEv>> {
        let mut book = self.inner.lock().ok()?;
        let open = book.open.get_mut(job_id)?;
        if open.tool != "ansible" {
            return None;
        }
        let playbook = open.playbook.clone();
        let steps = open.observer.push(line);
        let mut events = Vec::new();
        for step in steps {
            match step {
                Step::Host { role, result } => {
                    *book
                        .roles
                        .entry((playbook.clone(), role.clone(), result))
                        .or_default() += 1;
                    events.push(LogEv::Host {
                        playbook: playbook.clone(),
                        role,
                        result,
                    });
                }
                Step::Done(summary) => events.push(LogEv::Done {
                    playbook: playbook.clone(),
                    summary,
                }),
            }
        }
        Some(events)
    }

    fn record_close(&self, job_id: &str, status: &str) -> Option<ClosedJob> {
        let mut book = self.inner.lock().ok()?;
        let mut open = book.open.remove(job_id)?;
        let status = terminal_status(status).to_string();
        *book
            .finished
            .entry((open.tool.clone(), open.playbook.clone(), status.clone()))
            .or_default() += 1;
        let summary = open.observer.finish();
        let duration_ms = u64::try_from(open.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Some(ClosedJob {
            tool: open.tool,
            playbook: open.playbook,
            team_id: open.team_id,
            template: open.template,
            status,
            duration_ms,
            summary,
        })
    }
}

fn clone_open(open: &OpenJob) -> OpenJob {
    OpenJob {
        tool: open.tool.clone(),
        playbook: open.playbook.clone(),
        team_id: open.team_id.clone(),
        template: open.template.clone(),
        started: open.started,
        observer: PlayObserver::default(),
    }
}

fn emit(job_id: &str, events: Vec<LogEv>) {
    for event in events {
        match event {
            LogEv::Host {
                playbook,
                role,
                result,
            } if result == "failed" || result == "unreachable" => {
                tracing::warn!(
                    target: "koreograf::role",
                    job_id,
                    playbook,
                    role,
                    result,
                    "Rollenergebnis"
                );
            }
            LogEv::Host { .. } => {}
            LogEv::Done { playbook, summary } => log_role_finished(job_id, &playbook, &summary),
        }
    }
}

fn log_role_finished(job_id: &str, playbook: &str, summary: &RoleSummary) {
    tracing::info!(
        target: "koreograf::role",
        job_id,
        playbook,
        role = %summary.role,
        ok = summary.ok,
        changed = summary.changed,
        failed = summary.failed,
        unreachable = summary.unreachable,
        skipped = summary.skipped,
        "Rolle beendet"
    );
}

pub(crate) fn log_cache(data_dir: &Path, job_id: &str, team_id: &str) {
    let report = cacheinfo::team_cache(data_dir, team_id);
    log_cache_report(job_id, team_id, &report);
}

fn log_cache_report(job_id: &str, team_id: &str, report: &CacheReport) {
    tracing::info!(
        target: "koreograf::cache",
        job_id,
        team_id,
        collections = report.collections.len(),
        roles = report.roles.len(),
        collections_bytes = report.collections_bytes,
        roles_bytes = report.roles_bytes,
        tofu_bytes = report.tofu_bytes,
        "Cache-Stand"
    );
}

pub(crate) fn scrape(state: &Shared) -> String {
    let ids = cache_team_ids(&state.cfg.data_dir);
    let names = team_names(state, &ids);
    let gauges = gauges_from_dir(&state.cfg.data_dir, &names);
    state.metrics.render(&gauges)
}

pub(crate) fn gauges_from_dir(data_dir: &Path, names: &HashMap<String, String>) -> Vec<CacheGauge> {
    cache_team_ids(data_dir)
        .into_iter()
        .map(|id| {
            let report = cacheinfo::team_cache(data_dir, &id);
            let team = names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| sanitize_label(&id, LABEL_TEAM));
            CacheGauge {
                team_id: sanitize_label(&id, LABEL_TEAM),
                team: sanitize_label(&team, LABEL_TEAM),
                collections: as_u64(report.collections.len()),
                roles: as_u64(report.roles.len()),
                collections_bytes: report.collections_bytes,
                roles_bytes: report.roles_bytes,
                tofu_bytes: report.tofu_bytes,
            }
        })
        .collect()
}

fn team_names(state: &Shared, ids: &[String]) -> HashMap<String, String> {
    let Ok(db) = state.db() else {
        return HashMap::new();
    };
    let mut names = HashMap::new();
    for id in ids {
        if let Ok(Some(team)) = db::team_by_id(&db, id) {
            names.insert(id.clone(), team.name);
        }
    }
    names
}

fn cache_team_ids(data_dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(data_dir.join("cache")) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for entry in entries.flatten() {
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.is_empty() && !name.contains(['/', '\\']) {
                ids.push(name);
            }
        }
    }
    ids.sort();
    ids
}

fn as_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn terminal_status(status: &str) -> &str {
    match status {
        "successful" | "failed" | "canceled" => status,
        _ => "failed",
    }
}

impl PlayObserver {
    fn push(&mut self, line: &str) -> Vec<Step> {
        let cleaned = strip_ansi(line);
        let line = cleaned.trim();
        if line.is_empty() || line.starts_with("KOREOGRAF") {
            return Vec::new();
        }
        if line.starts_with("PLAY ") {
            self.saw_play = true;
            return self.end_role().into_iter().map(Step::Done).collect();
        }
        if host_result(line).is_some() || play_recap_host(line) {
            self.saw_host = true;
        }
        if let Some(task) = task_name(line) {
            if task != "Gathering Facts" {
                self.saw_work = true;
            }
            let mut steps = Vec::new();
            let next = role_of_task(&task);
            if self.role.as_deref() != next.as_deref() {
                if let Some(done) = self.end_role() {
                    steps.push(Step::Done(done));
                }
                self.role = next;
            }
            return steps;
        }
        let Some(result) = host_result(line) else {
            return Vec::new();
        };
        let Some(role) = self.role.clone() else {
            return Vec::new();
        };
        match result {
            "ok" => self.ok += 1,
            "changed" => self.changed += 1,
            "failed" => self.failed += 1,
            "unreachable" => self.unreachable += 1,
            "skipped" => self.skipped += 1,
            _ => {}
        }
        vec![Step::Host { role, result }]
    }

    fn finish(&mut self) -> Option<RoleSummary> {
        self.end_role()
    }

    fn matched_nothing(&self) -> bool {
        self.saw_play && self.saw_host && !self.saw_work
    }

    fn end_role(&mut self) -> Option<RoleSummary> {
        let role = self.role.take()?;
        let summary = RoleSummary {
            role,
            ok: self.ok,
            changed: self.changed,
            failed: self.failed,
            unreachable: self.unreachable,
            skipped: self.skipped,
        };
        self.ok = 0;
        self.changed = 0;
        self.failed = 0;
        self.unreachable = 0;
        self.skipped = 0;
        if summary.ok + summary.changed + summary.failed + summary.unreachable + summary.skipped
            == 0
        {
            None
        } else {
            Some(summary)
        }
    }
}

fn play_recap_host(line: &str) -> bool {
    let Some(marker) = line.find(" : ok=").or_else(|| line.find(": ok=")) else {
        return false;
    };
    let host = line[..marker].trim();
    !host.is_empty() && !host.contains(' ') && line.contains("changed=")
}

fn task_name(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("TASK ")
        .or_else(|| line.strip_prefix("RUNNING HANDLER "))?;
    let rest = rest.trim_start().strip_prefix('[')?;
    let end = rest.rfind(']')?;
    Some(rest[..end].trim().to_string())
}

fn role_of_task(task: &str) -> Option<String> {
    let (role, _) = task.split_once(" : ")?;
    let role = sanitize_label(role.trim(), LABEL_ROLE);
    if role == "-" {
        None
    } else {
        Some(role)
    }
}

fn host_result(line: &str) -> Option<&'static str> {
    let (kind, rest) = line.split_once(':')?;
    if !rest.trim_start().starts_with('[') {
        return None;
    }
    match kind.trim() {
        "ok" => Some("ok"),
        "changed" => Some("changed"),
        "fatal" | "failed" => Some("failed"),
        "unreachable" => Some("unreachable"),
        "skipping" => Some("skipped"),
        _ => None,
    }
}

fn strip_ansi(line: &str) -> Cow<'_, str> {
    if !line.contains('\u{1b}') {
        return Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.next() == Some('[') {
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(ch);
    }
    Cow::Owned(out)
}

fn sanitize_label(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in value.chars().take(max_chars) {
        match ch {
            '\n' | '\r' | '\t' => out.push(' '),
            ch if ch.is_control() => {}
            ch => out.push(ch),
        }
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        "-".to_string()
    } else {
        trimmed.to_string()
    }
}

fn prom_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '"' => out.push_str("\\\""),
            ch => out.push(ch),
        }
    }
    out
}

fn family(out: &mut String, name: &str, kind: &str, help: &str) {
    out.push_str("# HELP ");
    out.push_str(name);
    out.push(' ');
    out.push_str(help);
    out.push('\n');
    out.push_str("# TYPE ");
    out.push_str(name);
    out.push(' ');
    out.push_str(kind);
    out.push('\n');
}

fn sample(out: &mut String, name: &str, labels: &[(&str, &str)], value: u64) {
    out.push_str(name);
    if !labels.is_empty() {
        out.push('{');
        for (index, (key, value)) in labels.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(key);
            out.push_str("=\"");
            out.push_str(&prom_escape(value));
            out.push('"');
        }
        out.push('}');
    }
    out.push(' ');
    out.push_str(&value.to_string());
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
PLAY [web] ***

TASK [Gathering Facts] ***
ok: [web-1]

TASK [web : install nginx] ***
ok: [web-1]
changed: [web-2]
fatal: [web-3]: FAILED! => {"password":"s3cret"}
skipping: [web-4]
unreachable: [web-5]

KOREOGRAF Role from cache: web

RUNNING HANDLER [web : restart nginx] ***
changed: [web-1]

TASK [namespace.collection.role : configure] ***
ok: [web-1]

PLAY RECAP ***
web-1 : ok=4 changed=2 unreachable=1 failed=1 skipped=1 rescued=0 ignored=0
ok: [web-1]
"#;

    fn meta<'a>(tool: &'a str, playbook: &'a str) -> JobMeta<'a> {
        JobMeta {
            job_id: "job-1",
            team_id: "team-1",
            tool,
            playbook,
            template: "Heal",
        }
    }

    #[test]
    fn ansible_role_results_stay_inside_the_playbook() {
        let metrics = Metrics::new();
        metrics.begin(&meta("ansible", "site.yml"));
        metrics.begin(&meta("ansible", "site.yml"));
        for line in FIXTURE.lines() {
            metrics.observe("job-1", line);
        }
        metrics.observe("job-1", "\u{1b}[0;31mTASK [web : colored] ***\u{1b}[0m");
        metrics.observe(
            "job-1",
            "\u{1b}[0;31mfatal: [web-9]: FAILED! => {\"password\":\"s3cret\"}\u{1b}[0m",
        );
        metrics.close("job-1", "failed");
        metrics.close("job-1", "failed");
        let text = metrics.render(&[]);
        assert!(
            text.contains("koreograf_jobs_started_total{tool=\"ansible\",playbook=\"site.yml\"} 1")
        );
        assert!(text.contains(
            "koreograf_jobs_finished_total{tool=\"ansible\",playbook=\"site.yml\",status=\"failed\"} 1"
        ));
        assert!(text.contains(
            "koreograf_role_results_total{playbook=\"site.yml\",role=\"web\",result=\"ok\"} 1"
        ));
        assert!(text.contains(
            "koreograf_role_results_total{playbook=\"site.yml\",role=\"web\",result=\"changed\"} 2"
        ));
        assert!(text.contains(
            "koreograf_role_results_total{playbook=\"site.yml\",role=\"web\",result=\"failed\"} 2"
        ));
        assert!(text.contains(
            "koreograf_role_results_total{playbook=\"site.yml\",role=\"web\",result=\"unreachable\"} 1"
        ));
        assert!(text.contains(
            "koreograf_role_results_total{playbook=\"site.yml\",role=\"web\",result=\"skipped\"} 1"
        ));
        assert!(text.contains(
            "koreograf_role_results_total{playbook=\"site.yml\",role=\"namespace.collection.role\",result=\"ok\"} 1"
        ));
        assert!(!text.contains("Gathering Facts"));
        assert!(!text.contains("s3cret"));
        assert!(!text.contains("Role from cache"));
    }

    fn watch(lines: &[&str]) -> bool {
        let metrics = Metrics::new();
        metrics.begin(&meta("ansible", "site.yml"));
        for line in lines {
            metrics.observe("job-1", line);
        }
        metrics.tag_filter_matched_nothing("job-1")
    }

    #[test]
    fn a_tag_filter_that_only_gathers_facts_matches_nothing() {
        assert!(watch(&[
            "PLAY [all] ***",
            "TASK [Gathering Facts] ***",
            "ok: [web-1]",
            "PLAY RECAP ***",
            "web-1 : ok=1 changed=0 unreachable=0 failed=0 skipped=0 rescued=0 ignored=0",
        ]));
        assert!(watch(&[
            "PLAY [all] ***",
            "\u{1b}[0;32mTASK [Gathering Facts] ***\u{1b}[0m",
            "ok: [web-1]",
            "PLAY RECAP ***",
            "web-1 : ok=0 changed=0 unreachable=0 failed=0 skipped=0 rescued=0 ignored=0",
        ]));
        assert!(watch(&[
            "PLAY [all] ***",
            "PLAY RECAP ***",
            "web-1 : ok=0 changed=0 unreachable=0 failed=0 skipped=0 rescued=0 ignored=0",
        ]));
    }

    #[test]
    fn a_matching_tag_is_not_an_empty_filter() {
        assert!(!watch(&[
            "PLAY [all] ***",
            "TASK [Gathering Facts] ***",
            "ok: [web-1]",
            "TASK [users : create] ***",
            "ok: [web-1]",
            "PLAY RECAP ***",
            "web-1 : ok=2 changed=0 unreachable=0 failed=0 skipped=0 rescued=0 ignored=0",
        ]));
        assert!(!watch(&[
            "PLAY [all] ***",
            "TASK [Gathering Facts] ***",
            "ok: [web-1]",
            "\u{1b}[0;32mTASK [users : create] ***\u{1b}[0m",
            "changed: [web-1]",
        ]));
        assert!(!watch(&[
            "PLAY [all] ***",
            "RUNNING HANDLER [users : restart] ***",
            "changed: [web-1]",
        ]));
        assert!(!watch(&[
            "PLAY [all] ***",
            "skipping: no hosts matched",
            "PLAY RECAP ***",
        ]));
        let metrics = Metrics::new();
        metrics.begin(&meta("shell", "health.sh"));
        metrics.observe("job-1", "PLAY [all] ***");
        metrics.observe("job-1", "ok: [web-1]");
        assert!(!metrics.tag_filter_matched_nothing("job-1"));
    }

    #[test]
    fn shell_output_is_not_a_role_result() {
        let metrics = Metrics::new();
        metrics.begin(&meta("shell", "health.sh"));
        metrics.observe("job-1", "TASK [web : install nginx] ***");
        metrics.observe(
            "job-1",
            "fatal: [web-1]: FAILED! => {\"password\":\"s3cret\"}",
        );
        metrics.close("job-1", "successful");
        let text = metrics.render(&[]);
        assert!(text.contains(
            "koreograf_jobs_finished_total{tool=\"shell\",playbook=\"health.sh\",status=\"successful\"} 1"
        ));
        assert!(!text.contains("koreograf_role_results_total{"));
        assert!(!text.contains("s3cret"));
    }

    #[test]
    fn labels_are_escaped_and_unknown_status_is_failed() {
        let metrics = Metrics::new();
        metrics.begin(&meta("ansible", "a\"b\\c\n.yml"));
        metrics.close("job-1", "running");
        let text = metrics.render(&[]);
        assert!(text.contains("playbook=\"a\\\"b\\\\c .yml\""), "{text}");
        assert!(text.contains("status=\"failed\""), "{text}");
    }

    #[test]
    fn cache_dir_counts_collections_and_roles() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-metrics-{nanos}"));
        let collection = dir.join("cache/t1/collections/ansible_collections/acme/web");
        std::fs::create_dir_all(&collection).unwrap();
        std::fs::write(collection.join("galaxy.yml"), "version: 1.2.3\n").unwrap();
        std::fs::create_dir_all(dir.join("cache/t1/roles/nginx")).unwrap();
        let mut names = HashMap::new();
        names.insert("t1".into(), "A \"Plattform\"".into());
        let gauges = gauges_from_dir(&dir, &names);
        assert_eq!(gauges.len(), 1);
        assert_eq!(gauges[0].collections, 1);
        assert_eq!(gauges[0].roles, 1);
        assert!(gauges[0].collections_bytes > 0);
        let text = Metrics::new().render(&gauges);
        assert!(text.contains(
            "koreograf_cache_collections{team_id=\"t1\",team=\"A \\\"Plattform\\\"\"} 1"
        ));
        assert!(
            text.contains("koreograf_cache_roles{team_id=\"t1\",team=\"A \\\"Plattform\\\"\"} 1")
        );
        assert!(text.contains("kind=\"collections\""));
        assert!(text.contains("kind=\"roles\""));
        assert!(text.contains("kind=\"tofu\""));
        let _ = std::fs::remove_dir_all(dir);
    }
}
