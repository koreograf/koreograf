#![allow(clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::http::StatusCode;
use koreograf::LocalJobs;
use serde_json::{json, Value};

use super::{call_cookie as call, call_raw_headers as call_raw, config, Options, TestApp};

pub async fn launch_demo_workflow() -> (StatusCode, Value, StatusCode) {
    let harness = TestApp::boot("wf", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();
    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();
    let (status, workflows, _) = call(
        &app,
        "GET",
        "/api/workflows",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{workflows}");
    let workflow_id = workflows
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Self-Heal Ablauf")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, body, _) = call(
        &app,
        "POST",
        &format!("/api/workflows/{workflow_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    let (health, _, _) = call(&app, "GET", "/healthz", None, None, None).await;
    let _ = std::fs::remove_dir_all(&dir);
    (status, body, health)
}

pub async fn launch_options_admin_app(dir: &Path) -> (axum::Router, String, String, String) {
    let mut cfg = config(dir, &Options::default());
    cfg.local_jobs = LocalJobs::Admins;
    let state = koreograf::boot(cfg).unwrap();
    let app = koreograf::router(state);

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().unwrap().to_string();

    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Gated hosts", "body": "[web]\nweb-1\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap().to_string();

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Gated",
            "tool": "ansible",
            "entrypoint": "site.yml",
            "inventory_id": inventory_id,
            "spec": {"job_type": "run", "tags": "ping", "limit": "web"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();
    (app, cookie, team, template_id)
}

pub fn git(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("git ist nicht installiert");
    assert!(status.success(), "git {args:?} in {}", dir.display());
}

pub fn project_names(list: &Value) -> Vec<String> {
    list.as_array()
        .expect("Projektliste")
        .iter()
        .map(|item| item["name"].as_str().unwrap().to_string())
        .collect()
}

pub async fn netbox_mock_recording() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = seen.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route(
        "/api/dcim/devices/",
        axum::routing::get(move |headers: axum::http::HeaderMap| {
            let seen = recorded.clone();
            async move {
                let token = headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                seen.lock().unwrap().push(token);
                axum::Json(json!({
                    "next": null,
                    "results": [{
                        "id": 1,
                        "name": "web-1",
                        "primary_ip4": {"address": "10.0.0.8/24"},
                        "site": {"slug": "berlin"}
                    }]
                }))
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });
    (format!("http://{addr}"), seen)
}

pub async fn netbox_mock() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = hits.clone();
    let status_count = hits.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new()
        .route(
            "/api/dcim/devices/",
            axum::routing::get(move || {
                let count = count.clone();
                async move {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    axum::Json(json!({
                        "next": null,
                        "results": [{
                            "id": 1,
                            "name": "web-1",
                            "primary_ip4": {"address": "10.0.0.8/24"},
                            "site": {"slug": "berlin"}
                        }]
                    }))
                }
            }),
        )
        .route(
            "/api/status/",
            axum::routing::get(move || {
                let count = status_count.clone();
                async move {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    axum::Json(json!({"netbox-version": "4.2.1", "plugins": {}}))
                }
            }),
        );
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });
    (format!("http://{addr}"), hits)
}

pub async fn nb_editable(
    headers: axum::http::HeaderMap,
    request: axum::extract::Request,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some("Token secret-token")
    {
        return (StatusCode::FORBIDDEN, "no").into_response();
    }
    let query = request.uri().query().unwrap_or("");
    let results = if query.contains("status=planned") {
        json!([
            {
                "id": 1,
                "name": "web-1",
                "primary_ip4": {"address": "10.0.0.8/24"},
                "site": {"slug": "berlin"}
            },
            {
                "id": 2,
                "name": "web-2",
                "primary_ip4": {"address": "10.0.0.9/24"},
                "site": {"slug": "munich"}
            }
        ])
    } else {
        json!([{
            "id": 1,
            "name": "web-1",
            "primary_ip4": {"address": "10.0.0.8/24"},
            "site": {"slug": "berlin"}
        }])
    };
    axum::Json(json!({"next": null, "results": results})).into_response()
}

pub async fn nb_devices(
    headers: axum::http::HeaderMap,
    request: axum::extract::Request,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some("Token secret-token")
    {
        return (StatusCode::FORBIDDEN, "no").into_response();
    }
    let query = request.uri().query().unwrap_or("");
    if !query.contains("status=active") {
        return (StatusCode::BAD_REQUEST, "filter").into_response();
    }
    axum::Json(json!({
        "next": null,
        "results": [{
            "id": 1,
            "name": "web-1",
            "primary_ip4": {"address": "10.0.0.8/24"},
            "site": {"slug": "berlin"},
            "tags": [{"slug": "webserver"}],
            "config_context": {"http_port": 8080, "db_password": "secret"}
        }]
    }))
    .into_response()
}

pub async fn nb_status(headers: axum::http::HeaderMap) -> axum::response::Response {
    use axum::response::IntoResponse;
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some("Token secret-token")
    {
        return (StatusCode::FORBIDDEN, "no").into_response();
    }
    axum::Json(json!({"netbox-version": "4.2.1", "plugins": {}})).into_response()
}

pub fn shell_with_credentials(name: &str, credential_ids: Value) -> Value {
    json!({
        "name": name,
        "tool": "shell",
        "spec": {"script": "echo ok"},
        "credential_ids": credential_ids
    })
}

pub fn ansible_with_credentials(name: &str, credential_ids: Value, inventory_id: &str) -> Value {
    json!({
        "name": name,
        "tool": "ansible",
        "entrypoint": "site.yml",
        "inventory_id": inventory_id,
        "credential_ids": credential_ids
    })
}

pub fn assert_no_secrets(body: &Value) {
    let rendered = body.to_string();
    for secret in [
        "ssh-eins-geheim",
        "ssh-zwei-geheim",
        "vault-prod-geheim",
        "vault-stage-geheim",
        "vault-plain-geheim",
    ] {
        assert!(
            !rendered.contains(secret),
            "response echoed a credential secret"
        );
    }
}

pub async fn create_credential(
    app: &axum::Router,
    cookie: &str,
    team: &str,
    name: &str,
    kind: &str,
    secret: Value,
) -> String {
    let (status, created, _) = call(
        app,
        "POST",
        "/api/credentials",
        Some(cookie),
        Some(team),
        Some(json!({"name": name, "kind": kind, "secret": secret})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        created.get("error").unwrap_or(&Value::Null)
    );
    assert_no_secrets(&created);
    created["id"].as_str().expect("credential id").to_string()
}

pub async fn launch_and_wait(
    app: &axum::Router,
    cookie: &str,
    team: &str,
    template_id: &str,
) -> Value {
    let (status, job, _) = call(
        app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(cookie),
        Some(team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    let job_id = job["id"].as_str().unwrap();
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let (status, body, _) = call(
            app,
            "GET",
            &format!("/api/jobs/{job_id}"),
            Some(cookie),
            Some(team),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if matches!(
            body["status"].as_str(),
            Some("successful" | "failed" | "canceled")
        ) {
            return body;
        }
    }
    panic!("job {job_id} did not finish");
}

pub fn has_line(job: &Value, stream: &str, line: &str) -> bool {
    job["events"].as_array().is_some_and(|events| {
        events
            .iter()
            .any(|event| event["stream"] == stream && event["line"] == line)
    })
}

pub async fn expect_unknown_field(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    team: Option<&str>,
    body: &Value,
    field: &str,
) {
    let mut headers = vec![("content-type", "application/json")];
    if let Some(team) = team {
        headers.push(("x-team-id", team));
    }
    let (status, bytes, _) = call_raw(
        app,
        method,
        uri,
        cookie,
        &headers,
        Some(body.to_string().into_bytes()),
    )
    .await;
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    assert_eq!(parsed["error"], "json_field_is_not_supported", "{text}");
    assert_eq!(parsed["vars"]["field"], field, "{text}");
}

pub async fn job_count(app: &axum::Router, cookie: &str, team: &str) -> usize {
    let (status, jobs, _) = call(app, "GET", "/api/jobs", Some(cookie), Some(team), None).await;
    assert_eq!(status, StatusCode::OK, "{jobs}");
    jobs["items"].as_array().expect("job list");
    jobs["total"].as_u64().expect("job total") as usize
}

pub async fn expect_json_error(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    team: Option<&str>,
    body: &[u8],
    expected: (StatusCode, &str),
) {
    let mut headers = vec![("content-type", "application/json")];
    if let Some(team) = team {
        headers.push(("x-team-id", team));
    }
    let (status, bytes, _) =
        call_raw(app, method, uri, cookie, &headers, Some(body.to_vec())).await;
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(status, expected.0, "{text}");
    let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    assert_eq!(parsed["error"], expected.1, "{text}");
    assert!(!text.contains("Expected request"), "{text}");
    assert!(!text.contains("Failed to parse"), "{text}");
    assert!(!text.contains("Failed to deserialize"), "{text}");
    assert!(!text.contains("Failed to buffer"), "{text}");
}

pub fn finished_total(metrics: &str, status: &str) -> u64 {
    let needle = format!("status=\"{status}\"");
    metrics
        .lines()
        .filter(|line| line.starts_with("koreograf_jobs_finished_total{") && line.contains(&needle))
        .map(|line| {
            line.rsplit(' ')
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0)
        })
        .sum()
}

pub async fn metrics_text(app: &axum::Router) -> String {
    let (status, body, _) = call(app, "GET", "/metrics", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    body.as_str().unwrap_or("").to_string()
}

pub fn template_write_body(item: &Value) -> Value {
    let mut body = item.clone();
    let map = body.as_object_mut().expect("template object");
    for key in ["id", "team_id", "created_at"] {
        map.remove(key);
    }
    body
}

pub fn daily_schedule(name: &str) -> Value {
    json!({
        "name": name,
        "timezone": "Europe/Berlin",
        "start_date": "2027-06-01",
        "start_time": "06:00",
        "frequency": "daily",
        "end_kind": "never"
    })
}

pub fn set_start_date(state: &koreograf::Shared, id: &str, start_date: &str) {
    let db = state.db().expect("database");
    let changed = db
        .execute(
            "UPDATE schedules SET start_date = ?1 WHERE id = ?2",
            (start_date, id),
        )
        .expect("set start date");
    assert_eq!(changed, 1);
}

pub fn mark_missed(state: &koreograf::Shared, id: &str) {
    let db = state.db().expect("database");
    let changed = db
        .execute(
            "UPDATE schedules SET missed_at = ?1 WHERE id = ?2",
            ("2026-10-21T04:00:00Z", id),
        )
        .expect("mark missed");
    assert_eq!(changed, 1);
}

pub fn scheduled_jobs<'a>(jobs: &'a Value, template_id: &str) -> Vec<&'a Value> {
    jobs["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|job| {
            job["template_id"].as_str() == Some(template_id) && job["trigger"] == "schedule"
        })
        .collect()
}

pub async fn wait_for_job(app: &axum::Router, cookie: &str, team: &str, job_id: &str) -> Value {
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let (status, body, _) = call(
            app,
            "GET",
            &format!("/api/jobs/{job_id}"),
            Some(cookie),
            Some(team),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if matches!(
            body["status"].as_str(),
            Some("successful" | "failed" | "canceled")
        ) {
            return body;
        }
    }
    panic!("job {job_id} did not finish");
}

pub async fn wait_for_workflow(
    app: &axum::Router,
    cookie: &str,
    team: &str,
    run_id: &str,
) -> Value {
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let (status, body, _) = call(
            app,
            "GET",
            &format!("/api/workflow-jobs/{run_id}"),
            Some(cookie),
            Some(team),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if matches!(
            body["status"].as_str(),
            Some("successful" | "failed" | "canceled")
        ) {
            return body;
        }
    }
    panic!("workflow run {run_id} did not finish");
}

pub async fn assert_same_runner(app: &axum::Router, cookie: &str, team: &str, overview: &Value) {
    let (status, denied, _) = call(app, "GET", "/api/runner", None, Some(team), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{denied}");
    let (status, runner, _) = call(app, "GET", "/api/runner", Some(cookie), Some(team), None).await;
    assert_eq!(status, StatusCode::OK, "{runner}");
    assert_eq!(runner["runner_mode"], overview["runner_mode"]);
    assert_eq!(runner["runtime"], overview["runtime"]);
    assert!(runner["local_tools"].is_object(), "{runner}");
    assert!(runner["container_image"].is_boolean(), "{runner}");
    assert_eq!(runner.as_object().map(|item| item.len()), Some(8));
}

pub fn assert_catalog_sorted(path: &Path) {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    let keys = top_level_json_keys(&text);
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "{} keys are not alphabetical", path.display());
}

pub fn top_level_json_keys(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut keys = Vec::new();
    let mut depth = 0;
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '"' {
            let mut key = String::new();
            index += 1;
            while index < chars.len() {
                let item = chars[index];
                if item == '\\' {
                    index += 1;
                    if index < chars.len() {
                        key.push(chars[index]);
                    }
                } else if item == '"' {
                    break;
                } else {
                    key.push(item);
                }
                index += 1;
            }
            let mut next = index + 1;
            while chars.get(next).is_some_and(|item| item.is_whitespace()) {
                next += 1;
            }
            if depth == 1 && chars.get(next) == Some(&':') {
                keys.push(key);
            }
            index += 1;
            continue;
        }
        if ch == '{' || ch == '[' {
            depth += 1;
        } else if ch == '}' || ch == ']' {
            depth -= 1;
        }
        index += 1;
    }
    keys
}

pub fn load_catalog(path: &Path) -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    let value: Value =
        serde_json::from_str(&text).unwrap_or_else(|err| panic!("json {}: {err}", path.display()));
    let Some(object) = value.as_object() else {
        panic!("{} is not an object", path.display());
    };
    let mut catalog = BTreeMap::new();
    for (key, item) in object {
        let Some(text) = item.as_str() else {
            panic!("{key} in {} is not a string", path.display());
        };
        assert!(!text.trim().is_empty(), "{key} is empty");
        catalog.insert(key.clone(), text.to_string());
    }
    catalog
}

pub fn catalog_directories(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in
        std::fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
    {
        let path = entry.unwrap_or_else(|err| panic!("{err}")).path();
        if path.is_dir() {
            out.push(path);
        }
    }
    out.sort();
    out
}

pub fn load_catalogs(dir: &Path, language: &str) -> BTreeMap<String, String> {
    let mut merged = BTreeMap::new();
    let mut sources: BTreeMap<String, PathBuf> = BTreeMap::new();
    for directory in catalog_directories(dir) {
        let path = directory.join(format!("{language}.json"));
        if !path.is_file() {
            continue;
        }
        for (key, value) in load_catalog(&path) {
            if let Some(first) = sources.get(&key) {
                panic!(
                    "duplicate catalog key {key} in {} and {}",
                    first.display(),
                    path.display()
                );
            }
            sources.insert(key.clone(), path.clone());
            merged.insert(key, value);
        }
    }
    merged
}

pub fn has_umlaut(line: &str) -> bool {
    line.chars()
        .any(|ch| matches!(ch, 'ä' | 'ö' | 'ü' | 'Ä' | 'Ö' | 'Ü' | 'ß'))
}

pub fn preview(line: &str) -> String {
    let mut short: String = line.chars().take(160).collect();
    if line.chars().count() > 160 {
        short.push_str("...");
    }
    short
}

pub fn rust_message_keys(dir: &Path) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    collect_rust_keys(dir, &mut keys);
    keys
}

pub fn collect_rust_keys(dir: &Path, keys: &mut BTreeSet<String>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("{err}"));
        let path = entry.path();
        if path.is_dir() {
            collect_rust_keys(&path, keys);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        push_call_key(&text, "ApiError::bad(", keys);
        push_call_key(&text, "ApiError::bad_vars(", keys);
        push_call_key(&text, "ApiError::too_large(", keys);
        push_call_key(&text, "::message(", keys);
        push_err_key(&text, keys);
    }
}

pub fn push_call_key(text: &str, needle: &str, keys: &mut BTreeSet<String>) {
    let mut from = 0;
    while let Some(rel) = text[from..].find(needle) {
        let at = from + rel + needle.len();
        let tail = text[at..].trim_start();
        if let Some((key, _)) = take_quoted(tail) {
            keys.insert(key);
        }
        from = at;
    }
}

pub fn push_err_key(text: &str, keys: &mut BTreeSet<String>) {
    let mut from = 0;
    while let Some(rel) = text[from..].find("Err(") {
        let at = from + rel + 4;
        let tail = text[at..].trim_start();
        if let Some((key, used)) = take_quoted(tail) {
            let after = tail[used..].trim_start();
            if (after.starts_with(".into()") || after.starts_with(".to_string()"))
                && is_message_key(&key)
            {
                keys.insert(key);
            }
        }
        from = at;
    }
}

pub fn is_message_key(key: &str) -> bool {
    let mut chars = key.chars();
    if !chars.next().is_some_and(|ch| ch.is_ascii_lowercase()) {
        return false;
    }
    let mut marked = false;
    for ch in key.chars() {
        if ch == '_' || ch == '.' {
            marked = true;
            continue;
        }
        if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() {
            return false;
        }
    }
    marked
}

pub fn take_quoted(text: &str) -> Option<(String, usize)> {
    let mut chars = text.char_indices();
    if chars.next().is_none_or(|(_, ch)| ch != '"') {
        return None;
    }
    let mut key = String::new();
    let mut escaped = false;
    for (index, ch) in chars {
        if escaped {
            key.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            return Some((key, index + ch.len_utf8()));
        }
        key.push(ch);
    }
    None
}

pub fn leftover_german(dir: &Path, hits: &mut Vec<String>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("{err}"));
        let path = entry.path();
        if path.is_dir() {
            leftover_german(&path, hits);
            continue;
        }
        let ext = path.extension().and_then(|ext| ext.to_str());
        if ext != Some("rs") && ext != Some("sh") {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        for (number, line) in text.lines().enumerate() {
            if line.contains("nicht gefunden") || line.contains("konnte den remote-verweis") {
                continue;
            }
            if has_umlaut(line) {
                hits.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    number + 1,
                    preview(line)
                ));
            }
        }
    }
}

pub fn mcp_initialize_body() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "smoke", "version": "0.1"}
        }
    })
}

pub async fn mcp_request(
    app: &axum::Router,
    token: Option<&str>,
    session: Option<&str>,
    body: Value,
) -> (StatusCode, Vec<u8>, axum::http::HeaderMap) {
    let mut extra = vec![
        ("host", "localhost".to_string()),
        ("content-type", "application/json".to_string()),
        ("accept", "application/json, text/event-stream".to_string()),
    ];
    if let Some(token) = token {
        extra.push(("authorization", format!("Bearer {token}")));
    }
    if let Some(session) = session {
        extra.push(("mcp-session-id", session.to_string()));
    }
    let extra: Vec<(&str, &str)> = extra
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    call_raw(
        app,
        "POST",
        "/mcp",
        None,
        &extra,
        Some(body.to_string().into_bytes()),
    )
    .await
}

pub async fn mcp_session(app: &axum::Router, token: &str) -> String {
    let (status, bytes, headers) = mcp_request(app, Some(token), None, mcp_initialize_body()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let session = headers["mcp-session-id"].to_str().unwrap().to_string();
    let (status, bytes, _) = mcp_request(
        app,
        Some(token),
        Some(&session),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    session
}

pub fn sse_message(bytes: &[u8]) -> Value {
    for line in String::from_utf8_lossy(bytes).lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<Value>(data.trim()) {
            if value.get("jsonrpc").is_some() {
                return value;
            }
        }
    }
    panic!("no SSE JSON message in {}", String::from_utf8_lossy(bytes));
}
