#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use common::{
    call, call_cookie, config, create_database, drop_database, postgres_client, postgres_guard,
    postgres_test_url, Options,
};
use koreograf::LdapConfig;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PORTABLE_TABLES: &[&str] = &[
    "users",
    "organizations",
    "teams",
    "credentials",
    "projects",
    "inventories",
    "environments",
    "templates",
    "memberships",
    "inventory_sources",
    "template_credentials",
    "template_groups",
    "template_group_members",
    "workflows",
    "workflow_jobs",
    "jobs",
    "job_events",
    "host_results",
    "activity",
    "workflow_nodes",
    "hooks",
    "hook_deliveries",
    "project_teams",
    "settings",
    "seed_state",
    "schedules",
    "api_tokens",
    "sessions",
];

fn migrate_command(data_dir: &Path, target: &str) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_koreograf"))
        .args(["db", "migrate", "--to", target])
        .env("KOREOGRAF_DATA", data_dir)
        .env_remove("KOREOGRAF_DATABASE_URL")
        .env_remove("KOREOGRAF_DATABASE_PASSWORD_FILE")
        .env_remove("KOREOGRAF_DATABASE_CA_FILE")
        .output()
        .expect("run db migrate")
}

fn sqlite_counts(path: &Path) -> Vec<(&'static str, i64)> {
    let conn =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("sqlite source");
    PORTABLE_TABLES
        .iter()
        .map(|table| {
            let count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
                    row.get(0)
                })
                .expect("source count");
            (*table, count)
        })
        .collect()
}

fn seed_sqlite_source(dir: &Path) -> std::sync::Arc<koreograf::AppState> {
    let state = koreograf::boot(config(
        dir,
        &Options {
            mcp: false,
            ..Options::default()
        },
    ))
    .expect("sqlite boot");
    {
        let conn = state.db().expect("sqlite database");
        for line in ["first", "second"] {
            conn.execute(
                "INSERT INTO job_events (job_id, ts, stream, line)
                 VALUES ('job-copy', '2026-01-01T00:00:00Z', 'stdout', ?1)",
                [line],
            )
            .expect("job event");
        }
        conn.execute(
            "INSERT INTO host_results (
                job_id, team_id, inventory_id, host, playbook, role, result, ts
             ) VALUES (
                'job-copy', 'team-copy', 'inventory-copy', 'web-1', 'site.yml', 'web', 'ok',
                '2026-01-01T00:00:00Z'
             )",
            [],
        )
        .expect("host result");
        conn.execute(
            "INSERT INTO activity (
                id, team_id, actor, action, object_type, object_id, object_name, result, created_at
             ) VALUES (
                'activity-copy', 'team-copy', 'admin', 'create', 'project', 'project-copy',
                'Copied project', 'success', '2026-01-01T00:00:00Z'
             )",
            [],
        )
        .expect("activity entry");
    }
    state
}

#[tokio::test(flavor = "multi_thread")]
async fn db_migrate_copies_every_table_and_resets_sequences() {
    let Some(url) = postgres_test_url() else {
        return;
    };
    let _guard = postgres_guard().await;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let name = format!("koreograf_migrate_{nanos}");
    let target = create_database(&url, &name).await;
    let dir = std::env::temp_dir().join(format!("koreograf-migrate-source-{nanos}"));
    let state = seed_sqlite_source(&dir);
    drop(state);

    let output = migrate_command(&dir, &target);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("KOREOGRAF_DATABASE_URL='"), "{stdout}");
    assert!(stdout.contains(&name), "{stdout}");
    assert!(stdout.contains("sslmode=disable"), "{stdout}");

    let client = postgres_client(&target).await;
    let source = rusqlite::Connection::open_with_flags(
        dir.join("koreograf.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("source database");
    let source_version: u32 = source
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("source schema version");
    drop(source);
    let target_version: String = client
        .query_one(
            "SELECT value FROM schema_meta WHERE key = 'schema_version'",
            &[],
        )
        .await
        .expect("target schema version")
        .get(0);
    assert_eq!(target_version, source_version.to_string());
    for (table, expected) in sqlite_counts(&dir.join("koreograf.db")) {
        let actual: i64 = client
            .query_one(&format!("SELECT COUNT(*) FROM \"{table}\""), &[])
            .await
            .expect("target count")
            .get(0);
        assert_eq!(actual, expected, "table {table}");
    }
    client
        .execute(
            "INSERT INTO job_events (job_id, ts, stream, line)
             VALUES ('job-copy', '2026-01-01T00:00:00Z', 'stdout', 'third')",
            &[],
        )
        .await
        .expect("insert after copy");
    let next: i64 = client
        .query_one("SELECT MAX(id) FROM job_events", &[])
        .await
        .expect("max id")
        .get(0);
    assert_eq!(next, 3);
    client
        .execute(
            "INSERT INTO host_results (
                job_id, team_id, inventory_id, host, playbook, role, result, ts
             ) VALUES (
                'job-copy', 'team-copy', 'inventory-copy', 'web-2', 'site.yml', 'web', 'ok',
                '2026-01-01T00:00:00Z'
             )",
            &[],
        )
        .await
        .expect("insert host result after copy");
    let next: i64 = client
        .query_one("SELECT MAX(id) FROM host_results", &[])
        .await
        .expect("max host result id")
        .get(0);
    assert_eq!(next, 2);

    let again = migrate_command(&dir, &target);
    assert!(!again.status.success());
    assert!(
        String::from_utf8_lossy(&again.stderr).contains("target is not empty"),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );

    drop(client);
    drop_database(&url, &name).await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn db_migrate_refuses_while_the_data_directory_is_in_use() {
    let Some(url) = postgres_test_url() else {
        return;
    };
    let _guard = postgres_guard().await;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-migrate-locked-{nanos}"));
    let state = seed_sqlite_source(&dir);
    drop(state);
    let before = std::fs::read(dir.join("koreograf.db")).expect("sqlite file");
    let _lock = koreograf::lock_data_directory(&dir).expect("data directory lock");

    let output = migrate_command(&dir, &url);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("already in use"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let after = std::fs::read(dir.join("koreograf.db")).expect("sqlite file");
    assert_eq!(before, after);

    drop(_lock);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn db_migrate_failure_leaves_the_target_reusable() {
    let Some(url) = postgres_test_url() else {
        return;
    };
    let _guard = postgres_guard().await;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let name = format!("koreograf_migrate_retry_{nanos}");
    let target = create_database(&url, &name).await;
    let dir = std::env::temp_dir().join(format!("koreograf-migrate-retry-{nanos}"));
    let state = seed_sqlite_source(&dir);
    drop(state);

    let first = migrate_command(&dir, &target);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );

    let client = postgres_client(&target).await;
    let tables = PORTABLE_TABLES
        .iter()
        .map(|table| format!("\"{table}\""))
        .collect::<Vec<_>>()
        .join(", ");
    client
        .batch_execute(&format!("TRUNCATE TABLE {tables} RESTART IDENTITY CASCADE"))
        .await
        .expect("truncate target");
    client
        .batch_execute(
            "CREATE OR REPLACE FUNCTION koreograf_fail_migrate() RETURNS trigger
             LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'migration blocked'; END $$;
             CREATE TRIGGER koreograf_fail_migrate BEFORE INSERT ON users
             FOR EACH ROW EXECUTE FUNCTION koreograf_fail_migrate();",
        )
        .await
        .expect("failure trigger");

    let failed = migrate_command(&dir, &target);
    assert!(!failed.status.success());
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("rolled back and can be reused"),
        "{}",
        String::from_utf8_lossy(&failed.stderr)
    );
    for table in PORTABLE_TABLES {
        let count: i64 = client
            .query_one(&format!("SELECT COUNT(*) FROM \"{table}\""), &[])
            .await
            .expect("target count")
            .get(0);
        assert_eq!(count, 0, "table {table} kept rows after a failed run");
    }

    client
        .batch_execute(
            "DROP TRIGGER koreograf_fail_migrate ON users;
             DROP FUNCTION koreograf_fail_migrate();",
        )
        .await
        .expect("drop failure trigger");

    let retried = migrate_command(&dir, &target);
    assert!(
        retried.status.success(),
        "{}",
        String::from_utf8_lossy(&retried.stderr)
    );
    let users: i64 = client
        .query_one("SELECT COUNT(*) FROM users", &[])
        .await
        .expect("users count")
        .get(0);
    assert!(users > 0);

    drop(client);
    drop_database(&url, &name).await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn postgres_answers_teams_while_another_request_holds_a_connection() {
    let Some(url) = postgres_test_url() else {
        return;
    };
    let _guard = postgres_guard().await;
    let database = koreograf::parse_database_config(&url, None, None).expect("postgres url");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-postgres-pool-{nanos}"));
    let mut cfg = config(
        &dir,
        &Options {
            mcp: false,
            ..Options::default()
        },
    );
    let admin = format!("pgpooladmin{nanos}");
    cfg.admin_user = admin.clone();
    let state = koreograf::boot_with(cfg, database).expect("boot");
    let app = koreograf::router(state);

    let (status, _, cookie) = call_cookie(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": admin, "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("session cookie");
    let (status, teams) = call(&app, "GET", "/api/teams", Some(&cookie), None, None).await;
    assert_eq!(status, StatusCode::OK, "{teams}");
    let team_id = teams
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item["id"].as_str())
        .expect("team id")
        .to_string();

    let holder = postgres_client(&url).await;
    holder.batch_execute("BEGIN").await.expect("begin");
    holder
        .execute("SELECT 1 FROM teams WHERE id = $1 FOR UPDATE", &[&team_id])
        .await
        .expect("lock team");

    let put_app = app.clone();
    let put_cookie = cookie.clone();
    let put_team = team_id.clone();
    let put = tokio::spawn(async move {
        call(
            &put_app,
            "PUT",
            &format!("/api/teams/{put_team}"),
            Some(&put_cookie),
            None,
            Some(json!({"name": format!("Locked {nanos}"), "description": "pool test"})),
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    let started = Instant::now();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        call(&app, "GET", "/api/teams", Some(&cookie), None, None),
    )
    .await;
    let elapsed = started.elapsed();
    holder.batch_execute("ROLLBACK").await.expect("rollback");
    let update = put.await.expect("update task");
    assert_eq!(update.0, StatusCode::OK, "{}", update.1);
    let (status, teams) = response.expect("GET /api/teams waited for the blocked request");
    assert_eq!(status, StatusCode::OK, "{teams}");
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "GET /api/teams took {elapsed:?} while another request held a connection"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn postgres_ldap_sign_in_provisions_the_user_and_syncs_teams() {
    let Some(url) = postgres_test_url() else {
        return;
    };
    let _guard = postgres_guard().await;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let name = format!("koreograf_ldap_{nanos}");
    let target = create_database(&url, &name).await;
    let dir = std::env::temp_dir().join(format!("koreograf-postgres-ldap-{nanos}"));
    let ldap_username = format!("ldap{nanos}");
    let ldap_email = format!("{ldap_username}@example.com");
    let (ldap_url, ldap_server) = ldap_stub(ldap_username.clone()).await;
    let mut cfg = config(
        &dir,
        &Options {
            mcp: false,
            ..Options::default()
        },
    );
    cfg.admin_user = format!("ldapadmin{nanos}");
    cfg.ldap = Some(LdapConfig {
        url: ldap_url,
        mode: "ldap".into(),
        bind_dn: "cn=reader,dc=example,dc=com".into(),
        bind_password: "reader-password".into(),
        user_base: "ou=people,dc=example,dc=com".into(),
        user_filter: "(uid={username})".into(),
        username_attr: "uid".into(),
        group_attr: "memberOf".into(),
        group_base: String::new(),
        starttls: false,
        tls_insecure: false,
    });
    let database = koreograf::parse_database_config(&target, None, None).expect("postgres url");
    let state = koreograf::boot_with(cfg, database).expect("boot");
    assert_eq!(state.engine(), "postgresql");
    let app = koreograf::router(state);

    let (status, _, cookie) = call_cookie(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": format!("ldapadmin{nanos}"), "password": "test-admin-pass"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let admin = cookie.expect("admin session");

    let (status, org) = call(
        &app,
        "POST",
        "/api/organizations",
        Some(&admin),
        None,
        Some(json!({
            "name": format!("LDAP org {nanos}"),
            "description": "Postgres LDAP organization"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{org}");
    let org_id = org["id"].as_str().expect("organization id").to_string();
    let group = "cn=Plattform,ou=groups,dc=example,dc=com";
    let mut team_ids = Vec::new();
    for suffix in ["A", "B"] {
        let (status, team) = call(
            &app,
            "POST",
            "/api/teams",
            Some(&admin),
            None,
            Some(json!({
                "name": format!("LDAP team {suffix} {nanos}"),
                "description": "Postgres LDAP team",
                "ldap_group": group,
                "organization_id": org_id
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{team}");
        team_ids.push(team["id"].as_str().expect("team id").to_string());
    }

    let (status, profile, cookie) = call_cookie(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": ldap_username.clone(), "password": "correct horse"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{profile}");
    assert_eq!(profile["auth_source"], "ldap");
    assert_eq!(profile["display_name"], "Directory User");
    assert_eq!(profile["email"], ldap_email);
    let ldap_cookie = cookie.expect("ldap session");
    let memberships = profile["memberships"]
        .as_array()
        .expect("memberships")
        .clone();
    for team_id in &team_ids {
        let membership = memberships
            .iter()
            .find(|item| item["team_id"] == *team_id)
            .expect("ldap membership");
        assert_eq!(membership["role"], "operator");
        assert_eq!(membership["source"], "ldap");
    }

    let (status, _) = call(
        &app,
        "PUT",
        &format!("/api/teams/{}", team_ids[1]),
        Some(&admin),
        None,
        Some(json!({
            "name": format!("LDAP team B {nanos}"),
            "description": "Postgres LDAP team",
            "ldap_group": "cn=Other,ou=groups,dc=example,dc=com",
            "organization_id": org_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = call(
        &app,
        "POST",
        "/api/login",
        None,
        None,
        Some(json!({"username": ldap_username.clone(), "password": "correct horse"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, profile) = call(&app, "GET", "/api/me", Some(&ldap_cookie), None, None).await;
    assert_eq!(status, StatusCode::OK);
    let memberships = profile["memberships"].as_array().expect("memberships");
    assert!(memberships
        .iter()
        .any(|item| item["team_id"] == team_ids[0]));
    assert!(!memberships
        .iter()
        .any(|item| item["team_id"] == team_ids[1]));

    ldap_server.abort();
    drop_database(&url, &name).await;
    let _ = std::fs::remove_dir_all(&dir);
}

async fn ldap_stub(username: String) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("ldap stub bind");
    let addr = listener.local_addr().expect("ldap stub address");
    let username = std::sync::Arc::new(username);
    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let username = username.clone();
            tokio::spawn(async move {
                let _ = serve_ldap(&mut stream, &username).await;
            });
        }
    });
    (format!("ldap://{addr}"), handle)
}

async fn serve_ldap(stream: &mut tokio::net::TcpStream, username: &str) -> std::io::Result<()> {
    while let Some(message) = read_ldap_message(stream).await? {
        let Some(body) = ber_body(&message) else {
            return Ok(());
        };
        let Some((tag, _, id_len)) = ber_read(body) else {
            return Ok(());
        };
        if tag != 0x02 {
            return Ok(());
        }
        let message_id = &body[..id_len];
        let Some((op, _, _)) = ber_read(&body[id_len..]) else {
            return Ok(());
        };
        match op {
            0x60 => {
                let mut content = message_id.to_vec();
                content.extend(ldap_result(0x61, 0));
                stream.write_all(&ber(0x30, &content)).await?;
            }
            0x63 => {
                for response in [ldap_entry(username), ldap_result(0x65, 0)] {
                    let mut content = message_id.to_vec();
                    content.extend(response);
                    stream.write_all(&ber(0x30, &content)).await?;
                }
            }
            _ => return Ok(()),
        }
    }
    Ok(())
}

async fn read_ldap_message(stream: &mut tokio::net::TcpStream) -> std::io::Result<Option<Vec<u8>>> {
    let mut tag = [0u8; 1];
    match stream.read_exact(&mut tag).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let mut first = [0u8; 1];
    stream.read_exact(&mut first).await?;
    let mut message = vec![tag[0], first[0]];
    let len = if first[0] & 0x80 == 0 {
        usize::from(first[0])
    } else {
        let count = usize::from(first[0] & 0x7f);
        let mut bytes = vec![0u8; count];
        stream.read_exact(&mut bytes).await?;
        message.extend_from_slice(&bytes);
        bytes
            .iter()
            .fold(0usize, |value, byte| (value << 8) | usize::from(*byte))
    };
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    message.extend_from_slice(&body);
    Ok(Some(message))
}

fn ber_body(input: &[u8]) -> Option<&[u8]> {
    ber_read(input).map(|(_, body, _)| body)
}

fn ber_read(input: &[u8]) -> Option<(u8, &[u8], usize)> {
    if input.len() < 2 {
        return None;
    }
    let tag = input[0];
    let first = usize::from(input[1]);
    let (len, header) = if first & 0x80 == 0 {
        (first, 2)
    } else {
        let count = first & 0x7f;
        if count == 0 || input.len() < 2 + count {
            return None;
        }
        let len = input[2..2 + count]
            .iter()
            .fold(0usize, |value, byte| (value << 8) | usize::from(*byte));
        (len, 2 + count)
    };
    let end = header.checked_add(len)?;
    if input.len() < end {
        return None;
    }
    Some((tag, &input[header..end], end))
}

fn ber(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = body.len();
    if len < 128 {
        out.push(len as u8);
    } else {
        let mut bytes = Vec::new();
        let mut remaining = len;
        while remaining > 0 {
            bytes.push((remaining & 0xff) as u8);
            remaining >>= 8;
        }
        bytes.reverse();
        out.push(0x80 | bytes.len() as u8);
        out.extend_from_slice(&bytes);
    }
    out.extend_from_slice(body);
    out
}

fn ldap_result(tag: u8, code: u8) -> Vec<u8> {
    let body = vec![0x0a, 0x01, code, 0x04, 0x00, 0x04, 0x00];
    ber(tag, &body)
}

fn ldap_entry(username: &str) -> Vec<u8> {
    let email = format!("{username}@example.com");
    let attributes = [
        ("uid", vec![username]),
        ("cn", vec!["Directory User"]),
        ("displayName", vec!["Directory User"]),
        ("mail", vec![email.as_str()]),
        ("memberOf", vec!["cn=Plattform,ou=groups,dc=example,dc=com"]),
    ];
    let mut list = Vec::new();
    for (name, values) in attributes {
        let mut partial = ber(0x04, name.as_bytes());
        let mut set = Vec::new();
        for value in values {
            set.extend(ber(0x04, value.as_bytes()));
        }
        partial.extend(ber(0x31, &set));
        list.extend(ber(0x30, &partial));
    }
    let mut body = ber(
        0x04,
        format!("uid={username},ou=people,dc=example,dc=com").as_bytes(),
    );
    body.extend(ber(0x30, &list));
    ber(0x64, &body)
}
