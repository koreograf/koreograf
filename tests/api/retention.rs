use axum::http::StatusCode;
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::Value;

use crate::common::*;

fn stamp(days: i64) -> String {
    (Utc::now() - ChronoDuration::days(days))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

fn options(job_log_retention_days: u64) -> Options {
    Options {
        job_log_retention_days,
        ..Options::default()
    }
}

fn seed_rows(team: &str, old: &str, middle: &str, recent: &str) -> String {
    format!(
        "INSERT INTO jobs (
            id, team_id, name, tool, status, extra_vars, launched_by, trigger,
            started_at, finished_at, created_at, failed_hosts
         ) VALUES
            ('retention-old', '{team}', 'Old', 'shell', 'successful', '{{}}', 'admin',
             'manual', '{old}', '{old}', '{old}', '[\"web1\"]'),
            ('retention-middle', '{team}', 'Middle', 'ansible', 'failed', '{{}}', 'admin',
             'schedule', '{middle}', '{middle}', '{middle}', '[]'),
            ('retention-recent', '{team}', 'Recent', 'shell', 'canceled', '{{}}', 'admin',
             'manual', '{recent}', '{recent}', '{recent}', '[]'),
            ('retention-running', '{team}', 'Running', 'shell', 'running', '{{}}', 'admin',
             'manual', '{recent}', NULL, '{recent}', '[]'),
            ('retention-empty', '{team}', 'Empty', 'shell', 'successful', '{{}}', 'admin',
             'manual', '{recent}', '{recent}', '{recent}', '[]');
         INSERT INTO job_events (job_id, ts, stream, line) VALUES
            ('retention-old', '{old}', 'stdout', 'old one'),
            ('retention-old', '{old}', 'stderr', 'old two'),
            ('retention-middle', '{middle}', 'stdout', 'middle one'),
            ('retention-middle', '{middle}', 'stdout', 'middle two'),
            ('retention-recent', '{recent}', 'stdout', 'recent one'),
            ('retention-running', '{recent}', 'stdout', 'running one');"
    )
}

fn work_dir(harness: &TestApp, id: &str) -> std::path::PathBuf {
    harness.dir.join("jobs").join(id)
}

fn seed_work_dir(harness: &TestApp, id: &str) {
    let path = work_dir(harness, id);
    std::fs::create_dir_all(&path).expect("work dir");
    std::fs::write(path.join("extra.json"), "{}").expect("work file");
}

async fn event_count(harness: &TestApp, id: &str) -> i64 {
    harness
        .sql_i64(&format!(
            "SELECT COUNT(*) FROM job_events WHERE job_id = '{id}'"
        ))
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn retention_removes_old_finished_events_and_work_dirs_only() {
    let harness = TestApp::boot("retention", options(5));
    let (cookie, team) = harness.sign_in().await;
    let old = stamp(10);
    let middle = stamp(2);
    let recent = stamp(0);
    harness
        .execute_sql(&seed_rows(&team, &old, &middle, &recent))
        .await;
    let inventory = harness
        .sql_text(&format!(
            "SELECT id FROM inventories WHERE team_id = '{team}' ORDER BY id LIMIT 1"
        ))
        .await
        .remove(0);
    harness
        .execute_sql(&format!(
            "INSERT INTO host_results (
                job_id, team_id, inventory_id, host, playbook, role, result,
                ok, changed, failed, unreachable, skipped, ts
             ) VALUES ('retention-old', '{team}', '{inventory}', 'localhost',
                'site.yml', 'web', 'ok', 3, 1, 0, 0, 0, '{old}');"
        ))
        .await;
    for id in [
        "retention-old",
        "retention-middle",
        "retention-recent",
        "retention-running",
        "retention-empty",
    ] {
        seed_work_dir(&harness, id);
    }

    harness.state.prune_job_logs().expect("prune");

    assert_eq!(event_count(&harness, "retention-old").await, 0);
    assert_eq!(event_count(&harness, "retention-middle").await, 2);
    assert_eq!(event_count(&harness, "retention-recent").await, 1);
    assert_eq!(event_count(&harness, "retention-running").await, 1);
    assert_eq!(event_count(&harness, "retention-empty").await, 0);
    assert!(!work_dir(&harness, "retention-old").exists());
    for id in [
        "retention-middle",
        "retention-recent",
        "retention-running",
        "retention-empty",
    ] {
        assert!(work_dir(&harness, id).exists(), "{id} lost its work dir");
    }

    assert_eq!(
        harness
            .sql_i64("SELECT COUNT(*) FROM jobs WHERE id LIKE 'retention-%'")
            .await,
        5
    );
    assert_eq!(
        harness
            .sql_i64(
                "SELECT COUNT(*) FROM jobs
                 WHERE id = 'retention-old' AND status = 'successful'
                   AND finished_at IS NOT NULL AND failed_hosts = '[\"web1\"]'"
            )
            .await,
        1
    );

    let (status, list) = harness
        .call("GET", "/api/jobs?q=Old", Some(&cookie), Some(&team), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["total"], 1);
    assert_eq!(list["items"][0]["status"], "successful");
    assert_eq!(
        list["items"][0]["failed_hosts"],
        serde_json::json!(["web1"])
    );

    let (status, detail) = harness
        .call(
            "GET",
            "/api/jobs/retention-old",
            Some(&cookie),
            Some(&team),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["name"], "Old");
    assert_eq!(detail["trigger"], "manual");
    assert_eq!(detail["launched_by"], "admin");
    assert_eq!(detail["finished_at"], old);
    assert_eq!(detail["events"], Value::Array(Vec::new()));

    let (status, output) = harness
        .call(
            "GET",
            "/api/jobs/retention-old/output",
            Some(&cookie),
            Some(&team),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{output}");
    assert_eq!(output, Value::Null);

    let (status, metrics) = harness.call("GET", "/metrics", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(metrics
        .as_str()
        .unwrap_or("")
        .contains("# TYPE koreograf_jobs_started_total counter"));

    let (status, history) = harness
        .call(
            "GET",
            &format!("/api/inventories/{inventory}/hosts/localhost/history"),
            Some(&cookie),
            Some(&team),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert_eq!(history["total"], 1, "{history}");
    assert_eq!(history["entries"][0]["job_id"], "retention-old");
    assert_eq!(history["entries"][0]["job_status"], "successful");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_prune_changes_nothing() {
    let harness = TestApp::boot("retention-twice", options(5));
    let team = harness
        .sql_text("SELECT id FROM teams ORDER BY id LIMIT 1")
        .await
        .remove(0);
    let old = stamp(10);
    harness
        .execute_sql(&format!(
            "INSERT INTO jobs (
                id, team_id, name, tool, status, extra_vars, launched_by, trigger,
                started_at, finished_at, created_at
             ) VALUES ('retention-again', '{team}', 'Again', 'shell', 'successful',
                '{{}}', 'admin', 'manual', '{old}', '{old}', '{old}');
             INSERT INTO jobs (
                id, team_id, name, tool, status, extra_vars, launched_by, trigger,
                started_at, finished_at, created_at
             ) VALUES ('retention-nodir', '{team}', 'No dir', 'shell', 'failed',
                '{{}}', 'admin', 'manual', '{old}', '{old}', '{old}');
             INSERT INTO job_events (job_id, ts, stream, line)
             VALUES ('retention-again', '{old}', 'stdout', 'one'),
                    ('retention-nodir', '{old}', 'stdout', 'no dir');"
        ))
        .await;
    seed_work_dir(&harness, "retention-again");

    harness.state.prune_job_logs().expect("first prune");
    let first_events = event_count(&harness, "retention-again").await;
    let first_dir = work_dir(&harness, "retention-again").exists();
    assert_eq!(first_events, 0);
    assert!(!first_dir);
    assert_eq!(event_count(&harness, "retention-nodir").await, 0);

    harness.state.prune_job_logs().expect("second prune");
    assert_eq!(event_count(&harness, "retention-again").await, first_events);
    assert_eq!(event_count(&harness, "retention-nodir").await, 0);
    assert_eq!(work_dir(&harness, "retention-again").exists(), first_dir);
    assert_eq!(
        harness
            .sql_i64("SELECT COUNT(*) FROM jobs WHERE id = 'retention-again'")
            .await,
        1
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn the_prune_removes_a_symlinked_work_dir_without_following_it() {
    let harness = TestApp::boot("retention-symlink", options(5));
    let team = harness
        .sql_text("SELECT id FROM teams ORDER BY id LIMIT 1")
        .await
        .remove(0);
    let old = stamp(10);
    harness
        .execute_sql(&format!(
            "INSERT INTO jobs (
                id, team_id, name, tool, status, extra_vars, launched_by, trigger,
                started_at, finished_at, created_at
             ) VALUES ('retention-link', '{team}', 'Link', 'shell', 'successful',
                '{{}}', 'admin', 'manual', '{old}', '{old}', '{old}');"
        ))
        .await;
    let target = harness.dir.join("outside-job-dir");
    std::fs::create_dir_all(&target).expect("target");
    let marker = target.join("keep.txt");
    std::fs::write(&marker, "keep").expect("marker");
    let link = work_dir(&harness, "retention-link");
    std::fs::create_dir_all(link.parent().expect("jobs dir")).expect("jobs dir");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");

    harness.state.prune_job_logs().expect("prune");

    assert!(!link.exists());
    assert!(target.exists(), "the prune followed the symlink");
    assert_eq!(std::fs::read_to_string(&marker).expect("marker"), "keep");
}

#[tokio::test(flavor = "multi_thread")]
async fn zero_disables_retention() {
    let harness = TestApp::boot("retention-off", options(0));
    let team = harness
        .sql_text("SELECT id FROM teams ORDER BY id LIMIT 1")
        .await
        .remove(0);
    let old = stamp(10);
    harness
        .execute_sql(&format!(
            "INSERT INTO jobs (
                id, team_id, name, tool, status, extra_vars, launched_by, trigger,
                started_at, finished_at, created_at
             ) VALUES ('retention-kept', '{team}', 'Kept', 'shell', 'successful',
                '{{}}', 'admin', 'manual', '{old}', '{old}', '{old}');
             INSERT INTO job_events (job_id, ts, stream, line)
             VALUES ('retention-kept', '{old}', 'stdout', 'kept');"
        ))
        .await;
    seed_work_dir(&harness, "retention-kept");

    harness.state.prune_job_logs().expect("prune");

    assert_eq!(event_count(&harness, "retention-kept").await, 1);
    assert!(work_dir(&harness, "retention-kept").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn startup_prunes_finished_jobs_past_the_cutoff() {
    let mut harness = TestApp::boot("retention-startup", options(5));
    let team = harness
        .sql_text("SELECT id FROM teams ORDER BY id LIMIT 1")
        .await
        .remove(0);
    let old = stamp(10);
    harness
        .execute_sql(&format!(
            "INSERT INTO jobs (
                id, team_id, name, tool, status, extra_vars, launched_by, trigger,
                started_at, finished_at, created_at
             ) VALUES ('retention-boot', '{team}', 'Boot', 'shell', 'successful',
                '{{}}', 'admin', 'manual', '{old}', '{old}', '{old}');
             INSERT INTO job_events (job_id, ts, stream, line)
             VALUES ('retention-boot', '{old}', 'stdout', 'boot');"
        ))
        .await;
    seed_work_dir(&harness, "retention-boot");

    harness.reboot();

    assert_eq!(event_count(&harness, "retention-boot").await, 0);
    assert!(!work_dir(&harness, "retention-boot").exists());
    assert_eq!(
        harness
            .sql_i64("SELECT COUNT(*) FROM jobs WHERE id = 'retention-boot'")
            .await,
        1
    );
}
