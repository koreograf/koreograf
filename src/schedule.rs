//! Local-time schedules for templates and workflows.
//!
//! A schedule stores a clock time and a time zone. The next run is the next
//! moment that clock time exists. A time that falls in a daylight-saving gap
//! is skipped. A repeated local time, when the clocks go back, is used once,
//! at the earlier offset. Hourly schedules step by one hour of absolute time.

use std::time::Duration as StdDuration;

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use serde_json::json;

use crate::db::{self, Schedule};
use crate::error::{ApiError, AppResult};
use crate::jobs::{self, LaunchRequest};
use crate::Shared;

const SCAN_LIMIT: u32 = 200_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Frequency {
    Once,
    Hourly,
    Daily,
    Weekly,
    Monthly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum End {
    Never,
    After(u32),
    OnDate(NaiveDate),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Rule {
    pub timezone: Tz,
    pub start_date: NaiveDate,
    pub start_time: NaiveTime,
    pub frequency: Frequency,
    pub weekdays: Vec<Weekday>,
    pub end: End,
}

#[derive(Clone, Debug)]
pub(crate) struct Input<'a> {
    pub timezone: &'a str,
    pub start_date: &'a str,
    pub start_time: &'a str,
    pub frequency: &'a str,
    pub weekdays: &'a [String],
    pub end_kind: &'a str,
    pub end_count: Option<u32>,
    pub end_date: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Idle,
    Fire(DateTime<Utc>),
    Miss(DateTime<Utc>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Step {
    pub action: Action,
    pub occurrence_count: u32,
    pub next_run: Option<DateTime<Utc>>,
}

pub(crate) fn parse(input: &Input<'_>) -> Result<Rule, &'static str> {
    let timezone: Tz = input
        .timezone
        .parse()
        .map_err(|_| "schedule_needs_a_known_time_zone")?;
    let start_date = NaiveDate::parse_from_str(input.start_date, "%Y-%m-%d")
        .map_err(|_| "schedule_date_is_invalid")?;
    let start_time = parse_time(input.start_time)?;
    let frequency = match input.frequency {
        "once" => Frequency::Once,
        "hourly" => Frequency::Hourly,
        "daily" => Frequency::Daily,
        "weekly" => Frequency::Weekly,
        "monthly" => Frequency::Monthly,
        _ => return Err("schedule_repeat_is_invalid"),
    };
    let weekdays = if frequency == Frequency::Weekly {
        let mut days = Vec::new();
        for code in input.weekdays {
            let day = weekday(code).ok_or("schedule_needs_a_weekday")?;
            if !days.contains(&day) {
                days.push(day);
            }
        }
        if days.is_empty() {
            return Err("schedule_needs_a_weekday");
        }
        days.sort_by_key(|day| day.num_days_from_monday());
        days
    } else {
        Vec::new()
    };
    let end = match (frequency, input.end_kind) {
        (Frequency::Once, _) => End::Never,
        (_, "never") => End::Never,
        (_, "after") => {
            let count = input.end_count.ok_or("schedule_count_is_invalid")?;
            if !(1..=999).contains(&count) {
                return Err("schedule_count_is_invalid");
            }
            End::After(count)
        }
        (_, "on_date") => {
            let raw = input.end_date.ok_or("schedule_end_date_is_invalid")?;
            let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .map_err(|_| "schedule_end_date_is_invalid")?;
            if date < start_date {
                return Err("schedule_end_date_is_invalid");
            }
            End::OnDate(date)
        }
        _ => return Err("schedule_end_is_invalid"),
    };
    let rule = Rule {
        timezone,
        start_date,
        start_time,
        frequency,
        weekdays,
        end,
    };
    if matches!(frequency, Frequency::Once | Frequency::Hourly)
        && resolve(&rule, start_date, start_time).is_none()
    {
        return Err("schedule_time_does_not_exist");
    }
    Ok(rule)
}

pub(crate) fn weekday_codes(days: &[Weekday]) -> String {
    days.iter()
        .map(|day| match day {
            Weekday::Mon => "MO",
            Weekday::Tue => "TU",
            Weekday::Wed => "WE",
            Weekday::Thu => "TH",
            Weekday::Fri => "FR",
            Weekday::Sat => "SA",
            Weekday::Sun => "SU",
        })
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) fn format_utc(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub(crate) fn parse_utc(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// First occurrence at or after `not_before`, and its index in the series.
pub(crate) fn first_from(
    rule: &Rule,
    not_before: DateTime<Utc>,
) -> Result<(u32, DateTime<Utc>), &'static str> {
    if rule.frequency == Frequency::Hourly {
        let start = resolve(rule, rule.start_date, rule.start_time)
            .ok_or("schedule_time_does_not_exist")?;
        if start >= not_before {
            return Ok((0, start));
        }
        let hours = (not_before - start).num_hours();
        let mut index = u32::try_from(hours).map_err(|_| "schedule_has_no_occurrence")?;
        for _ in 0..3 {
            let Some(at) = nth(rule, index)? else {
                return Err("schedule_has_no_occurrence");
            };
            if at >= not_before {
                return Ok((index, at));
            }
            index = index.checked_add(1).ok_or("schedule_has_no_occurrence")?;
        }
        return Err("schedule_has_no_occurrence");
    }
    for index in 0..SCAN_LIMIT {
        match nth(rule, index)? {
            Some(at) if at >= not_before => return Ok((index, at)),
            Some(_) => {}
            None => return Err("schedule_has_no_occurrence"),
        }
    }
    Err("schedule_has_no_occurrence")
}

pub(crate) fn preview(
    rule: &Rule,
    not_before: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<DateTime<Utc>>, &'static str> {
    let (mut index, first) = first_from(rule, not_before)?;
    let mut out = vec![first];
    while out.len() < limit {
        index = index.checked_add(1).ok_or("schedule_has_no_occurrence")?;
        match nth(rule, index)? {
            Some(at) => out.push(at),
            None => break,
        }
    }
    Ok(out)
}

/// `startup` is the pass when the process has just come up. Times that are
/// already due were missed and are not started. While the process is up, one
/// due occurrence is started. The caller repeats the step for the next one.
pub(crate) fn step(
    rule: &Rule,
    occurrence_count: u32,
    now: DateTime<Utc>,
    startup: bool,
) -> Result<Step, &'static str> {
    if startup {
        let mut index = occurrence_count;
        let mut last = None;
        let mut walked = 0u32;
        while let Some(at) = nth(rule, index)? {
            if at > now {
                break;
            }
            last = Some(at);
            index = index.checked_add(1).ok_or("schedule_has_no_occurrence")?;
            walked += 1;
            if walked > SCAN_LIMIT {
                return Err("schedule_has_no_occurrence");
            }
        }
        let next_run = nth(rule, index)?;
        let action = match last {
            Some(at) if walked > 0 => Action::Miss(at),
            _ => Action::Idle,
        };
        return Ok(Step {
            action,
            occurrence_count: index,
            next_run,
        });
    }
    let Some(at) = nth(rule, occurrence_count)? else {
        return Ok(Step {
            action: Action::Idle,
            occurrence_count,
            next_run: None,
        });
    };
    if at > now {
        return Ok(Step {
            action: Action::Idle,
            occurrence_count,
            next_run: Some(at),
        });
    }
    let next_index = occurrence_count
        .checked_add(1)
        .ok_or("schedule_has_no_occurrence")?;
    Ok(Step {
        action: Action::Fire(at),
        occurrence_count: next_index,
        next_run: nth(rule, next_index)?,
    })
}

fn nth(rule: &Rule, index: u32) -> Result<Option<DateTime<Utc>>, &'static str> {
    if index > SCAN_LIMIT {
        return Err("schedule_has_no_occurrence");
    }
    match rule.frequency {
        Frequency::Once => {
            if index > 0 {
                return Ok(None);
            }
            Ok(resolve(rule, rule.start_date, rule.start_time))
        }
        Frequency::Hourly => {
            let start = resolve(rule, rule.start_date, rule.start_time)
                .ok_or("schedule_time_does_not_exist")?;
            let at = start + Duration::hours(i64::from(index));
            Ok(kept(rule, index, at))
        }
        Frequency::Daily => scan_days(rule, index, false),
        Frequency::Weekly => scan_days(rule, index, true),
        Frequency::Monthly => scan_months(rule, index),
    }
}

fn scan_days(rule: &Rule, index: u32, weekly: bool) -> Result<Option<DateTime<Utc>>, &'static str> {
    let mut seen = 0u32;
    let mut date = rule.start_date;
    for _ in 0..SCAN_LIMIT {
        let matches = !weekly || rule.weekdays.contains(&date.weekday());
        if matches {
            if let Some(at) = resolve(rule, date, rule.start_time) {
                if !within_end(rule, seen, at) {
                    return Ok(None);
                }
                if seen == index {
                    return Ok(Some(at));
                }
                seen += 1;
            }
        }
        date = date
            .checked_add_signed(Duration::days(1))
            .ok_or("schedule_has_no_occurrence")?;
    }
    Err("schedule_has_no_occurrence")
}

fn scan_months(rule: &Rule, index: u32) -> Result<Option<DateTime<Utc>>, &'static str> {
    let day = rule.start_date.day();
    let mut year = rule.start_date.year();
    let mut month = rule.start_date.month();
    let mut seen = 0u32;
    for _ in 0..SCAN_LIMIT {
        if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
            if date >= rule.start_date {
                if let Some(at) = resolve(rule, date, rule.start_time) {
                    if !within_end(rule, seen, at) {
                        return Ok(None);
                    }
                    if seen == index {
                        return Ok(Some(at));
                    }
                    seen += 1;
                }
            }
        }
        if month == 12 {
            month = 1;
            year = year.checked_add(1).ok_or("schedule_has_no_occurrence")?;
        } else {
            month += 1;
        }
    }
    Err("schedule_has_no_occurrence")
}

fn kept(rule: &Rule, index: u32, at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    within_end(rule, index, at).then_some(at)
}

fn within_end(rule: &Rule, index: u32, at: DateTime<Utc>) -> bool {
    match rule.end {
        End::Never => true,
        End::After(count) => index < count,
        End::OnDate(date) => at.with_timezone(&rule.timezone).date_naive() <= date,
    }
}

fn resolve(rule: &Rule, date: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
    let local = date.and_time(time);
    match rule.timezone.from_local_datetime(&local) {
        chrono::LocalResult::Single(at) | chrono::LocalResult::Ambiguous(at, _) => {
            Some(at.with_timezone(&Utc))
        }
        chrono::LocalResult::None => None,
    }
}

fn parse_time(text: &str) -> Result<NaiveTime, &'static str> {
    NaiveTime::parse_from_str(text, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(text, "%H:%M:%S"))
        .map_err(|_| "schedule_time_is_invalid")
}

pub(crate) fn spawn(state: Shared) {
    tokio::spawn(async move {
        if let Err(err) = pass(&state, true) {
            tracing::error!(error = %err, "schedule startup");
        }
        loop {
            tokio::time::sleep(StdDuration::from_secs(15)).await;
            if let Err(err) = pass(&state, false) {
                tracing::error!(error = %err, "schedule");
            }
        }
    });
}

fn pass(state: &Shared, startup: bool) -> AppResult<()> {
    let now = Utc::now();
    let due = {
        let db = state.db()?;
        db::due_schedules(&db, &format_utc(now))?
    };
    for item in due {
        if let Err(err) = one(state, &item, now, startup) {
            tracing::error!(schedule = %item.id, error = %err, "schedule");
        }
    }
    Ok(())
}

fn one(state: &Shared, item: &Schedule, now: DateTime<Utc>, startup: bool) -> AppResult<()> {
    let rule = rule_from(item).map_err(ApiError::bad)?;
    let count = u32::try_from(item.occurrence_count).unwrap_or(0);
    let decided = step(&rule, count, now, startup).map_err(ApiError::bad)?;
    let next = decided.next_run.map(format_utc);
    let count = i64::from(decided.occurrence_count);
    match decided.action {
        Action::Idle => {
            if item.next_run_at.as_deref() != next.as_deref() || item.occurrence_count != count {
                let db = state.db()?;
                db::advance_schedule(&db, &item.id, next.as_deref(), count)?;
            }
        }
        Action::Miss(at) => {
            let db = state.db()?;
            db::advance_schedule(&db, &item.id, next.as_deref(), count)?;
            db::note_miss(&db, &item.id, Some(&format_utc(at)))?;
        }
        Action::Fire(at) => {
            let actor = if item.created_by.is_empty() {
                "schedule"
            } else {
                item.created_by.as_str()
            };
            let started = start(state, item, actor);
            if let Err(err) = &started {
                tracing::warn!(schedule = %item.id, error = %err, "schedule did not start");
            }
            let db = state.db()?;
            db::advance_schedule(&db, &item.id, next.as_deref(), count)?;
            if started.is_err() {
                db::note_miss(&db, &item.id, Some(&format_utc(at)))?;
            }
        }
    }
    Ok(())
}

pub(crate) fn start(state: &Shared, item: &Schedule, actor: &str) -> AppResult<()> {
    let actor = if actor.is_empty() { "schedule" } else { actor };
    match item.target_kind.as_str() {
        "template" => {
            jobs::launch(
                state,
                LaunchRequest {
                    team_id: item.team_id.clone(),
                    template_id: item.target_id.clone(),
                    extra_vars: json!({}),
                    launched_by: actor.to_string(),
                    trigger: "schedule".to_string(),
                    workflow_job_id: None,
                    workflow_node_id: None,
                    parameters_override: None,
                },
            )?;
        }
        "workflow" => {
            jobs::launch_workflow(
                state,
                &item.team_id,
                &item.target_id,
                json!({}),
                actor,
                "schedule",
            )?;
        }
        _ => return Err(ApiError::bad("schedule_repeat_is_invalid")),
    }
    Ok(())
}

pub(crate) fn rule_from(item: &Schedule) -> Result<Rule, &'static str> {
    let days: Vec<String> = item
        .weekdays
        .split(',')
        .filter(|day| !day.is_empty())
        .map(str::to_string)
        .collect();
    parse(&Input {
        timezone: &item.timezone,
        start_date: &item.start_date,
        start_time: &item.start_time,
        frequency: &item.frequency,
        weekdays: &days,
        end_kind: &item.end_kind,
        end_count: item.end_count.and_then(|count| u32::try_from(count).ok()),
        end_date: item.end_date.as_deref(),
    })
}

fn weekday(code: &str) -> Option<Weekday> {
    match code {
        "MO" => Some(Weekday::Mon),
        "TU" => Some(Weekday::Tue),
        "WE" => Some(Weekday::Wed),
        "TH" => Some(Weekday::Thu),
        "FR" => Some(Weekday::Fri),
        "SA" => Some(Weekday::Sat),
        "SU" => Some(Weekday::Sun),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn a_missing_local_time_is_skipped_and_a_repeated_one_runs_once() {
        let rule = sample("daily", "2026-03-28", "02:30", &[], "never");
        let first = nth(&rule, 0).unwrap().unwrap();
        let second = nth(&rule, 1).unwrap().unwrap();
        assert_eq!(
            first.with_timezone(&berlin()).date_naive().to_string(),
            "2026-03-28"
        );
        assert_eq!(
            second.with_timezone(&berlin()).date_naive().to_string(),
            "2026-03-30"
        );

        let back = sample("daily", "2026-10-25", "02:30", &[], "never");
        let at = nth(&back, 0).unwrap().unwrap();
        assert_eq!(
            at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "2026-10-25T00:30:00Z"
        );
        assert_eq!(local_hour(at), (2, 30));
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
}
