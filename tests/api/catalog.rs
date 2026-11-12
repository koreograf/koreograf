use axum::http::StatusCode;
use serde_json::json;

use crate::common::call_cookie as call;

use crate::common::*;

#[tokio::test]
async fn teams_and_organizations_return_the_documented_shape() {
    let harness = TestApp::boot("catalog-teams", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, login, cookie) = call(
        &app,
        "POST",
        "/api/v1/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{login}");
    let cookie = cookie.expect("session cookie");
    let admin_id = login["id"].clone();
    let team_id = login["teams"][0]["id"].clone();
    let org_id = login["teams"][0]["organization_id"].clone();

    let (status, teams, _) = call(&app, "GET", "/api/v1/teams", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{teams}");
    let team_created_at = teams[0]["created_at"].clone();
    assert_eq!(
        teams,
        json!([{
            "created_at": team_created_at.clone(),
            "description": "Sample team. An LDAP group can later use this group name.",
            "id": team_id.clone(),
            "ldap_group": null,
            "name": "Plattform",
            "organization_id": org_id.clone(),
            "organization_name": "Plattform",
            "role": "owner"
        }])
    );

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/v1/teams",
        Some(&cookie),
        None,
        Some(json!({"name": "Shape Team", "description": "Shape"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let shape_team = json!({
        "created_at": created["created_at"].clone(),
        "description": "Shape",
        "id": created["id"].clone(),
        "ldap_group": null,
        "name": "Shape Team",
        "organization_id": created["organization_id"].clone(),
        "organization_name": "Shape Team"
    });
    assert_eq!(created, shape_team);
    assert!(created["id"].is_string());
    assert!(created["organization_id"].is_string());

    let (status, updated, _) = call(
        &app,
        "PUT",
        &format!("/api/v1/teams/{}", created["id"].as_str().expect("team id")),
        Some(&cookie),
        None,
        Some(json!({"name": "Shape Team 2", "description": "Shape"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    let mut expected_team = shape_team.clone();
    expected_team["name"] = json!("Shape Team 2");
    assert_eq!(updated, expected_team);

    let (status, members, _) = call(
        &app,
        "GET",
        &format!(
            "/api/v1/teams/{}/members",
            created["id"].as_str().expect("team id")
        ),
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{members}");
    assert_eq!(
        members,
        json!([{
            "display_name": "Administrator",
            "role": "owner",
            "source": "manual",
            "user_id": admin_id.clone(),
            "username": "admin"
        }])
    );

    let (status, member_user, _) = call(
        &app,
        "POST",
        "/api/v1/users",
        Some(&cookie),
        None,
        Some(json!({
            "username": "shape-member",
            "display_name": "Shape Member",
            "password": "shape-secret-1",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{member_user}");
    let member_id = member_user["id"].clone();

    let (status, added, _) = call(
        &app,
        "POST",
        &format!(
            "/api/v1/teams/{}/members",
            created["id"].as_str().expect("team id")
        ),
        Some(&cookie),
        None,
        Some(json!({"username": "shape-member", "role": "operator"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{added}");
    assert_eq!(
        added,
        json!({"user_id": member_id.clone(), "role": "operator"})
    );

    let (status, members, _) = call(
        &app,
        "GET",
        &format!(
            "/api/v1/teams/{}/members",
            created["id"].as_str().expect("team id")
        ),
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{members}");
    assert_eq!(
        members,
        json!([
            {
                "display_name": "Administrator",
                "role": "owner",
                "source": "manual",
                "user_id": admin_id.clone(),
                "username": "admin"
            },
            {
                "display_name": "Shape Member",
                "role": "operator",
                "source": "manual",
                "user_id": member_id.clone(),
                "username": "shape-member"
            }
        ])
    );

    let (status, removed, _) = call(
        &app,
        "DELETE",
        &format!(
            "/api/v1/teams/{}/members/{}",
            created["id"].as_str().expect("team id"),
            member_id.as_str().expect("member id")
        ),
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{removed}");

    let (status, orgs, _) = call(
        &app,
        "GET",
        "/api/v1/organizations",
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{orgs}");
    let plattform = orgs
        .as_array()
        .expect("organizations")
        .iter()
        .find(|item| item["id"] == org_id)
        .expect("Plattform organization");
    let shape_org = orgs
        .as_array()
        .expect("organizations")
        .iter()
        .find(|item| item["id"] == created["organization_id"])
        .expect("Shape organization");
    let shape_org_created_at = shape_org["created_at"].clone();
    assert_eq!(
        orgs,
        json!([
            {
                "created_at": plattform["created_at"].clone(),
                "description": "Demo organization for the sample team.",
                "id": org_id.clone(),
                "name": "Plattform"
            },
            {
                "created_at": shape_org_created_at.clone(),
                "description": "",
                "id": created["organization_id"].clone(),
                "name": "Shape Team"
            }
        ])
    );

    let (status, organization, _) = call(
        &app,
        "GET",
        &format!(
            "/api/v1/organizations/{}",
            created["organization_id"].as_str().expect("org id")
        ),
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{organization}");
    assert_eq!(
        organization,
        json!({
            "created_at": shape_org_created_at,
            "description": "",
            "id": created["organization_id"].clone(),
            "name": "Shape Team",
            "teams": [{
                "created_at": expected_team["created_at"].clone(),
                "description": "Shape",
                "id": expected_team["id"].clone(),
                "ldap_group": null,
                "name": "Shape Team 2",
                "organization_id": expected_team["organization_id"].clone(),
                "organization_name": "Shape Team"
            }]
        })
    );

    let (status, renamed, _) = call(
        &app,
        "PUT",
        &format!(
            "/api/v1/organizations/{}",
            created["organization_id"].as_str().expect("org id")
        ),
        Some(&cookie),
        None,
        Some(json!({"name": "Shape Org", "description": "Shape org"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(
        renamed,
        json!({
            "created_at": organization["created_at"].clone(),
            "description": "Shape org",
            "id": organization["id"].clone(),
            "name": "Shape Org"
        })
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn catalog_data_endpoints_return_the_documented_shape() {
    let harness = TestApp::boot("catalog-data", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, login, cookie) = call(
        &app,
        "POST",
        "/api/v1/login",
        None,
        None,
        Some(json!({"username": "admin", "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{login}");
    let cookie = cookie.expect("session cookie");
    let team_id = login["teams"][0]["id"].clone();
    let org_id = login["teams"][0]["organization_id"].clone();
    let examples = format!("{}/examples/demo", env!("CARGO_MANIFEST_DIR"));

    let (status, projects, _) = call(
        &app,
        "GET",
        "/api/v1/projects",
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{projects}");
    let demo_id = projects[0]["id"].clone();
    let demo = json!({
        "created_at": projects[0]["created_at"].clone(),
        "credential_id": null,
        "id": demo_id.clone(),
        "last_sync_at": "",
        "last_sync_commit": "",
        "last_sync_status": "",
        "name": "Demo",
        "organization_id": org_id.clone(),
        "organization_name": "Plattform",
        "revision": "",
        "source": examples.clone(),
        "source_type": "path",
        "team_id": team_id.clone(),
        "team_ids": []
    });
    assert_eq!(projects, json!([demo.clone()]));

    let (status, created_project, _) = call(
        &app,
        "POST",
        "/api/v1/projects",
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        Some(json!({
            "name": "Path Demo",
            "source_type": "path",
            "source": examples.clone()
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created_project}");
    let project_id = created_project["id"].clone();
    let project = json!({
        "created_at": created_project["created_at"].clone(),
        "credential_id": null,
        "id": project_id.clone(),
        "last_sync_at": "",
        "last_sync_commit": "",
        "last_sync_status": "",
        "name": "Path Demo",
        "organization_id": org_id.clone(),
        "organization_name": "Plattform",
        "revision": "",
        "source": examples.clone(),
        "source_type": "path",
        "team_id": team_id.clone(),
        "team_ids": []
    });
    assert_eq!(created_project, project);

    let (status, projects, _) = call(
        &app,
        "GET",
        "/api/v1/projects",
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{projects}");
    assert_eq!(projects, json!([demo, project.clone()]));

    let (status, one_project, _) = call(
        &app,
        "GET",
        &format!(
            "/api/v1/projects/{}",
            project_id.as_str().expect("project id")
        ),
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{one_project}");
    assert_eq!(one_project, project);

    let (status, updated_project, _) = call(
        &app,
        "PUT",
        &format!(
            "/api/v1/projects/{}",
            project_id.as_str().expect("project id")
        ),
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        Some(json!({"name": "Path Demo 2"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated_project}");
    let mut expected_project = project.clone();
    expected_project["name"] = json!("Path Demo 2");
    assert_eq!(updated_project, expected_project);

    let (status, synced, _) = call(
        &app,
        "POST",
        &format!(
            "/api/v1/projects/{}/sync",
            project_id.as_str().expect("project id")
        ),
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    assert_eq!(synced, json!({"path": examples}));

    let body =
        "[web]\nweb-1 ansible_host=10.0.0.8 role=frontend\n[db]\ndb-1\n[all:children]\nweb\ndb\n";
    let (status, created_inventory, _) = call(
        &app,
        "POST",
        "/api/v1/inventories",
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        Some(json!({"name": "Web", "body": body})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created_inventory}");
    let inventory_id = created_inventory["id"].clone();
    let inventory_source_id = created_inventory["sources"][0]["id"].clone();
    let detail_hosts = json!([
        {
            "groups": ["web"],
            "name": "web-1",
            "vars": {"ansible_host": "10.0.0.8", "role": "frontend"}
        },
        {
            "groups": ["db"],
            "name": "db-1",
            "vars": {}
        }
    ]);
    let page_hosts = json!([
        {
            "groups": ["db"],
            "name": "db-1",
            "vars": {}
        },
        {
            "groups": ["web"],
            "name": "web-1",
            "vars": {"ansible_host": "10.0.0.8", "role": "frontend"}
        }
    ]);
    let detail = json!({
        "body": body,
        "created_at": created_inventory["created_at"].clone(),
        "credential_id": null,
        "group_count": 2,
        "host_count": 2,
        "hosts": detail_hosts,
        "id": inventory_id.clone(),
        "name": "Web",
        "source": {},
        "source_count": 1,
        "source_kind": "ini",
        "sources": [{
            "body": body,
            "credential_id": null,
            "host_count": 2,
            "id": inventory_source_id.clone(),
            "kind": "ini",
            "name": "manual",
            "source": {}
        }],
        "team_id": team_id.clone()
    });
    assert_eq!(created_inventory, detail);

    let (status, one_inventory, _) = call(
        &app,
        "GET",
        &format!(
            "/api/v1/inventories/{}",
            inventory_id.as_str().expect("inventory id")
        ),
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{one_inventory}");
    assert_eq!(
        one_inventory,
        json!({
            "created_at": detail["created_at"].clone(),
            "credential_id": null,
            "group_count": 2,
            "host_count": 2,
            "id": inventory_id.clone(),
            "name": "Web",
            "source": {},
            "source_count": 1,
            "source_kind": "ini",
            "sources": [{
                "credential_id": null,
                "host_count": 2,
                "id": inventory_source_id.clone(),
                "kind": "ini",
                "name": "manual",
                "source": {}
            }],
            "team_id": team_id.clone()
        })
    );

    let (status, inventories, _) = call(
        &app,
        "GET",
        "/api/v1/inventories",
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{inventories}");
    let local = inventories
        .as_array()
        .expect("inventories")
        .iter()
        .find(|item| item["name"] == "local")
        .expect("seeded inventory");
    assert_eq!(
        inventories,
        json!([
            {
                "created_at": detail["created_at"].clone(),
                "credential_id": null,
                "group_count": 2,
                "host_count": 2,
                "id": inventory_id.clone(),
                "name": "Web",
                "source": {},
                "source_count": 1,
                "source_kind": "ini",
                "sources": [{
                    "credential_id": null,
                    "host_count": 2,
                    "id": inventory_source_id.clone(),
                    "kind": "ini",
                    "name": "manual",
                    "source": {}
                }],
                "team_id": team_id.clone()
            },
            {
                "created_at": local["created_at"].clone(),
                "credential_id": null,
                "group_count": 0,
                "host_count": 1,
                "id": local["id"].clone(),
                "name": "local",
                "source": {},
                "source_count": 1,
                "source_kind": "ini",
                "sources": [{
                    "credential_id": null,
                    "host_count": 1,
                    "id": local["sources"][0]["id"].clone(),
                    "kind": "ini",
                    "name": "manual",
                    "source": {}
                }],
                "team_id": team_id.clone()
            }
        ])
    );

    let (status, page, _) = call(
        &app,
        "GET",
        &format!(
            "/api/v1/inventories/{}/hosts",
            inventory_id.as_str().expect("inventory id")
        ),
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(
        page,
        json!({
            "groups": ["all", "db", "web"],
            "hosts": page_hosts,
            "id": inventory_id.clone(),
            "page": 1,
            "page_size": 50,
            "total": 2
        })
    );

    let (status, host, _) = call(
        &app,
        "GET",
        &format!(
            "/api/v1/inventories/{}/hosts/web-1",
            inventory_id.as_str().expect("inventory id")
        ),
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{host}");
    assert_eq!(
        host,
        json!({
            "groups": ["all", "web"],
            "id": inventory_id.clone(),
            "name": "web-1",
            "netbox_id": null,
            "netbox_url": null,
            "source": {
                "id": inventory_source_id.clone(),
                "kind": "ini",
                "name": "manual"
            },
            "vars": [
                {"name": "ansible_host", "origin": "host", "value": "10.0.0.8"},
                {"name": "role", "origin": "host", "value": "frontend"}
            ]
        })
    );

    let (status, groups, _) = call(
        &app,
        "GET",
        &format!(
            "/api/v1/inventories/{}/groups",
            inventory_id.as_str().expect("inventory id")
        ),
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{groups}");
    assert_eq!(
        groups,
        json!({
            "groups": [
                {"children": [], "host_count": 1, "name": "web", "vars": {}},
                {"children": [], "host_count": 1, "name": "db", "vars": {}},
                {"children": ["web", "db"], "host_count": 2, "name": "all", "vars": {}}
            ],
            "id": inventory_id.clone(),
            "total": 3
        })
    );

    let (status, group, _) = call(
        &app,
        "GET",
        &format!(
            "/api/v1/inventories/{}/groups/web",
            inventory_id.as_str().expect("inventory id")
        ),
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{group}");
    assert_eq!(
        group,
        json!({
            "children": [],
            "host_count": 1,
            "hosts": [{
                "groups": ["web"],
                "name": "web-1",
                "vars": {"ansible_host": "10.0.0.8", "role": "frontend"}
            }],
            "id": inventory_id.clone(),
            "name": "web",
            "page": 1,
            "page_size": 50,
            "parents": ["all"],
            "total": 1,
            "vars": {}
        })
    );

    let (status, updated_inventory, _) = call(
        &app,
        "PUT",
        &format!(
            "/api/v1/inventories/{}",
            inventory_id.as_str().expect("inventory id")
        ),
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        Some(json!({"name": "Web 2"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated_inventory}");
    let mut expected_detail = detail.clone();
    expected_detail["name"] = json!("Web 2");
    assert_eq!(updated_inventory, expected_detail);

    let (status, deleted, _) = call(
        &app,
        "DELETE",
        &format!(
            "/api/v1/inventories/{}",
            inventory_id.as_str().expect("inventory id")
        ),
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{deleted}");

    let (status, created_credential, _) = call(
        &app,
        "POST",
        "/api/v1/credentials",
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        Some(json!({
            "name": "Deploy key",
            "kind": "ssh",
            "secret": {"username": "deploy", "password": "ssh-only-pass"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created_credential}");
    let credential_id = created_credential["id"].clone();
    assert_eq!(
        created_credential,
        json!({
            "id": credential_id.clone(),
            "kind": "ssh",
            "name": "Deploy key"
        })
    );

    let (status, credentials, _) = call(
        &app,
        "GET",
        "/api/v1/credentials",
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{credentials}");
    let credential_created_at = credentials[0]["created_at"].clone();
    assert_eq!(
        credentials,
        json!([{
            "created_at": credential_created_at.clone(),
            "id": credential_id.clone(),
            "kind": "ssh",
            "name": "Deploy key",
            "usages": []
        }])
    );

    let (status, credential, _) = call(
        &app,
        "GET",
        &format!(
            "/api/v1/credentials/{}",
            credential_id.as_str().expect("credential id")
        ),
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{credential}");
    assert_eq!(
        credential,
        json!({
            "created_at": credential_created_at.clone(),
            "fields": {"username": "deploy"},
            "id": credential_id.clone(),
            "kind": "ssh",
            "name": "Deploy key",
            "set": ["password"],
            "usages": []
        })
    );

    let (status, updated_credential, _) = call(
        &app,
        "PUT",
        &format!(
            "/api/v1/credentials/{}",
            credential_id.as_str().expect("credential id")
        ),
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        Some(json!({"name": "Deploy key 2"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated_credential}");
    let mut expected_credential = credential.clone();
    expected_credential["name"] = json!("Deploy key 2");
    assert_eq!(updated_credential, expected_credential);

    let (status, created_environment, _) = call(
        &app,
        "POST",
        "/api/v1/environments",
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        Some(json!({
            "name": "Shape EE",
            "image": "quay.io/example/ee:1",
            "tool": "shell",
            "description": "Shape env"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created_environment}");
    let environment_id = created_environment["id"].clone();
    let environment = json!({
        "created_at": created_environment["created_at"].clone(),
        "description": "Shape env",
        "id": environment_id.clone(),
        "image": "quay.io/example/ee:1",
        "name": "Shape EE",
        "network": "",
        "organization_id": null,
        "organization_name": "",
        "pull_policy": "if_not_present",
        "scope": "team",
        "team_id": team_id.clone(),
        "team_name": "Plattform",
        "tool": "shell"
    });
    assert_eq!(created_environment, environment);

    let (status, one_environment, _) = call(
        &app,
        "GET",
        &format!(
            "/api/v1/environments/{}",
            environment_id.as_str().expect("environment id")
        ),
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{one_environment}");
    assert_eq!(one_environment, environment);

    let (status, environments, _) = call(
        &app,
        "GET",
        "/api/v1/environments",
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{environments}");
    let created_at = |id: &str| {
        environments
            .as_array()
            .expect("environments")
            .iter()
            .find(|item| item["id"] == id)
            .expect("environment")["created_at"]
            .clone()
    };
    assert_eq!(
        environments,
        json!([
            {
                "created_at": created_at("seeded-ansible"),
                "description": "Curated Ansible execution environment. The image is maintained upstream and pinned by digest.",
                "id": "seeded-ansible",
                "image": "ghcr.io/ansible-community/community-ee-base:2.21.5-2@sha256:d42f2597080e8e34ce4d1f70f66f987fc841bd0bbeb680f8d234e94ef1afc60a",
                "name": "Ansible (curated)",
                "network": "",
                "organization_id": null,
                "organization_name": "",
                "pull_policy": "if_not_present",
                "scope": "global",
                "team_id": null,
                "team_name": "",
                "tool": "ansible"
            },
            {
                "created_at": created_at("seeded-opentofu"),
                "description": "Curated OpenTofu execution environment. The image is maintained upstream and pinned by digest.",
                "id": "seeded-opentofu",
                "image": "ghcr.io/opentofu/opentofu:1.13.2@sha256:25bdfbe87000b8a88d44c7ef0cca53381da95f6203f7aed1e8cf5b81ecbe18eb",
                "name": "OpenTofu (curated)",
                "network": "",
                "organization_id": null,
                "organization_name": "",
                "pull_policy": "if_not_present",
                "scope": "global",
                "team_id": null,
                "team_name": "",
                "tool": "opentofu"
            },
            {
                "created_at": created_at("seeded-shell"),
                "description": "Curated shell environment with bash, curl, jq, and OpenSSH. The image is maintained upstream and pinned by digest.",
                "id": "seeded-shell",
                "image": "mcr.microsoft.com/devcontainers/base:bookworm@sha256:99f5250dd2d5f8b11046d5662dd94c1e3c1dae56b560c7caad5ceb7edf349046",
                "name": "Shell (curated)",
                "network": "",
                "organization_id": null,
                "organization_name": "",
                "pull_policy": "if_not_present",
                "scope": "global",
                "team_id": null,
                "team_name": "",
                "tool": "shell"
            },
            {
                "created_at": environments
                    .as_array()
                    .expect("environments")
                    .iter()
                    .find(|item| item["name"] == "AWX-EE")
                    .expect("demo environment")["created_at"].clone(),
                "description": "Same idea as an AWX execution environment. The Galaxy cache stays mounted on the host and is not cleared after a run.",
                "id": environments
                    .as_array()
                    .expect("environments")
                    .iter()
                    .find(|item| item["name"] == "AWX-EE")
                    .expect("demo environment")["id"].clone(),
                "image": "quay.io/ansible/awx-ee:latest",
                "name": "AWX-EE",
                "network": "",
                "organization_id": null,
                "organization_name": "",
                "pull_policy": "if_not_present",
                "scope": "team",
                "team_id": team_id.clone(),
                "team_name": "Plattform",
                "tool": "any"
            },
            {
                "created_at": environment["created_at"].clone(),
                "description": "Shape env",
                "id": environment_id.clone(),
                "image": "quay.io/example/ee:1",
                "name": "Shape EE",
                "network": "",
                "organization_id": null,
                "organization_name": "",
                "pull_policy": "if_not_present",
                "scope": "team",
                "team_id": team_id.clone(),
                "team_name": "Plattform",
                "tool": "shell"
            }
        ])
    );

    let (status, updated_environment, _) = call(
        &app,
        "PUT",
        &format!(
            "/api/v1/environments/{}",
            environment_id.as_str().expect("environment id")
        ),
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        Some(json!({
            "name": "Shape EE 2",
            "image": "quay.io/example/ee:1",
            "tool": "shell",
            "description": "Shape env"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated_environment}");
    let mut expected_environment = environment.clone();
    expected_environment["name"] = json!("Shape EE 2");
    assert_eq!(updated_environment, expected_environment);

    let (status, deleted, _) = call(
        &app,
        "DELETE",
        &format!(
            "/api/v1/environments/{}",
            environment_id.as_str().expect("environment id")
        ),
        Some(&cookie),
        Some(team_id.as_str().expect("team id")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{deleted}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn system_catalog_endpoints_return_the_documented_shape() {
    let harness = TestApp::boot("catalog-system", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();
    let (cookie, team) = harness.sign_in().await;

    let (status, plugins, _) =
        call(&app, "GET", "/api/v1/plugins", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{plugins}");
    assert_eq!(
        plugins,
        json!([
            {
                "id": "ansible",
                "summary": "ansible_playbook_collections_and_roles_stay_in_the"
            },
            {
                "id": "opentofu",
                "summary": "opentofu_init_then_plan_or_apply_providers_stay"
            },
            {
                "id": "shell",
                "summary": "shell_script_from_the_project_or_as_an"
            }
        ])
    );

    let (status, cache, _) = call(
        &app,
        "GET",
        "/api/v1/cache",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cache}");
    assert_eq!(
        cache,
        json!({
            "collections": [],
            "collections_bytes": 0,
            "note": "the_cache_lives_under_data_cache_and_stays",
            "roles": [],
            "roles_bytes": 0,
            "tofu_bytes": 0
        })
    );

    let (status, runner, _) = call(
        &app,
        "GET",
        "/api/v1/runner",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{runner}");
    assert_eq!(
        runner,
        json!({
            "container_image": false,
            "default_environment_ansible": "seeded-ansible",
            "default_environment_opentofu": "seeded-opentofu",
            "default_environment_shell": "seeded-shell",
            "local_jobs": "all",
            "local_tools": {
                "ansible": runner["local_tools"]["ansible"].clone(),
                "opentofu": runner["local_tools"]["opentofu"].clone(),
                "shell": runner["local_tools"]["shell"].clone()
            },
            "runner_mode": "local",
            "runtime": runner["runtime"].clone()
        })
    );

    let (status, overview, _) = call(
        &app,
        "GET",
        "/api/v1/overview",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{overview}");
    assert_eq!(
        overview,
        json!({
            "cache_bytes": 0,
            "deliveries": [],
            "hook_id": overview["hook_id"].clone(),
            "hook_name": overview["hook_name"].clone(),
            "import": null,
            "jobs": [],
            "ldap": false,
            "preflight": null,
            "runner_mode": "local",
            "runtime": overview["runtime"].clone(),
            "workflow_id": overview["workflow_id"].clone(),
            "workflow_name": overview["workflow_name"].clone()
        })
    );
    assert!(overview["workflow_id"].is_string());
    assert_eq!(overview["workflow_name"], "Self-Heal Ablauf");
    assert!(overview["hook_id"].is_string());
    assert_eq!(overview["hook_name"], "DiskSpaceLow");

    let (status, dashboard, _) = call(
        &app,
        "GET",
        "/api/v1/dashboard",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{dashboard}");
    assert_eq!(
        dashboard,
        json!({
            "cache": {"collections": 0, "roles": 0, "tofu_bytes": 0},
            "from": dashboard["from"].clone(),
            "range_days": 1,
            "series": [{
                "canceled": 0,
                "date": dashboard["series"][0]["date"].clone(),
                "failed": 0,
                "started": 0,
                "successful": 0
            }],
            "timezone": "UTC",
            "to": dashboard["to"].clone()
        })
    );

    let (status, preflight, _) = call(
        &app,
        "GET",
        "/api/v1/runner/preflight",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preflight}");
    let tools = preflight["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|tool| {
            json!({
                "found": tool["found"].clone(),
                "id": tool["id"].clone(),
                "kind": tool["kind"].clone(),
                "meets_minimum": tool["meets_minimum"].clone(),
                "minimum": tool["minimum"].clone(),
                "path": tool["path"].clone(),
                "reason": tool["reason"].clone(),
                "required": tool["required"].clone(),
                "template_tool": tool["template_tool"].clone(),
                "version": tool["version"].clone()
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        preflight,
        json!({
            "container_image": false,
            "messages": preflight["messages"].clone(),
            "missing": preflight["missing"].clone(),
            "mode": "local",
            "ok": preflight["ok"].clone(),
            "runtime": {
                "binary": preflight["runtime"]["binary"].clone(),
                "found": preflight["runtime"]["found"].clone(),
                "path": preflight["runtime"]["path"].clone(),
                "reason": preflight["runtime"]["reason"].clone(),
                "responding": preflight["runtime"]["responding"].clone(),
                "version": preflight["runtime"]["version"].clone()
            },
            "tools": tools
        })
    );
    assert_eq!(
        preflight["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .map(|tool| tool["id"].as_str().expect("tool id"))
            .collect::<Vec<_>>(),
        ["ansible-playbook", "tofu", "sh", "git", "ssh"]
    );

    let (status, overview, _) = call(
        &app,
        "GET",
        "/api/v1/overview",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{overview}");
    assert_eq!(overview["preflight"], preflight);

    let _ = std::fs::remove_dir_all(&dir);
}
