use serde_json::json;

use super::model::{ProjectResponse, SyncResponse};
use crate::db::Project;
use crate::error::ApiError;

fn project() -> Project {
    Project {
        id: "project".into(),
        team_id: "team".into(),
        organization_id: "org".into(),
        organization_name: "Org".into(),
        name: "Demo".into(),
        source_type: "git".into(),
        source: "https://user:secret@example.com/repo.git".into(),
        revision: "main".into(),
        credential_id: Some("credential".into()),
        created_at: "2026-01-01T00:00:00Z".into(),
        team_ids: vec!["team".into()],
        last_sync_at: String::new(),
        last_sync_commit: String::new(),
        last_sync_status: String::new(),
        admin_saved: true,
    }
}

#[test]
fn a_project_response_masks_the_source_and_hides_another_teams_credential() {
    let owner = serde_json::to_value(ProjectResponse::new(&project(), "team")).unwrap();
    assert_eq!(owner["source"], "https://[redacted]@example.com/repo.git");
    assert_eq!(owner["credential_id"], "credential");
    let other = serde_json::to_value(ProjectResponse::new(&project(), "other")).unwrap();
    assert_eq!(other["source"], "https://[redacted]@example.com/repo.git");
    assert!(other["credential_id"].is_null());
}

#[test]
fn a_sync_response_keeps_a_bad_request_key_and_vars() {
    let response = SyncResponse::from_result(Err(ApiError::bad_vars(
        "the_git_address_contains_credentials",
        [("field", "credential_id")],
    )));
    let body = serde_json::to_value(response).unwrap();
    assert_eq!(body["ok"], false);
    assert_eq!(body["error"], "the_git_address_contains_credentials");
    assert_eq!(body["vars"], json!({"field": "credential_id"}));
    assert!(body.get("path").is_none());
}

#[test]
fn a_successful_sync_response_reports_the_checkout_path() {
    let body = serde_json::to_value(SyncResponse::from_result(Ok("/tmp/repo".into()))).unwrap();
    assert_eq!(body["ok"], true);
    assert_eq!(body["path"], "/tmp/repo");
    assert!(body.get("error").is_none());
}
