#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{
    config, create_database, drop_database, postgres_client, postgres_database_name,
    postgres_guard, postgres_test_url, temp_dir, Options,
};
use koreograf::Shared;

const ALPHA10: &str = "alpha10";
const ALPHA12: &str = "alpha12";
const ALPHA15: &str = "alpha15";
/// The schema version the released `0.1.0-alpha.15` image wrote.
const ALPHA15_SCHEMA_VERSION: u32 = 10;

fn fixture(release: &str, name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(release)
        .join(name)
}

fn options() -> Options {
    Options {
        mcp: false,
        ..Options::default()
    }
}

fn latest_sqlite_version() -> u32 {
    let dir = temp_dir("upgrade-latest");
    let state = koreograf::boot(config(&dir, &options())).expect("fresh sqlite boot");
    let version = {
        let conn = state.db().expect("database");
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("schema version")
    };
    drop(state);
    let _ = fs::remove_dir_all(&dir);
    version
}

fn assert_fixture_rows(conn: &rusqlite::Connection) {
    let count = |table: &str| -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("fixture count")
    };
    assert!(count("users") >= 1);
    assert!(count("teams") >= 1);
    assert!(count("templates") >= 1);
    assert!(count("jobs") >= 1);
    let job: String = conn
        .query_row(
            "SELECT name FROM jobs WHERE id = 'fixture-job-1'",
            [],
            |row| row.get(0),
        )
        .expect("fixture job");
    assert_eq!(job, "Fixture check");
}

fn assert_alpha_12_fixture_rows(conn: &rusqlite::Connection) {
    let count = |table: &str| -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("fixture count")
    };
    assert!(count("users") >= 2);
    assert!(count("teams") >= 2);
    assert!(count("templates") >= 1);
    assert!(count("jobs") >= 2);
    assert!(count("workflows") >= 2);
    let user: String = conn
        .query_row(
            "SELECT username FROM users WHERE username = 'fixture-user'",
            [],
            |row| row.get(0),
        )
        .expect("fixture user");
    assert_eq!(user, "fixture-user");
    let team: String = conn
        .query_row(
            "SELECT name FROM teams WHERE name = 'Fixture Team'",
            [],
            |row| row.get(0),
        )
        .expect("fixture team");
    assert_eq!(team, "Fixture Team");
    let template: String = conn
        .query_row(
            "SELECT name FROM templates WHERE name = 'Fixture check'",
            [],
            |row| row.get(0),
        )
        .expect("fixture template");
    assert_eq!(template, "Fixture check");
    let jobs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM jobs WHERE name = 'Fixture check'",
            [],
            |row| row.get(0),
        )
        .expect("fixture jobs");
    assert_eq!(jobs, 2);
    let triggers: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT trigger) FROM jobs WHERE name = 'Fixture check'",
            [],
            |row| row.get(0),
        )
        .expect("fixture job triggers");
    assert_eq!(triggers, 2);
    let workflow: String = conn
        .query_row(
            "SELECT name FROM workflows WHERE name = 'Fixture workflow'",
            [],
            |row| row.get(0),
        )
        .expect("fixture workflow");
    assert_eq!(workflow, "Fixture workflow");
    let version: i64 = conn
        .query_row(
            "SELECT version FROM workflows WHERE name = 'Fixture workflow'",
            [],
            |row| row.get(0),
        )
        .expect("workflow version");
    assert_eq!(version, 1);
}

fn assert_alpha_15_fixture_rows(conn: &rusqlite::Connection) {
    let count = |table: &str| -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("fixture count")
    };
    assert!(count("users") >= 2);
    assert!(count("teams") >= 2);
    assert!(count("templates") >= 1);
    assert!(count("jobs") >= 3);
    assert!(count("workflows") >= 1);
    assert_eq!(count("host_results"), 3);
    assert_eq!(count("sessions"), 2);
    assert!(count("activity") >= 2);
    assert!(count("hook_deliveries") >= 2);
    assert!(count("api_tokens") >= 3);

    let user: String = conn
        .query_row(
            "SELECT username FROM users WHERE username = 'fixture-user'",
            [],
            |row| row.get(0),
        )
        .expect("fixture user");
    assert_eq!(user, "fixture-user");
    let team: String = conn
        .query_row(
            "SELECT name FROM teams WHERE name = 'Fixture Team'",
            [],
            |row| row.get(0),
        )
        .expect("fixture team");
    assert_eq!(team, "Fixture Team");
    let template: String = conn
        .query_row(
            "SELECT name FROM templates WHERE name = 'Fixture check'",
            [],
            |row| row.get(0),
        )
        .expect("fixture template");
    assert_eq!(template, "Fixture check");

    let jobs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM jobs WHERE name = 'Fixture check'",
            [],
            |row| row.get(0),
        )
        .expect("fixture jobs");
    assert_eq!(jobs, 3);
    let triggers: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT trigger) FROM jobs WHERE name = 'Fixture check'",
            [],
            |row| row.get(0),
        )
        .expect("fixture job triggers");
    assert_eq!(triggers, 3);

    let workflow: String = conn
        .query_row(
            "SELECT name FROM workflows WHERE name = 'Fixture workflow'",
            [],
            |row| row.get(0),
        )
        .expect("fixture workflow");
    assert_eq!(workflow, "Fixture workflow");
    let version: i64 = conn
        .query_row(
            "SELECT version FROM workflows WHERE name = 'Fixture workflow'",
            [],
            |row| row.get(0),
        )
        .expect("workflow version");
    assert_eq!(version, 3);

    let reachable: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM host_results WHERE host = 'web01.example.test'
             AND ok = 1 AND changed = 0 AND failed = 0
             AND result LIKE '%Fixture host is reachable%'",
            [],
            |row| row.get(0),
        )
        .expect("reachable host");
    assert_eq!(reachable, 1);
    let failed: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM host_results WHERE host = 'web01.example.test'
             AND ok = 0 AND failed = 1
             AND result LIKE '%Fixture host failed%'",
            [],
            |row| row.get(0),
        )
        .expect("failed host");
    assert_eq!(failed, 1);

    let activity: String = conn
        .query_row(
            "SELECT result FROM activity WHERE id = 'fixture-activity-failed'",
            [],
            |row| row.get(0),
        )
        .expect("fixture activity");
    assert_eq!(activity, "failed");
    let action: String = conn
        .query_row(
            "SELECT action FROM activity WHERE id = 'fixture-activity-failed'",
            [],
            |row| row.get(0),
        )
        .expect("fixture activity action");
    assert_eq!(action, "update");

    let write_scope: String = conn
        .query_row(
            "SELECT scope FROM api_tokens WHERE name = 'fixture-write-token'",
            [],
            |row| row.get(0),
        )
        .expect("write token");
    assert_eq!(write_scope, "write");
    let write_expiry: Option<String> = conn
        .query_row(
            "SELECT expires_at FROM api_tokens WHERE name = 'fixture-write-token'",
            [],
            |row| row.get(0),
        )
        .expect("write token expiry");
    assert!(write_expiry.is_none());
    let expired_scope: String = conn
        .query_row(
            "SELECT scope FROM api_tokens WHERE name = 'fixture-expired-token'",
            [],
            |row| row.get(0),
        )
        .expect("expired token");
    assert_eq!(expired_scope, "read");
    let expired_at: String = conn
        .query_row(
            "SELECT expires_at FROM api_tokens WHERE name = 'fixture-expired-token'",
            [],
            |row| row.get(0),
        )
        .expect("expired token expiry");
    assert_eq!(expired_at, "2026-01-01T09:00:00Z");

    let session: (String, Option<String>) = conn
        .query_row(
            "SELECT id, created_at FROM sessions WHERE token = 'fixture-session-token-user'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("fixture session");
    assert_eq!(session.0, "fixture-session-user");
    assert!(session.1.is_some());

    let delivery: String = conn
        .query_row(
            "SELECT status FROM hook_deliveries WHERE id = 'fixture-delivery-ignored'",
            [],
            |row| row.get(0),
        )
        .expect("fixture delivery");
    assert_eq!(delivery, "ignored");

    let vars: String = conn
        .query_row(
            "SELECT vars_json FROM inventories WHERE name = 'Fixture inventory'",
            [],
            |row| row.get(0),
        )
        .expect("inventory vars");
    assert!(vars.contains("group_vars/all.yml"));
    assert!(vars.contains("fixture_name: alpha15"));
    let source_vars: String = conn
        .query_row(
            "SELECT vars_json FROM inventory_sources WHERE inventory_id =
             (SELECT id FROM inventories WHERE name = 'Fixture inventory')",
            [],
            |row| row.get(0),
        )
        .expect("inventory source vars");
    assert!(source_vars.contains("group_vars/web.yml"));
    assert!(source_vars.contains("fixture_web: true"));

    let created_by: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('users') WHERE name = 'created_by'",
            [],
            |row| row.get(0),
        )
        .expect("created_by column");
    assert_eq!(created_by, 1);
    let service_accounts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM users WHERE auth_source = 'service'",
            [],
            |row| row.get(0),
        )
        .expect("service accounts");
    assert_eq!(service_accounts, 0);
    let with_creator: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM users WHERE created_by IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .expect("existing creators");
    assert_eq!(with_creator, 0);
}

async fn assert_alpha_15_postgres_rows(client: &tokio_postgres::Client) {
    for (table, minimum) in [
        ("users", 2),
        ("teams", 2),
        ("templates", 1),
        ("jobs", 3),
        ("workflows", 1),
        ("host_results", 3),
        ("sessions", 2),
        ("activity", 2),
        ("hook_deliveries", 2),
        ("api_tokens", 3),
    ] {
        let count: i64 = client
            .query_one(&format!("SELECT COUNT(*) FROM {table}"), &[])
            .await
            .expect("fixture count")
            .get(0);
        assert!(count >= minimum, "{table} is empty");
    }

    let user: String = client
        .query_one(
            "SELECT username FROM users WHERE username = 'fixture-user'",
            &[],
        )
        .await
        .expect("fixture user")
        .get(0);
    assert_eq!(user, "fixture-user");
    let team: String = client
        .query_one("SELECT name FROM teams WHERE name = 'Fixture Team'", &[])
        .await
        .expect("fixture team")
        .get(0);
    assert_eq!(team, "Fixture Team");
    let template: String = client
        .query_one(
            "SELECT name FROM templates WHERE name = 'Fixture check'",
            &[],
        )
        .await
        .expect("fixture template")
        .get(0);
    assert_eq!(template, "Fixture check");

    let jobs: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM jobs WHERE name = 'Fixture check'",
            &[],
        )
        .await
        .expect("fixture jobs")
        .get(0);
    assert_eq!(jobs, 3);
    let triggers: i64 = client
        .query_one(
            "SELECT COUNT(DISTINCT trigger) FROM jobs WHERE name = 'Fixture check'",
            &[],
        )
        .await
        .expect("fixture job triggers")
        .get(0);
    assert_eq!(triggers, 3);

    let workflow: String = client
        .query_one(
            "SELECT name FROM workflows WHERE name = 'Fixture workflow'",
            &[],
        )
        .await
        .expect("fixture workflow")
        .get(0);
    assert_eq!(workflow, "Fixture workflow");
    let version: i64 = client
        .query_one(
            "SELECT version FROM workflows WHERE name = 'Fixture workflow'",
            &[],
        )
        .await
        .expect("workflow version")
        .get(0);
    assert_eq!(version, 3);

    let reachable: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM host_results WHERE host = 'web01.example.test'
             AND ok = 1 AND changed = 0 AND failed = 0
             AND result LIKE '%Fixture host is reachable%'",
            &[],
        )
        .await
        .expect("reachable host")
        .get(0);
    assert_eq!(reachable, 1);
    let failed: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM host_results WHERE host = 'web01.example.test'
             AND ok = 0 AND failed = 1
             AND result LIKE '%Fixture host failed%'",
            &[],
        )
        .await
        .expect("failed host")
        .get(0);
    assert_eq!(failed, 1);

    let activity: String = client
        .query_one(
            "SELECT result FROM activity WHERE id = 'fixture-activity-failed'",
            &[],
        )
        .await
        .expect("fixture activity")
        .get(0);
    assert_eq!(activity, "failed");
    let action: String = client
        .query_one(
            "SELECT action FROM activity WHERE id = 'fixture-activity-failed'",
            &[],
        )
        .await
        .expect("fixture activity action")
        .get(0);
    assert_eq!(action, "update");

    let write_scope: String = client
        .query_one(
            "SELECT scope FROM api_tokens WHERE name = 'fixture-write-token'",
            &[],
        )
        .await
        .expect("write token")
        .get(0);
    assert_eq!(write_scope, "write");
    let write_expiry: Option<String> = client
        .query_one(
            "SELECT expires_at FROM api_tokens WHERE name = 'fixture-write-token'",
            &[],
        )
        .await
        .expect("write token expiry")
        .get(0);
    assert!(write_expiry.is_none());
    let expired_scope: String = client
        .query_one(
            "SELECT scope FROM api_tokens WHERE name = 'fixture-expired-token'",
            &[],
        )
        .await
        .expect("expired token")
        .get(0);
    assert_eq!(expired_scope, "read");
    let expired_at: String = client
        .query_one(
            "SELECT expires_at FROM api_tokens WHERE name = 'fixture-expired-token'",
            &[],
        )
        .await
        .expect("expired token expiry")
        .get(0);
    assert_eq!(expired_at, "2026-01-01T09:00:00Z");

    let session: (String, Option<String>) = client
        .query_one(
            "SELECT id, created_at FROM sessions WHERE token = 'fixture-session-token-user'",
            &[],
        )
        .await
        .map(|row| (row.get(0), row.get(1)))
        .expect("fixture session");
    assert_eq!(session.0, "fixture-session-user");
    assert!(session.1.is_some());

    let delivery: String = client
        .query_one(
            "SELECT status FROM hook_deliveries WHERE id = 'fixture-delivery-ignored'",
            &[],
        )
        .await
        .expect("fixture delivery")
        .get(0);
    assert_eq!(delivery, "ignored");

    let vars: String = client
        .query_one(
            "SELECT vars_json FROM inventories WHERE name = 'Fixture inventory'",
            &[],
        )
        .await
        .expect("inventory vars")
        .get(0);
    assert!(vars.contains("group_vars/all.yml"));
    assert!(vars.contains("fixture_name: alpha15"));
    let source_vars: String = client
        .query_one(
            "SELECT vars_json FROM inventory_sources WHERE inventory_id =
             (SELECT id FROM inventories WHERE name = 'Fixture inventory')",
            &[],
        )
        .await
        .expect("inventory source vars")
        .get(0);
    assert!(source_vars.contains("group_vars/web.yml"));
    assert!(source_vars.contains("fixture_web: true"));

    let created_by: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM information_schema.columns
             WHERE table_name = 'users' AND column_name = 'created_by'",
            &[],
        )
        .await
        .expect("created_by column")
        .get(0);
    assert_eq!(created_by, 1);
    let service_accounts: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM users WHERE auth_source = 'service'",
            &[],
        )
        .await
        .expect("service accounts")
        .get(0);
    assert_eq!(service_accounts, 0);
    let with_creator: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM users WHERE created_by IS NOT NULL",
            &[],
        )
        .await
        .expect("existing creators")
        .get(0);
    assert_eq!(with_creator, 0);
}

async fn assert_alpha_12_postgres_rows(client: &tokio_postgres::Client) {
    for (table, minimum) in [
        ("users", 2),
        ("teams", 2),
        ("templates", 1),
        ("jobs", 2),
        ("workflows", 2),
    ] {
        let count: i64 = client
            .query_one(&format!("SELECT COUNT(*) FROM {table}"), &[])
            .await
            .expect("fixture count")
            .get(0);
        assert!(count >= minimum, "{table} is empty");
    }
    let user: String = client
        .query_one(
            "SELECT username FROM users WHERE username = 'fixture-user'",
            &[],
        )
        .await
        .expect("fixture user")
        .get(0);
    assert_eq!(user, "fixture-user");
    let team: String = client
        .query_one("SELECT name FROM teams WHERE name = 'Fixture Team'", &[])
        .await
        .expect("fixture team")
        .get(0);
    assert_eq!(team, "Fixture Team");
    let template: String = client
        .query_one(
            "SELECT name FROM templates WHERE name = 'Fixture check'",
            &[],
        )
        .await
        .expect("fixture template")
        .get(0);
    assert_eq!(template, "Fixture check");
    let jobs: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM jobs WHERE name = 'Fixture check'",
            &[],
        )
        .await
        .expect("fixture jobs")
        .get(0);
    assert_eq!(jobs, 2);
    let triggers: i64 = client
        .query_one(
            "SELECT COUNT(DISTINCT trigger) FROM jobs WHERE name = 'Fixture check'",
            &[],
        )
        .await
        .expect("fixture job triggers")
        .get(0);
    assert_eq!(triggers, 2);
    let workflow: String = client
        .query_one(
            "SELECT name FROM workflows WHERE name = 'Fixture workflow'",
            &[],
        )
        .await
        .expect("fixture workflow")
        .get(0);
    assert_eq!(workflow, "Fixture workflow");
    let version: i64 = client
        .query_one(
            "SELECT version FROM workflows WHERE name = 'Fixture workflow'",
            &[],
        )
        .await
        .expect("workflow version")
        .get(0);
    assert_eq!(version, 1);
}

fn copy_sqlite_fixture(dir: &Path, release: &str) {
    fs::copy(fixture(release, "koreograf.db"), dir.join("koreograf.db")).expect("sqlite fixture");
}

fn boot_sqlite_fixture(dir: &Path, release: &str) -> Shared {
    copy_sqlite_fixture(dir, release);
    koreograf::boot(config(dir, &options())).expect("sqlite fixture boot")
}

#[test]
fn the_alpha_10_sqlite_fixture_upgrades_and_keeps_its_rows() {
    let dir = temp_dir("upgrade-alpha10-sqlite");
    let state = boot_sqlite_fixture(&dir, ALPHA10);
    let latest = latest_sqlite_version();
    {
        let conn = state.db().expect("database");
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("schema version");
        assert_eq!(version, latest);
        assert_fixture_rows(&conn);
        let marker: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM schema_meta WHERE key = 'legacy_reconciled'",
                [],
                |row| row.get(0),
            )
            .expect("legacy marker");
        assert_eq!(marker, 1);
    }
    drop(state);

    let state = koreograf::boot(config(&dir, &options())).expect("second sqlite boot");
    {
        let conn = state.db().expect("database");
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("schema version");
        assert_eq!(version, latest);
        assert_fixture_rows(&conn);
    }
    drop(state);
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_alpha_10_postgres_fixture_upgrades_and_keeps_its_rows() {
    let Some(url) = postgres_test_url() else {
        return;
    };
    let _guard = postgres_guard().await;
    let name = postgres_database_name("upgrade-alpha10");
    let target = create_database(&url, &name).await;
    let latest = latest_sqlite_version();

    let client = postgres_client(&target).await;
    let dump = fs::read_to_string(fixture(ALPHA10, "postgres.sql")).expect("postgres fixture");
    client
        .batch_execute(&dump)
        .await
        .expect("load postgres fixture");
    drop(client);

    let data_dir = temp_dir("upgrade-alpha10-postgres");
    let state = koreograf::boot_with(
        config(&data_dir, &options()),
        koreograf::parse_database_config(&target, None, None).expect("database config"),
    )
    .expect("postgres fixture boot");

    let client = postgres_client(&target).await;
    let version: String = client
        .query_one(
            "SELECT value FROM schema_meta WHERE key = 'schema_version'",
            &[],
        )
        .await
        .expect("schema version")
        .get(0);
    assert_eq!(version, latest.to_string());
    for table in ["users", "teams", "templates", "jobs"] {
        let count: i64 = client
            .query_one(&format!("SELECT COUNT(*) FROM {table}"), &[])
            .await
            .expect("fixture count")
            .get(0);
        assert!(count >= 1, "{table} is empty");
    }
    let job: String = client
        .query_one("SELECT name FROM jobs WHERE id = 'fixture-job-1'", &[])
        .await
        .expect("fixture job")
        .get(0);
    assert_eq!(job, "Fixture check");

    drop(client);
    drop(state);
    drop_database(&url, &name).await;
    let _ = fs::remove_dir_all(&data_dir);
}

#[test]
fn the_alpha_12_sqlite_fixture_upgrades_and_keeps_its_rows() {
    let dir = temp_dir("upgrade-alpha12-sqlite");
    let state = boot_sqlite_fixture(&dir, ALPHA12);
    let latest = latest_sqlite_version();
    {
        let conn = state.db().expect("database");
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("schema version");
        assert_eq!(version, latest);
        assert_alpha_12_fixture_rows(&conn);
        let marker: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM schema_meta WHERE key = 'legacy_reconciled'",
                [],
                |row| row.get(0),
            )
            .expect("legacy marker");
        assert_eq!(marker, 1);
    }
    drop(state);

    let state = koreograf::boot(config(&dir, &options())).expect("second sqlite boot");
    {
        let conn = state.db().expect("database");
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("schema version");
        assert_eq!(version, latest);
        assert_alpha_12_fixture_rows(&conn);
    }
    drop(state);
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_alpha_12_postgres_fixture_upgrades_and_keeps_its_rows() {
    let Some(url) = postgres_test_url() else {
        return;
    };
    let _guard = postgres_guard().await;
    let name = postgres_database_name("upgrade-alpha12");
    let target = create_database(&url, &name).await;
    let latest = latest_sqlite_version();

    let client = postgres_client(&target).await;
    let dump = fs::read_to_string(fixture(ALPHA12, "postgres.sql")).expect("postgres fixture");
    client
        .batch_execute(&dump)
        .await
        .expect("load postgres fixture");
    drop(client);

    let data_dir = temp_dir("upgrade-alpha12-postgres");
    let state = koreograf::boot_with(
        config(&data_dir, &options()),
        koreograf::parse_database_config(&target, None, None).expect("database config"),
    )
    .expect("postgres fixture boot");

    let client = postgres_client(&target).await;
    let version: String = client
        .query_one(
            "SELECT value FROM schema_meta WHERE key = 'schema_version'",
            &[],
        )
        .await
        .expect("schema version")
        .get(0);
    assert_eq!(version, latest.to_string());
    assert_alpha_12_postgres_rows(&client).await;

    drop(client);
    drop(state);
    drop_database(&url, &name).await;
    let _ = fs::remove_dir_all(&data_dir);
}

#[test]
fn the_alpha_15_sqlite_fixture_upgrades_and_keeps_its_rows() {
    let dir = temp_dir("upgrade-alpha15-sqlite");
    copy_sqlite_fixture(&dir, ALPHA15);
    let raw = rusqlite::Connection::open_with_flags(
        dir.join("koreograf.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("alpha.15 fixture");
    let fixture_version: u32 = raw
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("fixture schema version");
    assert_eq!(fixture_version, ALPHA15_SCHEMA_VERSION);
    drop(raw);

    let state = koreograf::boot(config(&dir, &options())).expect("sqlite fixture boot");
    let latest = latest_sqlite_version();
    {
        let conn = state.db().expect("database");
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("schema version");
        assert_eq!(version, latest);
        assert_alpha_15_fixture_rows(&conn);
        let marker: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM schema_meta WHERE key = 'legacy_reconciled'",
                [],
                |row| row.get(0),
            )
            .expect("legacy marker");
        assert_eq!(marker, 1);
    }
    drop(state);

    let state = koreograf::boot(config(&dir, &options())).expect("second sqlite boot");
    {
        let conn = state.db().expect("database");
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("schema version");
        assert_eq!(version, latest);
        assert_alpha_15_fixture_rows(&conn);
    }
    drop(state);
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_alpha_15_postgres_fixture_upgrades_and_keeps_its_rows() {
    let Some(url) = postgres_test_url() else {
        return;
    };
    let _guard = postgres_guard().await;
    let name = postgres_database_name("upgrade-alpha15");
    let target = create_database(&url, &name).await;
    let latest = latest_sqlite_version();

    let client = postgres_client(&target).await;
    let dump = fs::read_to_string(fixture(ALPHA15, "postgres.sql")).expect("postgres fixture");
    client
        .batch_execute(&dump)
        .await
        .expect("load postgres fixture");
    let fixture_version: String = client
        .query_one(
            "SELECT value FROM public.schema_meta WHERE key = 'schema_version'",
            &[],
        )
        .await
        .expect("fixture schema version")
        .get(0);
    assert_eq!(fixture_version, ALPHA15_SCHEMA_VERSION.to_string());
    drop(client);

    let data_dir = temp_dir("upgrade-alpha15-postgres");
    let state = koreograf::boot_with(
        config(&data_dir, &options()),
        koreograf::parse_database_config(&target, None, None).expect("database config"),
    )
    .expect("postgres fixture boot");

    let client = postgres_client(&target).await;
    let version: String = client
        .query_one(
            "SELECT value FROM schema_meta WHERE key = 'schema_version'",
            &[],
        )
        .await
        .expect("schema version")
        .get(0);
    assert_eq!(version, latest.to_string());
    assert_alpha_15_postgres_rows(&client).await;

    drop(client);
    drop(state);
    drop_database(&url, &name).await;
    let _ = fs::remove_dir_all(&data_dir);
}
