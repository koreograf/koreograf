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
async fn the_startup_prune_runs_in_the_background_task_after_boot() {
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

    // Boot leaves the history alone; the first prune is a background task.
    assert_eq!(event_count(&harness, "retention-boot").await, 1);
    assert!(work_dir(&harness, "retention-boot").exists());

    harness.state.spawn_log_retention();
    wait_for_prune(&harness, "retention-boot").await;

    assert_eq!(event_count(&harness, "retention-boot").await, 0);
    assert!(!work_dir(&harness, "retention-boot").exists());
    assert_eq!(
        harness
            .sql_i64("SELECT COUNT(*) FROM jobs WHERE id = 'retention-boot'")
            .await,
        1
    );
}

async fn wait_for_prune(harness: &TestApp, id: &str) {
    for _ in 0..300 {
        if event_count(harness, id).await == 0 && !work_dir(harness, id).exists() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the background retention task did not prune {id}");
}

#[tokio::test(flavor = "multi_thread")]
async fn many_old_jobs_lose_only_the_output_and_dirs_they_still_have() {
    let harness = TestApp::boot("retention-bulk", options(5));
    let team = harness
        .sql_text("SELECT id FROM teams ORDER BY id LIMIT 1")
        .await
        .remove(0);
    let old = stamp(10);
    let recent = stamp(0);
    let mut jobs = String::new();
    let mut events = String::new();
    let mut with_events = Vec::new();
    let mut with_dirs = Vec::new();
    let mut kept = Vec::new();
    for index in 0..24 {
        let id = format!("retention-bulk-{index:02}");
        let running = index % 6 == 5;
        let status = if running { "running" } else { "successful" };
        let finished = if running {
            "NULL".to_string()
        } else {
            format!("'{old}'")
        };
        jobs.push_str(&format!(
            "INSERT INTO jobs (
                id, team_id, name, tool, status, extra_vars, launched_by, trigger,
                started_at, finished_at, created_at
             ) VALUES ('{id}', '{team}', 'Bulk {index}', 'shell', '{status}',
                '{{}}', 'admin', 'manual', '{old}', {finished}, '{old}');\n"
        ));
        let has_events = running || index % 2 == 0;
        if has_events {
            events.push_str(&format!(
                "INSERT INTO job_events (job_id, ts, stream, line)
                 VALUES ('{id}', '{old}', 'stdout', 'line one'),
                        ('{id}', '{old}', 'stdout', 'line two');\n"
            ));
        }
        let has_dir = running || index % 3 == 0;
        if has_dir {
            seed_work_dir(&harness, &id);
        }
        if running {
            kept.push(id);
        } else {
            if has_events {
                with_events.push(id.clone());
            }
            if has_dir {
                with_dirs.push(id);
            }
        }
    }
    // A recent finished job keeps both, even though it also has events.
    let recent_id = "retention-bulk-recent";
    jobs.push_str(&format!(
        "INSERT INTO jobs (
            id, team_id, name, tool, status, extra_vars, launched_by, trigger,
            started_at, finished_at, created_at
         ) VALUES ('{recent_id}', '{team}', 'Recent', 'shell', 'failed',
            '{{}}', 'admin', 'manual', '{recent}', '{recent}', '{recent}');\n"
    ));
    events.push_str(&format!(
        "INSERT INTO job_events (job_id, ts, stream, line)
         VALUES ('{recent_id}', '{recent}', 'stdout', 'keep me');\n"
    ));
    seed_work_dir(&harness, recent_id);
    harness.execute_sql(&format!("{jobs}{events}")).await;

    harness.state.prune_job_logs().expect("first prune");

    for id in &with_events {
        assert_eq!(event_count(&harness, id).await, 0, "{id} kept events");
    }
    for id in &with_dirs {
        assert!(!work_dir(&harness, id).exists(), "{id} kept its work dir");
    }
    for id in &kept {
        assert_eq!(event_count(&harness, id).await, 2, "{id} lost events");
        assert!(work_dir(&harness, id).exists(), "{id} lost its work dir");
    }
    assert_eq!(event_count(&harness, recent_id).await, 1);
    assert!(work_dir(&harness, recent_id).exists());
    assert_eq!(
        harness
            .sql_i64("SELECT COUNT(*) FROM jobs WHERE id LIKE 'retention-bulk-%'")
            .await,
        25
    );

    // The second pass has no events to delete. A new old directory still goes.
    let late = "retention-bulk-late";
    harness
        .execute_sql(&format!(
            "INSERT INTO jobs (
                id, team_id, name, tool, status, extra_vars, launched_by, trigger,
                started_at, finished_at, created_at
             ) VALUES ('{late}', '{team}', 'Late', 'shell', 'failed',
                '{{}}', 'admin', 'manual', '{old}', '{old}', '{old}');"
        ))
        .await;
    seed_work_dir(&harness, late);
    harness.state.prune_job_logs().expect("second prune");

    for id in &with_events {
        assert_eq!(event_count(&harness, id).await, 0, "{id} came back");
    }
    assert!(!work_dir(&harness, late).exists());
    assert_eq!(
        harness
            .sql_i64(&format!(
                "SELECT COUNT(*) FROM job_events WHERE job_id = '{recent_id}'"
            ))
            .await,
        1
    );
}

fn workflow_options(workflow_run_retention_days: u64) -> Options {
    Options {
        workflow_run_retention_days,
        ..Options::default()
    }
}

fn seed_workflow_runs(team: &str, old: &str, recent: &str) -> String {
    format!(
        "INSERT INTO workflow_jobs (
            id, workflow_id, team_id, name, status, extra_vars, launched_by, trigger,
            created_at, finished_at
         ) VALUES
            ('wf-retention-successful', NULL, '{team}', 'Old successful', 'successful',
             '{{}}', 'admin', 'manual', '{old}', '{old}'),
            ('wf-retention-failed', NULL, '{team}', 'Old failed', 'failed',
             '{{}}', 'admin', 'manual', '{old}', '{old}'),
            ('wf-retention-canceled', NULL, '{team}', 'Old canceled', 'canceled',
             '{{}}', 'admin', 'manual', '{old}', '{old}'),
            ('wf-retention-recent', NULL, '{team}', 'Recent', 'successful',
             '{{}}', 'admin', 'manual', '{recent}', '{recent}'),
            ('wf-retention-running', NULL, '{team}', 'Running', 'running',
             '{{}}', 'admin', 'manual', '{old}', NULL),
            ('wf-retention-no-finish', NULL, '{team}', 'No finish', 'successful',
             '{{}}', 'admin', 'manual', '{old}', NULL);
         INSERT INTO workflow_nodes (workflow_job_id, node_id, phase, job_id) VALUES
            ('wf-retention-successful', 'one', 'successful', NULL),
            ('wf-retention-successful', 'two', 'skipped', NULL),
            ('wf-retention-failed', 'one', 'failed', NULL),
            ('wf-retention-canceled', 'one', 'canceled', NULL),
            ('wf-retention-recent', 'one', 'successful', NULL),
            ('wf-retention-recent', 'two', 'successful', NULL),
            ('wf-retention-running', 'one', 'running', NULL),
            ('wf-retention-no-finish', 'one', 'successful', NULL);"
    )
}

async fn workflow_run_count(harness: &TestApp, id: &str) -> i64 {
    harness
        .sql_i64(&format!(
            "SELECT COUNT(*) FROM workflow_jobs WHERE id = '{id}'"
        ))
        .await
}

async fn workflow_node_count(harness: &TestApp, id: &str) -> i64 {
    harness
        .sql_i64(&format!(
            "SELECT COUNT(*) FROM workflow_nodes WHERE workflow_job_id = '{id}'"
        ))
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_run_retention_removes_only_old_finished_runs_and_their_nodes() {
    let harness = TestApp::boot("workflow-retention", workflow_options(5));
    let (cookie, team) = harness.sign_in().await;
    let old = stamp(10);
    let recent = stamp(0);
    harness
        .execute_sql(&seed_workflow_runs(&team, &old, &recent))
        .await;

    harness.state.prune_workflow_runs().expect("prune");

    for id in [
        "wf-retention-successful",
        "wf-retention-failed",
        "wf-retention-canceled",
    ] {
        assert_eq!(workflow_run_count(&harness, id).await, 0, "{id} stayed");
        assert_eq!(
            workflow_node_count(&harness, id).await,
            0,
            "{id} kept nodes"
        );
    }
    for id in [
        "wf-retention-recent",
        "wf-retention-running",
        "wf-retention-no-finish",
    ] {
        assert_eq!(workflow_run_count(&harness, id).await, 1, "{id} went");
    }
    assert_eq!(
        workflow_node_count(&harness, "wf-retention-recent").await,
        2
    );
    assert_eq!(
        workflow_node_count(&harness, "wf-retention-running").await,
        1
    );
    assert_eq!(
        workflow_node_count(&harness, "wf-retention-no-finish").await,
        1
    );

    let (status, _) = harness
        .call(
            "GET",
            "/api/workflow-jobs/wf-retention-successful",
            Some(&cookie),
            Some(&team),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let runs = harness
        .sql_text(
            "SELECT id || '=' || COALESCE(finished_at, '')
             FROM workflow_jobs WHERE id LIKE 'wf-retention-%' ORDER BY id",
        )
        .await;
    let nodes = harness
        .sql_text(
            "SELECT workflow_job_id || '=' || node_id || '=' || phase
             FROM workflow_nodes
             WHERE workflow_job_id LIKE 'wf-retention-%'
             ORDER BY workflow_job_id, node_id",
        )
        .await;
    harness.state.prune_workflow_runs().expect("second prune");
    assert_eq!(
        harness
            .sql_text(
                "SELECT id || '=' || COALESCE(finished_at, '')
                 FROM workflow_jobs WHERE id LIKE 'wf-retention-%' ORDER BY id",
            )
            .await,
        runs
    );
    assert_eq!(
        harness
            .sql_text(
                "SELECT workflow_job_id || '=' || node_id || '=' || phase
                 FROM workflow_nodes
                 WHERE workflow_job_id LIKE 'wf-retention-%'
                 ORDER BY workflow_job_id, node_id",
            )
            .await,
        nodes
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn zero_disables_workflow_run_retention() {
    let harness = TestApp::boot("workflow-retention-off", workflow_options(0));
    let team = harness
        .sql_text("SELECT id FROM teams ORDER BY id LIMIT 1")
        .await
        .remove(0);
    let old = stamp(10);
    harness
        .execute_sql(&format!(
            "INSERT INTO workflow_jobs (
                id, workflow_id, team_id, name, status, extra_vars, launched_by, trigger,
                created_at, finished_at
             ) VALUES ('wf-retention-kept', NULL, '{team}', 'Kept', 'successful',
                '{{}}', 'admin', 'manual', '{old}', '{old}');
             INSERT INTO workflow_nodes (workflow_job_id, node_id, phase, job_id)
             VALUES ('wf-retention-kept', 'one', 'successful', NULL);"
        ))
        .await;

    harness.state.prune_workflow_runs().expect("prune");

    assert_eq!(workflow_run_count(&harness, "wf-retention-kept").await, 1);
    assert_eq!(workflow_node_count(&harness, "wf-retention-kept").await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_workflow_removes_its_runs_and_nodes_only() {
    let harness = TestApp::boot("workflow-delete", Options::default());
    let (cookie, team) = harness.sign_in().await;
    let old = stamp(10);
    harness
        .execute_sql(&format!(
            "INSERT INTO workflows (
                id, team_id, name, description, graph, created_at, admin_saved, version
             ) VALUES
                ('wf-delete-one', '{team}', 'Delete one', '', '{{}}', '{old}', 1, 1),
                ('wf-delete-two', '{team}', 'Delete two', '', '{{}}', '{old}', 1, 1);
             INSERT INTO workflow_jobs (
                id, workflow_id, team_id, name, status, extra_vars, launched_by, trigger,
                created_at, finished_at
             ) VALUES
                ('wf-run-one', 'wf-delete-one', '{team}', 'One', 'successful',
                 '{{}}', 'admin', 'manual', '{old}', '{old}'),
                ('wf-run-two', 'wf-delete-two', '{team}', 'Two', 'successful',
                 '{{}}', 'admin', 'manual', '{old}', '{old}'),
                ('wf-run-manual', NULL, '{team}', 'Manual', 'successful',
                 '{{}}', 'admin', 'manual', '{old}', '{old}');
             INSERT INTO workflow_nodes (workflow_job_id, node_id, phase, job_id) VALUES
                ('wf-run-one', 'one', 'successful', NULL),
                ('wf-run-two', 'one', 'successful', NULL),
                ('wf-run-manual', 'one', 'successful', NULL);"
        ))
        .await;

    let (status, body) = harness
        .call(
            "DELETE",
            "/api/workflows/wf-delete-one",
            Some(&cookie),
            Some(&team),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    assert_eq!(
        harness
            .sql_i64("SELECT COUNT(*) FROM workflows WHERE id = 'wf-delete-one'")
            .await,
        0
    );
    assert_eq!(workflow_run_count(&harness, "wf-run-one").await, 0);
    assert_eq!(workflow_node_count(&harness, "wf-run-one").await, 0);
    assert_eq!(
        harness
            .sql_i64("SELECT COUNT(*) FROM workflows WHERE id = 'wf-delete-two'")
            .await,
        1
    );
    assert_eq!(workflow_run_count(&harness, "wf-run-two").await, 1);
    assert_eq!(workflow_node_count(&harness, "wf-run-two").await, 1);
    assert_eq!(workflow_run_count(&harness, "wf-run-manual").await, 1);
    assert_eq!(workflow_node_count(&harness, "wf-run-manual").await, 1);

    let (status, _) = harness
        .call(
            "GET",
            "/api/workflows/wf-delete-one",
            Some(&cookie),
            Some(&team),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = harness
        .call(
            "GET",
            "/api/workflow-jobs/wf-run-one",
            Some(&cookie),
            Some(&team),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_startup_workflow_prune_runs_in_the_background_task_after_boot() {
    let mut harness = TestApp::boot("workflow-retention-startup", workflow_options(5));
    let team = harness
        .sql_text("SELECT id FROM teams ORDER BY id LIMIT 1")
        .await
        .remove(0);
    let old = stamp(10);
    harness
        .execute_sql(&format!(
            "INSERT INTO workflow_jobs (
                id, workflow_id, team_id, name, status, extra_vars, launched_by, trigger,
                created_at, finished_at
             ) VALUES ('wf-retention-boot', NULL, '{team}', 'Boot', 'successful',
                '{{}}', 'admin', 'manual', '{old}', '{old}');
             INSERT INTO workflow_nodes (workflow_job_id, node_id, phase, job_id)
             VALUES ('wf-retention-boot', 'one', 'successful', NULL);"
        ))
        .await;

    harness.reboot();

    // Boot leaves the history alone; the first prune is a background task.
    assert_eq!(workflow_run_count(&harness, "wf-retention-boot").await, 1);
    assert_eq!(workflow_node_count(&harness, "wf-retention-boot").await, 1);

    harness.state.spawn_workflow_run_retention();
    for _ in 0..300 {
        if workflow_run_count(&harness, "wf-retention-boot").await == 0
            && workflow_node_count(&harness, "wf-retention-boot").await == 0
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the background workflow retention task did not prune wf-retention-boot");
}
