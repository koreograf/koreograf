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
        last_sync_by: String::new(),
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

#[cfg(test)]
mod template_groups {
    use crate::backend::{Backend, PostgresBackend};
    use crate::config::DatabaseConfig;
    use crate::db::{self, Template, TemplateGroup};
    use crate::features::projects::store;

    fn template(id: &str, team: &str, name: &str) -> Template {
        Template {
            id: id.into(),
            team_id: team.into(),
            name: name.into(),
            description: String::new(),
            tool: "shell".into(),
            project_id: None,
            inventory_id: None,
            credential_id: None,
            environment_id: None,
            no_environment: true,
            entrypoint: String::new(),
            extra_vars: "{}".into(),
            spec: r#"{"script":"true"}"#.into(),
            created_at: db::now(),
            update_on_launch: true,
            admin_saved: true,
            prompt_on_launch: false,
            prompt_fields: "[]".into(),
        }
    }

    fn group(id: &str, team: &str, name: &str, position: i64) -> TemplateGroup {
        TemplateGroup {
            id: id.into(),
            team_id: team.into(),
            name: name.into(),
            position,
        }
    }

    fn sqlite_connection() -> (std::path::PathBuf, rusqlite::Connection) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "koreograf-template-groups-{nanos}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = db::open(&dir.join("koreograf.db")).unwrap();
        (dir, conn)
    }

    fn seed_team(conn: &rusqlite::Connection, id: &str, name: &str) {
        conn.execute(
            "INSERT INTO teams (id, name, description, created_at) VALUES (?1, ?2, '', ?3)",
            rusqlite::params![id, name, db::now()],
        )
        .unwrap();
    }

    fn assert_store_rules(backend: &dyn Backend) {
        backend
            .insert_team("tg-team", "", "Group Team", "", None)
            .unwrap();
        backend
            .insert_team("tg-other", "", "Other Team", "", None)
            .unwrap();
        backend
            .insert_template(&template("tg-template", "tg-team", "One"), &[], None)
            .unwrap();
        backend
            .insert_template(&template("tg-second", "tg-team", "Two"), &[], None)
            .unwrap();
        backend
            .insert_template_group(&group("tg-patchday", "tg-team", "Patchday", 0))
            .unwrap();
        backend
            .insert_template_group(&group("tg-kubernetes", "tg-team", "Kubernetes", 1))
            .unwrap();
        backend
            .insert_template_group(&group("tg-other-group", "tg-other", "Patchday", 0))
            .unwrap();

        let duplicate = backend
            .insert_template_group(&group("tg-duplicate", "tg-team", "Patchday", 2))
            .unwrap_err();
        assert_eq!(
            duplicate.to_string(),
            "that_name_is_already_used_in_this_team"
        );

        let names: Vec<String> = backend
            .list_template_groups("tg-team")
            .unwrap()
            .into_iter()
            .map(|item| item.name)
            .collect();
        assert_eq!(names, ["Patchday", "Kubernetes"]);

        backend
            .update_template(
                &template("tg-template", "tg-team", "One"),
                &[],
                Some(&["tg-patchday".to_string(), "tg-kubernetes".to_string()]),
            )
            .unwrap();
        assert_eq!(
            backend.template_group_ids("tg-template").unwrap(),
            ["tg-patchday", "tg-kubernetes"]
        );

        backend
            .delete_template_group("tg-kubernetes", "tg-team")
            .unwrap();
        assert_eq!(
            backend.template_group_ids("tg-template").unwrap(),
            ["tg-patchday"]
        );
        assert!(backend.template("tg-template").unwrap().is_some());

        backend.delete_template("tg-second", "tg-team").unwrap();
        assert!(backend.template("tg-second").unwrap().is_none());

        backend
            .delete_template_group("tg-patchday", "tg-team")
            .unwrap();
        assert!(backend
            .template_group_ids("tg-template")
            .unwrap()
            .is_empty());
        assert!(backend.template("tg-template").unwrap().is_some());
    }

    #[test]
    fn sqlite_groups_are_unique_and_keep_templates_when_a_group_or_team_goes() {
        let (dir, conn) = sqlite_connection();
        seed_team(&conn, "tg-team", "Group Team");
        seed_team(&conn, "tg-other", "Other Team");
        store::insert_template(&conn, &template("tg-template", "tg-team", "One")).unwrap();
        store::insert_template(&conn, &template("tg-second", "tg-team", "Two")).unwrap();
        store::insert_template_group(&conn, &group("tg-patchday", "tg-team", "Patchday", 0))
            .unwrap();
        store::insert_template_group(&conn, &group("tg-kubernetes", "tg-team", "Kubernetes", 1))
            .unwrap();
        store::insert_template_group(&conn, &group("tg-other-group", "tg-other", "Patchday", 0))
            .unwrap();

        let duplicate =
            store::insert_template_group(&conn, &group("tg-duplicate", "tg-team", "Patchday", 2))
                .unwrap_err();
        assert_eq!(
            duplicate.to_string(),
            "that_name_is_already_used_in_this_team"
        );

        store::save_template_groups(
            &conn,
            "tg-template",
            &["tg-patchday".to_string(), "tg-kubernetes".to_string()],
        )
        .unwrap();
        assert_eq!(
            store::template_group_ids(&conn, "tg-template").unwrap(),
            ["tg-patchday", "tg-kubernetes"]
        );

        store::delete_template_group(&conn, "tg-kubernetes", "tg-team").unwrap();
        assert_eq!(
            store::template_group_ids(&conn, "tg-template").unwrap(),
            ["tg-patchday"]
        );
        assert!(store::template(&conn, "tg-template").unwrap().is_some());

        conn.execute("DELETE FROM teams WHERE id = 'tg-team'", [])
            .unwrap();
        assert!(store::template_group(&conn, "tg-patchday")
            .unwrap()
            .is_none());
        assert!(store::template(&conn, "tg-template").unwrap().is_none());
        assert!(store::template_group_ids(&conn, "tg-template")
            .unwrap()
            .is_empty());
        assert!(store::template_group(&conn, "tg-other-group")
            .unwrap()
            .is_some());

        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    async fn create_postgres_database(name: &str) -> String {
        let base = std::env::var("KOREOGRAF_TEST_POSTGRES_URL").unwrap();
        let mut admin_url = url::Url::parse(&base).unwrap();
        admin_url.set_path("/postgres");
        let (admin, connection) =
            tokio_postgres::connect(admin_url.as_str(), tokio_postgres::NoTls)
                .await
                .unwrap();
        tokio::spawn(async move {
            let _ = connection.await;
        });
        admin
            .batch_execute(&format!("CREATE DATABASE \"{name}\""))
            .await
            .unwrap();
        let mut target = url::Url::parse(&base).unwrap();
        target.set_path(&format!("/{name}"));
        target.to_string()
    }

    async fn drop_postgres_database(name: &str) {
        let base = std::env::var("KOREOGRAF_TEST_POSTGRES_URL").unwrap();
        let mut admin_url = url::Url::parse(&base).unwrap();
        admin_url.set_path("/postgres");
        let (admin, connection) =
            tokio_postgres::connect(admin_url.as_str(), tokio_postgres::NoTls)
                .await
                .unwrap();
        tokio::spawn(async move {
            let _ = connection.await;
        });
        admin
            .execute(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity
                 WHERE datname = $1 AND pid <> pg_backend_pid()",
                &[&name],
            )
            .await
            .unwrap();
        admin
            .batch_execute(&format!("DROP DATABASE \"{name}\""))
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn postgres_groups_are_unique_and_keep_templates_when_a_group_or_team_goes() {
        let Ok(url) = std::env::var("KOREOGRAF_TEST_POSTGRES_URL") else {
            eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is not set");
            return;
        };
        if url.trim().is_empty() {
            eprintln!("skipping PostgreSQL test: KOREOGRAF_TEST_POSTGRES_URL is empty");
            return;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name = format!("koreograf_groups_{}_{nanos}", std::process::id());
        let target = create_postgres_database(&name).await;
        let DatabaseConfig::Postgres(config) =
            crate::config::parse_database_config(&target, None, None).unwrap()
        else {
            panic!("postgres config");
        };
        let backend = PostgresBackend::open(&config).unwrap();
        assert_store_rules(&backend);

        let client = crate::backend::connect_client(&config).await.unwrap();
        client
            .execute("DELETE FROM teams WHERE id = 'tg-team'", &[])
            .await
            .unwrap();
        let remaining: i64 = client
            .query_one(
                "SELECT COUNT(*) FROM template_groups WHERE team_id = 'tg-team'",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(remaining, 0);
        let templates: i64 = client
            .query_one(
                "SELECT COUNT(*) FROM templates WHERE team_id = 'tg-team'",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(templates, 0);
        let other: i64 = client
            .query_one(
                "SELECT COUNT(*) FROM template_groups WHERE id = 'tg-other-group'",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(other, 1);
        drop(client);
        drop(backend);
        drop_postgres_database(&name).await;
    }
}
