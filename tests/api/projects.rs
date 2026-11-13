use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use serde_json::json;

use crate::common::call_cookie as call;

use crate::common::*;

#[tokio::test]
async fn organizations_inventories_and_git_sync() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = temp_dir("org");
    let origin = temp_dir("origin");
    let mut cfg = config(&dir, &Options::default());
    cfg.project_roots.push(origin.clone());
    let state = koreograf::boot(cfg).unwrap();
    let app = koreograf::router(state);

    let (status, _, _) = call(&app, "GET", "/js/main.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = web_source();
    assert!(script.contains("inventories"), "{script}");
    assert!(script.contains("organizations"));
    assert!(script.contains("nav-group"));
    let (status, style, _) = call(&app, "GET", "/app.css", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(style.as_str().unwrap_or("").contains("nav-label"));

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
    let org_on_team = me["teams"][0]["organization_id"].as_str().unwrap();
    assert!(!org_on_team.is_empty());

    let (status, orgs, _) = call(
        &app,
        "GET",
        "/api/organizations",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{orgs}");
    let org_id = orgs
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Plattform")
        .expect("Organisation Plattform")["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(org_on_team, org_id);

    let inventory_body = "[web]\nweb-1 ansible_host=10.0.0.8 ansible_connection=ssh\n\n[db]\ndb-1\n\n[all:children]\nweb\ndb\n";
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Web", "body": inventory_body})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["host_count"], 2);
    let inventory_id = created["id"].as_str().unwrap();
    let web = created["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|host| host["name"] == "web-1")
        .unwrap();
    assert_eq!(web["groups"], json!(["web"]));
    assert_eq!(web["vars"]["ansible_host"], "10.0.0.8");
    assert_eq!(web["vars"]["ansible_connection"], "ssh");
    assert!(created["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|host| host["name"] != "web" && host["name"] != "db"));

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Web", "body": format!("{inventory_body}[web]\nweb-2\n")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["host_count"], 3);

    git(&origin, &["init", "-b", "main"]);
    git(&origin, &["config", "user.email", "test@example.com"]);
    git(&origin, &["config", "user.name", "Koreograf Test"]);
    std::fs::write(origin.join("marker.txt"), "one\n").unwrap();
    git(&origin, &["add", "marker.txt"]);
    git(&origin, &["commit", "-m", "one"]);
    let url = format!("file://{}", origin.display());

    let (status, project, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Syncprobe", "source_type": "git", "source": url})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    assert_eq!(project["organization_id"], org_id);
    let project_id = project["id"].as_str().unwrap();

    let (status, synced, _) = call(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/sync"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    let checkout = std::path::PathBuf::from(synced["path"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(checkout.join("marker.txt")).unwrap(),
        "one\n"
    );

    std::fs::write(origin.join("two.txt"), "two\n").unwrap();
    git(&origin, &["add", "two.txt"]);
    git(&origin, &["commit", "-m", "two"]);
    let (status, synced, _) = call(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/sync"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    assert_eq!(
        std::fs::read_to_string(checkout.join("two.txt")).unwrap(),
        "two\n"
    );

    let missing = format!("file:///tmp/koreograf-missing-{nanos}");
    let (status, broken, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Kaputt", "source_type": "git", "source": missing})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{broken}");
    let broken_id = broken["id"].as_str().unwrap();
    for _ in 0..2 {
        let (status, failed, _) = call(
            &app,
            "POST",
            &format!("/api/projects/{broken_id}/sync"),
            Some(&cookie),
            Some(&team),
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{failed}");
        let message = failed["error"].as_str().unwrap_or("");
        let lower = message.to_ascii_lowercase();
        assert!(
            lower.contains("fatal")
                || lower.contains("repository")
                || lower.contains("exist")
                || message == "the_project_path_is_not_allowed",
            "{failed}"
        );
        assert!(!message.contains("git clone ist fehlgeschlagen"));
    }

    let (status, other_org, _) = call(
        &app,
        "POST",
        "/api/organizations",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Andere", "description": "Zweite Organisation"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other_org}");
    let other_org_id = other_org["id"].as_str().unwrap();

    let (status, _user, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "operator",
            "display_name": "operator",
            "password": "operator-pass",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = call(
        &app,
        "POST",
        &format!("/api/teams/{team}/members"),
        Some(&cookie),
        Some(&team),
        Some(json!({"username": "operator", "role": "operator"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, operator_me, operator_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "operator", "password": "operator-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{operator_me}");
    let operator_cookie = operator_cookie.expect("operator cookie");
    let (status, denied, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&operator_cookie),
        Some(&team),
        Some(json!({"organization_id": other_org_id, "team_ids": []})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");

    let (status, second, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Betrieb",
            "description": "Zweites Team",
            "organization_id": org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    let betrieb = second["id"].as_str().unwrap();
    assert_eq!(second["organization_id"], org_id);

    let (status, visible, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{visible}");
    assert!(project_names(&visible).contains(&"Syncprobe".to_string()));
    let (status, visible_b, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(betrieb),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{visible_b}");
    assert!(
        project_names(&visible_b).contains(&"Syncprobe".to_string()),
        "{visible_b}"
    );

    let (status, assigned, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"organization_id": org_id, "team_ids": [betrieb]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{assigned}");
    let (status, hidden, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{hidden}");
    assert!(
        !project_names(&hidden)
            .iter()
            .any(|name| name == "Syncprobe"),
        "{hidden}"
    );
    let (status, only_b, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(betrieb),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{only_b}");
    assert!(
        project_names(&only_b).contains(&"Syncprobe".to_string()),
        "{only_b}"
    );

    let (status, nacht, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Nacht",
            "description": "Team der anderen Organisation",
            "organization_id": other_org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{nacht}");
    let nacht = nacht["id"].as_str().unwrap();
    let (status, moved, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(betrieb),
        Some(json!({"organization_id": other_org_id, "team_ids": []})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["organization_id"], other_org_id);
    let (status, gone, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(betrieb),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{gone}");
    assert!(
        !project_names(&gone).iter().any(|name| name == "Syncprobe"),
        "{gone}"
    );
    let (status, there, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(nacht),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{there}");
    assert!(
        project_names(&there).contains(&"Syncprobe".to_string()),
        "{there}"
    );

    let _ = std::fs::remove_dir_all(&origin);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_project_can_be_edited_and_syncs_a_new_revision() {
    let dir = temp_dir("edit");
    let origin = temp_dir("edit-origin");
    let mut cfg = config(&dir, &Options::default());
    cfg.project_roots.push(origin.clone());
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

    git(&origin, &["init", "-b", "main"]);
    git(&origin, &["config", "user.email", "test@example.com"]);
    git(&origin, &["config", "user.name", "Koreograf Test"]);
    std::fs::write(origin.join("main.txt"), "main\n").unwrap();
    git(&origin, &["add", "main.txt"]);
    git(&origin, &["commit", "-m", "main"]);
    git(&origin, &["checkout", "-b", "dev"]);
    std::fs::write(origin.join("dev.txt"), "dev\n").unwrap();
    git(&origin, &["add", "dev.txt"]);
    git(&origin, &["commit", "-m", "dev"]);
    git(&origin, &["checkout", "main"]);
    let url = format!("file://{}", origin.display());

    let (status, project, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Editable",
            "source_type": "git",
            "source": url,
            "revision": "main"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().unwrap();
    let (status, synced, _) = call(
        &app,
        "POST",
        &format!("/api/projects/{project_id}/sync"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    let checkout = std::path::PathBuf::from(synced["path"].as_str().unwrap());
    assert!(checkout.join("main.txt").exists());

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Renamed", "revision": "dev"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "Renamed");
    assert_eq!(updated["revision"], "dev");
    assert_eq!(updated["sync"]["ok"], true);
    assert_eq!(updated["last_sync_status"], "success");
    assert!(!updated["last_sync_at"].as_str().unwrap().is_empty());
    assert!(!updated["last_sync_commit"].as_str().unwrap().is_empty());
    assert!(checkout.join("dev.txt").exists());

    let (status, stored, _) = call(
        &app,
        "GET",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_eq!(stored["name"], "Renamed");
    assert_eq!(stored["revision"], "dev");

    let (status, invalid, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"source": "ftp://example.com/repo.git"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{invalid}");
    assert_eq!(invalid["error"], "the_git_address_must_be_http_s_ssh");
    let (status, created_invalid, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Broken",
            "source_type": "git",
            "source": "ftp://example.com/repo.git"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{created_invalid}");
    assert_eq!(created_invalid["error"], invalid["error"]);

    let (status, _viewer, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "username": "viewer",
            "display_name": "viewer",
            "password": "viewer-pass",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = call(
        &app,
        "POST",
        &format!("/api/teams/{team}/members"),
        Some(&cookie),
        Some(&team),
        Some(json!({"username": "viewer", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, viewer_me, viewer_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "viewer", "password": "viewer-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{viewer_me}");
    let viewer_cookie = viewer_cookie.expect("viewer cookie");
    let (status, denied, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&viewer_cookie),
        Some(&team),
        Some(json!({"name": "Nope"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");

    let _ = std::fs::remove_dir_all(&origin);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn projects_use_a_source_control_credential() {
    let harness = TestApp::boot("project-credential", Options::default());
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

    let (status, scm, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Git access",
            "kind": "scm",
            "secret": {"username": "deploy", "password": "glpat-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{scm}");
    let scm_id = scm["id"].as_str().unwrap().to_string();
    let (status, second, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Git access two",
            "kind": "scm",
            "secret": {"password": "second-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    let second_id = second["id"].as_str().unwrap().to_string();

    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/demo")
        .display()
        .to_string();
    let (status, project, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Private project",
            "source_type": "path",
            "source": source,
            "credential_id": scm_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().unwrap().to_string();
    assert_eq!(project["credential_id"], scm_id);
    assert!(!project.to_string().contains("glpat-token"));

    let (status, stored, _) = call(
        &app,
        "GET",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_eq!(stored["credential_id"], scm_id);
    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(listed
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["id"] == project_id && item["credential_id"] == scm_id));

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"credential_id": second_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["credential_id"], second_id);

    let (status, ssh, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "SSH access",
            "kind": "ssh",
            "secret": {"username": "deploy", "password": "ssh-pass"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{ssh}");
    let ssh_id = ssh["id"].as_str().unwrap();
    let (status, wrong_kind, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Wrong kind",
            "source_type": "path",
            "source": source,
            "credential_id": ssh_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{wrong_kind}");
    assert_eq!(
        wrong_kind["error"],
        "the_credential_is_not_a_source_control_credential"
    );

    let (status, other_team, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Other Git team"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other_team}");
    let other = other_team["id"].as_str().unwrap();
    let (status, foreign, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(other),
        Some(json!({
            "name": "Foreign Git access",
            "kind": "scm",
            "secret": {"password": "foreign-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{foreign}");
    let foreign_id = foreign["id"].as_str().unwrap();
    let (status, wrong_team, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Wrong team",
            "source_type": "path",
            "source": source,
            "credential_id": foreign_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{wrong_team}");
    assert_eq!(wrong_team["error"], "not_found");

    let (status, embedded, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Embedded",
            "source_type": "git",
            "source": "https://user:secret@example.com/repo.git"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{embedded}");
    assert_eq!(embedded["error"], "the_git_address_contains_credentials");
    assert_eq!(embedded["vars"]["field"], "credential_id");
    let (status, embedded_update, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"source_type": "git", "source": "https://user:secret@example.com/repo.git"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{embedded_update}");
    assert_eq!(
        embedded_update["error"],
        "the_git_address_contains_credentials"
    );

    let (status, credentials, _) = call(
        &app,
        "GET",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{credentials}");
    let row = credentials
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == second_id)
        .expect("used credential");
    assert!(row["usages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|usage| usage["project_id"] == project_id));

    let (status, refused, _) = call(
        &app,
        "DELETE",
        &format!("/api/credentials/{second_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["error"], "credential_is_in_use");
    assert!(
        refused["vars"]["names"]
            .as_str()
            .unwrap_or("")
            .contains("Private project"),
        "{refused}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn project_credentials_stay_with_the_project_team() {
    let harness = TestApp::boot("project-credential-team", Options::default());
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
    let admin_team = me["teams"][0]["id"].as_str().unwrap().to_string();

    let (status, org, _) = call(
        &app,
        "POST",
        "/api/organizations",
        Some(&cookie),
        Some(&admin_team),
        Some(json!({"name": "Credential projects"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{org}");
    let org_id = org["id"].as_str().unwrap();
    let (status, owners, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&admin_team),
        Some(json!({"name": "Credential owners", "organization_id": org_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{owners}");
    let owner_team = owners["id"].as_str().unwrap().to_string();
    let (status, guests, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&admin_team),
        Some(json!({"name": "Credential guests", "organization_id": org_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{guests}");
    let guest_team = guests["id"].as_str().unwrap().to_string();

    for (username, team) in [
        ("credential-owner", &owner_team),
        ("credential-guest", &guest_team),
    ] {
        let (status, _user, _) = call(
            &app,
            "POST",
            "/api/users",
            Some(&cookie),
            None,
            Some(json!({
                "username": username,
                "display_name": username,
                "password": "operator-pass"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _, _) = call(
            &app,
            "POST",
            &format!("/api/teams/{team}/members"),
            Some(&cookie),
            Some(team),
            Some(json!({"username": username, "role": "operator"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    let (status, owner_me, owner_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "credential-owner", "password": "operator-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{owner_me}");
    let owner_cookie = owner_cookie.expect("owner cookie");
    let (status, guest_me, guest_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "credential-guest", "password": "operator-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{guest_me}");
    let guest_cookie = guest_cookie.expect("guest cookie");

    let (status, owner_credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&owner_team),
        Some(json!({
            "name": "Owner Git",
            "kind": "scm",
            "secret": {"username": "deploy", "password": "owner-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{owner_credential}");
    let owner_credential = owner_credential["id"].as_str().unwrap().to_string();
    let (status, guest_credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&guest_team),
        Some(json!({
            "name": "Guest Git",
            "kind": "scm",
            "secret": {"password": "guest-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{guest_credential}");
    let guest_credential = guest_credential["id"].as_str().unwrap().to_string();

    let (status, foreign, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&owner_cookie),
        Some(&owner_team),
        Some(json!({
            "name": "Foreign credential project",
            "source_type": "git",
            "source": "https://example.com/owner.git",
            "team_ids": [guest_team],
            "credential_id": guest_credential
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{foreign}");
    assert_eq!(foreign["error"], "not_found");

    let demo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/demo")
        .display()
        .to_string();
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&owner_cookie),
        Some(&owner_team),
        Some(json!({
            "name": "Owner credential project",
            "source_type": "path",
            "source": demo,
            "team_ids": [owner_team, guest_team],
            "credential_id": owner_credential
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let project_id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["credential_id"], owner_credential);

    let (status, guest_view, _) = call(
        &app,
        "GET",
        &format!("/api/projects/{project_id}"),
        Some(&guest_cookie),
        Some(&guest_team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{guest_view}");
    assert!(guest_view["credential_id"].is_null(), "{guest_view}");
    let (status, guest_list, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&guest_cookie),
        Some(&guest_team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{guest_list}");
    assert!(guest_list
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item["id"] == project_id && item["credential_id"].is_null() }));

    let inventory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/demo/inventory")
        .display()
        .to_string();
    let (status, changed, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&guest_cookie),
        Some(&guest_team),
        Some(json!({"source": inventory})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert!(changed["credential_id"].is_null(), "{changed}");
    let (status, owner_view, _) = call(
        &app,
        "GET",
        &format!("/api/projects/{project_id}"),
        Some(&owner_cookie),
        Some(&owner_team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{owner_view}");
    assert!(owner_view["credential_id"].is_null(), "{owner_view}");

    let (status, kept, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&owner_cookie),
        Some(&owner_team),
        Some(json!({"source": demo, "credential_id": owner_credential})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["credential_id"], owner_credential);
    let (status, synced_owner, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{project_id}"),
        Some(&owner_cookie),
        Some(&owner_team),
        Some(json!({"source": inventory})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced_owner}");
    assert_eq!(synced_owner["credential_id"], owner_credential);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn playbooks_project_inventory_and_netbox() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let harness = TestApp::boot("src", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, _, _) = call(&app, "GET", "/js/main.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = web_source();
    assert!(script.contains("playbook_from_the_project"), "{script}");
    assert!(script.contains("data-add-kind=\"project\""));
    assert!(script.contains("data-add-kind=\"netbox\""));
    assert!(script.contains("refresh"));
    assert!(script.contains("netbox: t(\"netbox\")"));
    assert!(script.contains("[\"netbox\", \"netbox\", \"netbox_credential_address_hint\"]"));
    assert!(script.contains("data-netbox-address"));
    assert!(script.contains("data-new-netbox-credential"));
    assert!(!script.contains("data-token-box"));
    let inspector = script
        .split_once("function renderInspector")
        .expect("workflow inspector")
        .1;
    let node_at = inspector.find("this_node").expect("selected node panel");
    let add_at = inspector[..node_at]
        .rfind("addNodeControls()")
        .expect("add stays above the selected node");
    assert!(
        node_at - add_at < 800,
        "add node drifted away from the selected node"
    );
    let node_panel = &inspector[node_at..inspector.find("node-delete").expect("delete node")];
    assert!(node_panel.contains("on_success_go_to"));
    assert!(node_panel.contains("on_failure_go_to"));
    assert!(node_panel.contains("always_go_to"));
    assert!(node_panel.contains("nodeParams("));
    assert!(script.contains("applies_only_to_this_node_the_template_stays"));
    assert!(script.contains("node-inventory"));
    assert!(script.contains("no_inventory"));

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

    let (status, projects, _) = call(
        &app,
        "GET",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{projects}");
    let demo = projects
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Demo")
        .expect("Demo-Projekt")["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, contents, _) = call(
        &app,
        "GET",
        &format!("/api/projects/{demo}/contents"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{contents}");
    assert!(contents["playbooks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item == "site.yml"));
    assert!(contents["inventories"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item["path"] == "inventory/hosts.ini" && item["format"] == "ini" }));

    let (status, imported, _) = call(
        &app,
        "POST",
        "/api/inventories/import",
        Some(&cookie),
        Some(&team),
        Some(json!({"project_id": demo, "path": "inventory/hosts.ini"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{imported}");
    assert_eq!(imported["source_kind"], "project");
    assert_eq!(imported["source"]["path"], "inventory/hosts.ini");
    let host = imported["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "localhost")
        .expect("localhost");
    assert_eq!(host["vars"]["ansible_connection"], "local");
    let imported_id = imported["id"].as_str().unwrap();

    let (status, refreshed, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{imported_id}/sync"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{refreshed}");
    assert!(refreshed["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "localhost"));

    let (status, plain, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Hand", "body": "[all]\nbox ansible_connection=local\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{plain}");
    let plain_id = plain["id"].as_str().unwrap();
    let (status, denied, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{plain_id}/sync"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");
    assert!(denied["error"]
        .as_str()
        .unwrap_or("")
        .contains("this_inventory_has_no_external_source"));

    let (status, unsynced, _) = call(
        &app,
        "POST",
        "/api/projects",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Noch nicht synchron",
            "source_type": "git",
            "source": format!("file:///tmp/koreograf-empty-{nanos}")
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{unsynced}");
    let unsynced_id = unsynced["id"].as_str().unwrap();
    let (status, missing, _) = call(
        &app,
        "GET",
        &format!("/api/projects/{unsynced_id}/contents"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{missing}");
    assert!(missing["error"]
        .as_str()
        .unwrap_or("")
        .contains("sync_the_git_project_first"));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route("/api/dcim/devices/", axum::routing::get(nb_devices));
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });
    let (status, credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Source NetBox",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": format!("http://{addr}")}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let credential_id = credential["id"].as_str().unwrap().to_string();

    let (status, preview, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox/preview",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "credential_id": credential_id,
            "group_by": ["tags"],
            "group_names_raw": true,
            "load_config_context": true,
            "query_filters": ["status=active"],
            "virtual_machines": false,
            "max_hosts": 100
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["host_count"], 1);
    assert_eq!(preview["groups"][0]["name"], "webserver");
    assert_eq!(preview["groups"][0]["host_count"], 1);
    assert_eq!(preview["sample_host"]["name"], "web-1");
    assert_eq!(preview["sample_host"]["vars"]["http_port"], "8080");
    assert!(preview["collisions"].as_array().unwrap().is_empty());
    assert!(!preview.to_string().contains("secret-token"));

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Aus NetBox",
            "credential_id": credential_id,
            "group_by": ["tags"],
            "group_names_raw": true,
            "load_config_context": true,
            "query_filters": ["status=active"],
            "virtual_machines": false,
            "max_hosts": 100
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["source_kind"], "netbox");
    assert!(created["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "web-1"));
    let body = created["body"].as_str().unwrap_or("");
    assert!(body.starts_with("all:\n  hosts:\n"), "{body}");
    assert!(body.contains("http_port: 8080"), "{body}");
    assert!(body.contains("    webserver:\n"), "{body}");
    assert!(!body.contains("secret-token"));
    let netbox_id = created["id"].as_str().unwrap();
    let (status, again, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{netbox_id}/sync"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert!(again["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "web-1"));

    let (status, wrong_credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Wrong NetBox token",
            "kind": "netbox",
            "secret": {"token": "wrong-token", "url": format!("http://{addr}")}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{wrong_credential}");
    let wrong_credential_id = wrong_credential["id"].as_str().unwrap().to_string();
    let (status, bad, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Kaputtes NetBox",
            "credential_id": wrong_credential_id,
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    assert_eq!(bad["error"], "netbox.token_rejected");
    assert_eq!(bad["vars"]["status"], "403", "{bad}");
    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(listed
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["name"] != "Kaputtes NetBox"));

    let _ = std::fs::remove_dir_all(&dir);
}
