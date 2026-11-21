//! Structured Ansible events, captured next to the raw stdout log.
//!
//! The job writes [`FILE`] through the callback plugin in
//! [`PLUGIN`]; a small tailer reads the JSON lines while the job runs and
//! stores each event in `job_events` under [`STREAM`]. The raw `stdout`,
//! `stderr`, and `system` streams are untouched, and every bounded field and
//! the total volume are enforced again here before a row is written.

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
const POLL: Duration = Duration::from_millis(200);

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
fn open_sidecar(path: &Path) -> std::io::Result<std::fs::File> {
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
fn open_sidecar(path: &Path) -> std::io::Result<std::fs::File> {
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
    use super::*;

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
