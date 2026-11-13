use crate::features::jobs::routes::events_for_page;

use super::*;
use crate::config::Config;
use std::path::PathBuf;

#[test]
fn prompt_spec_replaces_listed_fields_and_clears_blanks() {
    let base = json!({"job_type": "run", "tags": "ping", "limit": "web", "forks": 5, "diff": true});
    let merged = merge_prompt_spec(
        &base,
        &json!({"limit": "db", "tags": "", "skip_tags": "slow", "job_type": "check", "verbosity": 0}),
    )
    .expect("merge");
    assert_eq!(merged["limit"], "db");
    assert_eq!(merged["job_type"], "check");
    assert_eq!(merged["skip_tags"], "slow");
    assert_eq!(merged["forks"], 5);
    assert_eq!(merged["diff"], true);
    assert!(merged.get("tags").is_none());
    assert!(merged.get("verbosity").is_none());
    let rejected = merge_prompt_spec(&base, &json!({"forks": 2})).expect_err("forks");
    assert_eq!(rejected, "launch_fields_are_only_for_ansible");
}

fn template_without_environment() -> db::Template {
    db::Template {
        id: "template".into(),
        team_id: "team".into(),
        name: "Run".into(),
        description: String::new(),
        tool: "shell".into(),
        project_id: None,
        inventory_id: None,
        credential_id: None,
        environment_id: None,
        no_environment: true,
        entrypoint: "run.sh".into(),
        extra_vars: "{}".into(),
        spec: "{}".into(),
        created_at: db::now(),
        update_on_launch: false,
        admin_saved: true,
        prompt_on_launch: false,
    }
}

#[test]
fn a_migrated_no_environment_template_uses_the_tool_default_in_the_container() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-capture-env-{nanos}"));
    std::fs::create_dir_all(&dir).expect("dir");
    let conn =
        crate::backend::open(&crate::config::DatabaseConfig::Sqlite, &dir).expect("database");
    let template = template_without_environment();
    let kept = db::json_object(&capture_parameters(
        &*conn,
        &template,
        &json!({}),
        None,
        "tool-default",
        true,
    ));
    assert!(kept["environment_id"].is_null());
    let inherited = db::json_object(&capture_parameters(
        &*conn,
        &template,
        &json!({}),
        None,
        "tool-default",
        false,
    ));
    assert_eq!(inherited["environment_id"], "tool-default");
    drop(conn);
    let _ = std::fs::remove_dir_all(dir);
}

fn config(dir: &std::path::Path) -> Config {
    Config {
        bind: "127.0.0.1:0".into(),
        data_dir: dir.to_path_buf(),
        web_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
        examples_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
        admin_user: "admin".into(),
        admin_password: Some("test-admin-pass".into()),
        runner: RunnerMode::Local,
        container_image: false,
        job_timeout_secs: 60,
        job_output_lines: 8_000,
        local_jobs: crate::config::LocalJobs::All,
        project_roots: Vec::new(),
        hsts: false,
        cookie_secure: crate::config::CookieSecure::Auto,
        trusted_proxy: false,
        mcp: true,
        public_url: Vec::new(),
        data_host: None,
        ldap: None,
    }
}

#[tokio::test]
async fn relaunch_copies_parameters_and_limits_failed_hosts() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-relaunch-{nanos}"));
    let state = crate::boot(config(&dir)).expect("boot");
    let (template_id, team_id) = {
        let db = state.db().expect("db");
        db.query_row(
            "SELECT id, team_id FROM templates WHERE name = 'Self-Heal'",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("template")
    };
    let inventory_id = {
        let db = state.db().expect("db");
        insert_inventory(&db, &team_id, false)
    };
    {
        let db = state.db().expect("db");
        crate::features::jobs::store::insert_job(
            &db,
            &db::Job {
                id: "src-failed".into(),
                team_id,
                template_id: Some(template_id),
                workflow_job_id: None,
                workflow_node_id: None,
                name: "Self-Heal".into(),
                tool: "ansible".into(),
                status: "failed".into(),
                extra_vars: r#"{"alertname":"manual"}"#.into(),
                launched_by: "admin".into(),
                trigger: "manual".into(),
                started_at: None,
                finished_at: Some(db::now()),
                created_at: db::now(),
                parameters: json!({
                    "template_name": "Self-Heal",
                    "tool": "ansible",
                    "inventory_id": inventory_id,
                    "entrypoint": "missing.yml",
                    "extra_vars": {"alertname": "manual"},
                    "spec": {"tags": "heal"},
                    "project_commit": "1234567",
                    "project_commit_full": "1234567890abcdef1234567890abcdef12345678"
                })
                .to_string(),
                failed_hosts: json!({"hosts": ["web-1", "db-1"], "recap": true}).to_string(),
            },
        )
        .expect("insert");
    }
    let copy = relaunch(&state, "src-failed", "admin", true).expect("relaunch");
    let params = {
        let db = state.db().expect("db");
        let job = crate::features::jobs::store::job(&db, &copy)
            .expect("read")
            .expect("job");
        assert_eq!(job.trigger, "relaunch");
        assert_eq!(job.status, "pending");
        assert!(!job.parameters.contains("secret"));
        db::json_object(&job.parameters)
    };
    assert_eq!(params["spec"]["limit"], "web-1,db-1");
    assert_eq!(params["spec"]["tags"], "heal");
    assert_eq!(params["extra_vars"]["alertname"], "manual");
    assert!(params.get("project_commit").is_none());
    assert!(params.get("project_commit_full").is_none());
    {
        let db = state.db().expect("db");
        db.execute("UPDATE jobs SET tool = 'shell' WHERE id = 'src-failed'", [])
            .expect("tool");
        db.execute(
            "UPDATE jobs SET status = 'successful' WHERE id = ?1",
            [&copy],
        )
        .expect("status");
    }
    let shell = relaunch(&state, "src-failed", "admin", true).expect_err("shell");
    assert!(shell.to_string().contains("ansible"), "{shell}");
    let running = relaunch(&state, &copy, "admin", false).expect_err("not failed");
    assert!(
        running
            .to_string()
            .contains("only_a_failed_or_canceled_job_can_be_relaunched"),
        "{running}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn relaunch_accepts_a_canceled_job_and_keeps_the_failed_host_limit() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-relaunch-canceled-{nanos}"));
    let state = crate::boot(config(&dir)).expect("boot");
    let (template_id, team_id) = {
        let db = state.db().expect("db");
        db.query_row(
            "SELECT id, team_id FROM templates WHERE name = 'Self-Heal'",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("template")
    };
    let inventory_id = {
        let db = state.db().expect("db");
        insert_inventory(&db, &team_id, false)
    };
    let parameters = json!({
        "template_name": "Self-Heal",
        "tool": "ansible",
        "inventory_id": inventory_id,
        "entrypoint": "site.yml",
        "extra_vars": {"alertname": "canceled"},
        "spec": {"tags": "heal"}
    })
    .to_string();
    {
        let db = state.db().expect("db");
        crate::features::jobs::store::insert_job(
            &db,
            &db::Job {
                id: "src-canceled".into(),
                team_id: team_id.clone(),
                template_id: Some(template_id.clone()),
                workflow_job_id: None,
                workflow_node_id: None,
                name: "Self-Heal".into(),
                tool: "ansible".into(),
                status: "canceled".into(),
                extra_vars: r#"{"alertname":"canceled"}"#.into(),
                launched_by: "admin".into(),
                trigger: "manual".into(),
                started_at: None,
                finished_at: Some(db::now()),
                created_at: db::now(),
                parameters: parameters.clone(),
                failed_hosts: "[]".into(),
            },
        )
        .expect("insert canceled");
        crate::features::jobs::store::insert_job(
            &db,
            &db::Job {
                id: "src-canceled-hosts".into(),
                team_id,
                template_id: Some(template_id),
                workflow_job_id: None,
                workflow_node_id: None,
                name: "Self-Heal".into(),
                tool: "ansible".into(),
                status: "canceled".into(),
                extra_vars: r#"{"alertname":"canceled"}"#.into(),
                launched_by: "admin".into(),
                trigger: "manual".into(),
                started_at: None,
                finished_at: Some(db::now()),
                created_at: db::now(),
                parameters,
                failed_hosts: json!({"hosts": ["web-1"], "recap": true}).to_string(),
            },
        )
        .expect("insert hosts");
    }
    let empty = relaunch(&state, "src-canceled", "admin", true).expect_err("no hosts");
    assert!(
        empty
            .to_string()
            .contains("no_failed_hosts_are_stored_for_this_job"),
        "{empty}"
    );
    let copy = relaunch(&state, "src-canceled", "admin", false).expect("relaunch");
    {
        let db = state.db().expect("db");
        let job = crate::features::jobs::store::job(&db, &copy)
            .expect("read")
            .expect("job");
        assert_eq!(job.trigger, "relaunch");
        assert_eq!(job.status, "pending");
        let params = db::json_object(&job.parameters);
        assert_eq!(params["extra_vars"]["alertname"], "canceled");
        assert_eq!(params["spec"]["tags"], "heal");
        assert!(params["spec"].get("limit").is_none());
    }
    for status in ["pending", "running", "successful"] {
        {
            let db = state.db().expect("db");
            db.execute("UPDATE jobs SET status = ?1 WHERE id = ?2", (status, &copy))
                .expect("status");
        }
        let rejected = relaunch(&state, &copy, "admin", false).expect_err(status);
        assert!(
            rejected
                .to_string()
                .contains("only_a_failed_or_canceled_job_can_be_relaunched"),
            "{status}: {rejected}"
        );
    }
    let limited = relaunch(&state, "src-canceled-hosts", "admin", true).expect("limit");
    {
        let db = state.db().expect("db");
        let job = crate::features::jobs::store::job(&db, &limited)
            .expect("read")
            .expect("job");
        let params = db::json_object(&job.parameters);
        assert_eq!(params["spec"]["limit"], "web-1");
        assert_eq!(params["spec"]["tags"], "heal");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "current_thread")]
async fn cancel_before_execution_counts_the_job_once() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-cancel-pending-{nanos}"));
    let state = crate::boot(config(&dir)).expect("boot");
    let (template_id, team_id) = {
        let db = state.db().expect("db");
        db.query_row(
            "SELECT id, team_id FROM templates WHERE name = 'Healthcheck'",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("template")
    };
    let job_id = launch(
        &state,
        LaunchRequest {
            team_id,
            template_id,
            extra_vars: json!({}),
            launched_by: "admin".into(),
            trigger: "manual".into(),
            workflow_job_id: None,
            workflow_node_id: None,
            parameters_override: None,
            overrides: LaunchOverrides::default(),
        },
    )
    .expect("launch");
    cancel(&state, &job_id).expect("cancel");
    let text = state.metrics.render(&[]);
    assert!(
        text.contains(
            "koreograf_jobs_finished_total{tool=\"shell\",playbook=\"health.sh\",status=\"canceled\"} 1"
        ),
        "{text}"
    );
    assert!(!text.contains("koreograf_jobs_started_total{"), "{text}");
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    let text = state.metrics.render(&[]);
    assert!(
        text.contains(
            "koreograf_jobs_finished_total{tool=\"shell\",playbook=\"health.sh\",status=\"canceled\"} 1"
        ),
        "{text}"
    );
    assert!(!text.contains("} 2"), "{text}");
    assert!(!text.contains("koreograf_jobs_started_total{"), "{text}");
    {
        let db = state.db().expect("db");
        let job = crate::features::jobs::store::job(&db, &job_id)
            .expect("read")
            .expect("job");
        assert_eq!(job.status, "canceled");
    }
    let _ = std::fs::remove_dir_all(dir);
}

fn admins(dir: &std::path::Path, runner: RunnerMode) -> Config {
    let mut cfg = config(dir);
    cfg.local_jobs = crate::config::LocalJobs::Admins;
    cfg.runner = runner;
    cfg
}

fn team_of(state: &Shared) -> String {
    let db = state.db().expect("db");
    db.query_row("SELECT team_id FROM templates LIMIT 1", [], |row| {
        row.get(0)
    })
    .expect("team")
}

fn insert_shell(
    db: &rusqlite::Connection,
    team_id: &str,
    inventory_id: Option<&str>,
    script: &str,
) -> String {
    let id = Uuid::new_v4().to_string();
    crate::store::templates::insert_template(
        db,
        &db::Template {
            id: id.clone(),
            team_id: team_id.to_string(),
            name: format!("Inline {id}"),
            description: String::new(),
            tool: "shell".into(),
            project_id: None,
            inventory_id: inventory_id.map(str::to_string),
            credential_id: None,
            environment_id: None,
            no_environment: true,
            entrypoint: String::new(),
            extra_vars: "{}".into(),
            spec: json!({"script": script}).to_string(),
            created_at: db::now(),
            update_on_launch: false,
            admin_saved: true,
            prompt_on_launch: false,
        },
    )
    .expect("template");
    id
}

fn insert_inventory(db: &rusqlite::Connection, team_id: &str, admin_saved: bool) -> String {
    let id = Uuid::new_v4().to_string();
    crate::store::inventories::insert_inventory(
        db,
        &db::Inventory {
            id: id.clone(),
            team_id: team_id.to_string(),
            name: format!("Hosts {id}"),
            body: "[all]\nlocalhost ansible_connection=local\n".into(),
            created_at: db::now(),
            source_kind: "ini".into(),
            source_json: "{}".into(),
            credential_id: None,
            admin_saved,
        },
    )
    .expect("inventory");
    id
}

fn insert_override_workflow(
    db: &rusqlite::Connection,
    team_id: &str,
    template_id: &str,
    overrides: Value,
    admin_saved: bool,
) -> String {
    let id = Uuid::new_v4().to_string();
    let graph = json!({
        "nodes": [{
            "id": "run",
            "template_id": template_id,
            "name": "Run",
            "overrides": overrides
        }],
        "edges": []
    });
    crate::features::workflows::store::insert_workflow(
        db,
        &db::Workflow {
            id: id.clone(),
            team_id: team_id.to_string(),
            name: format!("Flow {id}"),
            description: String::new(),
            graph: graph.to_string(),
            created_at: db::now(),
            admin_saved,
        },
    )
    .expect("workflow");
    id
}

async fn wait_workflow(state: &Shared, id: &str) -> String {
    for _ in 0..50 {
        if let Ok(db) = state.db() {
            if let Ok(Some(run)) = crate::features::workflows::store::workflow_job(&db, id) {
                if matches!(run.status.as_str(), "successful" | "failed" | "canceled") {
                    return run.status;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    "timeout".into()
}

fn jobs_for(state: &Shared, template_id: &str) -> i64 {
    let db = state.db().expect("db");
    db.query_row(
        "SELECT COUNT(*) FROM jobs WHERE template_id = ?1",
        [template_id],
        |row| row.get(0),
    )
    .expect("count")
}

#[tokio::test]
async fn an_operator_inventory_blocks_a_local_run() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-inv-release-{nanos}"));
    let state = crate::boot(admins(&dir, RunnerMode::Local)).expect("boot");
    let team_id = team_of(&state);
    let inventory_id = {
        let db = state.db().expect("db");
        insert_inventory(&db, &team_id, false)
    };
    let template_id = {
        let db = state.db().expect("db");
        insert_shell(&db, &team_id, Some(&inventory_id), "true")
    };
    let denied = launch(
        &state,
        LaunchRequest {
            team_id: team_id.clone(),
            template_id: template_id.clone(),
            extra_vars: json!({}),
            launched_by: "admin".into(),
            trigger: "manual".into(),
            workflow_job_id: None,
            workflow_node_id: None,
            parameters_override: None,
            overrides: LaunchOverrides::default(),
        },
    )
    .expect_err("operator inventory");
    assert!(
        denied
            .to_string()
            .contains("local_runs_are_limited_to_administrators"),
        "{denied}"
    );
    {
        let db = state.db().expect("db");
        db.execute(
            "UPDATE inventories SET admin_saved = 1 WHERE id = ?1",
            [&inventory_id],
        )
        .expect("release");
    }
    launch(
        &state,
        LaunchRequest {
            team_id,
            template_id,
            extra_vars: json!({}),
            launched_by: "admin".into(),
            trigger: "manual".into(),
            workflow_job_id: None,
            workflow_node_id: None,
            parameters_override: None,
            overrides: LaunchOverrides::default(),
        },
    )
    .expect("admin inventory");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_netbox_config_context_inventory_blocks_a_local_run() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-inv-context-{nanos}"));
    let state = crate::boot(admins(&dir, RunnerMode::Local)).expect("boot");
    let team_id = team_of(&state);
    let inventory_id = {
        let db = state.db().expect("db");
        insert_inventory(&db, &team_id, true)
    };
    let template_id = {
        let db = state.db().expect("db");
        db.execute(
            "UPDATE inventories SET source_kind = 'netbox', source_json = ?2 WHERE id = ?1",
            rusqlite::params![&inventory_id, r#"{"load_config_context":true}"#],
        )
        .expect("netbox source");
        insert_shell(&db, &team_id, Some(&inventory_id), "true")
    };
    let denied = launch(
        &state,
        LaunchRequest {
            team_id: team_id.clone(),
            template_id: template_id.clone(),
            extra_vars: json!({}),
            launched_by: "admin".into(),
            trigger: "manual".into(),
            workflow_job_id: None,
            workflow_node_id: None,
            parameters_override: None,
            overrides: LaunchOverrides::default(),
        },
    )
    .expect_err("config context inventory");
    assert!(
        denied
            .to_string()
            .contains("local_runs_are_limited_to_administrators"),
        "{denied}"
    );
    {
        let db = state.db().expect("db");
        db.execute(
            "UPDATE inventories SET source_json = ?2 WHERE id = ?1",
            rusqlite::params![&inventory_id, r#"{"load_config_context":false}"#],
        )
        .expect("plain source");
    }
    launch(
        &state,
        LaunchRequest {
            team_id,
            template_id,
            extra_vars: json!({}),
            launched_by: "admin".into(),
            trigger: "manual".into(),
            workflow_job_id: None,
            workflow_node_id: None,
            parameters_override: None,
            overrides: LaunchOverrides::default(),
        },
    )
    .expect("plain netbox inventory");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn an_operator_workflow_cannot_replace_a_local_shell_script() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-wf-shell-{nanos}"));
    let marker = dir.join("operator-script-ran");
    let state = crate::boot(admins(&dir, RunnerMode::Local)).expect("boot");
    let team_id = team_of(&state);
    let template_id = {
        let db = state.db().expect("db");
        insert_shell(&db, &team_id, None, "true")
    };
    let workflow_id = {
        let db = state.db().expect("db");
        let script = format!("touch '{}'", marker.display());
        insert_override_workflow(
            &db,
            &team_id,
            &template_id,
            json!({"spec": {"script": script}}),
            false,
        )
    };
    let run_id = launch_workflow(&state, &team_id, &workflow_id, json!({}), "admin", "manual")
        .expect("start");
    let status = wait_workflow(&state, &run_id).await;
    assert_eq!(status, "failed", "{status}");
    assert_eq!(jobs_for(&state, &template_id), 0);
    assert!(!marker.exists(), "operator script ran");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn an_admin_workflow_cannot_point_a_local_run_at_an_operator_inventory() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-wf-inv-{nanos}"));
    let state = crate::boot(admins(&dir, RunnerMode::Local)).expect("boot");
    let team_id = team_of(&state);
    let (template_id, workflow_id) = {
        let db = state.db().expect("db");
        let template_id = insert_shell(&db, &team_id, None, "true");
        let inventory_id = insert_inventory(&db, &team_id, false);
        let workflow_id = insert_override_workflow(
            &db,
            &team_id,
            &template_id,
            json!({"inventory_id": inventory_id}),
            true,
        );
        (template_id, workflow_id)
    };
    let run_id = launch_workflow(&state, &team_id, &workflow_id, json!({}), "admin", "manual")
        .expect("start");
    let status = wait_workflow(&state, &run_id).await;
    assert_eq!(status, "failed", "{status}");
    assert_eq!(jobs_for(&state, &template_id), 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn relaunch_rejects_stored_parameters_the_template_does_not_have() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-relaunch-gate-{nanos}"));
    let marker = dir.join("stored-script-ran");
    let state = crate::boot(admins(&dir, RunnerMode::Auto)).expect("boot");
    let team_id = team_of(&state);
    let (template_id, environment_id) = {
        let db = state.db().expect("db");
        let template_id = insert_shell(&db, &team_id, None, "true");
        let environment_id: String = db
            .query_row("SELECT id FROM environments LIMIT 1", [], |row| row.get(0))
            .expect("environment");
        db.execute(
            "UPDATE templates SET environment_id = ?2 WHERE id = ?1",
            rusqlite::params![template_id, environment_id],
        )
        .expect("environment");
        (template_id, environment_id)
    };
    let script = format!("touch '{}'", marker.display());
    let stored = json!({
        "tool": "shell",
        "entrypoint": "",
        "spec": {"script": script},
        "environment_id": null,
        "extra_vars": {}
    })
    .to_string();
    {
        let db = state.db().expect("db");
        crate::features::jobs::store::insert_job(
            &db,
            &db::Job {
                id: "stored-shell".into(),
                team_id: team_id.clone(),
                template_id: Some(template_id.clone()),
                workflow_job_id: None,
                workflow_node_id: None,
                name: "Inline".into(),
                tool: "shell".into(),
                status: "failed".into(),
                extra_vars: "{}".into(),
                launched_by: "operator".into(),
                trigger: "manual".into(),
                started_at: None,
                finished_at: Some(db::now()),
                created_at: db::now(),
                parameters: stored,
                failed_hosts: "[]".into(),
            },
        )
        .expect("job");
    }
    let operator = relaunch(&state, "stored-shell", "operator", false).expect_err("operator");
    assert!(
        operator
            .to_string()
            .contains("local_runs_are_limited_to_administrators"),
        "{operator}"
    );
    let admin = relaunch(&state, "stored-shell", "admin", false).expect_err("admin");
    assert!(
        admin
            .to_string()
            .contains("local_runs_are_limited_to_administrators"),
        "{admin}"
    );
    assert!(!marker.exists(), "stored script ran");
    let matching = json!({
        "tool": "shell",
        "entrypoint": "",
        "spec": {"script": "true"},
        "environment_id": environment_id,
        "extra_vars": {}
    })
    .to_string();
    {
        let db = state.db().expect("db");
        db.execute(
            "UPDATE jobs SET parameters = ?1 WHERE id = 'stored-shell'",
            [&matching],
        )
        .expect("match");
    }
    relaunch(&state, "stored-shell", "admin", false).expect("matching snapshot");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn an_admin_workflow_may_replace_a_local_shell_script() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-wf-admin-shell-{nanos}"));
    let state = crate::boot(admins(&dir, RunnerMode::Local)).expect("boot");
    let team_id = team_of(&state);
    let (template_id, workflow_id) = {
        let db = state.db().expect("db");
        let template_id = insert_shell(&db, &team_id, None, "true");
        let workflow_id = insert_override_workflow(
            &db,
            &team_id,
            &template_id,
            json!({"spec": {"script": ":"}}),
            true,
        );
        (template_id, workflow_id)
    };
    let run_id = launch_workflow(&state, &team_id, &workflow_id, json!({}), "admin", "manual")
        .expect("start");
    let status = wait_workflow(&state, &run_id).await;
    assert_eq!(status, "successful", "{status}");
    assert_eq!(jobs_for(&state, &template_id), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn relaunch_of_a_workflow_snapshot_needs_that_workflow_release() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-wf-relaunch-{nanos}"));
    let state = crate::boot(admins(&dir, RunnerMode::Local)).expect("boot");
    let team_id = team_of(&state);
    let workflow_id = {
        let db = state.db().expect("db");
        let template_id = insert_shell(&db, &team_id, None, "true");
        let workflow_id = insert_override_workflow(&db, &team_id, &template_id, json!({}), false);
        crate::features::workflows::store::insert_workflow_job(
            &db,
            &db::WorkflowJob {
                id: "wf-run".into(),
                workflow_id: Some(workflow_id.clone()),
                team_id: team_id.clone(),
                name: "Flow".into(),
                status: "failed".into(),
                extra_vars: "{}".into(),
                launched_by: "admin".into(),
                trigger: "manual".into(),
                created_at: db::now(),
                finished_at: Some(db::now()),
            },
        )
        .expect("workflow job");
        crate::features::jobs::store::insert_job(
            &db,
            &db::Job {
                id: "node-job".into(),
                team_id: team_id.clone(),
                template_id: Some(template_id),
                workflow_job_id: Some("wf-run".into()),
                workflow_node_id: Some("run".into()),
                name: "Inline".into(),
                tool: "shell".into(),
                status: "failed".into(),
                extra_vars: "{}".into(),
                launched_by: "workflow:wf-run".into(),
                trigger: "workflow".into(),
                started_at: None,
                finished_at: Some(db::now()),
                created_at: db::now(),
                parameters: json!({
                    "tool": "shell",
                    "entrypoint": "",
                    "spec": {"script": ":"},
                    "extra_vars": {}
                })
                .to_string(),
                failed_hosts: "[]".into(),
            },
        )
        .expect("job");
        workflow_id
    };
    let denied = relaunch(&state, "node-job", "admin", false).expect_err("unreleased");
    assert!(
        denied
            .to_string()
            .contains("local_runs_are_limited_to_administrators"),
        "{denied}"
    );
    {
        let db = state.db().expect("db");
        db.execute(
            "UPDATE workflows SET admin_saved = 1 WHERE id = ?1",
            [&workflow_id],
        )
        .expect("release");
    }
    relaunch(&state, "node-job", "admin", false).expect("released workflow");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_stored_shell_snapshot_does_not_run_on_the_controller() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-stored-run-{nanos}"));
    let marker = dir.join("runner-script-ran");
    let state = crate::boot(admins(&dir, RunnerMode::Auto)).expect("boot");
    let team_id = team_of(&state);
    let template_id = {
        let db = state.db().expect("db");
        let template_id = insert_shell(&db, &team_id, None, "true");
        let environment_id: String = db
            .query_row("SELECT id FROM environments LIMIT 1", [], |row| row.get(0))
            .expect("environment");
        db.execute(
            "UPDATE templates SET environment_id = ?2 WHERE id = ?1",
            rusqlite::params![template_id, environment_id],
        )
        .expect("environment");
        template_id
    };
    let script = format!("touch '{}'", marker.display());
    {
        let db = state.db().expect("db");
        crate::features::jobs::store::insert_job(
            &db,
            &db::Job {
                id: "bypass".into(),
                team_id,
                template_id: Some(template_id),
                workflow_job_id: None,
                workflow_node_id: None,
                name: "Inline".into(),
                tool: "shell".into(),
                status: "pending".into(),
                extra_vars: "{}".into(),
                launched_by: "admin".into(),
                trigger: "manual".into(),
                started_at: None,
                finished_at: None,
                created_at: db::now(),
                parameters: json!({
                    "tool": "shell",
                    "entrypoint": "",
                    "spec": {"script": script},
                    "environment_id": null,
                    "extra_vars": {}
                })
                .to_string(),
                failed_hosts: "[]".into(),
            },
        )
        .expect("job");
    }
    runner::execute(state.clone(), "bypass".into()).await;
    let (status, log) = {
        let db = state.db().expect("db");
        let job = crate::features::jobs::store::job(&db, "bypass")
            .expect("read")
            .expect("job");
        let events =
            crate::features::jobs::store::events_after(&db, "bypass", 0, 50).expect("events");
        let log = events
            .iter()
            .map(|event| event.line.clone())
            .collect::<Vec<_>>()
            .join("\n");
        (job.status, log)
    };
    assert_eq!(status, "failed", "{log}");
    assert!(
        log.contains("Local runs are limited to administrators."),
        "{log}"
    );
    assert!(!marker.exists(), "stored script ran");
    let _ = std::fs::remove_dir_all(dir);
}

fn page_state(tag: &str) -> crate::Shared {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-page-{tag}-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    crate::boot(crate::Config {
        bind: "127.0.0.1:0".into(),
        data_dir: dir,
        web_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
        examples_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
        admin_user: "admin".into(),
        admin_password: Some("test-admin-pass".into()),
        runner: crate::config::RunnerMode::Local,
        container_image: false,
        job_timeout_secs: 60,
        job_output_lines: 8_000,
        local_jobs: crate::config::LocalJobs::All,
        project_roots: Vec::new(),
        hsts: false,
        cookie_secure: crate::config::CookieSecure::Auto,
        trusted_proxy: false,
        mcp: true,
        public_url: Vec::new(),
        data_host: None,
        ldap: None,
    })
    .expect("boot")
}

#[test]
fn job_page_keeps_the_start_and_the_end() {
    let state = page_state("head-tail");
    {
        let db = state.db().unwrap();
        for i in 1..=2_500 {
            crate::features::jobs::store::insert_event(&db, "job", "stdout", &format!("line-{i}"))
                .unwrap();
        }
    }
    let events = events_for_page(state.store(), "job").unwrap();
    let gaps: Vec<_> = events
        .iter()
        .filter(|event| event.stream == "gap")
        .collect();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].line, "500");
    let lines: Vec<_> = events.iter().map(|event| event.line.as_str()).collect();
    assert_eq!(lines[0], "line-1");
    assert_eq!(lines[1_499], "line-1500");
    assert_eq!(*lines.last().unwrap(), "line-2500");
    assert!(!lines.contains(&"line-1600"));
    assert!(lines.contains(&"line-2001"));
    let _ = std::fs::remove_dir_all(&state.cfg.data_dir);
}

#[test]
fn job_page_counts_only_this_jobs_omitted_events() {
    let state = page_state("gap-count");
    {
        let db = state.db().unwrap();
        for i in 1..=2_500 {
            crate::features::jobs::store::insert_event(&db, "job", "stdout", &format!("line-{i}"))
                .unwrap();
            crate::features::jobs::store::insert_event(
                &db,
                "other",
                "stdout",
                &format!("other-{i}"),
            )
            .unwrap();
        }
    }
    let events = events_for_page(state.store(), "job").unwrap();
    let gaps: Vec<_> = events
        .iter()
        .filter(|event| event.stream == "gap")
        .collect();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].line, "500");
    assert_eq!(
        state
            .store()
            .events_count_between("job", 0, i64::MAX)
            .unwrap(),
        2_500
    );
    assert_eq!(
        state
            .store()
            .events_count_between("other", 0, i64::MAX)
            .unwrap(),
        2_500
    );
    assert!(events.iter().all(|event| !event.line.starts_with("other-")));
    let _ = std::fs::remove_dir_all(&state.cfg.data_dir);
}
