use std::time::Duration;

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::common::call_cookie as call;

use crate::common::*;

#[tokio::test]
async fn owner_can_store_an_ssh_password_without_a_key() {
    let harness = TestApp::boot("cred", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("machine_ssh"));
    assert!(script.contains("amazon_web_services"));
    assert!(script.contains("a_key_or_a_password_or_both"));
    assert!(script.contains("source_control_credential_hint"));
    assert!(script.contains("leave_empty_to_keep_the_stored_value"));
    assert!(script.contains("data-stored-mask=\"1\""));
    assert!(script.contains("class=\"secret-mask\""));
    assert!(!script.contains("*** bedeutet, dass ein Wert gespeichert ist."));
    assert!(script.contains("the_type_cannot_be_changed"));
    assert!(script.contains("#/credentials?edit=${esc(item.id)}"));

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
    let (status, created, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "SSH Passwort",
            "kind": "ssh",
            "secret": {"username": "deploy", "password": "ssh-only-pass"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["kind"], "ssh");
    assert!(
        !created.to_string().contains("ssh-only-pass"),
        "response echoed the secret"
    );
    let id = created["id"].as_str().unwrap();
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
    assert!(
        !listed.to_string().contains("ssh-only-pass"),
        "list echoed the secret"
    );
    assert!(listed
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["id"] == id && item["kind"] == "ssh"));
    let (status, one, _) = call(
        &app,
        "GET",
        &format!("/api/credentials/{id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{one}");
    assert_eq!(one["fields"]["username"], "deploy");
    assert_eq!(one["set"], json!(["password"]));
    assert!(
        !one.to_string().contains("ssh-only-pass"),
        "credential view echoed the secret"
    );
    let (status, kept, _) = call(
        &app,
        "PUT",
        &format!("/api/credentials/{id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "SSH Passwort neu",
            "secret": {"username": "deploy", "password": ""}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["name"], "SSH Passwort neu");
    assert_eq!(kept["set"], json!(["password"]));
    assert!(!kept.to_string().contains("ssh-only-pass"));
    let (status, replaced, _) = call(
        &app,
        "PUT",
        &format!("/api/credentials/{id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "SSH Passwort neu",
            "secret": {
                "username": "deploy",
                "password": "ssh-new-pass",
                "private_key": "KEYMATERIAL"
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replaced}");
    assert_eq!(replaced["set"], json!(["password", "private_key"]));
    let replaced_text = replaced.to_string();
    assert!(!replaced_text.contains("ssh-new-pass"));
    assert!(!replaced_text.contains("ssh-only-pass"));
    assert!(!replaced_text.contains("KEYMATERIAL"));
    let (status, cleared, _) = call(
        &app,
        "PUT",
        &format!("/api/credentials/{id}"),
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "SSH Passwort neu",
            "secret": {"username": "deploy"},
            "clear": ["password", "private_key"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{cleared}");
    assert_eq!(cleared["error"], "ssh_needs_a_key_or_a_password");
    let (status, still, _) = call(
        &app,
        "GET",
        &format!("/api/credentials/{id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{still}");
    assert_eq!(still["set"], json!(["password", "private_key"]));
    assert!(!still.to_string().contains("KEYMATERIAL"));
    let (status, rejected, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Leer", "kind": "ssh", "secret": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(rejected["error"], "ssh_needs_a_key_or_a_password");
    let (status, unknown, _) = call(
        &app,
        "POST",
        "/api/credentials",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Fremd", "kind": "gpg", "secret": {"token": "x"}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{unknown}");
    assert_eq!(unknown["error"], "unknown_credential_type");
    let (status, gone, _) = call(
        &app,
        "DELETE",
        &format!("/api/credentials/{id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{gone}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn template_accepts_one_credential_per_kind_and_many_vaults() {
    let harness = TestApp::boot("vault", Options::default());
    let app = harness.app.clone();
    let dir = harness.dir.clone();

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("one_credential_of_each_kind_choose_vault_credentials"));
    assert!(script.contains("applies_only_to_this_node_the_template_stays"));
    assert!(script.contains("id=\"vault-pills\""));
    assert!(!script.contains("name=\"vault_ids\""));
    assert!(script.contains("no_vault_credentials_in_this_team"));
    assert!(script.contains("remove_vault_credential_name"));

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

    let ssh = create_credential(
        &app,
        &cookie,
        &team,
        "SSH eins",
        "ssh",
        json!({"username": "deploy", "password": "ssh-eins-geheim"}),
    )
    .await;
    let ssh_other = create_credential(
        &app,
        &cookie,
        &team,
        "SSH zwei",
        "ssh",
        json!({"username": "deploy", "password": "ssh-zwei-geheim"}),
    )
    .await;
    let vault_prod = create_credential(
        &app,
        &cookie,
        &team,
        "Vault prod",
        "vault",
        json!({"password": "vault-prod-geheim", "vault_id": "prod"}),
    )
    .await;
    let vault_stage = create_credential(
        &app,
        &cookie,
        &team,
        "Vault stage",
        "vault",
        json!({"password": "vault-stage-geheim", "vault_id": "stage"}),
    )
    .await;
    let vault_prod_again = create_credential(
        &app,
        &cookie,
        &team,
        "Vault prod nochmal",
        "vault",
        json!({"password": "vault-prod-geheim", "vault_id": "prod"}),
    )
    .await;
    let vault_plain = create_credential(
        &app,
        &cookie,
        &team,
        "Vault ohne Id",
        "vault",
        json!({"password": "vault-plain-geheim"}),
    )
    .await;
    let vault_plain_two = create_credential(
        &app,
        &cookie,
        &team,
        "Vault ohne Id zwei",
        "vault",
        json!({"password": "vault-plain-geheim"}),
    )
    .await;

    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Vault hosts", "body": "[web]\nweb-1\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap().to_string();

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "Vault-Zuordnung",
            json!([ssh, vault_prod, vault_stage]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        created.get("error").unwrap_or(&Value::Null)
    );
    assert_no_secrets(&created);
    assert_eq!(created["credential_id"], ssh);
    let ids = created["credential_ids"]
        .as_array()
        .expect("credential_ids");
    assert_eq!(ids.len(), 3);
    assert_eq!(ids[0], ssh);
    assert_eq!(ids[1], vault_prod);
    assert_eq!(ids[2], vault_stage);
    let template_id = created["id"].as_str().unwrap().to_string();

    let (status, single, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Alter Zugang",
            "tool": "shell",
            "spec": {"script": "echo ok"},
            "credential_id": ssh
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{single}");
    assert_eq!(single["credential_id"], ssh);
    assert_eq!(single["credential_ids"], json!([ssh]));

    let (status, second_ssh, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "Vault-Zuordnung",
            json!([ssh, ssh_other]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{second_ssh}");
    assert_eq!(
        second_ssh["error"],
        "only_one_credential_of_each_kind_is_allowed"
    );

    let (status, same_label, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "Vault-Zuordnung",
            json!([vault_prod, vault_prod_again]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{same_label}");
    assert_eq!(same_label["error"], "this_vault_id_is_already_selected");

    let (status, two_plain, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "Vault-Zuordnung",
            json!([vault_plain, vault_plain_two]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{two_plain}");
    assert_eq!(
        two_plain["error"],
        "only_one_vault_credential_may_omit_the_vault"
    );

    let (status, listed, _) = call(
        &app,
        "GET",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let saved = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == template_id)
        .expect("template");
    assert_eq!(saved["credential_id"], ssh);
    assert_eq!(saved["credential_ids"].as_array().unwrap().len(), 3);

    let (status, job, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    assert_no_secrets(&job);
    let launched = job["parameters"]["credentials"]
        .as_array()
        .expect("credentials");
    assert_eq!(launched.len(), 3);
    assert_eq!(launched[0]["id"], ssh);
    assert_eq!(launched[0]["name"], "SSH eins");
    assert_eq!(launched[0]["kind"], "ssh");
    assert_eq!(launched[1]["id"], vault_prod);
    assert_eq!(launched[1]["name"], "Vault prod");
    assert_eq!(launched[1]["kind"], "vault");
    assert_eq!(launched[2]["id"], vault_stage);
    assert_eq!(launched[2]["kind"], "vault");
    let job_id = job["id"].as_str().unwrap();
    let (status, stored, _) = call(
        &app,
        "GET",
        &format!("/api/jobs/{job_id}"),
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_no_secrets(&stored);
    assert_eq!(
        stored["parameters"]["credentials"]
            .as_array()
            .unwrap()
            .len(),
        3
    );

    let (status, workflow, _) = call(
        &app,
        "POST",
        "/api/workflows",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "Anderer SSH-Zugang",
            "graph": {
                "nodes": [{
                    "id": "step",
                    "template_id": template_id,
                    "name": "Mit anderem SSH",
                    "overrides": {"credential_id": ssh_other}
                }],
                "edges": []
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");
    let workflow_id = workflow["id"].as_str().unwrap();
    let (status, run, _) = call(
        &app,
        "POST",
        &format!("/api/workflows/{workflow_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{run}");

    let mut node_job = None;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (status, jobs, _) =
            call(&app, "GET", "/api/jobs", Some(&cookie), Some(&team), None).await;
        assert_eq!(status, StatusCode::OK, "{jobs}");
        if let Some(found) = jobs["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["workflow_node_id"] == "step")
        {
            node_job = Some(found.clone());
            break;
        }
    }
    let node_job = node_job.expect("workflow node job");
    assert_no_secrets(&node_job);
    let overridden = node_job["parameters"]["credentials"]
        .as_array()
        .expect("credentials");
    assert_eq!(overridden.len(), 3);
    let overridden_ids: Vec<&str> = overridden
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert!(overridden_ids.contains(&ssh_other.as_str()));
    assert!(overridden_ids.contains(&vault_prod.as_str()));
    assert!(overridden_ids.contains(&vault_stage.as_str()));
    assert!(!overridden_ids.contains(&ssh.as_str()));
    assert!(overridden
        .iter()
        .any(|item| item["id"] == ssh_other && item["kind"] == "ssh"));
    assert_eq!(
        overridden
            .iter()
            .filter(|item| item["kind"] == "vault")
            .count(),
        2
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn template_credential_kinds_follow_the_tool() {
    let dir = temp_dir("tool-credentials");
    let state = koreograf::boot(config(&dir, &Options::default())).unwrap();
    let db_state = state.clone();
    let app = koreograf::router(state);

    let (status, script, _) = call(&app, "GET", "/app.js", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let script = script.as_str().unwrap_or("");
    assert!(script.contains("const TOOL_CREDENTIAL_KINDS = {"));
    assert!(script.contains("ansible: [\"ssh\", \"vault\"]"));
    assert!(script.contains("opentofu: [\"aws\", \"azure\", \"gce\", \"vmware\", \"openstack\", \"kubernetes\", \"env\"]"));
    assert!(script.contains("shell: [\"ssh\", \"env\"]"));
    assert!(script.contains("data-credential-field"));
    assert!(script.contains("template-legacy-credentials"));
    assert!(script.contains("legacy-pills"));
    assert!(script.contains("remove_credential_name"));

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

    let ssh = create_credential(
        &app,
        &cookie,
        &team,
        "SSH",
        "ssh",
        json!({"username": "deploy", "password": "tool-ssh-geheim"}),
    )
    .await;
    let vault = create_credential(
        &app,
        &cookie,
        &team,
        "Vault",
        "vault",
        json!({"password": "tool-vault-geheim"}),
    )
    .await;
    let netbox = create_credential(
        &app,
        &cookie,
        &team,
        "NetBox",
        "netbox",
        json!({"token": "tool-netbox-geheim", "url": "https://netbox.example.com"}),
    )
    .await;
    let aws = create_credential(
        &app,
        &cookie,
        &team,
        "AWS",
        "aws",
        json!({"access_key": "AKIAEXAMPLE", "secret_key": "tool-aws-geheim"}),
    )
    .await;
    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Tool hosts", "body": "[web]\nweb-1\n"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inventory}");
    let inventory_id = inventory["id"].as_str().unwrap().to_string();

    let (status, refused, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "NetBox-Ansible",
            json!([netbox]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(
        refused["error"],
        "this_credential_kind_does_not_fit_the_tool"
    );

    let (status, refused, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "SSH-OpenTofu",
            "tool": "opentofu",
            "entrypoint": ".",
            "credential_ids": [ssh]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(
        refused["error"],
        "this_credential_kind_does_not_fit_the_tool"
    );

    let (status, refused, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(shell_with_credentials("Vault-Shell", json!([vault]))),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(
        refused["error"],
        "this_credential_kind_does_not_fit_the_tool"
    );

    let (status, created, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "SSH-Ansible",
            json!([ssh, vault]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let template_id = created["id"].as_str().unwrap().to_string();

    let (status, unchanged, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{template_id}"),
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "SSH-Ansible",
            json!([ssh, vault]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{unchanged}");
    assert_eq!(unchanged["credential_ids"], json!([ssh, vault]));

    let (status, tofu, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(json!({
            "name": "AWS-OpenTofu",
            "tool": "opentofu",
            "entrypoint": ".",
            "credential_ids": [aws]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{tofu}");
    assert_eq!(tofu["credential_ids"], json!([aws]));

    let (status, plain, _) = call(
        &app,
        "POST",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "Alt-NetBox",
            json!([]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{plain}");
    let legacy_id = plain["id"].as_str().unwrap().to_string();
    {
        let db = db_state.db().unwrap();
        db.execute(
            "INSERT INTO template_credentials (template_id, credential_id, position) VALUES (?1, ?2, 0)",
            rusqlite::params![legacy_id, netbox],
        )
        .unwrap();
    }
    let (status, legacy, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{legacy_id}"),
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "Alt-NetBox",
            json!([netbox]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{legacy}");
    assert_eq!(legacy["credential_ids"], json!([netbox]));

    let (status, refused, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{legacy_id}"),
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "Alt-NetBox",
            json!([netbox, aws]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(
        refused["error"],
        "this_credential_kind_does_not_fit_the_tool"
    );

    let (status, removed, _) = call(
        &app,
        "PUT",
        &format!("/api/templates/{legacy_id}"),
        Some(&cookie),
        Some(&team),
        Some(ansible_with_credentials(
            "Alt-NetBox",
            json!([]),
            &inventory_id,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{removed}");
    assert!(removed["credential_ids"].as_array().unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn launch_overrides_follow_the_tool_credential_kinds() {
    let dir = temp_dir("launch-kinds");
    let state = koreograf::boot(config(&dir, &Options::default())).unwrap();
    let db_state = state.clone();
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

    let ssh = create_credential(
        &app,
        &cookie,
        &team,
        "Launch SSH",
        "ssh",
        json!({"username": "deploy", "password": "launch-ssh-geheim"}),
    )
    .await;
    let aws = create_credential(
        &app,
        &cookie,
        &team,
        "Launch AWS",
        "aws",
        json!({"access_key": "AKIAEXAMPLE", "secret_key": "launch-aws-geheim"}),
    )
    .await;
    let netbox = create_credential(
        &app,
        &cookie,
        &team,
        "Launch NetBox",
        "netbox",
        json!({"token": "launch-netbox-geheim", "url": "https://netbox.example.com"}),
    )
    .await;
    let (status, inventory, _) = call(
        &app,
        "POST",
        "/api/inventories",
        Some(&cookie),
        Some(&team),
        Some(json!({"name": "Launch hosts", "body": "[web]\nweb-1\n"})),
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
            "name": "Launch kinds",
            "tool": "ansible",
            "entrypoint": "site.yml",
            "inventory_id": inventory_id,
            "prompt_on_launch": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    let template_id = template["id"].as_str().unwrap().to_string();
    {
        let db = db_state.db().unwrap();
        db.execute(
            "INSERT INTO template_credentials (template_id, credential_id, position) VALUES (?1, ?2, 0)",
            rusqlite::params![template_id, netbox],
        )
        .unwrap();
    }

    let (status, launched, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{launched}");
    assert_eq!(
        launched["parameters"]["credentials"][0]["id"],
        Value::String(netbox.clone())
    );
    let job_id = launched["id"].as_str().unwrap();
    let _ = call(
        &app,
        "POST",
        &format!("/api/jobs/{job_id}/cancel"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;

    let (status, refused, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}, "credential_ids": [aws]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(
        refused["error"],
        "this_credential_kind_does_not_fit_the_tool"
    );

    let (status, overridden, _) = call(
        &app,
        "POST",
        &format!("/api/templates/{template_id}/launch"),
        Some(&cookie),
        Some(&team),
        Some(json!({"extra_vars": {}, "credential_ids": [ssh]})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{overridden}");
    assert_eq!(overridden["parameters"]["credentials"][0]["id"], ssh);
    let job_id = overridden["id"].as_str().unwrap();
    let _ = call(
        &app,
        "POST",
        &format!("/api/jobs/{job_id}/cancel"),
        Some(&cookie),
        Some(&team),
        Some(json!({})),
    )
    .await;

    let (status, stored, _) = call(
        &app,
        "GET",
        "/api/templates",
        Some(&cookie),
        Some(&team),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    let row = stored
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == template_id)
        .unwrap();
    assert_eq!(row["credential_ids"], json!([netbox]));

    let _ = std::fs::remove_dir_all(&dir);
}
