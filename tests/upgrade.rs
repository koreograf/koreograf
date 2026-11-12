#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{
    config, create_database, drop_database, postgres_client, postgres_database_name,
    postgres_guard, postgres_test_url, temp_dir, Options,
};
use koreograf::Shared;

const FIXTURE_RELEASE: &str = "alpha10";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(FIXTURE_RELEASE)
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

fn boot_sqlite_fixture(dir: &Path) -> Shared {
    fs::copy(fixture("koreograf.db"), dir.join("koreograf.db")).expect("sqlite fixture");
    koreograf::boot(config(dir, &options())).expect("sqlite fixture boot")
}

#[test]
fn the_alpha_10_sqlite_fixture_upgrades_and_keeps_its_rows() {
    let dir = temp_dir("upgrade-alpha10-sqlite");
    let state = boot_sqlite_fixture(&dir);
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
    let dump = fs::read_to_string(fixture("postgres.sql")).expect("postgres fixture");
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
