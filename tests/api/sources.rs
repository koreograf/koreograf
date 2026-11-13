use std::path::Path;

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::common::call_cookie as call;

use crate::common::*;

const PROJECT_INI: &str = "[db]\ndb-1 ansible_host=10.0.0.9\n";
const STATIC_INI: &str = "[web]\nweb-1 ansible_host=10.0.0.8\n";
const MIXED_INI: &str = "[web]\nweb-1 ansible_host=10.0.0.8\n[db]\ndb-1 ansible_host=10.0.0.9";

async fn admin(app: &axum::Router) -> (String, String) {
    let (status, me, cookie) = call(
        app,
        "POST",
        "/api/v1/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let cookie = cookie.expect("session cookie");
    let team = me["teams"][0]["id"].as_str().expect("team id").to_string();
    (cookie, team)
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("directory");
    std::fs::write(path, text).expect("file");
}

async fn path_project(app: &axum::Router, cookie: &str, team: &str, root: &Path) -> String {
    let (status, project, _) = call(
        app,
        "POST",
        "/api/v1/projects",
        Some(cookie),
        Some(team),
        Some(json!({
            "name": "Source shape project",
            "source_type": "path",
            "source": root.display().to_string(),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    project["id"].as_str().expect("project id").to_string()
}

fn host(name: &str, groups: Value, vars: Value) -> Value {
    json!({"name": name, "groups": groups, "vars": vars})
}

fn source_shape(
    value: &Value,
    name: &str,
    kind: &str,
    source: Value,
    credential_id: Value,
    host_count: usize,
    body: &str,
) -> Value {
    json!({
        "id": value["id"].clone(),
        "name": name,
        "kind": kind,
        "source": source,
        "credential_id": credential_id,
        "host_count": host_count,
        "body": body,
    })
}

#[allow(clippy::too_many_arguments)]
fn inventory_shape(
    value: &Value,
    team: &str,
    name: &str,
    body: &str,
    source_kind: &str,
    source: Value,
    credential_id: Value,
    sources: Vec<Value>,
    hosts: Vec<Value>,
    group_count: usize,
) -> Value {
    json!({
        "id": value["id"].clone(),
        "team_id": team,
        "name": name,
        "body": body,
        "created_at": value["created_at"].clone(),
        "source_kind": source_kind,
        "source": source,
        "credential_id": credential_id,
        "source_count": sources.len(),
        "sources": sources,
        "host_count": hosts.len(),
        "group_count": group_count,
        "hosts": hosts,
    })
}

fn with_extras(mut value: Value, extras: Value) -> Value {
    let extras = extras.as_object().expect("extras object");
    for (key, item) in extras {
        value[key] = item.clone();
    }
    value
}

#[tokio::test]
async fn project_contents_and_import_return_the_documented_shape() {
    let root = temp_dir("sources-project");
    write(&root.join("site.yml"), "- hosts: all\n");
    write(&root.join("inventory/hosts.ini"), PROJECT_INI);
    let options = Options {
        project_roots: vec![root.clone()],
        ..Options::default()
    };
    let harness = TestApp::boot("sources-contents", options);
    let app = harness.app.clone();
    let (cookie, team) = admin(&app).await;
    let project_id = path_project(&app, &cookie, &team, &root).await;

    let (status, contents, _) = call(
        &app,
        "GET",
        &format!("/api/v1/projects/{project_id}/contents"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{contents}");
    assert_eq!(
        contents,
        json!({
            "playbooks": ["site.yml"],
            "inventories": [{
                "path": "inventory/hosts.ini",
                "format": "ini",
                "plugin": null,
                "api_endpoint": null,
                "group_by": [],
                "query_filters": [],
                "validate_certs": null,
                "virtual_machines": null,
            }],
        })
    );

    let source = json!({"path": "inventory/hosts.ini", "project_id": project_id});
    let (status, imported, _) = call(
        &app,
        "POST",
        "/api/v1/inventories/import",
        Some(&cookie),
        Some(&team),
        Some(json!({"project_id": project_id, "path": "inventory/hosts.ini"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{imported}");
    let expected_source = source_shape(
        &imported["sources"][0],
        "hosts",
        "project",
        source.clone(),
        Value::Null,
        1,
        PROJECT_INI,
    );
    assert_eq!(
        imported,
        inventory_shape(
            &imported,
            &team,
            "hosts",
            PROJECT_INI,
            "project",
            source,
            Value::Null,
            vec![expected_source],
            vec![host(
                "db-1",
                json!(["db"]),
                json!({"ansible_host": "10.0.0.9"})
            )],
            1,
        )
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn source_endpoints_return_the_documented_shape() {
    let root = temp_dir("sources-project");
    write(&root.join("inventory/hosts.ini"), PROJECT_INI);
    let options = Options {
        project_roots: vec![root.clone()],
        ..Options::default()
    };
    let harness = TestApp::boot("sources-crud", options);
    let app = harness.app.clone();
    let (cookie, team) = admin(&app).await;
    let project_id = path_project(&app, &cookie, &team, &root).await;

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/v1/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Sources shape",
            "sources": [{"kind": "ini", "name": "static", "body": STATIC_INI}],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let inventory_id = created["id"].as_str().expect("inventory id").to_string();
    let static_source = created["sources"][0]["id"]
        .as_str()
        .expect("static source id")
        .to_string();

    let project_source = json!({"path": "inventory/hosts.ini", "project_id": project_id});
    let (status, added, _) = call(
        &app,
        "POST",
        &format!("/api/v1/inventories/{inventory_id}/sources"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "kind": "project",
            "name": "project hosts",
            "project_id": project_id,
            "path": "inventory/hosts.ini",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{added}");
    let project_source_id = added["sources"][1]["id"]
        .as_str()
        .expect("project source id")
        .to_string();
    let static_shape = source_shape(
        &added["sources"][0],
        "static",
        "ini",
        json!({}),
        Value::Null,
        1,
        STATIC_INI,
    );
    let project_shape = source_shape(
        &added["sources"][1],
        "project hosts",
        "project",
        project_source.clone(),
        Value::Null,
        1,
        PROJECT_INI,
    );
    assert_eq!(
        added,
        inventory_shape(
            &added,
            &team,
            "Sources shape",
            MIXED_INI,
            "mixed",
            json!({}),
            Value::Null,
            vec![static_shape.clone(), project_shape.clone()],
            vec![
                host("web-1", json!(["web"]), json!({"ansible_host": "10.0.0.8"})),
                host("db-1", json!(["db"]), json!({"ansible_host": "10.0.0.9"})),
            ],
            2,
        )
    );

    let (status, fetched, _) = call(
        &app,
        "GET",
        &format!("/api/v1/inventories/{inventory_id}/sources/{project_source_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(
        fetched,
        with_extras(project_shape.clone(), json!({"inventory_id": inventory_id}))
    );

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/v1/inventories/{inventory_id}/sources/{project_source_id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "renamed project"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    let renamed_shape = source_shape(
        &updated["sources"][1],
        "renamed project",
        "project",
        project_source,
        Value::Null,
        1,
        PROJECT_INI,
    );
    assert_eq!(
        updated,
        with_extras(
            inventory_shape(
                &updated,
                &team,
                "Sources shape",
                MIXED_INI,
                "mixed",
                json!({}),
                Value::Null,
                vec![static_shape.clone(), renamed_shape.clone()],
                vec![
                    host("web-1", json!(["web"]), json!({"ansible_host": "10.0.0.8"})),
                    host("db-1", json!(["db"]), json!({"ansible_host": "10.0.0.9"})),
                ],
                2,
            ),
            json!({
                "sync": {"ok": true, "before": 2, "after": 2},
                "collisions": [],
                "dropped_vars": [],
            }),
        )
    );

    let (status, synced_source, _) = call(
        &app,
        "POST",
        &format!("/api/v1/inventories/{inventory_id}/sources/{project_source_id}/sync"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced_source}");
    assert_eq!(
        synced_source,
        with_extras(
            inventory_shape(
                &synced_source,
                &team,
                "Sources shape",
                MIXED_INI,
                "mixed",
                json!({}),
                Value::Null,
                vec![static_shape.clone(), renamed_shape.clone()],
                vec![
                    host("web-1", json!(["web"]), json!({"ansible_host": "10.0.0.8"})),
                    host("db-1", json!(["db"]), json!({"ansible_host": "10.0.0.9"})),
                ],
                2,
            ),
            json!({"collisions": [], "dropped_vars": []}),
        )
    );

    let (status, synced, _) = call(
        &app,
        "POST",
        &format!("/api/v1/inventories/{inventory_id}/sync"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    assert_eq!(
        synced,
        with_extras(
            inventory_shape(
                &synced,
                &team,
                "Sources shape",
                MIXED_INI,
                "mixed",
                json!({}),
                Value::Null,
                vec![static_shape, renamed_shape],
                vec![
                    host("web-1", json!(["web"]), json!({"ansible_host": "10.0.0.8"})),
                    host("db-1", json!(["db"]), json!({"ansible_host": "10.0.0.9"})),
                ],
                2,
            ),
            json!({"collisions": [], "dropped_vars": []}),
        )
    );

    let (status, deleted, _) = call(
        &app,
        "DELETE",
        &format!("/api/v1/inventories/{inventory_id}/sources/{static_source}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    let remaining = source_shape(
        &deleted["sources"][0],
        "renamed project",
        "project",
        json!({"path": "inventory/hosts.ini", "project_id": project_id}),
        Value::Null,
        1,
        PROJECT_INI,
    );
    assert_eq!(
        deleted,
        inventory_shape(
            &deleted,
            &team,
            "Sources shape",
            PROJECT_INI,
            "project",
            json!({"path": "inventory/hosts.ini", "project_id": project_id}),
            Value::Null,
            vec![remaining],
            vec![host(
                "db-1",
                json!(["db"]),
                json!({"ansible_host": "10.0.0.9"})
            )],
            1,
        )
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn netbox_endpoints_return_the_documented_shape() {
    let harness = TestApp::boot("sources-netbox", Options::default());
    let app = harness.app.clone();
    let (cookie, team) = admin(&app).await;
    let (endpoint, _) = netbox_mock().await;

    let (status, credential, _) = call(
        &app,
        "POST",
        "/api/v1/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "NetBox shape",
            "kind": "netbox",
            "secret": {"token": "shape-token", "url": endpoint},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential}");
    let credential_id = credential["id"]
        .as_str()
        .expect("credential id")
        .to_string();

    let (status, tested, _) = call(
        &app,
        "POST",
        "/api/v1/inventories/netbox/test",
        Some(&cookie),
        Some(&team),
        Some(json!({"credential_id": credential_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{tested}");
    assert_eq!(tested, json!({"version": "4.2.1"}));

    let (status, preview, _) = call(
        &app,
        "POST",
        "/api/v1/inventories/netbox/preview",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "credential_id": credential_id,
            "group_by": ["sites"],
            "query_filters": ["status=active"],
            "virtual_machines": false,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(
        preview,
        json!({
            "host_count": 1,
            "groups": [{"name": "sites_berlin", "host_count": 1}],
            "sample_host": {
                "name": "web-1",
                "vars": {"ansible_host": "10.0.0.8", "netbox_id": "1"},
            },
            "collisions": [],
            "dropped_vars": [],
        })
    );

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/v1/inventories/netbox",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "NetBox shape",
            "credential_id": credential_id,
            "group_by": ["sites"],
            "query_filters": ["status=active"],
            "virtual_machines": false,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let body =
        "[all]\nweb-1 ansible_host=10.0.0.8 netbox_id=1\n\n[sites_berlin]\nweb-1\n".to_string();
    let source = json!({
        "validate_certs": true,
        "group_by": ["sites"],
        "group_names_raw": false,
        "load_config_context": false,
        "max_hosts": 2000,
        "query": [["status", "active"]],
        "virtual_machines": false,
        "collisions": [],
    });
    let expected_source = source_shape(
        &created["sources"][0],
        "netbox",
        "netbox",
        source.clone(),
        json!(credential_id),
        1,
        &body,
    );
    assert_eq!(
        created,
        inventory_shape(
            &created,
            &team,
            "NetBox shape",
            &body,
            "netbox",
            source,
            json!(credential_id),
            vec![expected_source],
            vec![host(
                "web-1",
                json!(["all", "sites_berlin"]),
                json!({"ansible_host": "10.0.0.8", "netbox_id": "1"}),
            )],
            1,
        )
    );
}
