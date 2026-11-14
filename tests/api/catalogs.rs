use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use axum::http::{header, StatusCode};
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
    let de = load_catalogs(&root.join("web/i18n"), "de");
    let en = load_catalogs(&root.join("web/i18n"), "en");
    assert_eq!(
        de.keys().collect::<Vec<_>>(),
        en.keys().collect::<Vec<_>>(),
        "catalog keys differ"
    );

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
fn catalogs_are_split_per_feature_without_duplicates() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("web/i18n");
    let directories = catalog_directories(&dir);
    assert!(!directories.is_empty(), "no catalog directories");
    for entry in std::fs::read_dir(&dir).expect("catalog directory") {
        let path = entry.expect("catalog entry").path();
        assert!(
            path.is_dir(),
            "{} is outside a feature directory",
            path.display()
        );
    }
    for directory in directories {
        let de_path = directory.join("de.json");
        let en_path = directory.join("en.json");
        assert!(de_path.is_file(), "{} is missing", de_path.display());
        assert!(en_path.is_file(), "{} is missing", en_path.display());
        assert_catalog_sorted(&de_path);
        assert_catalog_sorted(&en_path);
        let de = load_catalog(&de_path);
        let en = load_catalog(&en_path);
        assert_eq!(
            de.keys().collect::<Vec<_>>(),
            en.keys().collect::<Vec<_>>(),
            "{} keys differ between de and en",
            directory.display()
        );
    }
    let de = load_catalogs(&dir, "de");
    let en = load_catalogs(&dir, "en");
    assert_eq!(
        de.keys().collect::<Vec<_>>(),
        en.keys().collect::<Vec<_>>(),
        "merged catalog keys differ"
    );
}

#[test]
fn dynamic_catalog_prefixes_have_their_keys() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let de = load_catalogs(&root.join("web/i18n"), "de");
    let en = load_catalogs(&root.join("web/i18n"), "en");
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

fn any_value_contains(catalog: &BTreeMap<String, String>, needle: &str) -> bool {
    catalog.values().any(|value| value.contains(needle))
}

#[test]
fn auto_mode_copy_names_the_missing_runtime() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let runner = std::fs::read_to_string(root.join("src/runner.rs")).expect("runner.rs");
    assert!(runner
        .contains("Mount the Docker socket, or set the runner mode to docker to fail instead."));
    let en = load_catalogs(&root.join("web/i18n"), "en");
    let de = load_catalogs(&root.join("web/i18n"), "de");
    assert!(any_value_contains(
        &en,
        "Mount the Docker socket, or set the runner mode to docker to fail instead."
    ));
    assert!(any_value_contains(
        &en,
        "In automatic mode a set image is also ignored"
    ));
    assert!(any_value_contains(&de, "Docker-Socket"));
    assert!(any_value_contains(&de, "Im automatischen Modus"));
    let docs = std::fs::read_to_string(root.join("docs/configuration.md")).expect("docs");
    assert!(docs.contains("mount the Docker socket"));
    assert!(docs.contains("stops after ten seconds"));
    assert!(docs.contains("GET /api/runner"));
}

#[test]
fn source_control_token_hints_are_in_the_forms_and_catalogs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let en = load_catalogs(&root.join("web/i18n"), "en");
    let de = load_catalogs(&root.join("web/i18n"), "de");
    for catalog in [&en, &de] {
        assert!(any_value_contains(catalog, "x-access-token"));
        assert!(any_value_contains(catalog, "oauth2"));
        assert!(catalog.contains_key("source_control_credential_hint"));
        assert!(catalog.contains_key("source_control_username_hint"));
        assert!(catalog.contains_key("source_control_project_hint"));
    }
    let docs = std::fs::read_to_string(root.join("docs/jobs.md")).expect("docs");
    assert!(docs.contains("https://github.com/example/project.git"));
    assert!(docs.contains("x-access-token"));
    assert!(docs.contains("https://gitlab.com/example/project.git"));
    assert!(docs.contains("oauth2"));
}

#[tokio::test]
async fn the_catalog_routes_serve_the_merged_catalogs() {
    let harness = TestApp::boot("catalog-http", Options::default());
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for language in ["de", "en"] {
        let expected = load_catalogs(&root.join("web/i18n"), language);
        let path = format!("/i18n/{language}.json");
        let (status, headers, body) = request(&harness.app, "GET", &path, &[], None).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(
            headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/json; charset=utf-8"),
            "{path}"
        );
        let served: BTreeMap<String, String> = serde_json::from_slice(&body).expect("json");
        assert_eq!(served, expected, "{path}");
    }
    let dir = harness.dir.clone();
    let _ = std::fs::remove_dir_all(&dir);
}
