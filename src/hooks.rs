use serde_json::{json, Map, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertEvent {
    pub alertname: String,
    pub status: String,
    pub instance: String,
    pub summary: String,
    pub labels: Vec<(String, String)>,
}

pub fn parse_webhook(body: &Value) -> Vec<AlertEvent> {
    if let Some(alerts) = body.get("alerts").and_then(Value::as_array) {
        let top = text(body, "status");
        return alerts
            .iter()
            .filter_map(|alert| alert_from_item(alert, &top))
            .collect();
    }
    if let Some(state) = body.get("state").and_then(Value::as_str) {
        let name = text(body, "ruleName");
        let name = if name.is_empty() {
            text(body, "title")
        } else {
            name
        };
        if name.is_empty() {
            return Vec::new();
        }
        let status = normalize_status(state);
        return vec![AlertEvent {
            alertname: name,
            status,
            instance: text(body, "instance"),
            summary: text(body, "message"),
            labels: Vec::new(),
        }];
    }
    let name = text(body, "alertname");
    if name.is_empty() {
        return Vec::new();
    }
    vec![AlertEvent {
        alertname: name,
        status: normalize_status(&text(body, "status")),
        instance: text(body, "instance"),
        summary: text(body, "summary"),
        labels: Vec::new(),
    }]
}

fn alert_from_item(alert: &Value, top_status: &str) -> Option<AlertEvent> {
    let labels = string_map(alert.get("labels"));
    let annotations = string_map(alert.get("annotations"));
    let alertname = labels
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("alertname"))
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    if alertname.is_empty() {
        return None;
    }
    let own = text(alert, "status");
    let status = normalize_status(if own.is_empty() { top_status } else { &own });
    let instance = labels
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("instance"))
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let summary = annotations
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("summary") || k.eq_ignore_ascii_case("description"))
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    Some(AlertEvent {
        alertname,
        status,
        instance,
        summary,
        labels,
    })
}

fn string_map(value: Option<&Value>) -> Vec<(String, String)> {
    let Some(Value::Object(map)) = value else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .collect()
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

pub fn normalize_status(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "firing" | "alerting" | "alert" | "critical" => "firing".into(),
        "resolved" | "ok" | "normal" => "resolved".into(),
        other => other.to_string(),
    }
}

pub fn matches(alertname: &str, status: &str, event: &AlertEvent) -> bool {
    if !alertname.is_empty() && !alertname.eq_ignore_ascii_case(&event.alertname) {
        return false;
    }
    if status.is_empty() || status.eq_ignore_ascii_case("any") {
        return true;
    }
    status.eq_ignore_ascii_case(&event.status)
}

pub fn event_vars(event: &AlertEvent) -> Value {
    let mut labels = Map::new();
    for (key, value) in &event.labels {
        labels.insert(key.clone(), Value::String(value.clone()));
    }
    let instance = if event.instance.is_empty() {
        "localhost".to_string()
    } else {
        event.instance.clone()
    };
    json!({
        "alertname": event.alertname,
        "status": event.status,
        "instance": instance,
        "summary": event.summary,
        "event": Value::Object(labels),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_alertmanager_and_matches() {
        let body = json!({
            "status": "firing",
            "alerts": [{
                "status": "firing",
                "labels": {"alertname": "DiskSpaceLow", "instance": "web-1"},
                "annotations": {"summary": "voll"}
            }]
        });
        let events = parse_webhook(&body);
        assert_eq!(events.len(), 1);
        assert!(matches("DiskSpaceLow", "firing", &events[0]));
        assert!(!matches("DiskSpaceLow", "resolved", &events[0]));
        assert!(!matches("Other", "firing", &events[0]));
        let vars = event_vars(&events[0]);
        assert_eq!(vars["instance"], "web-1");
    }

    #[test]
    fn parses_grafana_legacy_state() {
        let body = json!({"state": "alerting", "ruleName": "DiskSpaceLow", "message": "voll"});
        let events = parse_webhook(&body);
        assert_eq!(events[0].status, "firing");
        assert!(matches("diskspacelow", "firing", &events[0]));
    }
}
