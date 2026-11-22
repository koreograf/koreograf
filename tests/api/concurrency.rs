use std::path::Path;
use std::time::Duration;

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::common::*;

const POLLS: usize = 120;
const POLL_STEP: Duration = Duration::from_millis(250);

fn options(max_concurrent_jobs: usize, max_concurrent_jobs_per_team: usize) -> Options {
    Options {
        max_concurrent_jobs,
        max_concurrent_jobs_per_team,
        ..Options::default()
    }
}

/// A shell template that touches `marker` and then waits for `release`.
/// Blocked jobs keep the slot until the test writes that file.
fn blocked_script(release: &Path, marker: &Path) -> String {
    format!(
        "touch '{}'\nwhile [ ! -f '{}' ]; do sleep 1; done\nprintf 'released\\n'\n",
        marker.display(),
        release.display()
    )
}

async fn shell_template(
    app: &TestApp,
    cookie: &str,
    team: &str,
    name: &str,
    script: &str,
) -> String {
    let (status, template) = app
        .call(
            "POST",
            "/api/templates",
            Some(cookie),
            Some(team),
            Some(json!({
                "name": name,
                "tool": "shell",
                "entrypoint": "",
                "spec": {"script": script}
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    template["id"].as_str().expect("template id").to_string()
}

async fn launch(app: &TestApp, cookie: &str, team: &str, template_id: &str) -> String {
    let (status, job) = app
        .call(
            "POST",
            &format!("/api/templates/{template_id}/launch"),
            Some(cookie),
            Some(team),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    job["id"].as_str().expect("job id").to_string()
}

async fn create_team(app: &TestApp, cookie: &str, from_team: &str, name: &str) -> String {
    let (status, team) = app
        .call(
            "POST",
            "/api/teams",
            Some(cookie),
            Some(from_team),
            Some(json!({"name": name})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{team}");
    team["id"].as_str().expect("team id").to_string()
}

async fn job_status(app: &TestApp, cookie: &str, team: &str, job_id: &str) -> String {
    let (status, body) = app
        .call(
            "GET",
            &format!("/api/jobs/{job_id}"),
            Some(cookie),
            Some(team),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["status"].as_str().expect("job status").to_string()
}

async fn statuses(app: &TestApp, cookie: &str, team: &str, ids: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for id in ids {
        out.push(job_status(app, cookie, team, id).await);
    }
    out
}

async fn cancel(app: &TestApp, cookie: &str, team: &str, job_id: &str) {
    let (status, body) = app
        .call(
            "POST",
            &format!("/api/jobs/{job_id}/cancel"),
            Some(cookie),
            Some(team),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// Waits until `wanted` of the launched jobs are `running`, and fails as soon
/// as more than that run at the same time.
async fn wait_for_running(
    app: &TestApp,
    cookie: &str,
    team: &str,
    ids: &[String],
    wanted: usize,
    limit: usize,
) {
    for _ in 0..POLLS {
        let current = statuses(app, cookie, team, ids).await;
        let running = current.iter().filter(|status| *status == "running").count();
        assert!(
            running <= limit,
            "{running} jobs ran at once with a limit of {limit}: {current:?}"
        );
        if running == wanted {
            return;
        }
        tokio::time::sleep(POLL_STEP).await;
    }
    panic!(
        "only {:?} after waiting for {wanted} of {} jobs",
        statuses(app, cookie, team, ids).await,
        ids.len()
    );
}

async fn wait_for_all(app: &TestApp, cookie: &str, team: &str, ids: &[String], expected: &str) {
    for _ in 0..POLLS {
        let current = statuses(app, cookie, team, ids).await;
        if current.iter().all(|status| status == expected) {
            return;
        }
        assert!(
            current
                .iter()
                .all(|status| !matches!(status.as_str(), "failed" | "canceled")),
            "a job ended unexpectedly while waiting for {expected}: {current:?}"
        );
        tokio::time::sleep(POLL_STEP).await;
    }
    panic!(
        "jobs did not reach {expected}: {:?}",
        statuses(app, cookie, team, ids).await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_limit_keeps_the_third_launch_pending_until_a_slot_frees() {
    let harness = TestApp::boot("concurrency-queue", options(2, 2));
    let (cookie, team) = harness.sign_in().await;
    let release = harness.dir.join("release");

    let mut ids = Vec::new();
    for index in 1..=3 {
        let marker = harness.dir.join(format!("started-{index}"));
        let script = blocked_script(&release, &marker);
        let template = shell_template(
            &harness,
            &cookie,
            &team,
            &format!("Blocked {index}"),
            &script,
        )
        .await;
        ids.push(launch(&harness, &cookie, &team, &template).await);
    }

    wait_for_running(&harness, &cookie, &team, &ids, 2, 2).await;
    let mut current = statuses(&harness, &cookie, &team, &ids).await;
    current.sort();
    assert_eq!(
        current,
        vec![
            "pending".to_string(),
            "running".to_string(),
            "running".to_string()
        ],
        "the third job must wait for a slot"
    );
    let (status, metrics, _) = call_raw_peer(
        &harness.app,
        "GET",
        "/metrics",
        loopback_peer(),
        None,
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let metrics = String::from_utf8_lossy(&metrics);
    assert!(
        metrics.contains("koreograf_jobs_pending 1"),
        "the waiting job must show in the pending gauge: {metrics}"
    );

    std::fs::write(&release, "go").expect("release");
    wait_for_all(&harness, &cookie, &team, &ids, "successful").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_canceled_while_it_waits_never_starts() {
    let harness = TestApp::boot("concurrency-cancel", options(1, 1));
    let (cookie, team) = harness.sign_in().await;
    let release = harness.dir.join("release");

    let running_marker = harness.dir.join("started-running");
    let running_template = shell_template(
        &harness,
        &cookie,
        &team,
        "Running",
        &blocked_script(&release, &running_marker),
    )
    .await;
    let waiting_marker = harness.dir.join("started-waiting");
    let waiting_template = shell_template(
        &harness,
        &cookie,
        &team,
        "Waiting",
        &blocked_script(&release, &waiting_marker),
    )
    .await;

    let running = launch(&harness, &cookie, &team, &running_template).await;
    let waiting = launch(&harness, &cookie, &team, &waiting_template).await;
    wait_for_running(
        &harness,
        &cookie,
        &team,
        std::slice::from_ref(&running),
        1,
        1,
    )
    .await;
    assert_eq!(
        job_status(&harness, &cookie, &team, &waiting).await,
        "pending"
    );

    cancel(&harness, &cookie, &team, &waiting).await;
    assert_eq!(
        job_status(&harness, &cookie, &team, &waiting).await,
        "canceled"
    );
    assert!(
        !harness.dir.join("jobs").join(&waiting).exists(),
        "a canceled job must not create a work directory"
    );

    // The permit this job waited for is offered once the running job ends.
    std::fs::write(&release, "go").expect("release");
    wait_for_all(
        &harness,
        &cookie,
        &team,
        std::slice::from_ref(&running),
        "successful",
    )
    .await;
    // Give the waiting task time to see the canceled row and give the slot up.
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert_eq!(
        job_status(&harness, &cookie, &team, &waiting).await,
        "canceled"
    );
    assert!(
        !waiting_marker.exists(),
        "a canceled job must not run after its slot frees"
    );
    assert!(
        !harness.dir.join("jobs").join(&waiting).exists(),
        "a canceled job must not create a work directory"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_limit_of_one_runs_one_job_at_a_time() {
    let harness = TestApp::boot("concurrency-serial", options(1, 1));
    let (cookie, team) = harness.sign_in().await;
    let release = harness.dir.join("release");

    let mut ids = Vec::new();
    for index in 1..=2 {
        let marker = harness.dir.join(format!("serial-{index}"));
        let script = blocked_script(&release, &marker);
        let template = shell_template(
            &harness,
            &cookie,
            &team,
            &format!("Serial {index}"),
            &script,
        )
        .await;
        ids.push(launch(&harness, &cookie, &team, &template).await);
    }

    wait_for_running(&harness, &cookie, &team, &ids, 1, 1).await;
    let mut current = statuses(&harness, &cookie, &team, &ids).await;
    current.sort();
    assert_eq!(
        current,
        vec!["pending".to_string(), "running".to_string()],
        "a limit of one keeps the second job pending"
    );

    std::fs::write(&release, "go").expect("release");
    wait_for_all(&harness, &cookie, &team, &ids, "successful").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_zero_limit_uses_the_documented_default_of_four() {
    // The default of four has room for all three jobs of this one team, so the
    // per-team limit is set to the same value.
    let harness = TestApp::boot("concurrency-zero", options(0, 4));
    let (cookie, team) = harness.sign_in().await;
    let release = harness.dir.join("release");

    let mut ids = Vec::new();
    for index in 1..=3 {
        let marker = harness.dir.join(format!("zero-{index}"));
        let script = blocked_script(&release, &marker);
        let template = shell_template(
            &harness,
            &cookie,
            &team,
            &format!("Default {index}"),
            &script,
        )
        .await;
        ids.push(launch(&harness, &cookie, &team, &template).await);
    }

    // The default of four has room for all three. A limit of zero would queue
    // them and a limit of one or two would keep the last one pending.
    wait_for_running(&harness, &cookie, &team, &ids, 3, 4).await;
    std::fs::write(&release, "go").expect("release");
    wait_for_all(&harness, &cookie, &team, &ids, "successful").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_single_launch_still_runs_to_success_with_the_default_limit() {
    let harness = TestApp::boot("concurrency-single", Options::default());
    let (cookie, team) = harness.sign_in().await;
    let marker = harness.dir.join("single-started");
    let script = format!("touch '{}'\nprintf 'single job\\n'\n", marker.display());
    let template = shell_template(&harness, &cookie, &team, "Single", &script).await;

    let job = launch(&harness, &cookie, &team, &template).await;
    wait_for_all(
        &harness,
        &cookie,
        &team,
        std::slice::from_ref(&job),
        "successful",
    )
    .await;
    assert!(marker.exists(), "the single job must run");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workflow_wider_than_the_limit_finishes_without_a_deadlock() {
    // Three parallel nodes in one team, and only one place for that team. The
    // run waits for the queue instead of holding a slot while it waits.
    let harness = TestApp::boot("concurrency-workflow", options(3, 1));
    let (cookie, team) = harness.sign_in().await;

    let template = shell_template(
        &harness,
        &cookie,
        &team,
        "Workflow node",
        "sleep 1\nprintf 'node finished\\n'\n",
    )
    .await;

    let (status, workflow) = harness
        .call(
            "POST",
            "/api/workflows",
            Some(&cookie),
            Some(&team),
            Some(json!({
                "name": "Wide fan-out",
                "description": "",
                "graph": {
                    "nodes": [
                        {"id": "a", "template_id": template, "name": "a"},
                        {"id": "b", "template_id": template, "name": "b"},
                        {"id": "c", "template_id": template, "name": "c"}
                    ],
                    "edges": []
                }
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");
    let workflow_id = workflow["id"].as_str().expect("workflow id");

    let (status, run) = harness
        .call(
            "POST",
            &format!("/api/workflows/{workflow_id}/launch"),
            Some(&cookie),
            Some(&team),
            Some(json!({"extra_vars": {}})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{run}");
    let run_id = run["id"].as_str().expect("run id");

    let mut finished = Value::Null;
    for _ in 0..POLLS {
        tokio::time::sleep(POLL_STEP).await;
        let (status, body) = harness
            .call(
                "GET",
                &format!("/api/workflow-jobs/{run_id}"),
                Some(&cookie),
                Some(&team),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if matches!(
            body["status"].as_str(),
            Some("successful" | "failed" | "canceled")
        ) {
            finished = body;
            break;
        }
    }
    assert_eq!(
        finished["status"], "successful",
        "a workflow wider than the limit must not deadlock: {finished}"
    );
    let nodes = finished["nodes"].as_array().expect("nodes");
    assert_eq!(nodes.len(), 3, "{finished}");
    assert!(
        nodes.iter().all(|node| node["phase"] == "successful"),
        "{finished}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn two_teams_share_the_global_slots_and_each_stays_below_its_own_limit() {
    // Four places overall, two per team. Three jobs in each team mean both
    // teams keep one job waiting while every global place is in use.
    let harness = TestApp::boot("concurrency-teams", options(4, 2));
    let (cookie, team_a) = harness.sign_in().await;
    let team_b = create_team(&harness, &cookie, &team_a, "Second team").await;
    let release_a = harness.dir.join("release-a");
    let release_b = harness.dir.join("release-b");

    let template_a = shell_template(
        &harness,
        &cookie,
        &team_a,
        "Team A job",
        &blocked_script(&release_a, &harness.dir.join("started-a")),
    )
    .await;
    let template_b = shell_template(
        &harness,
        &cookie,
        &team_b,
        "Team B job",
        &blocked_script(&release_b, &harness.dir.join("started-b")),
    )
    .await;

    // Team A is queued first, so arrival order is A1, A2, A3, B1, B2, B3.
    let mut a = Vec::new();
    let mut b = Vec::new();
    for _ in 1..=3 {
        a.push(launch(&harness, &cookie, &team_a, &template_a).await);
    }
    for _ in 1..=3 {
        b.push(launch(&harness, &cookie, &team_b, &template_b).await);
    }

    let mut admitted = false;
    for _ in 0..POLLS {
        let current_a = statuses(&harness, &cookie, &team_a, &a).await;
        let current_b = statuses(&harness, &cookie, &team_b, &b).await;
        let running_a = current_a.iter().filter(|item| *item == "running").count();
        let running_b = current_b.iter().filter(|item| *item == "running").count();
        assert!(
            running_a <= 2,
            "team A ran {running_a} jobs with a per-team limit of 2: {current_a:?}"
        );
        assert!(
            running_b <= 2,
            "team B ran {running_b} jobs with a per-team limit of 2: {current_b:?}"
        );
        assert!(
            running_a + running_b <= 4,
            "more than four jobs ran at once: {current_a:?} {current_b:?}"
        );
        if running_a == 2 && running_b == 2 {
            admitted = true;
            break;
        }
        tokio::time::sleep(POLL_STEP).await;
    }
    assert!(
        admitted,
        "the remaining global places must serve the second team: {:?} {:?}",
        statuses(&harness, &cookie, &team_a, &a).await,
        statuses(&harness, &cookie, &team_b, &b).await
    );
    assert_eq!(
        statuses(&harness, &cookie, &team_a, &a).await,
        vec![
            "running".to_string(),
            "running".to_string(),
            "pending".to_string()
        ],
        "team A must keep its third job waiting"
    );
    assert_eq!(
        statuses(&harness, &cookie, &team_b, &b).await,
        vec![
            "running".to_string(),
            "running".to_string(),
            "pending".to_string()
        ],
        "team B must keep its third job waiting"
    );

    // Team B frees both of its places. Its oldest waiter runs at once.
    std::fs::write(&release_b, "go").expect("release");
    wait_for_all(&harness, &cookie, &team_b, &b, "successful").await;
    // Team A still holds both of its places, so its third job keeps waiting
    // even though two global places are free again.
    assert_eq!(
        statuses(&harness, &cookie, &team_a, &a).await,
        vec![
            "running".to_string(),
            "running".to_string(),
            "pending".to_string()
        ],
        "a team must not refill a place above its own limit"
    );

    std::fs::write(&release_a, "go").expect("release");
    wait_for_all(&harness, &cookie, &team_a, &a, "successful").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_canceled_waiter_gives_its_place_to_the_oldest_waiter_of_another_team() {
    let harness = TestApp::boot("concurrency-team-cancel", options(1, 1));
    let (cookie, team_a) = harness.sign_in().await;
    let team_b = create_team(&harness, &cookie, &team_a, "Cancelled team").await;
    let release_a = harness.dir.join("release-a");
    let release_b = harness.dir.join("release-b");

    let template_a = shell_template(
        &harness,
        &cookie,
        &team_a,
        "Team A job",
        &blocked_script(&release_a, &harness.dir.join("started-a")),
    )
    .await;
    let template_b = shell_template(
        &harness,
        &cookie,
        &team_b,
        "Team B job",
        &blocked_script(&release_b, &harness.dir.join("started-b")),
    )
    .await;

    let running = launch(&harness, &cookie, &team_a, &template_a).await;
    let canceled = launch(&harness, &cookie, &team_a, &template_a).await;
    let waiting = launch(&harness, &cookie, &team_b, &template_b).await;
    wait_for_running(
        &harness,
        &cookie,
        &team_a,
        std::slice::from_ref(&running),
        1,
        1,
    )
    .await;
    assert_eq!(
        job_status(&harness, &cookie, &team_a, &canceled).await,
        "pending"
    );
    assert_eq!(
        job_status(&harness, &cookie, &team_b, &waiting).await,
        "pending"
    );

    // The second team A job is the oldest waiter. Once it is canceled, the
    // next freed place belongs to the older remaining waiter, team B's job.
    cancel(&harness, &cookie, &team_a, &canceled).await;
    std::fs::write(&release_a, "go").expect("release");
    wait_for_all(
        &harness,
        &cookie,
        &team_a,
        std::slice::from_ref(&running),
        "successful",
    )
    .await;
    wait_for_running(
        &harness,
        &cookie,
        &team_b,
        std::slice::from_ref(&waiting),
        1,
        1,
    )
    .await;
    assert!(
        !harness.dir.join("jobs").join(&canceled).exists(),
        "a canceled waiter must not create a work directory"
    );

    std::fs::write(&release_b, "go").expect("release");
    wait_for_all(
        &harness,
        &cookie,
        &team_b,
        std::slice::from_ref(&waiting),
        "successful",
    )
    .await;
    assert_eq!(
        job_status(&harness, &cookie, &team_a, &canceled).await,
        "canceled"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_per_team_limit_above_the_global_limit_uses_the_global_limit() {
    let harness = TestApp::boot("concurrency-team-clamp", options(2, 8));
    let (cookie, team) = harness.sign_in().await;
    let release = harness.dir.join("release");
    let template = shell_template(
        &harness,
        &cookie,
        &team,
        "Clamped",
        &blocked_script(&release, &harness.dir.join("started")),
    )
    .await;

    let mut ids = Vec::new();
    for _ in 1..=3 {
        ids.push(launch(&harness, &cookie, &team, &template).await);
    }
    // The per-team value of eight is clamped to the global limit of two, so
    // the third job still waits.
    wait_for_running(&harness, &cookie, &team, &ids, 2, 2).await;
    assert_eq!(
        statuses(&harness, &cookie, &team, &ids).await,
        vec![
            "running".to_string(),
            "running".to_string(),
            "pending".to_string()
        ],
        "a per-team limit above the global limit must not raise the global limit"
    );

    std::fs::write(&release, "go").expect("release");
    wait_for_all(&harness, &cookie, &team, &ids, "successful").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workflow_cancel_removes_a_node_that_waits_for_a_slot() {
    let harness = TestApp::boot("concurrency-workflow-cancel", options(1, 1));
    let (cookie, team) = harness.sign_in().await;
    let release = harness.dir.join("release");
    let marker = harness.dir.join("started");
    let template = shell_template(
        &harness,
        &cookie,
        &team,
        "Held node",
        &blocked_script(&release, &marker),
    )
    .await;

    // A direct launch takes the only place, so the workflow node has to wait.
    let running = launch(&harness, &cookie, &team, &template).await;
    wait_for_running(
        &harness,
        &cookie,
        &team,
        std::slice::from_ref(&running),
        1,
        1,
    )
    .await;

    let (status, workflow) = harness
        .call(
            "POST",
            "/api/workflows",
            Some(&cookie),
            Some(&team),
            Some(json!({
                "name": "Queued node",
                "description": "",
                "graph": {
                    "nodes": [{"id": "one", "template_id": template, "name": "one"}],
                    "edges": []
                }
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");
    let workflow_id = workflow["id"].as_str().expect("workflow id");
    let (status, run) = harness
        .call(
            "POST",
            &format!("/api/workflows/{workflow_id}/launch"),
            Some(&cookie),
            Some(&team),
            Some(json!({"extra_vars": {}})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{run}");
    let run_id = run["id"].as_str().expect("run id");

    let mut node_job = None;
    for _ in 0..POLLS {
        let (status, live) = harness
            .call(
                "GET",
                &format!("/api/workflow-jobs/{run_id}"),
                Some(&cookie),
                Some(&team),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{live}");
        node_job = live["nodes"][0]["job_id"].as_str().map(str::to_string);
        if node_job.is_some() {
            break;
        }
        tokio::time::sleep(POLL_STEP).await;
    }
    let node_job = node_job.expect("the workflow did not launch its node job");
    assert_eq!(
        job_status(&harness, &cookie, &team, &node_job).await,
        "pending"
    );

    let (status, canceled) = harness
        .call(
            "POST",
            &format!("/api/workflow-jobs/{run_id}/cancel"),
            Some(&cookie),
            Some(&team),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{canceled}");
    assert_eq!(canceled["status"], "canceled");

    // The freed place only appears after the running job ends. The canceled
    // node must not take it.
    std::fs::write(&release, "go").expect("release");
    wait_for_all(
        &harness,
        &cookie,
        &team,
        std::slice::from_ref(&running),
        "successful",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    assert_eq!(
        job_status(&harness, &cookie, &team, &node_job).await,
        "canceled"
    );
    assert!(
        !harness.dir.join("jobs").join(&node_job).exists(),
        "a canceled node must not create a work directory"
    );
}
