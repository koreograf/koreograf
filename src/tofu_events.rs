//! Structured OpenTofu events, parsed from the machine-readable UI on stdout.
//!
//! `tofu plan` and `tofu apply` run with `-json`, so every UI line is one JSON
//! object. The runner turns each accepted line into a small structured event in
//! `job_events` under [`STREAM`], and derives a human line for the raw `stdout`
//! stream so the log, the download, search, wrap, and follow stay readable.
//! `tofu init` stays human-readable and bypasses this parser.

use std::path::Path;

use serde_json::{Map, Value};

/// The `job_events.stream` value of one structured OpenTofu event.
pub(crate) const STREAM: &str = "tofu";
/// One text field is bounded like a raw log line is.
pub(crate) const MAX_FIELD_CHARS: usize = 4_000;
/// The structured view keeps at most this many events per job.
pub(crate) const MAX_EVENTS: usize = 2_000;
/// The whole structured stream stays under this many bytes.
pub(crate) const MAX_BYTES: usize = 4 * 1024 * 1024;
/// Late `change_summary` events keep their own small budget, so a very large
/// apply still ends with a complete add / change / destroy summary.
pub(crate) const MAX_SUMMARY_EVENTS: usize = 16;
const MAX_SUMMARY_BYTES: usize = 64 * 1024;
/// A single JSON line above this is dropped instead of parsed.
pub(crate) const MAX_LINE_BYTES: usize = 64 * 1024;

/// True when this tool runs `tofu plan` or `tofu apply` with `-json`.
pub(crate) fn captures(tool: &str) -> bool {
    tool == "opentofu"
}

/// The work and data directories a structured event must not reveal.
pub(crate) fn scrub_paths(work: &Path, data_dir: &Path) -> Vec<String> {
    let mut paths = vec![work.display().to_string(), data_dir.display().to_string()];
    paths.retain(|path| !path.is_empty());
    paths.sort_by_key(|path| std::cmp::Reverse(path.len()));
    paths.dedup();
    paths
}

/// One accepted JSON UI line: the bounded structured event and the human
/// lines the raw log should show instead of the JSON object.
pub(crate) struct Parsed {
    pub structured: Vec<String>,
    pub human: Vec<String>,
}

/// Bounds and sanitises the JSON UI stream before it reaches the database.
pub(crate) struct Capture {
    scrub: Vec<String>,
    count: usize,
    bytes: usize,
    summary_count: usize,
    summary_bytes: usize,
    limited: bool,
}

impl Capture {
    pub(crate) fn new(scrub: Vec<String>) -> Self {
        Self {
            scrub,
            count: 0,
            bytes: 0,
            summary_count: 0,
            summary_bytes: 0,
            limited: false,
        }
    }

    /// One accepted line, or `None` when it is not a bounded JSON UI object.
    /// A recognised line always keeps its human text, even after the
    /// structured budget is spent, so the raw log stays complete.
    pub(crate) fn accept(&mut self, raw: &str) -> Option<Parsed> {
        if raw.len() > MAX_LINE_BYTES {
            return None;
        }
        let value: Value = serde_json::from_str(raw).ok()?;
        let input = value.as_object()?;
        let kind = input.get("type")?.as_str()?;
        let message = text_field(input, "@message", &self.scrub);
        let mut out = Map::new();
        out.insert("type".into(), Value::String(kind.into()));
        let mut human = Vec::new();
        match kind {
            "planned_change" => {
                let change = input.get("change")?.as_object()?;
                let action = action_field(change)?;
                let resource = change.get("resource")?.as_object()?;
                let addr = text_field(resource, "addr", &self.scrub)?;
                out.insert("action".into(), Value::String(action.into()));
                out.insert("addr".into(), Value::String(addr.clone()));
                push_human(
                    &mut human,
                    message.unwrap_or_else(|| format!("Plan: {action} {addr}")),
                );
            }
            "change_summary" => {
                let changes = input.get("changes")?.as_object()?;
                let operation = match changes.get("operation").and_then(Value::as_str) {
                    Some("apply") => "apply",
                    _ => "plan",
                };
                let add = count_field(changes, "add");
                let change = count_field(changes, "change");
                let remove = count_field(changes, "remove");
                let import = count_field(changes, "import");
                out.insert("operation".into(), Value::String(operation.into()));
                out.insert("add".into(), Value::Number(add.into()));
                out.insert("change".into(), Value::Number(change.into()));
                out.insert("remove".into(), Value::Number(remove.into()));
                out.insert("import".into(), Value::Number(import.into()));
                push_human(
                    &mut human,
                    message.unwrap_or_else(|| {
                        format!(
                            "{}: {add} to add, {change} to change, {remove} to destroy.",
                            title_case(operation)
                        )
                    }),
                );
            }
            "apply_start" | "apply_progress" | "apply_complete" | "apply_errored" => {
                let hook = input.get("hook")?.as_object()?;
                let action = action_field(hook)?;
                let resource = hook.get("resource")?.as_object()?;
                let addr = text_field(resource, "addr", &self.scrub)?;
                out.insert("action".into(), Value::String(action.into()));
                out.insert("addr".into(), Value::String(addr.clone()));
                if let Some(elapsed) = input.get("elapsed_seconds").and_then(Value::as_f64) {
                    if let Some(number) = serde_json::Number::from_f64(elapsed) {
                        if elapsed >= 0.0 {
                            out.insert("elapsed_seconds".into(), Value::Number(number));
                        }
                    }
                }
                let fallback = match kind {
                    "apply_start" => format!("Apply: {action} {addr}"),
                    "apply_progress" => format!("Applying: {action} {addr}"),
                    "apply_complete" => format!("Applied: {action} {addr}"),
                    _ => format!("Apply failed: {action} {addr}"),
                };
                push_human(&mut human, message.unwrap_or(fallback));
            }
            "diagnostic" => {
                let diagnostic = input.get("diagnostic")?.as_object()?;
                let severity = match diagnostic.get("severity").and_then(Value::as_str) {
                    Some("warning") => "warning",
                    _ => "error",
                };
                let summary = text_field(diagnostic, "summary", &self.scrub)?;
                out.insert("severity".into(), Value::String(severity.into()));
                out.insert("summary".into(), Value::String(summary.clone()));
                if let Some(detail) = text_field(diagnostic, "detail", &self.scrub) {
                    out.insert("detail".into(), Value::String(detail));
                }
                push_human(
                    &mut human,
                    message.unwrap_or_else(|| format!("{}: {summary}", title_case(severity))),
                );
            }
            "outputs" => {
                let outputs = input.get("outputs")?.as_object()?;
                let mut clean = Map::new();
                for (name, output) in outputs {
                    let Some(output) = output.as_object() else {
                        continue;
                    };
                    let name = single_line(name);
                    if name.is_empty() {
                        continue;
                    }
                    let sensitive = output.get("sensitive").and_then(Value::as_bool) == Some(true);
                    let mut entry = Map::new();
                    entry.insert("sensitive".into(), Value::Bool(sensitive));
                    if sensitive {
                        push_human(&mut human, format!("Output {name}: (sensitive value)"));
                    } else {
                        let value = output
                            .get("value")
                            .map(|value| scrub_text(&value_text(value), &self.scrub))
                            .unwrap_or_default();
                        let value = single_line(&value);
                        entry.insert("value".into(), Value::String(value.clone()));
                        push_human(&mut human, format!("Output {name}: {value}"));
                    }
                    clean.insert(name, Value::Object(entry));
                }
                out.insert("outputs".into(), Value::Object(clean));
            }
            _ => {
                // A UI event without a structured model still has useful
                // human text. Keeping only `@message` keeps the log free of
                // raw JSON objects.
                if let Some(message) = message {
                    push_human(&mut human, message);
                }
                return Some(Parsed {
                    structured: Vec::new(),
                    human,
                });
            }
        }
        let line = serde_json::to_string(&Value::Object(out)).ok()?;
        Some(Parsed {
            structured: self.reserve(line, kind),
            human,
        })
    }

    /// Stores one bounded event or one final limit marker. A late
    /// `change_summary` uses its own budget so the summary survives the cap,
    /// including the summary that itself crosses the main budget.
    fn reserve(&mut self, line: String, kind: &str) -> Vec<String> {
        if self.limited {
            return self.reserve_summary(line, kind).into_iter().collect();
        }
        if self.count >= MAX_EVENTS || self.bytes.saturating_add(line.len()) > MAX_BYTES {
            self.limited = true;
            let mut events = vec![format!(
                "{{\"type\":\"limit\",\"events\":{MAX_EVENTS},\"bytes\":{MAX_BYTES}}}"
            )];
            events.extend(self.reserve_summary(line, kind));
            return events;
        }
        self.count += 1;
        self.bytes = self.bytes.saturating_add(line.len());
        vec![line]
    }

    fn reserve_summary(&mut self, line: String, kind: &str) -> Option<String> {
        if kind != "change_summary" {
            return None;
        }
        if self.summary_count >= MAX_SUMMARY_EVENTS
            || self.summary_bytes.saturating_add(line.len()) > MAX_SUMMARY_BYTES
        {
            return None;
        }
        self.summary_count += 1;
        self.summary_bytes = self.summary_bytes.saturating_add(line.len());
        Some(line)
    }
}

fn action_field(input: &Map<String, Value>) -> Option<&'static str> {
    match input.get("action").and_then(Value::as_str)? {
        "create" => Some("create"),
        "update" => Some("update"),
        "delete" => Some("delete"),
        "replace" => Some("replace"),
        "read" => Some("read"),
        "no-op" => Some("no-op"),
        _ => None,
    }
}

fn count_field(input: &Map<String, Value>, key: &str) -> i64 {
    input.get(key).and_then(Value::as_i64).unwrap_or(0).max(0)
}

fn text_field(input: &Map<String, Value>, key: &str, scrub: &[String]) -> Option<String> {
    let text = input.get(key)?.as_str()?;
    if text.is_empty() {
        return None;
    }
    Some(scrub_text(text, scrub))
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
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

fn clip(text: &str) -> String {
    if text.chars().count() <= MAX_FIELD_CHARS {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(MAX_FIELD_CHARS - 1).collect();
    clipped.push('…');
    clipped
}

fn single_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn push_human(human: &mut Vec<String>, text: String) {
    for line in text.lines() {
        let line = line.trim_end_matches('\r').trim();
        if !line.is_empty() {
            human.push(clip(line));
        }
    }
}

fn title_case(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture() -> Capture {
        Capture::new(vec!["/srv/koreograf/jobs/job-1".to_string()])
    }

    fn event(line: &str) -> Value {
        serde_json::from_str(line).expect("json")
    }

    #[test]
    fn a_planned_change_keeps_action_and_address() {
        let mut capture = capture();
        let parsed = capture
            .accept(
                r#"{"@level":"info","@message":"terraform_data.web: Plan to create","type":"planned_change","change":{"resource":{"addr":"terraform_data.web","resource_type":"terraform_data","resource_name":"web"},"action":"create"}}"#,
            )
            .expect("planned change");
        let value = event(
            parsed
                .structured
                .first()
                .map(String::as_str)
                .expect("structured"),
        );
        assert_eq!(value["type"], "planned_change");
        assert_eq!(value["action"], "create");
        assert_eq!(value["addr"], "terraform_data.web");
        assert!(!value.to_string().contains("resource_type"));
        assert_eq!(
            parsed.human,
            vec!["terraform_data.web: Plan to create".to_string()]
        );
    }

    #[test]
    fn a_change_summary_keeps_the_counts() {
        let mut capture = capture();
        let parsed = capture
            .accept(
                r#"{"type":"change_summary","changes":{"add":2,"change":1,"remove":3,"import":0,"operation":"plan"}}"#,
            )
            .expect("summary");
        let value = event(
            parsed
                .structured
                .first()
                .map(String::as_str)
                .expect("structured"),
        );
        assert_eq!(value["operation"], "plan");
        assert_eq!(value["add"], 2);
        assert_eq!(value["change"], 1);
        assert_eq!(value["remove"], 3);
        assert!(parsed.human[0].contains("2 to add"));
    }

    #[test]
    fn an_apply_complete_keeps_progress_and_action() {
        let mut capture = capture();
        let parsed = capture
            .accept(
                r#"{"type":"apply_complete","hook":{"resource":{"addr":"terraform_data.web"},"action":"update"},"elapsed_seconds":1.5}"#,
            )
            .expect("apply complete");
        let value = event(
            parsed
                .structured
                .first()
                .map(String::as_str)
                .expect("structured"),
        );
        assert_eq!(value["type"], "apply_complete");
        assert_eq!(value["elapsed_seconds"], 1.5);
        assert_eq!(value["addr"], "terraform_data.web");
        assert!(parsed.human[0].contains("Applied: update"));
    }

    #[test]
    fn a_diagnostic_keeps_its_severity_and_detail() {
        let mut capture = capture();
        let parsed = capture
            .accept(
                r#"{"type":"diagnostic","diagnostic":{"severity":"warning","summary":"Deprecated attribute","detail":"Use id instead."}}"#,
            )
            .expect("diagnostic");
        let value = event(
            parsed
                .structured
                .first()
                .map(String::as_str)
                .expect("structured"),
        );
        assert_eq!(value["severity"], "warning");
        assert_eq!(value["summary"], "Deprecated attribute");
        assert_eq!(value["detail"], "Use id instead.");
    }

    #[test]
    fn a_sensitive_output_is_hidden_from_both_streams() {
        let mut capture = capture();
        let parsed = capture
            .accept(
                r#"{"type":"outputs","@message":"Outputs:\n\nmessage = \"hello\"\nsecret = \"hunter2\"\n","outputs":{"message":{"sensitive":false,"type":"string","value":"hello"},"secret":{"sensitive":true,"type":"string","value":"hunter2"}}}"#,
            )
            .expect("outputs");
        let line = parsed
            .structured
            .first()
            .map(String::as_str)
            .expect("structured");
        assert!(!line.contains("hunter2"));
        let value = event(line);
        assert_eq!(value["outputs"]["secret"]["sensitive"], true);
        assert!(value["outputs"]["secret"].get("value").is_none());
        assert_eq!(value["outputs"]["message"]["value"], "hello");
        let human = parsed.human.join("\n");
        assert!(!human.contains("hunter2"));
        assert!(human.contains("Output secret: (sensitive value)"));
        assert!(human.contains("Output message: hello"));
    }

    #[test]
    fn only_allowlisted_fields_survive() {
        let mut capture = capture();
        let parsed = capture
            .accept(
                r#"{"type":"diagnostic","diagnostic":{"severity":"error","summary":"boom","detail":"detail","token":"hunter2","extra":{"secret":"hunter2"}},"extra":{"secret":"hunter2"}}"#,
            )
            .expect("diagnostic");
        let line = parsed
            .structured
            .first()
            .map(String::as_str)
            .expect("structured");
        assert!(!line.contains("hunter2"));
        assert!(!line.contains("extra"));
        assert!(!line.contains("token"));
        assert!(!parsed.human.iter().any(|text| text.contains("hunter2")));
    }

    #[test]
    fn an_oversized_field_is_truncated() {
        let mut capture = capture();
        let text = "x".repeat(MAX_FIELD_CHARS + 500);
        let parsed = capture
            .accept(&format!(
                r#"{{"type":"diagnostic","diagnostic":{{"severity":"error","summary":"{text}","detail":"{text}"}}}}"#
            ))
            .expect("diagnostic");
        let value = event(
            parsed
                .structured
                .first()
                .map(String::as_str)
                .expect("structured"),
        );
        let summary = value["summary"].as_str().expect("summary");
        assert_eq!(summary.chars().count(), MAX_FIELD_CHARS);
        assert!(summary.ends_with('…'));
        assert!(parsed.human[0].chars().count() <= MAX_FIELD_CHARS);
    }

    #[test]
    fn the_work_directory_is_replaced() {
        let mut capture = capture();
        let parsed = capture
            .accept(
                r#"{"type":"diagnostic","diagnostic":{"severity":"error","summary":"failed in /srv/koreograf/jobs/job-1/project"}}"#,
            )
            .expect("diagnostic");
        let line = parsed
            .structured
            .first()
            .map(String::as_str)
            .expect("structured");
        assert!(!line.contains("/srv/koreograf/jobs/job-1"));
        assert!(line.contains("[work]/project"));
        assert!(!parsed.human[0].contains("/srv/koreograf/jobs/job-1"));
        assert!(parsed.human[0].contains("[work]/project"));
    }

    #[test]
    fn unknown_types_keep_only_the_human_message() {
        let mut capture = capture();
        let parsed = capture
            .accept(r#"{"type":"version","@message":"OpenTofu v1.8.0","terraform":"1.8.0"}"#)
            .expect("version");
        assert!(parsed.structured.is_empty());
        assert_eq!(parsed.human, vec!["OpenTofu v1.8.0".to_string()]);
        assert!(capture.accept("not json").is_none());
        assert!(capture
            .accept(
                r#"{"type":"planned_change","change":{"resource":{"addr":"x"},"action":"explode"}}"#
            )
            .is_none());
    }

    #[test]
    fn the_event_budget_ends_with_one_limit_marker() {
        let mut capture = capture();
        for index in 0..MAX_EVENTS {
            let parsed = capture
                .accept(&format!(
                    r#"{{"type":"planned_change","change":{{"resource":{{"addr":"terraform_data.r{index}"}},"action":"create"}}}}"#
                ))
                .expect("event");
            assert!(!parsed.human.is_empty());
        }
        let marker = capture.accept(
            r#"{"type":"planned_change","change":{"resource":{"addr":"terraform_data.late"},"action":"create"}}"#,
        );
        let marker = marker.expect("marker");
        assert!(marker
            .structured
            .iter()
            .any(|line| line.contains("\"type\":\"limit\"")));
        let later = capture.accept(
            r#"{"type":"planned_change","change":{"resource":{"addr":"terraform_data.later"},"action":"create"}}"#,
        );
        assert!(later.expect("later").structured.is_empty());
        let summary = capture
            .accept(
                r#"{"type":"change_summary","changes":{"add":7,"change":0,"remove":0,"operation":"apply"}}"#,
            )
            .expect("summary after limit");
        assert!(summary
            .structured
            .iter()
            .any(|line| line.contains("\"add\":7")));
        for _ in 0..MAX_SUMMARY_EVENTS {
            let _ = capture.accept(
                r#"{"type":"change_summary","changes":{"add":1,"change":0,"remove":0,"operation":"apply"}}"#,
            );
        }
        assert!(capture
            .accept(
                r#"{"type":"change_summary","changes":{"add":1,"change":0,"remove":0,"operation":"apply"}}"#,
            )
            .expect("summary")
            .structured
            .is_empty());
    }

    #[test]
    fn the_summary_that_crosses_the_budget_keeps_the_marker_and_itself() {
        let mut capture = capture();
        for index in 0..MAX_EVENTS {
            assert!(capture
                .accept(&format!(
                    r#"{{"type":"planned_change","change":{{"resource":{{"addr":"terraform_data.r{index}"}},"action":"create"}}}}"#
                ))
                .is_some());
        }
        let crossing = capture
            .accept(
                r#"{"type":"change_summary","changes":{"add":3,"change":0,"remove":0,"operation":"apply"}}"#,
            )
            .expect("crossing summary");
        assert_eq!(crossing.structured.len(), 2);
        assert!(crossing.structured[0].contains("\"type\":\"limit\""));
        assert!(crossing.structured[1].contains("\"add\":3"));
        let later = capture
            .accept(
                r#"{"type":"change_summary","changes":{"add":4,"change":0,"remove":0,"operation":"apply"}}"#,
            )
            .expect("later summary");
        assert_eq!(later.structured.len(), 1);
        assert!(later.structured[0].contains("\"add\":4"));
    }

    #[test]
    fn only_opentofu_captures_structured_events() {
        assert!(captures("opentofu"));
        assert!(!captures("ansible"));
        assert!(!captures("shell"));
    }
}
