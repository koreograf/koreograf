use std::collections::BTreeSet;
use std::path::Path;

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::common::{call_cookie as call, call_raw_headers as call_raw};

use crate::common::*;

const DYNAMIC_PREFIX_KEYS: [(&str, &[&str]); 4] = [
    (
        "check_",
        &[
            "check_reachable",
            "check_rights",
            "check_server_version",
            "check_tls",
        ],
    ),
    (
        "preflight_runtime_",
        &[
            "preflight_runtime_missing",
            "preflight_runtime_not_answering",
            "preflight_runtime_timeout",
        ],
    ),
    (
        "schedule.day.",
        &[
            "schedule.day.fr",
            "schedule.day.mo",
            "schedule.day.sa",
            "schedule.day.su",
            "schedule.day.th",
            "schedule.day.tu",
            "schedule.day.we",
        ],
    ),
    (
        "schedule.short.",
        &[
            "schedule.short.fr",
            "schedule.short.mo",
            "schedule.short.sa",
            "schedule.short.su",
            "schedule.short.th",
            "schedule.short.tu",
            "schedule.short.we",
        ],
    ),
];

#[tokio::test]
async fn a_request_without_json_returns_a_catalog_error() {
    let harness = TestApp::boot("json", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, bytes, _) = call_raw(&app, "POST", "/api/login", None, &[], None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let missing: Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(missing["error"], "json_body_required");
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("Expected request"), "{text}");

    let (status, bytes, _) = call_raw(
        &app,
        "POST",
        "/api/login",
        None,
        &[("content-type", "text/plain")],
        Some(b"{}".to_vec()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let wrong_type: Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(wrong_type["error"], "json_body_required");

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
    let team = me["teams"][0]["id"].as_str().unwrap();
    let (status, templates, _) = call(
        &app,
        "GET",
        "/api/templates",
        Some(&cookie),
        Some(team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{templates}");
    let health_id = templates
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Healthcheck")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let (status, bytes, _) = call_raw(
        &app,
        "POST",
        &format!("/api/templates/{health_id}/launch"),
        Some(&cookie),
        &[("x-team-id", team)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let launch: Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(launch["error"], "json_body_required");

    let finished = launch_and_wait(&app, &cookie, team, health_id).await;
    assert_eq!(finished["status"], "successful", "{}", finished["events"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn message_catalogs_match_and_cover_the_ui() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let de_path = root.join("web/i18n/de.json");
    let en_path = root.join("web/i18n/en.json");
    assert_catalog_sorted(&de_path);
    assert_catalog_sorted(&en_path);
    let de = load_catalog(&de_path);
    let en = load_catalog(&en_path);
    assert_eq!(
        de.keys().collect::<Vec<_>>(),
        en.keys().collect::<Vec<_>>(),
        "catalog keys differ"
    );

    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    let mut missing = BTreeSet::new();
    for key in ui_message_keys(&script) {
        if !de.contains_key(&key) {
            missing.insert(key);
        }
    }
    assert!(
        missing.is_empty(),
        "app.js keys missing from the catalogs: {missing:?}"
    );

    let german_ui = umlaut_lines("web/app.js", &script);
    assert!(german_ui.is_empty(), "German left in app.js: {german_ui:?}");
    let mut german_src = Vec::new();
    leftover_german(&root.join("src"), &mut german_src);
    assert!(german_src.is_empty(), "German left in src: {german_src:?}");

    let api_keys = rust_message_keys(&root.join("src"));
    let mut api_missing = BTreeSet::new();
    for key in &api_keys {
        if !de.contains_key(key) {
            api_missing.insert(key.clone());
        }
    }
    assert!(
        api_missing.is_empty(),
        "API keys missing from the catalogs: {api_missing:?}"
    );

    let dynamic_prefixes = DYNAMIC_PREFIX_KEYS.map(|(prefix, _)| prefix);
    let index = std::fs::read_to_string(root.join("web/index.html")).expect("index.html");
    let web = format!("{script}\n{index}");
    let rust = rust_sources(&root.join("src"));
    let mut unused = BTreeSet::new();
    for key in de.keys() {
        if quoted_literal(&web, key)
            || quoted_literal(&rust, key)
            || dynamic_prefixes
                .iter()
                .any(|prefix| key.starts_with(*prefix))
        {
            continue;
        }
        unused.insert(key.clone());
    }
    assert!(
        unused.is_empty(),
        "catalog keys used by neither the UI nor the API: {unused:?}"
    );
    for catalog in [&de, &en] {
        let hint = catalog
            .get("confirm_the_replacement_with_ersetzen")
            .map(String::as_str)
            .unwrap_or("");
        assert!(
            hint.contains("ERSETZEN"),
            "restore hint must name the confirm token"
        );
    }

    let runner = std::fs::read_to_string(root.join("src/runner.rs")).expect("runner.rs");
    assert!(
        runner.contains("nicht gefunden"),
        "git still reports a missing revision in German"
    );
    assert!(
        runner.contains("konnte den remote-verweis"),
        "git still reports a missing remote ref in German"
    );
}

#[test]
fn dynamic_catalog_prefixes_have_their_keys() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let de = load_catalog(&root.join("web/i18n/de.json"));
    let en = load_catalog(&root.join("web/i18n/en.json"));
    for (prefix, keys) in DYNAMIC_PREFIX_KEYS {
        assert!(!keys.is_empty(), "{prefix} has no expected keys");
        for key in keys {
            assert!(
                key.starts_with(prefix),
                "{key} does not start with {prefix}"
            );
            assert!(de.contains_key(*key), "de is missing {key}");
            assert!(en.contains_key(*key), "en is missing {key}");
        }
    }
    for id in koreograf::DATABASE_CHECK_IDS {
        let key = format!("check_{id}");
        assert!(de.contains_key(&key), "de is missing {key}");
        assert!(en.contains_key(&key), "en is missing {key}");
    }
}

fn quoted_literal(source: &str, key: &str) -> bool {
    ["\"", "'", "`"]
        .iter()
        .any(|quote| source.contains(&format!("{quote}{key}{quote}")))
}

fn rust_sources(dir: &Path) -> String {
    let mut text = String::new();
    for entry in
        std::fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
    {
        let entry = entry.unwrap_or_else(|err| panic!("{err}"));
        let path = entry.path();
        if path.is_dir() {
            text.push_str(&rust_sources(&path));
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            text.push_str(
                &std::fs::read_to_string(&path)
                    .unwrap_or_else(|err| panic!("read {}: {err}", path.display())),
            );
            text.push('\n');
        }
    }
    text
}

#[test]
fn source_control_token_hints_are_in_the_forms_and_catalogs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("web/app.js")).expect("app.js");
    assert!(script.contains("[\"scm\", \"source_control\", \"source_control_credential_hint\"]"));
    assert!(script.contains("credHint(\"source_control_username_hint\")"));
    assert!(script.contains("h(\"source_control_project_hint\")"));
    assert!(script.contains("h(\"source_control_key_hint\")"));
    let en = std::fs::read_to_string(root.join("web/i18n/en.json")).expect("en");
    let de = std::fs::read_to_string(root.join("web/i18n/de.json")).expect("de");
    for catalog in [&en, &de] {
        assert!(catalog.contains("x-access-token"));
        assert!(catalog.contains("oauth2"));
        assert!(catalog.contains("source_control_credential_hint"));
        assert!(catalog.contains("source_control_username_hint"));
        assert!(catalog.contains("source_control_project_hint"));
    }
    let docs = std::fs::read_to_string(root.join("docs/jobs.md")).expect("docs");
    assert!(docs.contains("https://github.com/example/project.git"));
    assert!(docs.contains("x-access-token"));
    assert!(docs.contains("https://gitlab.com/example/project.git"));
    assert!(docs.contains("oauth2"));
}
