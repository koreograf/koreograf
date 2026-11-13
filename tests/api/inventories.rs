use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::common::call_cookie as call;

use crate::common::*;

#[tokio::test]
async fn inventory_hosts_page_search_and_filter() {
    let harness = TestApp::boot("host-page", Options::default());
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

    let body = "[web]\n\
web-01 ansible_host=10.0.0.1 role=frontend\n\
web-02 ansible_host=10.0.0.2 role=frontend\n\
web-03 ansible_host=10.0.0.3 role=frontend\n\
\n\
[db]\n\
db-01 ansible_host=10.0.0.11 role=database\n\
db-02 ansible_host=10.0.0.12 role=database\n\
\n\
[cache]\n\
cache-01 ansible_host=10.0.0.42 role=cache\n";
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Paged hosts", "body": body})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["host_count"], 6);

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
    let summary = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id)
        .unwrap();
    assert!(summary.get("hosts").is_none(), "{summary}");
    assert!(summary.get("body").is_none(), "{summary}");
    assert_eq!(summary["host_count"], 6);
    assert_eq!(summary["group_count"], 3);
    assert_eq!(summary["source_count"], 1);
    assert!(summary["sources"]
        .as_array()
        .unwrap()
        .iter()
        .all(|source| source.get("body").is_none()));

    let (status, detail, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert!(detail.get("hosts").is_none(), "{detail}");
    assert!(detail.get("body").is_none(), "{detail}");
    assert!(detail["sources"]
        .as_array()
        .unwrap()
        .iter()
        .all(|source| source.get("body").is_none()));

    let source_id = detail["sources"][0]["id"].as_str().unwrap();
    let (status, source, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{source}");
    assert!(source["body"].as_str().unwrap_or("").contains("web-01"));
    assert_eq!(source["host_count"], 6);

    let hosts_url = |query: &str| format!("/api/inventories/{id}/hosts{query}");
    let (status, first, _) = call(
        &app,
        "GET",
        &hosts_url(""),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["total"], 6);
    assert_eq!(first["page"], 1);
    assert_eq!(first["page_size"], 50);
    assert_eq!(first["hosts"].as_array().unwrap().len(), 6);
    assert_eq!(first["groups"], json!(["cache", "db", "web"]));

    let (status, second, _) = call(
        &app,
        "GET",
        &hosts_url("?page=2&page_size=2"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["total"], 6);
    assert_eq!(second["page"], 2);
    assert_eq!(second["hosts"][0]["name"], "db-02");
    assert_eq!(second["hosts"][1]["name"], "web-01");
    assert_eq!(second["hosts"].as_array().unwrap().len(), 2);

    let (status, beyond, _) = call(
        &app,
        "GET",
        &hosts_url("?page=4&page_size=2"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{beyond}");
    assert_eq!(beyond["total"], 6);
    assert_eq!(beyond["page"], 4);
    assert_eq!(beyond["hosts"], json!([]));

    let (status, huge, _) = call(
        &app,
        "GET",
        &hosts_url("?page=18446744073709551615&page_size=2"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{huge}");
    assert_eq!(huge["total"], 6);
    assert_eq!(huge["hosts"], json!([]));

    let (status, zero, _) = call(
        &app,
        "GET",
        &hosts_url("?page=0"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{zero}");
    assert_eq!(zero["page"], 1);
    assert_eq!(zero["hosts"].as_array().unwrap().len(), 6);

    let (status, capped, _) = call(
        &app,
        "GET",
        &hosts_url("?limit=5000"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{capped}");
    assert_eq!(capped["page_size"], 200);

    let (status, offset, _) = call(
        &app,
        "GET",
        &hosts_url("?offset=4&limit=2"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{offset}");
    assert_eq!(offset["page"], 3);
    assert_eq!(offset["hosts"][0]["name"], "web-02");
    assert_eq!(offset["hosts"].as_array().unwrap().len(), 2);

    let (status, search, _) = call(
        &app,
        "GET",
        &hosts_url("?search=web"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{search}");
    assert_eq!(search["total"], 3);
    assert!(search["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|host| host["name"].as_str().unwrap().starts_with("web-")));

    let (status, address, _) = call(
        &app,
        "GET",
        &hosts_url("?search=10.0.0.42"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{address}");
    assert_eq!(address["total"], 1);
    assert_eq!(address["hosts"][0]["name"], "cache-01");

    let (status, variable, _) = call(
        &app,
        "GET",
        &hosts_url("?search=database"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{variable}");
    assert_eq!(variable["total"], 2);

    let (status, grouped, _) = call(
        &app,
        "GET",
        &hosts_url("?group=db"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{grouped}");
    assert_eq!(grouped["total"], 2);
    assert!(grouped["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|host| host["groups"] == json!(["db"])));

    let (status, combined, _) = call(
        &app,
        "GET",
        &hosts_url("?group=web&search=web-02"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{combined}");
    assert_eq!(combined["total"], 1);

    let (status, empty, _) = call(
        &app,
        "GET",
        &hosts_url("?search=missing"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{empty}");
    assert_eq!(empty["total"], 0);
    assert_eq!(empty["hosts"], json!([]));

    let (status, reversed, _) = call(
        &app,
        "GET",
        &hosts_url("?sort=-name&page_size=1"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reversed}");
    assert_eq!(reversed["hosts"][0]["name"], "web-03");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn inventory_host_and_group_details_include_effective_variables() {
    let harness = TestApp::boot("host-details", Options::default());
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

    let body = "\
[all:vars]\n\
ansible_user=ansible\n\
\n\
[app:children]\n\
web\n\
\n\
[app:vars]\n\
app_port=1\n\
\n\
[web:vars]\n\
http_port=8080\n\
\n\
[web]\n\
web-01 ansible_host=10.0.0.1\n\
web-02 ansible_host=10.0.0.2\n";
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Host details", "body": body})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_string();
    let source_id = created["sources"][0]["id"].as_str().unwrap().to_string();

    let (status, groups, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{id}/groups"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{groups}");
    assert_eq!(groups["total"], 3);
    assert_eq!(groups["groups"][0]["name"], "all");
    assert_eq!(groups["groups"][0]["host_count"], 2);
    let app_group = groups["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group["name"] == "app")
        .unwrap();
    assert_eq!(app_group["children"], json!(["web"]));
    assert_eq!(app_group["vars"]["app_port"], "1");
    assert_eq!(app_group["host_count"], 2);
    let web_group = groups["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group["name"] == "web")
        .unwrap();
    assert_eq!(web_group["vars"]["http_port"], "8080");
    assert_eq!(web_group["host_count"], 2);

    let (status, group, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{id}/groups/web?page=2&page_size=1"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{group}");
    assert_eq!(group["name"], "web");
    assert_eq!(group["parents"], json!(["app"]));
    assert_eq!(group["total"], 2);
    assert_eq!(group["page"], 2);
    assert_eq!(group["hosts"].as_array().unwrap().len(), 1);
    assert_eq!(group["hosts"][0]["name"], "web-02");

    let (status, host, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{id}/hosts/web-01"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{host}");
    assert_eq!(host["name"], "web-01");
    assert_eq!(host["groups"], json!(["all", "app", "web"]));
    assert_eq!(host["source"]["id"], source_id);
    assert_eq!(host["source"]["kind"], "ini");
    let vars = host["vars"].as_array().unwrap();
    let find = |name: &str| vars.iter().find(|item| item["name"] == name).unwrap();
    assert_eq!(find("ansible_host")["value"], "10.0.0.1");
    assert_eq!(find("ansible_host")["origin"], "host");
    assert_eq!(find("http_port")["origin"], "web");
    assert_eq!(find("app_port")["origin"], "app");
    assert_eq!(find("ansible_user")["value"], "ansible");
    assert_eq!(find("ansible_user")["origin"], "all");
    assert!(host["netbox_id"].is_null());
    assert!(host["netbox_url"].is_null());

    let (status, missing_host, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{id}/hosts/missing"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing_host}");
    let (status, missing_group, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{id}/groups/missing"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing_group}");

    let (status, _, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        None,
        Some(json!({
            "username": "detail-viewer",
            "password": "viewer-pass",
            "display_name": "Detail viewer"
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
        Some(json!({"username": "detail-viewer", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, viewer_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "detail-viewer", "password": "viewer-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let viewer_cookie = viewer_cookie.expect("viewer cookie");
    for path in [
        format!("/api/inventories/{id}/hosts/web-01"),
        format!("/api/inventories/{id}/groups"),
        format!("/api/inventories/{id}/groups/web"),
    ] {
        let (status, body, _) =
            call(&app, "GET", &path, Some(&viewer_cookie), Some(&team), None).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn inventory_sources_can_be_edited_and_inventories_renamed() {
    let harness = TestApp::boot("source-edit", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, _, _) = call(&app, "GET", "/js/main.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = web_source();
    assert!(script.contains("data-edit-source"));
    assert!(script.contains("hosts_before_after"));

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

    let (status, _, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        None,
        Some(json!({
            "username": "source-viewer",
            "password": "viewer-pass",
            "display_name": "Source viewer"
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
        Some(json!({"username": "source-viewer", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route("/api/dcim/devices/", axum::routing::get(nb_editable));
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
            "name": "Edit NetBox",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": format!("http://{addr}")}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let credential_id = credential["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "NetBox edit",
            "credential_id": credential_id,
            "group_by": ["sites"],
            "query_filters": ["status=active"],
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let inventory_id = created["id"].as_str().unwrap().to_string();
    let source_id = created["sources"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(created["host_count"], 1);
    assert!(created["sources"][0]["body"]
        .as_str()
        .unwrap_or("")
        .contains("sites_berlin"));

    let (status, edited, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Edited NetBox",
            "credential_id": credential_id,
            "group_by": ["site"],
            "query_filters": ["status=planned"],
            "validate_certs": true,
            "ca_file": "/no/such/koreograf-ca.pem",
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["sources"][0]["id"], source_id);
    assert_eq!(edited["sources"][0]["name"], "Edited NetBox");
    assert_eq!(edited["sources"][0]["source"]["group_by"], json!(["site"]));
    assert!(edited["sources"][0]["source"].get("ca_file").is_none());
    assert_eq!(
        edited["sources"][0]["source"]["query"],
        json!([["status", "planned"]])
    );
    assert!(edited["sources"][0]["body"]
        .as_str()
        .unwrap_or("")
        .contains("site_berlin"));
    assert!(edited["sources"][0]["body"]
        .as_str()
        .unwrap_or("")
        .contains("web-2"));
    assert_eq!(edited["host_count"], 2);
    assert_eq!(edited["sync"]["ok"], true);
    assert_eq!(edited["sync"]["before"], 1);
    assert_eq!(edited["sync"]["after"], 2);
    assert!(!edited.to_string().contains("secret-token"));

    let (status, stored, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_eq!(stored["sources"][0]["id"], source_id);
    assert_eq!(stored["sources"][0]["name"], "Edited NetBox");
    assert_eq!(stored["sources"][0]["source"]["group_by"], json!(["site"]));

    let (status, foreign, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"path": "inventory/hosts.ini"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{foreign}");
    assert_eq!(foreign["error"], "json_field_is_not_supported");
    assert_eq!(foreign["vars"]["field"], "path");

    let (status, renamed, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Renamed inventory"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["name"], "Renamed inventory");
    assert_eq!(renamed["sources"][0]["id"], source_id);

    let (status, _, viewer_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "source-viewer", "password": "viewer-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let viewer_cookie = viewer_cookie.expect("viewer cookie");
    let (status, forbidden, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&viewer_cookie),
        Some(&team),
        Some(json!({"name": "Not allowed"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{forbidden}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn netbox_sources_follow_the_credential_address() {
    let harness = TestApp::boot("netbox-credential-url", Options::default());
    let state = harness.state.clone();
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

    let (first, first_hits) = netbox_mock().await;
    let (second, second_hits) = netbox_mock().await;

    let (status, credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "NetBox address",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": first}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let credential_id = credential["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Credential address",
            "credential_id": credential_id,
            "query_filters": ["status=active"],
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["sources"][0]["credential_id"], credential_id);
    assert!(created["sources"][0]["source"]
        .get("api_endpoint")
        .is_none());
    assert!(!created.to_string().contains("secret-token"));
    assert!(!created.to_string().contains("secret_blob"));
    assert_eq!(
        first_hits.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "{created}"
    );
    let inventory_id = created["id"].as_str().unwrap().to_string();
    let source_id = created["sources"][0]["id"].as_str().unwrap().to_string();

    let (status, rotated, _) = call(
        &app,
        "PUT",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "NetBox address", "secret": {"url": second}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rotated}");
    assert_eq!(rotated["fields"]["url"], second);
    assert!(!rotated.to_string().contains("secret-token"));

    let (status, synced, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}/sync"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    assert_eq!(
        second_hits.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "{synced}"
    );
    assert_eq!(
        first_hits.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "{synced}"
    );

    {
        let db = state.db().unwrap();
        let source_json: String = db
            .query_row(
                "SELECT source_json FROM inventory_sources WHERE id = ?1",
                [&source_id],
                |row| row.get(0),
            )
            .unwrap();
        let mut value: Value = serde_json::from_str(&source_json).unwrap();
        value["api_endpoint"] = json!(first);
        db.execute(
            "UPDATE inventory_sources SET source_json = ?2 WHERE id = ?1",
            rusqlite::params![source_id, value.to_string()],
        )
        .unwrap();
    }
    let (status, legacy, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}/sync"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{legacy}");
    assert_eq!(
        second_hits.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "{legacy}"
    );
    assert_eq!(
        first_hits.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "{legacy}"
    );

    let (status, mismatch, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Wrong address",
            "credential_id": credential_id,
            "api_endpoint": first,
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{mismatch}");
    assert_eq!(mismatch["error"], "netbox_endpoint_must_match_credential");
    assert_eq!(
        second_hits.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "{mismatch}"
    );

    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let before = listed.as_array().unwrap().len();
    let (status, token_only, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Token shortcut",
            "api_endpoint": second,
            "token": "secret-token",
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{token_only}");
    assert_eq!(token_only["error"], "netbox_token_needs_a_credential");
    let (status, missing, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "No credential", "virtual_machines": false})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{missing}");
    assert_eq!(missing["error"], "netbox_credential_is_required");
    let (status, listed_again, _) = call(
        &app,
        "GET",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed_again}");
    assert_eq!(listed_again.as_array().unwrap().len(), before);
    assert!(!listed_again.to_string().contains("secret-token"));
    assert!(!listed_again.to_string().contains("secret_blob"));

    let (status, stored, _) = call(
        &app,
        "GET",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert!(!stored.to_string().contains("secret-token"));
    assert!(!stored.to_string().contains("secret_blob"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn netbox_credentials_without_a_url_cannot_reach_another_endpoint() {
    let harness = TestApp::boot("netbox-no-url", Options::default());
    let state = harness.state.clone();
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

    let (status, _, _) = call(
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

    let (first, first_hits) = netbox_mock().await;
    let (second, second_hits) = netbox_mock().await;

    let (status, legacy, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Legacy token",
            "kind": "github",
            "secret": {"token": "legacy-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{legacy}");
    let legacy_id = legacy["id"].as_str().unwrap().to_string();
    {
        let db = state.db().unwrap();
        let changed = db
            .execute(
                "UPDATE credentials SET kind = 'netbox' WHERE id = ?1",
                [&legacy_id],
            )
            .unwrap();
        assert_eq!(changed, 1);
    }

    let (status, missing, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "No address",
            "kind": "netbox",
            "secret": {"token": "secret-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{missing}");
    assert_eq!(missing["error"], "netbox_needs_an_address");

    let (status, valid, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Valid NetBox",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": first}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{valid}");
    let valid_id = valid["id"].as_str().unwrap().to_string();

    let (status, cleared, _) = call(
        &app,
        "PUT",
        &format!("/api/credentials/{valid_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Valid NetBox", "secret": {"url": ""}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{cleared}");
    assert_eq!(cleared["error"], "netbox_needs_an_address");

    let (status, denied_create, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&operator_cookie),
        Some(&team),
        Some(json!({
            "name": "Foreign NetBox",
            "sources": [{
                "kind": "netbox",
                "credential_id": legacy_id,
                "api_endpoint": second
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied_create}");
    assert_eq!(denied_create["error"], "netbox_needs_an_address");

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&operator_cookie),
        Some(&team),
        Some(json!({
            "name": "Bound NetBox",
            "sources": [{
                "kind": "netbox",
                "credential_id": valid_id,
                "api_endpoint": first,
                "virtual_machines": false
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(
        first_hits.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "{created}"
    );
    let inventory_id = created["id"].as_str().unwrap().to_string();
    let source_id = created["sources"][0]["id"].as_str().unwrap().to_string();

    let (status, denied_add, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources"),
        Some(&operator_cookie),
        Some(&team),
        Some(json!({
            "kind": "netbox",
            "credential_id": legacy_id,
            "api_endpoint": second
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied_add}");
    assert_eq!(denied_add["error"], "netbox_needs_an_address");

    let (status, denied_update, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&operator_cookie),
        Some(&team),
        Some(json!({"credential_id": legacy_id, "api_endpoint": second})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied_update}");
    assert_eq!(denied_update["error"], "netbox_needs_an_address");

    let (status, denied_test, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox/test",
        Some(&operator_cookie),
        Some(&team),
        Some(json!({"credential_id": legacy_id, "api_endpoint": second})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied_test}");
    assert_eq!(denied_test["error"], "netbox_needs_an_address");
    assert_eq!(
        second_hits.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "{denied_test}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn netbox_credentials_without_a_source_are_disabled() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-netbox-disabled-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir, &Options::default())).unwrap();
    let app = koreograf::router(state.clone());

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

    let (status, legacy, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Orphan token",
            "kind": "github",
            "secret": {"token": "orphan-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{legacy}");
    let legacy_id = legacy["id"].as_str().unwrap().to_string();
    {
        let db = state.db().unwrap();
        db.execute(
            "UPDATE credentials SET kind = 'netbox' WHERE id = ?1",
            [&legacy_id],
        )
        .unwrap();
    }
    drop(app);
    drop(state);

    let state = koreograf::boot(config(&dir, &Options::default())).unwrap();
    let app = koreograf::router(state.clone());
    {
        let db = state.db().unwrap();
        let disabled: i64 = db
            .query_row(
                "SELECT disabled FROM credentials WHERE id = ?1",
                [&legacy_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(disabled, 1);
    }

    let (status, _, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("session cookie");
    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(!listed
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["id"] == legacy_id));

    let (status, denied, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox/test",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "credential_id": legacy_id,
            "api_endpoint": "http://127.0.0.1:9"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");
    assert_eq!(denied["error"], "netbox_credential_is_disabled");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn netbox_source_update_can_switch_the_credential() {
    let harness = TestApp::boot("netbox-switch", Options::default());
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

    let (status, _, _) = call(
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

    let (first, first_seen) = netbox_mock_recording().await;
    let (second, second_seen) = netbox_mock_recording().await;

    let (status, shared, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Shared NetBox",
            "kind": "netbox",
            "secret": {"token": "shared-token", "url": first}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{shared}");
    let shared_id = shared["id"].as_str().unwrap().to_string();

    let (status, private, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Private NetBox",
            "kind": "netbox",
            "secret": {"token": "private-token", "url": second}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{private}");
    let private_id = private["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Switch source",
            "sources": [{
                "kind": "netbox",
                "credential_id": shared_id,
                "virtual_machines": false
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(
        first_seen.lock().unwrap().last().map(String::as_str),
        Some("Token shared-token")
    );
    let inventory_id = created["id"].as_str().unwrap().to_string();
    let source_id = created["sources"][0]["id"].as_str().unwrap().to_string();

    let (status, edited, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&operator_cookie),
        Some(&team),
        Some(json!({"credential_id": private_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["sources"][0]["credential_id"], private_id);
    assert_eq!(
        second_seen.lock().unwrap().last().map(String::as_str),
        Some("Token private-token")
    );
    assert!(!edited.to_string().contains("private-token"));
    assert!(!edited.to_string().contains("secret_blob"));

    let (status, stored, _) = call(
        &app,
        "GET",
        &format!("/api/credentials/{shared_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_eq!(stored["fields"]["url"], first);
    assert!(!stored.to_string().contains("shared-token"));

    let (status, denied, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&operator_cookie),
        Some(&team),
        Some(json!({"validate_certs": false})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    assert_eq!(denied["error"], "you_are_not_allowed_to_do_this");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn netbox_sources_share_a_normal_credential() {
    let harness = TestApp::boot("netbox-shared", Options::default());
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

    let (addr, seen) = netbox_mock_recording().await;
    let (status, credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "NetBox Prod",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": addr}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let credential_id = credential["id"].as_str().unwrap().to_string();

    let (status, first, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "NetBox shared",
            "sources": [{
                "kind": "netbox",
                "credential_id": credential_id,
                "query_filters": ["status=active"],
                "virtual_machines": false
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(first["sources"][0]["credential_id"], credential_id);
    let first_id = first["id"].as_str().unwrap().to_string();
    let first_source = first["sources"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(
        seen.lock().unwrap().last().map(String::as_str),
        Some("Token secret-token")
    );

    let (status, second, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "NetBox second",
            "sources": [{
                "kind": "netbox",
                "credential_id": credential_id,
                "api_endpoint": addr,
                "query_filters": ["status=active"],
                "virtual_machines": false
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    let second_id = second["id"].as_str().unwrap().to_string();
    assert_eq!(
        seen.lock().unwrap().last().map(String::as_str),
        Some("Token secret-token")
    );

    let (status, rotated, _) = call(
        &app,
        "PUT",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "NetBox Prod", "secret": {"token": "rotated-token"}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rotated}");
    assert!(!rotated.to_string().contains("rotated-token"));
    for id in [&first_id, &second_id] {
        let (status, synced, _) = call(
            &app,
            "POST",
            &format!("/api/inventories/{id}/sync"),
            Some(&cookie),
            Some(&team),
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{synced}");
        assert!(synced["hosts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "web-1"));
        assert_eq!(
            seen.lock().unwrap().last().map(String::as_str),
            Some("Token rotated-token")
        );
    }

    let (status, ssh, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Not NetBox",
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
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Wrong kind",
            "sources": [{
                "kind": "netbox",
                "credential_id": ssh_id,
                "api_endpoint": addr
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{wrong_kind}");
    assert_eq!(wrong_kind["error"], "netbox_credential_has_the_wrong_kind");

    let (status, other_team, _) = call(
        &app,
        "POST",
        "/api/teams",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Other NetBox team"})),
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
            "name": "Foreign NetBox",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": addr}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{foreign}");
    let foreign_id = foreign["id"].as_str().unwrap();
    let (status, wrong_team, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Wrong team",
            "sources": [{
                "kind": "netbox",
                "credential_id": foreign_id,
                "api_endpoint": addr
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{wrong_team}");
    assert_eq!(wrong_team["error"], "not_found");

    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let row = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == credential_id)
        .expect("shared credential");
    assert_eq!(row["usages"].as_array().unwrap().len(), 2, "{row}");
    assert!(row["usages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|usage| usage["inventory_name"] == "NetBox shared"));

    let (status, refused, _) = call(
        &app,
        "DELETE",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["error"], "credential_is_in_use");
    let names = refused["vars"]["names"].as_str().unwrap_or("");
    assert!(names.contains("NetBox shared / netbox"), "{names}");
    assert!(names.contains("NetBox second / netbox"), "{names}");

    let (status, synced, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{first_id}/sources/{first_source}/sync"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    assert_eq!(
        seen.lock().unwrap().last().map(String::as_str),
        Some("Token rotated-token")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn netbox_automatic_credentials_become_normal_and_get_their_url() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-netbox-migrate-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let state = koreograf::boot(config(&dir, &Options::default())).unwrap();
    let app = koreograf::router(state.clone());

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

    let (addr, _) = netbox_mock_recording().await;
    let (status, credential, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Legacy NetBox Token",
            "kind": "github",
            "secret": {"token": "secret-token"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let credential_id = credential["id"].as_str().unwrap().to_string();
    let inventory_id = format!("legacy-inventory-{nanos}");
    let source_id = format!("legacy-source-{nanos}");
    let source_json = json!({
        "api_endpoint": addr,
        "validate_certs": true,
        "ca_file": "/no/such/koreograf-ca.pem",
        "group_by": [],
        "query": [["status", "active"]],
        "virtual_machines": false
    })
    .to_string();
    let created_at = "2024-01-01T00:00:00Z";

    {
        let db = state.db().unwrap();
        let changed = db
            .execute(
                "UPDATE credentials SET kind = 'netbox', auto = 1 WHERE id = ?1",
                [&credential_id],
            )
            .unwrap();
        assert_eq!(changed, 1);
        db.execute(
            "INSERT INTO inventories (id, team_id, name, body, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            [
                inventory_id.as_str(),
                team.as_str(),
                "Legacy NetBox",
                "",
                created_at,
            ],
        )
        .unwrap();
        db.execute(
            "INSERT INTO inventory_sources (
                id, inventory_id, name, kind, source_json, credential_id, body, position, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            [
                source_id.as_str(),
                inventory_id.as_str(),
                "Legacy NetBox",
                "netbox",
                source_json.as_str(),
                credential_id.as_str(),
                "",
                "0",
                created_at,
            ],
        )
        .unwrap();
        db.execute(
            "DELETE FROM schema_meta WHERE key = 'legacy_reconciled'",
            [],
        )
        .unwrap();
    }
    drop(app);
    drop(state);

    let state = koreograf::boot(config(&dir, &Options::default())).unwrap();
    let app = koreograf::router(state.clone());

    {
        let db = state.db().unwrap();
        let automatic: i64 = db
            .query_row(
                "SELECT auto FROM credentials WHERE id = ?1",
                [&credential_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(automatic, 0);
    }

    let (status, _, cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("session cookie");
    let (status, stored, _) = call(
        &app,
        "GET",
        &format!("/api/credentials/{credential_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_eq!(stored["name"], "Legacy NetBox Token");
    assert_eq!(stored["fields"]["url"], addr);
    assert_eq!(stored["usages"][0]["inventory_id"], inventory_id);

    let (status, synced, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}/sync"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    assert!(synced["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "web-1"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn netbox_test_connection_reports_the_version_or_a_mapped_error() {
    let harness = TestApp::boot("netbox-test", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route("/api/status/", axum::routing::get(nb_status));
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });

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

    let good = create_credential(
        &app,
        &cookie,
        &team,
        "Test NetBox",
        "netbox",
        json!({"token": "secret-token", "url": format!("http://{addr}")}),
    )
    .await;
    let (status, ok, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox/test",
        Some(&cookie),
        Some(&team),
        Some(json!({"credential_id": good})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ok}");
    assert_eq!(ok["version"], "4.2.1", "{ok}");

    let denied = create_credential(
        &app,
        &cookie,
        &team,
        "Wrong NetBox token",
        "netbox",
        json!({"token": "wrong-token", "url": format!("http://{addr}")}),
    )
    .await;
    let (status, denied, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox/test",
        Some(&cookie),
        Some(&team),
        Some(json!({"credential_id": denied})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");
    assert_eq!(denied["error"], "netbox.token_rejected", "{denied}");
    assert_eq!(denied["vars"]["status"], "403", "{denied}");

    let missing = create_credential(
        &app,
        &cookie,
        &team,
        "Wrong NetBox path",
        "netbox",
        json!({"token": "secret-token", "url": format!("http://{addr}/wrong")}),
    )
    .await;
    let (status, missing, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox/test",
        Some(&cookie),
        Some(&team),
        Some(json!({"credential_id": missing})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{missing}");
    assert_eq!(missing["error"], "netbox.address_not_found", "{missing}");
    assert_eq!(missing["vars"]["status"], "404", "{missing}");

    let unreachable = create_credential(
        &app,
        &cookie,
        &team,
        "Unreachable NetBox",
        "netbox",
        json!({"token": "secret-token", "url": "http://127.0.0.1:9"}),
    )
    .await;
    let (status, unreachable, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox/test",
        Some(&cookie),
        Some(&team),
        Some(json!({"credential_id": unreachable})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{unreachable}");
    assert_eq!(
        unreachable["error"], "netbox_is_not_reachable_err",
        "{unreachable}"
    );

    let (status, _, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        None,
        Some(json!({
            "username": "watcher",
            "password": "watcher-pass",
            "display_name": "Watcher"
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
        Some(json!({"username": "watcher", "role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, viewer_cookie) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": "watcher", "password": "watcher-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let viewer_cookie = viewer_cookie.expect("viewer cookie");
    let (status, forbidden, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox/test",
        Some(&viewer_cookie),
        Some(&team),
        Some(json!({"credential_id": good})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{forbidden}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn deleting_an_inventory_keeps_its_shortcut_netbox_credential() {
    let harness = TestApp::boot("netbox-cred", Options::default());
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

    let (status, manual, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Unrelated NetBox Token",
            "kind": "netbox",
            "secret": {"token": "keep-me", "url": "http://127.0.0.1:9"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{manual}");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route("/api/dcim/devices/", axum::routing::get(nb_devices));
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });

    let (status, prod, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Prod NB Token",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": format!("http://{addr}")}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{prod}");
    let prod_id = prod["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Prod NB",
            "credential_id": prod_id,
            "query_filters": ["status=active"],
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let netbox_id = created["id"].as_str().unwrap();

    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let names = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"Prod NB Token"), "{names:?}");
    assert!(names.contains(&"Unrelated NetBox Token"), "{names:?}");

    let (status, deleted, _) = call(
        &app,
        "DELETE",
        &format!("/api/inventories/{netbox_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{deleted}");

    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let names = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"Prod NB Token"), "{names:?}");
    assert!(names.contains(&"Unrelated NetBox Token"), "{names:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn deleting_a_used_inventory_is_refused_with_the_template_names() {
    let harness = TestApp::boot("inv-in-use", Options::default());
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

    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Prod", "body": "[web]\nweb-1\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap();

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Ping",
            "tool": "ansible",
            "entrypoint": "ping.yml",
            "inventory_id": inventory_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap();

    let (status, refused, _) = call(
        &app,
        "DELETE",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["error"], "inventory_is_in_use");
    assert!(
        refused["vars"]["names"].as_str().unwrap().contains("Ping"),
        "{refused}"
    );

    let (status, still_there, _) = call(
        &app,
        "GET",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{still_there}");

    let (status, detached, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Ping",
            "tool": "ansible",
            "entrypoint": "ping.yml",
            "inventory_id": ""
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detached}");

    let (status, deleted, _) = call(
        &app,
        "DELETE",
        &format!("/api/inventories/{inventory_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{deleted}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn deleting_an_inventory_keeps_shortcut_credentials_from_sources() {
    let harness = TestApp::boot("src-cred", Options::default());
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

    let (status, manual, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Unrelated NetBox Token",
            "kind": "netbox",
            "secret": {"token": "keep-me", "url": "http://127.0.0.1:9"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{manual}");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route("/api/dcim/devices/", axum::routing::get(nb_devices));
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });

    let (status, prod, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Prod NB Token",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": format!("http://{addr}")}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{prod}");
    let prod_id = prod["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Prod NB",
            "sources": [{
                "kind": "netbox",
                "name": "Prod NB",
                "credential_id": prod_id,
                "query_filters": ["status=active"],
                "virtual_machines": false
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let netbox_id = created["id"].as_str().unwrap();

    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let names = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"Prod NB Token"), "{names:?}");
    assert!(names.contains(&"Unrelated NetBox Token"), "{names:?}");

    let (status, deleted, _) = call(
        &app,
        "DELETE",
        &format!("/api/inventories/{netbox_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{deleted}");

    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let names = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"Prod NB Token"), "{names:?}");
    assert!(names.contains(&"Unrelated NetBox Token"), "{names:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_broken_ini_inventory_is_rejected_with_the_line_number() {
    let harness = TestApp::boot("bad-ini", Options::default());
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

    let (status, broken, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Broken INI", "body": "[all\nh1 ansible_host=\n[[["})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{broken}");
    assert_eq!(broken["error"], "inventory_ini_is_invalid");
    assert_eq!(broken["vars"]["line"], "1");

    let (status, valid, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Good INI", "body": "[web]\nweb-1 ansible_host=10.0.0.5\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{valid}");
    let valid_id = valid["id"].as_str().unwrap();

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{valid_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Good INI", "body": "[web]\n[db\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{updated}");
    assert_eq!(updated["error"], "inventory_ini_is_invalid");
    assert_eq!(updated["vars"]["line"], "2");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn broken_ini_bodies_are_rejected_on_the_source_endpoints() {
    let harness = TestApp::boot("bad-ini-sources", Options::default());
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

    let broken = "[all\nh1 ansible_host=\n[[[";

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Broken sources", "sources": [{"kind": "ini", "body": broken}]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{created}");
    assert_eq!(created["error"], "inventory_ini_is_invalid");
    assert_eq!(created["vars"]["line"], "1");

    let (status, good, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Good sources", "sources": [{"kind": "ini", "body": "[web]\nweb-1 ansible_host=10.0.0.5\n"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{good}");
    let inventory_id = good["id"].as_str().unwrap();
    let source_id = good["sources"][0]["id"].as_str().unwrap();

    let (status, added, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources"),
        Some(&cookie),
        Some(&team),
        Some(json!({"kind": "ini", "body": broken})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{added}");
    assert_eq!(added["error"], "inventory_ini_is_invalid");
    assert_eq!(added["vars"]["line"], "1");

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/inventories/{inventory_id}/sources/{source_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"body": broken})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{updated}");
    assert_eq!(updated["error"], "inventory_ini_is_invalid");
    assert_eq!(updated["vars"]["line"], "1");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn deleting_an_inventory_keeps_shortcut_credentials_referenced_in_workflows() {
    let harness = TestApp::boot("cred-guard", Options::default());
    let state = harness.state.clone();
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

    let (status, manual, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Unrelated NetBox Token",
            "kind": "netbox",
            "secret": {"token": "keep-me", "url": "http://127.0.0.1:9"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{manual}");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route("/api/dcim/devices/", axum::routing::get(nb_devices));
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });

    let (status, prod, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Prod NB Token",
            "kind": "netbox",
            "secret": {"token": "secret-token", "url": format!("http://{addr}")}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{prod}");
    let prod_id = prod["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Prod NB",
            "sources": [{
                "kind": "netbox",
                "name": "Prod NB",
                "credential_id": prod_id.clone(),
                "query_filters": ["status=active"],
                "virtual_machines": false
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let netbox_id = created["id"].as_str().unwrap();
    {
        let db = state.db().unwrap();
        db.execute("UPDATE credentials SET auto = 1 WHERE id = ?1", [&prod_id])
            .unwrap();
    }

    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let auto_credential = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Prod NB Token")
        .expect("auto credential")["id"]
        .as_str()
        .unwrap()
        .to_string();

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
        .expect("Demo project")["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, plain, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Testsysteme", "body": "test-1 ansible_connection=local\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{plain}");
    let plain_id = plain["id"].as_str().unwrap().to_string();

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "patch-server",
            "tool": "ansible",
            "project_id": demo,
            "inventory_id": plain_id,
            "entrypoint": "site.yml",
            "spec": {"job_type": "run", "limit": "test", "forks": 5}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();

    let (status, workflow, _) = call(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Guard",
            "graph": {
                "nodes": [{
                    "id": "test",
                    "template_id": template_id,
                    "name": "test",
                    "x": 40,
                    "y": 40,
                    "convergence": "any",
                    "overrides": {"credential_id": auto_credential}
                }],
                "edges": []
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");

    let (status, deleted, _) = call(
        &app,
        "DELETE",
        &format!("/api/inventories/{netbox_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{deleted}");

    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let names = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"Prod NB Token"), "{names:?}");
    assert!(names.contains(&"Unrelated NetBox Token"), "{names:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn inventory_sources_and_ansible_check_options() {
    let harness = TestApp::boot("mix", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, _, _) = call(&app, "GET", "/js/main.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = web_source();
    assert!(script.contains("id=\"inventory-search\""));
    assert!(script.contains("inventory_2"));
    assert!(script.contains("project_2"));
    assert!(script.contains("credential"));
    assert!(script.contains("environment"));
    assert!(script.contains("data-lang-select"));
    assert!(script.contains("FLAG_SVG"));
    assert!(script.contains("country: \"gb\""));
    assert!(script.contains("data-add-kind"));
    assert!(script.contains("check_dry_run"));
    assert!(script.contains("skip_tags"));
    assert!(script.contains("0_normal"));
    assert!(script.contains("1_verbose"));
    assert!(script.contains("2_more_verbose"));
    assert!(script.contains("3_debug"));
    assert!(script.contains("4_connection_debug"));
    assert!(script.contains("data-chips"));
    assert!(script.contains("chip-entry"));
    let (status, style, _) = call(&app, "GET", "/app.css", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let style = style.as_str().unwrap_or("");
    assert!(style.contains(".chips"));
    assert!(style.contains(".chip-entry"));

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
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Gemischt",
            "sources": [
                {"kind": "ini", "name": "Extra", "body": "[web]\nweb-1 ansible_host=10.0.0.8\n"},
                {"kind": "project", "project_id": demo, "path": "inventory/hosts.ini"}
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["source_kind"], "mixed");
    assert_eq!(created["source_count"], 2);
    let names: Vec<&str> = created["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|host| host["name"].as_str())
        .collect();
    assert!(names.contains(&"web-1"), "{created}");
    assert!(names.contains(&"localhost"), "{created}");
    let inventory_id = created["id"].as_str().unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = axum::Router::new().route("/api/dcim/devices/", axum::routing::get(nb_devices));
    tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });
    let netbox_credential = create_credential(
        &app,
        &cookie,
        &team,
        "Mixed NetBox",
        "netbox",
        json!({"token": "secret-token", "url": format!("http://{addr}")}),
    )
    .await;
    let (status, with_netbox, _) = call(
        &app,
        "POST",
        &format!("/api/inventories/{inventory_id}/sources"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "kind": "netbox",
            "name": "DC",
            "credential_id": netbox_credential,
            "group_by": ["sites"],
            "query_filters": ["status=active"],
            "virtual_machines": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{with_netbox}");
    assert_eq!(with_netbox["source_count"], 3);
    assert!(with_netbox["body"].as_str().unwrap_or("").contains("web-1"));
    assert!(!with_netbox["body"]
        .as_str()
        .unwrap_or("")
        .contains("secret-token"));
    let ini_source = with_netbox["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["kind"] == "ini")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, dropped, _) = call(
        &app,
        "DELETE",
        &format!("/api/inventories/{inventory_id}/sources/{ini_source}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{dropped}");
    assert_eq!(dropped["source_count"], 2);
    assert!(dropped["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|host| host["name"] == "localhost"));

    let (status, template, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Dry-Run",
            "tool": "ansible",
            "project_id": demo,
            "inventory_id": inventory_id,
            "entrypoint": "site.yml",
            "spec": {
                "job_type": "check",
                "tags": "ping",
                "skip_tags": "slow",
                "limit": "web:&db",
                "diff": true,
                "verbosity": 2,
                "forks": 5
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    assert_eq!(template["spec"]["job_type"], "check");
    assert_eq!(template["spec"]["tags"], "ping");
    assert_eq!(template["spec"]["limit"], "web:&db");
    let (status, rejected, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Kaputte Tags",
            "tool": "ansible",
            "project_id": demo,
            "entrypoint": "site.yml",
            "spec": {"tags": "-evil"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");

    let _ = std::fs::remove_dir_all(&dir);
}
