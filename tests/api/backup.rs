use axum::http::StatusCode;
use serde_json::json;

use crate::common::{call_cookie as call, call_raw_headers as call_raw};

use crate::common::*;

#[tokio::test]
async fn admin_responses_have_the_documented_shape() {
    let harness = TestApp::boot("admin-shape", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, me, cookie) = call(
        &app,
        "POST",
        "/api/v1/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");

    let (status, maintenance, _) = call(&app, "GET", "/api/v1/maintenance", None, None, None).await;
    assert_eq!(status, StatusCode::OK, "{maintenance}");
    assert_eq!(maintenance, json!({"active": false, "since": null}));

    let (status, archive, _) = call_raw(
        &app,
        "GET",
        "/api/v1/admin/backup",
        Some(&cookie),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(archive.starts_with(b"PK"), "zip magic");

    let (status, restored, headers) = call_raw(
        &app,
        "POST",
        "/api/v1/admin/backup",
        Some(&cookie),
        &[("x-restore-confirm", "ERSETZEN")],
        Some(archive),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&restored)
    );
    let restored: serde_json::Value = serde_json::from_slice(&restored).expect("restore body");
    assert_eq!(
        restored,
        json!({
            "restored": true,
            "engine": "sqlite",
            "previous": "restore-previous.zip"
        })
    );
    assert!(!restored.to_string().contains("secret.key"), "{restored}");
    assert!(!restored.to_string().contains("koreograf.db"), "{restored}");
    let set_cookie = headers
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(set_cookie.contains("Max-Age=0"), "{set_cookie}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn admin_backup_roundtrip_replaces_the_installation() {
    let harness = TestApp::boot("backup-http", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, _, _) = call(&app, "GET", "/js/main.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);

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

    let (status, archive, headers) =
        call_raw(&app, "GET", "/api/admin/backup", Some(&cookie), &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/zip")
    );
    assert!(archive.starts_with(b"PK"), "zip magic");

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/organizations",
        Some(&cookie),
        None,
        Some(json!({"name": "NurKurz"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");

    let (status, _, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        None,
        Some(json!({
            "username": "viewer",
            "password": "viewer-pass",
            "display_name": "Viewer"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, viewer, viewer_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "viewer", "password": "viewer-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer}");
    let viewer_cookie = viewer_cookie.expect("viewer cookie");
    let (status, denied, _) = call_raw(
        &app,
        "GET",
        "/api/admin/backup",
        Some(&viewer_cookie),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!denied
        .windows(b"secret.key".len())
        .any(|window| window == b"secret.key"));

    let (status, rejected, _) = call_raw(
        &app,
        "POST",
        "/api/admin/backup",
        Some(&cookie),
        &[],
        Some(archive.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let rejected = String::from_utf8_lossy(&rejected);
    assert!(
        rejected.contains("confirm_the_replacement_with_ersetzen"),
        "{rejected}"
    );

    let (status, orgs, _) =
        call(&app, "GET", "/api/organizations", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{orgs}");
    assert!(orgs
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "NurKurz"));

    let (status, restored, headers) = call_raw(
        &app,
        "POST",
        "/api/admin/backup",
        Some(&cookie),
        &[("x-restore-confirm", "ERSETZEN")],
        Some(archive),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&restored)
    );
    assert!(String::from_utf8_lossy(&restored).contains("\"restored\":true"));
    let set_cookie = headers
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(set_cookie.contains("Max-Age=0"), "{set_cookie}");

    let (status, orgs, _) =
        call(&app, "GET", "/api/organizations", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{orgs}");
    assert!(orgs
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["name"] != "NurKurz"));
    assert!(std::path::Path::new(&dir)
        .join("restore-previous.zip")
        .is_file());

    let _ = std::fs::remove_dir_all(&dir);
}
