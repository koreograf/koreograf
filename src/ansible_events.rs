//! Structured Ansible events, captured next to the raw stdout log.
//!
//! The job writes [`FILE`] through the callback plugin in
//! [`PLUGIN`]; a small tailer reads the JSON lines while the job runs and
//! stores each event in `job_events` under [`STREAM`]. The raw `stdout`,
//! `stderr`, and `system` streams are untouched, and every bounded field and
//! the total volume are enforced again here before a row is written.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value};
use tokio::io::AsyncReadExt;

use crate::Shared;

/// The `job_events.stream` value of one structured Ansible event.
pub(crate) const STREAM: &str = "ansible";
/// Sidecar in the job work directory. The callback appends one JSON object
/// per line.
pub(crate) const FILE: &str = "ansible-events.jsonl";
/// Directory name for the callback plugin inside the job work directory.
pub(crate) const PLUGIN_DIR: &str = "callback_plugins";
/// Callback plugin file name. Ansible loads it by this name.
pub(crate) const PLUGIN_FILE: &str = "koreograf_events.py";
/// The plugin source that is written into every Ansible job work directory.
pub(crate) const PLUGIN: &str = include_str!("ansible_events.py");
/// One text field is bounded like a raw log line is.
pub(crate) const MAX_FIELD_CHARS: usize = 4_000;
/// The structured view keeps at most this many events per job.
pub(crate) const MAX_EVENTS: usize = 2_000;
/// The whole structured stream stays under this many bytes.
pub(crate) const MAX_BYTES: usize = 4 * 1024 * 1024;
/// The final recap chunks have their own budget, so a long run still ends with
/// a complete totals bar and problem-host list after [`MAX_EVENTS`].
pub(crate) const MAX_STATS_EVENTS: usize = 256;
const MAX_STATS_BYTES: usize = 3 * 1024 * 1024;
/// One stats chunk carries at most this many host rows.
const MAX_STATS_HOSTS: usize = 256;
/// A single JSON line above this is dropped instead of buffered forever.
const MAX_LINE_BYTES: usize = 64 * 1024;
/// One host detail response carries at most this many events.
pub(crate) const MAX_HOST_EVENTS: usize = 500;
/// One host detail response stays under this many bytes.
pub(crate) const MAX_HOST_BYTES: usize = 1024 * 1024;
/// A read for one host stops after this much of the sidecar, so a pruned or
/// oversized file cannot hold the request forever.
const MAX_HOST_SCAN_BYTES: usize = 64 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(200);

/// Why a host detail request has no events.
pub(crate) const UNAVAILABLE: &str = "ansible_events_unavailable";
/// The sidecar exists but has no result for the requested host.
pub(crate) const UNKNOWN_HOST: &str = "host_not_in_ansible_events";
/// The host was not found before the bounded scan stopped.
pub(crate) const SCAN_LIMITED: &str = "ansible_events_scan_limited";

/// The bounded sidecar slice for one host.
pub(crate) struct HostEvents {
    pub events: Vec<Value>,
    pub truncated: bool,
    pub reason: Option<&'static str>,
}

/// Reads `ansible-events.jsonl` and returns only the results of `host`, each
/// preceded by its task header once. Every line goes through the same
/// sanitisation as the live capture, so `no_log` output and the work paths
/// stay out.
pub(crate) fn host_events(work: &Path, data_dir: &Path, host: &str) -> HostEvents {
    let Ok(meta) = std::fs::symlink_metadata(work) else {
        return unavailable();
    };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return unavailable();
    }
    let path = work.join(FILE);
    let file = match open_sidecar(&path) {
        Ok(file) => file,
        Err(_) => return unavailable(),
    };
    read_host_events(file, scrub_paths(work, data_dir), host)
}

fn unavailable() -> HostEvents {
    HostEvents {
        events: Vec::new(),
        truncated: false,
        reason: Some(UNAVAILABLE),
    }
}

fn read_host_events(file: std::fs::File, scrub: Vec<String>, host: &str) -> HostEvents {
    read_host_events_with_limit(BufReader::new(file), scrub, host, MAX_HOST_SCAN_BYTES)
}

fn read_host_events_with_limit(
    mut reader: impl BufRead,
    scrub: Vec<String>,
    host: &str,
    scan_limit: usize,
) -> HostEvents {
    let mut line = Vec::new();
    let mut headers: HashMap<String, Value> = HashMap::new();
    let mut emitted: HashSet<String> = HashSet::new();
    let mut events: Vec<Value> = Vec::new();
    let mut bytes = 0usize;
    let mut scanned = 0usize;
    let mut matched = false;
    let mut truncated = false;
    let mut scan_limited = false;
    loop {
        line.clear();
        let read = match reader.read_until(b'\n', &mut line) {
            Ok(read) => read,
            Err(_) if matched => {
                truncated = true;
                break;
            }
            Err(_) => return unavailable(),
        };
        if read == 0 {
            break;
        }
        scanned = scanned.saturating_add(read);
        if scanned > scan_limit {
            truncated = true;
            scan_limited = true;
            break;
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        if line.len() > MAX_LINE_BYTES {
            continue;
        }
        let Ok(raw) = std::str::from_utf8(&line) else {
            continue;
        };
        let Some((event, stats)) = sanitize(raw, &scrub) else {
            continue;
        };
        if stats {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&event) else {
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("task_start") => {
                headers.insert(task_key(&value), value);
            }
            Some("host_result") => {
                if value.get("host").and_then(Value::as_str) != Some(host) {
                    continue;
                }
                matched = true;
                let key = task_key(&value);
                let mut items = Vec::with_capacity(2);
                if let Some(header) = headers.get(&key) {
                    if !emitted.contains(&key) {
                        items.push(header.clone());
                    }
                }
                items.push(value);
                let sizes = items
                    .iter()
                    .map(|item| {
                        serde_json::to_string(item)
                            .map(|text| text.len())
                            .unwrap_or(0)
                    })
                    .collect::<Vec<_>>();
                let added = sizes
                    .iter()
                    .fold(0usize, |sum, size| sum.saturating_add(*size));
                if events.len().saturating_add(items.len()) > MAX_HOST_EVENTS
                    || bytes.saturating_add(added) > MAX_HOST_BYTES
                {
                    truncated = true;
                    return HostEvents {
                        events,
                        truncated,
                        reason: None,
                    };
                }
                bytes = bytes.saturating_add(added);
                events.extend(items);
                emitted.insert(key);
            }
            _ => {}
        }
    }
    if !matched {
        return HostEvents {
            events: Vec::new(),
            truncated,
            reason: Some(if scan_limited {
                SCAN_LIMITED
            } else {
                UNKNOWN_HOST
            }),
        };
    }
    HostEvents {
        events,
        truncated,
        reason: None,
    }
}

fn task_key(event: &Value) -> String {
    let play = event.get("play_index").and_then(Value::as_i64);
    let index = event.get("index").and_then(Value::as_i64);
    match (play, index) {
        (Some(play), Some(index)) => format!("{play}:{index}"),
        _ => format!(
            "{}:{}",
            event.get("play").and_then(Value::as_str).unwrap_or(""),
            event.get("task").and_then(Value::as_str).unwrap_or("")
        ),
    }
}

const TEXT_FIELDS: &[&str] = &["play", "task", "role", "host", "msg", "stdout", "stderr"];
const NUMBER_FIELDS: &[&str] = &["index", "play_index", "task_index", "rc"];
/// The per-host counters Ansible reports in `PLAY RECAP`.
const STATS_FIELDS: &[&str] = &[
    "ok",
    "changed",
    "unreachable",
    "failed",
    "skipped",
    "rescued",
    "ignored",
];

/// Writes the callback plugin into `work`. Only Ansible jobs call this.
pub(crate) fn install(work: &Path) -> std::io::Result<()> {
    let dir = work.join(PLUGIN_DIR);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(PLUGIN_FILE), PLUGIN)
}

/// True when this tool has structured Ansible events. Shell and OpenTofu keep
/// the raw log only.
pub(crate) fn captures(tool: &str) -> bool {
    tool == "ansible"
}

pub(crate) struct Tailer {
    stop: Arc<AtomicBool>,
    handle: tokio::task::JoinHandle<()>,
}

impl Tailer {
    pub(crate) fn start(state: Shared, job_id: String, work: PathBuf, data_dir: PathBuf) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let handle = tokio::spawn(tail_loop(state, job_id, work, data_dir, stop.clone()));
        Self { stop, handle }
    }

    /// Stop after one last read. The job process has exited by now, so no
    /// writer can add more lines.
    pub(crate) async fn finish(self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.handle.await;
    }
}

async fn tail_loop(
    state: Shared,
    job_id: String,
    work: PathBuf,
    data_dir: PathBuf,
    stop: Arc<AtomicBool>,
) {
    let path = work.join(FILE);
    let mut capture = Capture::new(scrub_paths(&work, &data_dir));
    let mut file: Option<tokio::fs::File> = None;
    let mut pending: Vec<u8> = Vec::new();
    let mut dropping = false;
    loop {
        if file.is_none() {
            if let Ok(handle) = open_sidecar(&path) {
                file = Some(tokio::fs::File::from_std(handle));
            }
        }
        if let Some(handle) = file.as_mut() {
            let mut buf = [0_u8; 8 * 1024];
            loop {
                match handle.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(read) => {
                        pending.extend_from_slice(&buf[..read]);
                        while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
                            let mut line: Vec<u8> = pending.drain(..=end).collect();
                            line.pop();
                            if line.last() == Some(&b'\r') {
                                line.pop();
                            }
                            if dropping {
                                dropping = false;
                                continue;
                            }
                            if line.len() > MAX_LINE_BYTES {
                                continue;
                            }
                            if let Ok(text) = std::str::from_utf8(&line) {
                                store(&state, &job_id, capture.accept(text));
                            }
                        }
                        if !dropping && pending.len() > MAX_LINE_BYTES {
                            pending.clear();
                            dropping = true;
                        }
                    }
                    Err(_) => {
                        file = None;
                        break;
                    }
                }
            }
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(POLL).await;
    }
    if !pending.is_empty() && !dropping {
        if let Ok(text) = std::str::from_utf8(&pending) {
            store(&state, &job_id, capture.accept(text));
        }
    }
}

fn store(state: &Shared, job_id: &str, event: Option<String>) {
    if let Some(line) = event {
        let _ = state.store().insert_event(job_id, STREAM, &line);
    }
}

/// Opens the sidecar without following a symlink. The work directory is
/// writable by the job, so a job must not be able to point the tailer at a
/// host file.
#[cfg(unix)]
pub(crate) fn open_sidecar(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::fd::OwnedFd;

    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;

    let fd: OwnedFd = open(
        path,
        OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    Ok(std::fs::File::from(fd))
}

#[cfg(not(unix))]
pub(crate) fn open_sidecar(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

fn scrub_paths(work: &Path, data_dir: &Path) -> Vec<String> {
    let mut paths = vec![work.display().to_string(), data_dir.display().to_string()];
    paths.retain(|path| !path.is_empty());
    paths.sort_by_key(|path| std::cmp::Reverse(path.len()));
    paths.dedup();
    paths
}

/// Bounds and sanitises the sidecar stream before it reaches the database.
pub(crate) struct Capture {
    scrub: Vec<String>,
    count: usize,
    bytes: usize,
    limited: bool,
    stats_count: usize,
    stats_bytes: usize,
}

impl Capture {
    pub(crate) fn new(scrub: Vec<String>) -> Self {
        Self {
            scrub,
            count: 0,
            bytes: 0,
            limited: false,
            stats_count: 0,
            stats_bytes: 0,
        }
    }

    /// One accepted line for `job_events`, or `None` when the line is invalid
    /// or the budget is already spent.
    ///
    /// A `stats` chunk uses its own budget and is still accepted after the
    /// per-task stream reached [`MAX_EVENTS`], so the recap survives.
    pub(crate) fn accept(&mut self, raw: &str) -> Option<String> {
        let (event, stats) = sanitize(raw, &self.scrub)?;
        if stats {
            if self.stats_count >= MAX_STATS_EVENTS
                || self.stats_bytes.saturating_add(event.len()) > MAX_STATS_BYTES
            {
                return None;
            }
            self.stats_count += 1;
            self.stats_bytes = self.stats_bytes.saturating_add(event.len());
            return Some(event);
        }
        if self.limited {
            return None;
        }
        if self.count >= MAX_EVENTS || self.bytes.saturating_add(event.len()) > MAX_BYTES {
            self.limited = true;
            return Some(format!(
                "{{\"type\":\"limit\",\"events\":{MAX_EVENTS},\"bytes\":{MAX_BYTES}}}"
            ));
        }
        self.count += 1;
        self.bytes = self.bytes.saturating_add(event.len());
        Some(event)
    }
}

fn sanitize(raw: &str, scrub: &[String]) -> Option<(String, bool)> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let input = value.as_object()?;
    let kind = input.get("type")?.as_str()?;
    let mut out = Map::new();
    let stats = match kind {
        "play_start" => {
            let play = text_field(input, "play", scrub)?;
            out.insert("type".into(), Value::String(kind.into()));
            out.insert("play".into(), Value::String(play));
            copy_numbers(input, &mut out, &["index"]);
            false
        }
        "task_start" => {
            let task = text_field(input, "task", scrub)?;
            out.insert("type".into(), Value::String(kind.into()));
            out.insert("task".into(), Value::String(task));
            if let Some(play) = text_field(input, "play", scrub) {
                out.insert("play".into(), Value::String(play));
            }
            if let Some(role) = text_field(input, "role", scrub) {
                out.insert("role".into(), Value::String(role));
            }
            copy_numbers(input, &mut out, &["index", "play_index"]);
            if let Some(handler) = input.get("handler").and_then(Value::as_bool) {
                if handler {
                    out.insert("handler".into(), Value::Bool(true));
                }
            }
            false
        }
        "host_result" => {
            let host = text_field(input, "host", scrub)?;
            let status = input.get("status")?.as_str()?;
            if !matches!(
                status,
                "ok" | "changed" | "failed" | "skipped" | "unreachable"
            ) {
                return None;
            }
            out.insert("type".into(), Value::String(kind.into()));
            out.insert("host".into(), Value::String(host));
            out.insert("status".into(), Value::String(status.into()));
            for key in ["play", "task", "role"] {
                if let Some(text) = text_field(input, key, scrub) {
                    out.insert(key.into(), Value::String(text));
                }
            }
            copy_numbers(input, &mut out, &["index", "play_index", "rc"]);
            if let Some(duration) = input.get("duration").and_then(Value::as_f64) {
                if let Some(number) = serde_json::Number::from_f64(duration) {
                    if duration >= 0.0 {
                        out.insert("duration".into(), Value::Number(number));
                    }
                }
            }
            for key in ["changed", "ignored"] {
                if input.get(key).and_then(Value::as_bool) == Some(true) {
                    out.insert(key.into(), Value::Bool(true));
                }
            }
            let censored = input.get("censored").and_then(Value::as_bool) == Some(true)
                || input.get("_ansible_no_log").and_then(Value::as_bool) == Some(true);
            if censored {
                out.insert("censored".into(), Value::Bool(true));
            } else {
                for key in ["msg", "stdout", "stderr"] {
                    if let Some(text) = text_field(input, key, scrub) {
                        out.insert(key.into(), Value::String(text));
                    }
                }
                if let Some(text) = diff_field(input, scrub) {
                    out.insert("diff".into(), Value::String(text));
                }
            }
            false
        }
        "stats" => {
            let total = input
                .get("total")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .max(0);
            let part = input
                .get("part")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .max(0);
            let parts = input
                .get("parts")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .max(0);
            let hosts = input.get("hosts")?.as_array()?;
            if hosts.len() > MAX_STATS_HOSTS {
                return None;
            }
            let mut rows = Vec::with_capacity(hosts.len());
            for host in hosts {
                let row = host.as_object()?;
                let name = text_field(row, "host", scrub)?;
                let mut entry = Map::new();
                entry.insert("host".into(), Value::String(name));
                for key in STATS_FIELDS {
                    let count = row.get(*key).and_then(Value::as_i64).unwrap_or(0).max(0);
                    entry.insert((*key).into(), Value::Number(count.into()));
                }
                rows.push(Value::Object(entry));
            }
            out.insert("type".into(), Value::String(kind.into()));
            out.insert("total".into(), Value::Number(total.into()));
            out.insert("part".into(), Value::Number(part.into()));
            out.insert("parts".into(), Value::Number(parts.into()));
            out.insert("hosts".into(), Value::Array(rows));
            true
        }
        "playbook_end" => {
            out.insert("type".into(), Value::String(kind.into()));
            false
        }
        _ => return None,
    };
    let line = serde_json::to_string(&Value::Object(out)).ok()?;
    Some((line, stats))
}

fn text_field(input: &Map<String, Value>, key: &str, scrub: &[String]) -> Option<String> {
    if !TEXT_FIELDS.contains(&key) {
        return None;
    }
    let text = input.get(key)?.as_str()?;
    if text.is_empty() {
        return None;
    }
    Some(scrub_text(text, scrub))
}

fn diff_field(input: &Map<String, Value>, scrub: &[String]) -> Option<String> {
    let value = input.get("diff")?;
    let text = match value {
        Value::String(text) if !text.is_empty() => text.clone(),
        Value::String(_) | Value::Null => return None,
        other => serde_json::to_string(other).ok()?,
    };
    Some(scrub_text(&text, scrub))
}

fn scrub_text(text: &str, scrub: &[String]) -> String {
    let mut text = text.to_string();
    for path in scrub {
        if !path.is_empty() {
            text = text.replace(path.as_str(), "[work]");
        }
    }
    clip(&text)
}

fn copy_numbers(input: &Map<String, Value>, out: &mut Map<String, Value>, keys: &[&str]) {
    for key in keys {
        if !NUMBER_FIELDS.contains(key) {
            continue;
        }
        if let Some(number) = input.get(*key).and_then(Value::as_i64) {
            out.insert((*key).into(), Value::Number(number.into()));
        }
    }
}

fn clip(text: &str) -> String {
    if text.chars().count() <= MAX_FIELD_CHARS {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(MAX_FIELD_CHARS - 1).collect();
    clipped.push('…');
    clipped
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn a_scan_limit_before_a_match_keeps_its_own_reason() {
        let raw = r#"{"type":"host_result","host":"other","status":"ok","msg":"x"}"#;
        let page = read_host_events_with_limit(Cursor::new(raw), Vec::new(), "late", 8);
        assert_eq!(page.reason, Some(SCAN_LIMITED));
        assert!(page.truncated);
        assert!(page.events.is_empty());
    }

    #[test]
    fn a_complete_scan_without_the_host_reports_an_unknown_host() {
        let raw = r#"{"type":"host_result","host":"other","status":"ok","msg":"x"}"#;
        let page = read_host_events_with_limit(Cursor::new(raw), Vec::new(), "late", 4096);
        assert_eq!(page.reason, Some(UNKNOWN_HOST));
        assert!(!page.truncated);
        assert!(page.events.is_empty());
    }

    fn capture() -> Capture {
        Capture::new(vec!["/srv/koreograf/jobs/job-1".to_string()])
    }

    #[test]
    fn a_failed_host_result_keeps_status_duration_and_output() {
        let mut capture = capture();
        let line = capture
            .accept(
                r#"{"type":"host_result","play":"Site","task":"Deploy","role":"web","host":"web-1","status":"failed","duration":1.25,"msg":"boom","stdout":"out","stderr":"err","diff":[{"before":"a","after":"b"}],"rc":2}"#,
            )
            .expect("event");
        let value: Value = serde_json::from_str(&line).expect("json");
        assert_eq!(value["type"], "host_result");
        assert_eq!(value["status"], "failed");
        assert_eq!(value["duration"], 1.25);
        assert_eq!(value["msg"], "boom");
        assert_eq!(value["stdout"], "out");
        assert_eq!(value["stderr"], "err");
        assert!(value["diff"].as_str().unwrap_or("").contains("before"));
    }

    #[test]
    fn a_no_log_result_stores_only_a_censor_marker() {
        let mut capture = capture();
        let line = capture
            .accept(
                r#"{"type":"host_result","host":"db-1","status":"ok","msg":"secret=abc","stdout":"secret=abc","stderr":"secret=abc","diff":"secret","censored":true,"_ansible_no_log":true}"#,
            )
            .expect("event");
        assert!(!line.contains("secret"));
        let value: Value = serde_json::from_str(&line).expect("json");
        assert_eq!(value["censored"], true);
        assert!(value.get("msg").is_none());
        assert!(value.get("stdout").is_none());
        assert!(value.get("stderr").is_none());
        assert!(value.get("diff").is_none());
    }

    #[test]
    fn a_missing_no_log_flag_still_censors_the_result() {
        let mut capture = capture();
        let line = capture
            .accept(r#"{"type":"host_result","host":"db-1","status":"failed","msg":"password=hunter2","censored":true}"#)
            .expect("event");
        assert!(!line.contains("hunter2"));
        let value: Value = serde_json::from_str(&line).expect("json");
        assert_eq!(value["censored"], true);
    }

    #[test]
    fn an_oversized_field_is_truncated() {
        let mut capture = capture();
        let text = "x".repeat(MAX_FIELD_CHARS + 500);
        let line = capture
            .accept(&format!(
                r#"{{"type":"host_result","host":"web-1","status":"ok","msg":"{text}"}}"#
            ))
            .expect("event");
        let value: Value = serde_json::from_str(&line).expect("json");
        let msg = value["msg"].as_str().expect("msg");
        assert_eq!(msg.chars().count(), MAX_FIELD_CHARS);
        assert!(msg.ends_with('…'));
    }

    #[test]
    fn the_work_directory_is_replaced() {
        let mut capture = capture();
        let line = capture
            .accept(r#"{"type":"host_result","host":"web-1","status":"ok","msg":"written to /srv/koreograf/jobs/job-1/project/report"}"#)
            .expect("event");
        assert!(!line.contains("/srv/koreograf/jobs/job-1"));
        assert!(line.contains("[work]/project/report"));
    }

    #[test]
    fn unknown_types_and_fields_are_dropped() {
        let mut capture = capture();
        assert!(capture
            .accept(r#"{"type":"host_result","host":"web-1","status":"bogus"}"#)
            .is_none());
        assert!(capture
            .accept(r#"{"type":"surprise","host":"web-1"}"#)
            .is_none());
        assert!(capture.accept("not json").is_none());
        let line = capture
            .accept(
                r#"{"type":"task_start","task":"Ping","play":"Site","index":3,"extra_vars":{"secret":"x"}}"#,
            )
            .expect("event");
        assert!(!line.contains("extra_vars"));
        assert!(!line.contains("secret"));
    }

    #[test]
    fn the_event_budget_ends_with_one_limit_marker() {
        let mut capture = capture();
        for index in 0..MAX_EVENTS {
            assert!(capture
                .accept(&format!(
                    r#"{{"type":"task_start","task":"Task {index}","index":{index}}}"#
                ))
                .is_some());
        }
        let marker = capture.accept(r#"{"type":"task_start","task":"late"}"#);
        assert!(marker
            .as_deref()
            .is_some_and(|line| line.contains("\"type\":\"limit\"")));
        assert!(capture
            .accept(r#"{"type":"task_start","task":"later"}"#)
            .is_none());
    }

    #[test]
    fn a_stats_chunk_keeps_the_host_counters_and_is_bounded() {
        let mut capture = capture();
        let line = capture
            .accept(
                r#"{"type":"stats","total":2,"part":1,"parts":1,"hosts":[{"host":"web-1","ok":3,"changed":1,"unreachable":0,"failed":0,"skipped":1,"rescued":0,"ignored":0,"extra":"x"},{"host":"web-2","ok":1,"changed":0,"unreachable":1,"failed":1,"skipped":0,"rescued":0,"ignored":0}]}"#,
            )
            .expect("stats");
        let value: Value = serde_json::from_str(&line).expect("json");
        assert_eq!(value["type"], "stats");
        assert_eq!(value["total"], 2);
        assert_eq!(value["parts"], 1);
        assert_eq!(value["hosts"][0]["changed"], 1);
        assert_eq!(value["hosts"][1]["unreachable"], 1);
        assert!(value["hosts"][0].get("extra").is_none());
        let mut too_many = vec![serde_json::json!({"host": "h"})];
        too_many.extend(
            (0..MAX_STATS_HOSTS).map(|index| serde_json::json!({"host": format!("h{index}")})),
        );
        let raw = serde_json::json!({
            "type": "stats",
            "total": too_many.len(),
            "hosts": too_many
        })
        .to_string();
        assert!(capture.accept(&raw).is_none());
    }

    #[test]
    fn stats_chunks_are_kept_after_the_per_task_budget() {
        let mut capture = capture();
        for index in 0..MAX_EVENTS {
            assert!(capture
                .accept(&format!(
                    r#"{{"type":"task_start","task":"Task {index}","index":{index}}}"#
                ))
                .is_some());
        }
        let marker = capture.accept(r#"{"type":"task_start","task":"late"}"#);
        assert!(marker
            .as_deref()
            .is_some_and(|line| line.contains("\"type\":\"limit\"")));
        assert!(capture
            .accept(r#"{"type":"task_start","task":"later"}"#)
            .is_none());
        let stats = capture
            .accept(
                r#"{"type":"stats","total":5000,"part":19,"parts":19,"hosts":[{"host":"bulk-4999","ok":1,"failed":1,"changed":0,"unreachable":0,"skipped":0,"rescued":0,"ignored":0}]}"#,
            )
            .expect("stats after the limit");
        let value: Value = serde_json::from_str(&stats).expect("json");
        assert_eq!(value["type"], "stats");
        assert_eq!(value["total"], 5000);
        assert_eq!(value["hosts"][0]["host"], "bulk-4999");
    }

    #[test]
    fn only_ansible_captures_structured_events() {
        assert!(captures("ansible"));
        assert!(!captures("shell"));
        assert!(!captures("opentofu"));
    }

    #[test]
    fn the_plugin_emits_a_stats_chunk_before_the_end() {
        assert!(PLUGIN.contains("v2_playbook_on_stats"));
        assert!(PLUGIN.contains("\"type\": \"stats\""));
        assert!(PLUGIN.contains("\"type\": \"playbook_end\""));
    }
}
