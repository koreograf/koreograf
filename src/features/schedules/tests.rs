use super::*;
use crate::db;
use chrono::Timelike;

fn berlin() -> Tz {
    chrono_tz::Europe::Berlin
}

fn sample(frequency: &str, start: &str, time: &str, days: &[&str], end_kind: &str) -> Rule {
    let owned: Vec<String> = days.iter().map(|day| (*day).to_string()).collect();
    parse(&Input {
        timezone: "Europe/Berlin",
        start_date: start,
        start_time: time,
        frequency,
        weekdays: &owned,
        end_kind,
        end_count: None,
        end_date: None,
    })
    .expect("rule")
}

fn local_hour(at: DateTime<Utc>) -> (u32, u32) {
    let local = at.with_timezone(&berlin());
    (local.hour(), local.minute())
}

#[test]
fn daily_six_stays_six_across_daylight_saving() {
    let rule = sample("daily", "2026-03-28", "06:00", &[], "never");
    let (index, _) = first_from(
        &rule,
        DateTime::parse_from_rfc3339("2026-03-28T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    )
    .unwrap();
    assert_eq!(index, 0);
    let before = nth(&rule, 0).unwrap().unwrap();
    let after = nth(&rule, 1).unwrap().unwrap();
    assert_eq!(local_hour(before), (6, 0));
    assert_eq!(local_hour(after), (6, 0));
    assert_eq!(before.format("%H:%M").to_string(), "05:00");
    assert_eq!(after.format("%H:%M").to_string(), "04:00");

    let autumn = sample("daily", "2026-10-24", "06:00", &[], "never");
    let before = nth(&autumn, 0).unwrap().unwrap();
    let after = nth(&autumn, 1).unwrap().unwrap();
    assert_eq!(local_hour(before), (6, 0));
    assert_eq!(local_hour(after), (6, 0));
    assert_eq!(before.format("%H:%M").to_string(), "04:00");
    assert_eq!(after.format("%H:%M").to_string(), "05:00");
}

#[test]
fn a_missing_local_time_shifts_forward_and_a_repeated_one_runs_once() {
    let rule = sample("daily", "2026-03-28", "02:30", &[], "never");
    let first = nth(&rule, 0).unwrap().unwrap();
    assert_eq!(
        first.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "2026-03-28T01:30:00Z"
    );
    let second = nth(&rule, 1).unwrap().unwrap();
    assert_eq!(
        second.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "2026-03-29T01:30:00Z"
    );
    assert_eq!(local_hour(second), (3, 30));

    let back = sample("daily", "2026-10-25", "02:30", &[], "never");
    let at = nth(&back, 0).unwrap().unwrap();
    assert_eq!(
        at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "2026-10-25T00:30:00Z"
    );
    assert_eq!(local_hour(at), (2, 30));
}

#[test]
fn gap_days_shift_forward_and_overlap_days_run_once() {
    let rule = sample("daily", "2027-03-28", "02:30", &[], "never");
    let at = nth(&rule, 0).unwrap().unwrap();
    assert_eq!(
        at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "2027-03-28T01:30:00Z"
    );
    assert_eq!(local_hour(at), (3, 30));

    let back = sample("daily", "2027-10-31", "02:30", &[], "never");
    let at = nth(&back, 0).unwrap().unwrap();
    assert_eq!(
        at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "2027-10-31T00:30:00Z"
    );
    assert_eq!(local_hour(at), (2, 30));

    let rule = sample("daily", "2027-03-27", "02:30", &[], "never");
    let not_before = DateTime::parse_from_rfc3339("2027-03-27T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let out = preview(&rule, not_before, 3).unwrap();
    assert_eq!(out.len(), 3);
    assert_eq!(
        out[0].format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "2027-03-27T01:30:00Z"
    );
    assert_eq!(
        out[1].format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "2027-03-28T01:30:00Z"
    );
    assert_eq!(local_hour(out[1]), (3, 30));
    assert_eq!(
        out[2].format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "2027-03-29T00:30:00Z"
    );
}

#[test]
fn weekly_days_and_monthly_day_follow_the_calendar() {
    let weekly = sample("weekly", "2026-10-19", "06:00", &["TH", "TU"], "never");
    assert_eq!(weekday_codes(&weekly.weekdays), "TU,TH");
    let first = nth(&weekly, 0).unwrap().unwrap();
    let second = nth(&weekly, 1).unwrap().unwrap();
    assert_eq!(
        first.with_timezone(&berlin()).date_naive().to_string(),
        "2026-10-20"
    );
    assert_eq!(
        second.with_timezone(&berlin()).date_naive().to_string(),
        "2026-10-22"
    );

    let monthly = sample("monthly", "2026-01-31", "06:00", &[], "never");
    let second = nth(&monthly, 1).unwrap().unwrap();
    assert_eq!(
        second.with_timezone(&berlin()).date_naive().to_string(),
        "2026-03-31"
    );
}

#[test]
fn the_end_stops_the_series() {
    let mut owned = Vec::new();
    let after = parse(&Input {
        timezone: "Europe/Berlin",
        start_date: "2026-10-21",
        start_time: "06:00",
        frequency: "daily",
        weekdays: &owned,
        end_kind: "after",
        end_count: Some(2),
        end_date: None,
    })
    .unwrap();
    assert!(nth(&after, 1).unwrap().is_some());
    assert!(nth(&after, 2).unwrap().is_none());

    owned.clear();
    let on_date = parse(&Input {
        timezone: "Europe/Berlin",
        start_date: "2026-10-21",
        start_time: "06:00",
        frequency: "daily",
        weekdays: &owned,
        end_kind: "on_date",
        end_count: None,
        end_date: Some("2026-10-22"),
    })
    .unwrap();
    assert!(nth(&on_date, 1).unwrap().is_some());
    assert!(nth(&on_date, 2).unwrap().is_none());
}

#[test]
fn a_stopped_process_misses_the_last_time_and_a_running_one_starts_one() {
    let rule = sample("daily", "2026-10-19", "06:00", &[], "never");
    let now = DateTime::parse_from_rfc3339("2026-10-21T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let missed = step(&rule, 0, now, true).unwrap();
    assert_eq!(
        missed.action,
        Action::Miss(
            DateTime::parse_from_rfc3339("2026-10-21T04:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        )
    );
    assert_eq!(missed.occurrence_count, 3);
    assert_eq!(
        missed
            .next_run
            .unwrap()
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string(),
        "2026-10-22T04:00:00Z"
    );

    let fired = step(&rule, 0, now, false).unwrap();
    assert_eq!(
        fired.action,
        Action::Fire(
            DateTime::parse_from_rfc3339("2026-10-19T04:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        )
    );
    assert_eq!(fired.occurrence_count, 1);
    let again = step(&rule, fired.occurrence_count, now, false).unwrap();
    assert!(matches!(again.action, Action::Fire(_)));
}

#[test]
fn a_daily_run_far_ahead_uses_the_calendar_day() {
    let rule = sample("daily", "2026-01-01", "06:00", &[], "never");
    let not_before = DateTime::parse_from_rfc3339("2026-10-01T03:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let (index, at) = first_from(&rule, not_before).unwrap();
    assert_eq!(index, 273);
    assert_eq!(
        at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "2026-10-01T04:00:00Z"
    );
}

#[test]
fn a_start_more_than_a_year_ago_is_rejected() {
    let zone = berlin();
    let today = Utc::now().with_timezone(&zone).date_naive();
    let recent = today
        .checked_sub_months(chrono::Months::new(12))
        .expect("year");
    assert!(ensure_start_is_recent(recent, zone).is_ok());
    let too_old = recent.pred_opt().expect("day");
    assert_eq!(
        ensure_start_is_recent(too_old, zone),
        Err("schedule_date_is_too_far_in_the_past")
    );
}

#[tokio::test]
async fn an_operator_schedule_does_not_start_an_admin_only_local_run() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-schedule-operator-{nanos}"));
    let state = crate::boot(admins_config(&dir)).expect("boot");
    let team_id: String = {
        let db = state.db().expect("db");
        db.query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team")
    };
    let yesterday = Utc::now()
        .date_naive()
        .pred_opt()
        .expect("yesterday")
        .format("%Y-%m-%d")
        .to_string();
    let template_id = {
        let db = state.db().expect("db");
        crate::features::identity::store::insert_user(
            &db,
            "operator-user",
            "operator",
            "Operator",
            None,
            "local",
            false,
        )
        .expect("operator");
        let id = "operator-shell".to_string();
        crate::features::projects::store::insert_template(
            &db,
            &db::Template {
                id: id.clone(),
                team_id: team_id.clone(),
                name: "Operator shell".into(),
                description: String::new(),
                tool: "shell".into(),
                project_id: None,
                inventory_id: None,
                credential_id: None,
                environment_id: None,
                no_environment: true,
                entrypoint: String::new(),
                extra_vars: "{}".into(),
                spec: r#"{"script":":"}"#.to_string(),
                created_at: db::now(),
                update_on_launch: false,
                admin_saved: true,
                prompt_on_launch: false,
            },
        )
        .expect("template");
        let now = db::now();
        crate::features::schedules::store::insert_schedule(
            &db,
            &db::Schedule {
                id: "operator-schedule".into(),
                team_id: team_id.clone(),
                name: "Operator clock".into(),
                target_kind: "template".into(),
                target_id: id.clone(),
                timezone: "UTC".into(),
                start_date: yesterday,
                start_time: "00:00".into(),
                frequency: "once".into(),
                weekdays: String::new(),
                end_kind: "never".into(),
                end_count: None,
                end_date: None,
                enabled: true,
                next_run_at: Some("2000-01-01T00:00:00Z".into()),
                missed_at: None,
                occurrence_count: 0,
                created_by: "operator".into(),
                updated_by: "operator".into(),
                created_at: now.clone(),
                updated_at: now,
                launch_block: None,
            },
        )
        .expect("schedule");
        id
    };
    let item = {
        let db = state.db().expect("db");
        crate::features::schedules::store::schedule(&db, "operator-schedule")
            .expect("read")
            .expect("schedule")
    };
    one(&state, &item, Utc::now(), false).await.expect("step");
    {
        let db = state.db().expect("db");
        let jobs: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE template_id = ?1",
                [&template_id],
                |row| row.get(0),
            )
            .expect("jobs");
        assert_eq!(jobs, 0);
        let missed: Option<String> = db
            .query_row(
                "SELECT missed_at FROM schedules WHERE id = 'operator-schedule'",
                [],
                |row| row.get(0),
            )
            .expect("miss");
        assert!(missed.is_some(), "a refused run is kept as a miss");
        db.execute(
            "UPDATE schedules
             SET occurrence_count = 0, missed_at = NULL, updated_by = 'admin'
             WHERE id = 'operator-schedule'",
            [],
        )
        .expect("reset");
    }
    let item = {
        let db = state.db().expect("db");
        crate::features::schedules::store::schedule(&db, "operator-schedule")
            .expect("read")
            .expect("schedule")
    };
    one(&state, &item, Utc::now(), false)
        .await
        .expect("admin step");
    let db = state.db().expect("db");
    let launched: String = db
        .query_row(
            "SELECT launched_by FROM jobs WHERE template_id = ?1",
            [&template_id],
            |row| row.get(0),
        )
        .expect("admin job");
    assert_eq!(launched, "admin");
    let creator: String = db
        .query_row(
            "SELECT created_by FROM schedules WHERE id = 'operator-schedule'",
            [],
            |row| row.get(0),
        )
        .expect("creator");
    assert_eq!(creator, "operator");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_removed_member_does_not_start_the_next_run() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("koreograf-schedule-member-{nanos}"));
    let mut cfg = admins_config(&dir);
    cfg.local_jobs = crate::config::LocalJobs::All;
    let state = crate::boot(cfg).expect("boot");
    let team_id: String = {
        let db = state.db().expect("db");
        db.query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team")
    };
    let yesterday = Utc::now()
        .date_naive()
        .pred_opt()
        .expect("yesterday")
        .format("%Y-%m-%d")
        .to_string();
    let template_id = {
        let db = state.db().expect("db");
        crate::features::identity::store::insert_user(
            &db,
            "operator-user",
            "operator",
            "Operator",
            None,
            "local",
            false,
        )
        .expect("operator");
        crate::features::identity::store::grant_manual_membership(
            &db,
            &team_id,
            "operator-user",
            "operator",
        )
        .expect("member");
        crate::features::identity::store::delete_membership(&db, &team_id, "operator-user")
            .expect("remove");
        let id = "member-shell".to_string();
        crate::features::projects::store::insert_template(
            &db,
            &db::Template {
                id: id.clone(),
                team_id: team_id.clone(),
                name: "Member shell".into(),
                description: String::new(),
                tool: "shell".into(),
                project_id: None,
                inventory_id: None,
                credential_id: None,
                environment_id: None,
                no_environment: true,
                entrypoint: String::new(),
                extra_vars: "{}".into(),
                spec: r#"{"script":":"}"#.to_string(),
                created_at: db::now(),
                update_on_launch: false,
                admin_saved: true,
                prompt_on_launch: false,
            },
        )
        .expect("template");
        let now = db::now();
        crate::features::schedules::store::insert_schedule(
            &db,
            &db::Schedule {
                id: "member-schedule".into(),
                team_id: team_id.clone(),
                name: "Member clock".into(),
                target_kind: "template".into(),
                target_id: id.clone(),
                timezone: "UTC".into(),
                start_date: yesterday,
                start_time: "00:00".into(),
                frequency: "once".into(),
                weekdays: String::new(),
                end_kind: "never".into(),
                end_count: None,
                end_date: None,
                enabled: true,
                next_run_at: Some("2000-01-01T00:00:00Z".into()),
                missed_at: None,
                occurrence_count: 0,
                created_by: "operator".into(),
                updated_by: "operator".into(),
                created_at: now.clone(),
                updated_at: now,
                launch_block: None,
            },
        )
        .expect("schedule");
        id
    };
    let load = || {
        let db = state.db().expect("db");
        crate::features::schedules::store::schedule(&db, "member-schedule")
            .expect("read")
            .expect("schedule")
    };
    one(&state, &load(), Utc::now(), false)
        .await
        .expect("removed");
    {
        let db = state.db().expect("db");
        let jobs: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE template_id = ?1",
                [&template_id],
                |row| row.get(0),
            )
            .expect("jobs");
        assert_eq!(jobs, 0);
        let missed: Option<String> = db
            .query_row(
                "SELECT missed_at FROM schedules WHERE id = 'member-schedule'",
                [],
                |row| row.get(0),
            )
            .expect("miss");
        assert!(missed.is_some(), "a refused run is kept as a miss");
        let reason: Option<String> = db
            .query_row(
                "SELECT launch_block FROM schedules WHERE id = 'member-schedule'",
                [],
                |row| row.get(0),
            )
            .expect("reason");
        assert_eq!(reason.as_deref(), Some("schedule_needs_a_new_owner"));
        crate::features::identity::store::grant_manual_membership(
            &db,
            &team_id,
            "operator-user",
            "viewer",
        )
        .expect("viewer");
        db.execute(
            "UPDATE schedules
             SET occurrence_count = 0, missed_at = NULL
             WHERE id = 'member-schedule'",
            [],
        )
        .expect("reset viewer");
    }
    one(&state, &load(), Utc::now(), false)
        .await
        .expect("viewer");
    {
        let db = state.db().expect("db");
        let jobs: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE template_id = ?1",
                [&template_id],
                |row| row.get(0),
            )
            .expect("jobs");
        assert_eq!(jobs, 0, "a viewer still cannot start the scheduled run");
        db.execute(
            "UPDATE schedules
             SET occurrence_count = 0, missed_at = NULL, updated_by = 'admin'
             WHERE id = 'member-schedule'",
            [],
        )
        .expect("reassign");
    }
    one(&state, &load(), Utc::now(), false)
        .await
        .expect("admin step");
    let db = state.db().expect("db");
    let launched: String = db
        .query_row(
            "SELECT launched_by FROM jobs WHERE template_id = ?1",
            [&template_id],
            |row| row.get(0),
        )
        .expect("admin job");
    assert_eq!(launched, "admin");
    let reason: Option<String> = db
        .query_row(
            "SELECT launch_block FROM schedules WHERE id = 'member-schedule'",
            [],
            |row| row.get(0),
        )
        .expect("cleared");
    assert_eq!(reason, None);
    let _ = std::fs::remove_dir_all(dir);
}

fn admins_config(dir: &std::path::Path) -> crate::config::Config {
    crate::config::Config {
        bind: "127.0.0.1:0".into(),
        data_dir: dir.to_path_buf(),
        web_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
        examples_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
        admin_user: "admin".into(),
        admin_password: Some("test-admin-pass".into()),
        runner: crate::config::RunnerMode::Local,
        container_image: false,
        job_timeout_secs: 60,
        job_output_lines: 8_000,
        local_jobs: crate::config::LocalJobs::Admins,
        project_roots: vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")],
        hsts: false,
        cookie_secure: crate::config::CookieSecure::Auto,
        trusted_proxy: false,
        mcp: true,
        public_url: Vec::new(),
        data_host: None,
        ldap: None,
    }
}
