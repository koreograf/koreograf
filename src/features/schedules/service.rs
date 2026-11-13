//! Local-time schedules for templates and workflows.
//!
//! A schedule stores a clock time and a time zone. The next run is the next
//! moment that clock time exists. A time that falls in a daylight-saving gap
//! runs shifted forward by the width of the gap, so 02:30 becomes 03:30 when
//! clocks jump from 02:00 to 03:00. A repeated local time, when the clocks go
//! back, is used once, at the earlier offset. Hourly schedules step by one
//! hour of absolute time.
//! Daily and weekly schedules step by the calendar, not by walking each day.

use std::time::Duration as StdDuration;

use chrono::{
    DateTime, Datelike, Duration, Months, NaiveDate, NaiveDateTime, NaiveTime, Offset, TimeZone,
    Utc, Weekday,
};
use chrono_tz::Tz;
use serde_json::Value;

use crate::db::Schedule;
use crate::error::{ApiError, AppResult};
use crate::features::jobs::{self, LaunchRequest};
use crate::Shared;

use super::model::{Action, End, Frequency, Input, Rule, Step};

const SCAN_LIMIT: u32 = 200_000;

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

/// A start more than one calendar year before today, or more than ten years
/// after today, in `timezone` is rejected. Stored schedules keep their
/// original start and are not checked again.
pub(crate) fn ensure_start_is_recent(
    start_date: NaiveDate,
    timezone: Tz,
) -> Result<(), &'static str> {
    let today = Utc::now().with_timezone(&timezone).date_naive();
    let Some(limit) = today.checked_sub_months(Months::new(12)) else {
        return Ok(());
    };
    if start_date < limit {
        return Err("schedule_date_is_too_far_in_the_past");
    }
    let Some(limit) = today.checked_add_months(Months::new(120)) else {
        return Ok(());
    };
    if start_date > limit {
        return Err("schedule_date_is_too_far_in_the_future");
    }
    Ok(())
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
    match rule.frequency {
        Frequency::Hourly => first_hourly(rule, not_before),
        Frequency::Daily => {
            let guess = daily_guess(rule, not_before)?;
            first_near(rule, not_before, guess)
        }
        Frequency::Weekly => {
            let guess = weekly_guess(rule, not_before)?;
            first_near(rule, not_before, guess)
        }
        Frequency::Once | Frequency::Monthly => first_by_scan(rule, not_before),
    }
}

pub(crate) async fn first_from_blocking(
    rule: Rule,
    not_before: DateTime<Utc>,
) -> Result<(u32, DateTime<Utc>), &'static str> {
    tokio::task::spawn_blocking(move || first_from(&rule, not_before))
        .await
        .map_err(|_| "schedule_has_no_occurrence")?
}

pub(crate) async fn preview_blocking(
    rule: Rule,
    not_before: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<DateTime<Utc>>, &'static str> {
    tokio::task::spawn_blocking(move || preview(&rule, not_before, limit))
        .await
        .map_err(|_| "schedule_has_no_occurrence")?
}

pub(crate) async fn step_blocking(
    rule: Rule,
    occurrence_count: u32,
    now: DateTime<Utc>,
    startup: bool,
) -> Result<Step, &'static str> {
    tokio::task::spawn_blocking(move || step(&rule, occurrence_count, now, startup))
        .await
        .map_err(|_| "schedule_has_no_occurrence")?
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
        if matches!(
            rule.frequency,
            Frequency::Hourly | Frequency::Daily | Frequency::Weekly
        ) {
            return startup_jump(rule, occurrence_count, now);
        }
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
        Frequency::Daily => nth_daily(rule, index),
        Frequency::Weekly => nth_weekly(rule, index),
        Frequency::Monthly => scan_months(rule, index),
    }
}

fn first_hourly(
    rule: &Rule,
    not_before: DateTime<Utc>,
) -> Result<(u32, DateTime<Utc>), &'static str> {
    let start =
        resolve(rule, rule.start_date, rule.start_time).ok_or("schedule_time_does_not_exist")?;
    if start >= not_before {
        return Ok((0, start));
    }
    let hours = (not_before - start).num_hours();
    let index = u32::try_from(hours).map_err(|_| "schedule_has_no_occurrence")?;
    first_near(rule, not_before, index.saturating_sub(1))
}

fn first_by_scan(
    rule: &Rule,
    not_before: DateTime<Utc>,
) -> Result<(u32, DateTime<Utc>), &'static str> {
    for index in 0..SCAN_LIMIT {
        match nth(rule, index)? {
            Some(at) if at >= not_before => return Ok((index, at)),
            Some(_) => {}
            None => return Err("schedule_has_no_occurrence"),
        }
    }
    Err("schedule_has_no_occurrence")
}

fn first_near(
    rule: &Rule,
    not_before: DateTime<Utc>,
    guess: u32,
) -> Result<(u32, DateTime<Utc>), &'static str> {
    let mut index = guess;
    for _ in 0..8 {
        match nth(rule, index)? {
            Some(at) if at >= not_before => return Ok((index, at)),
            Some(_) => index = index.checked_add(1).ok_or("schedule_has_no_occurrence")?,
            None => return Err("schedule_has_no_occurrence"),
        }
    }
    Err("schedule_has_no_occurrence")
}

fn daily_guess(rule: &Rule, not_before: DateTime<Utc>) -> Result<u32, &'static str> {
    let local = not_before.with_timezone(&rule.timezone).date_naive();
    if local <= rule.start_date {
        return Ok(0);
    }
    let calendar = local
        .signed_duration_since(rule.start_date)
        .num_days()
        .max(0);
    let guess = u32::try_from(calendar).unwrap_or(u32::MAX);
    Ok(guess.saturating_sub(1))
}

fn weekly_guess(rule: &Rule, not_before: DateTime<Utc>) -> Result<u32, &'static str> {
    let local = not_before.with_timezone(&rule.timezone).date_naive();
    if local <= rule.start_date {
        return Ok(0);
    }
    let before = count_selected(rule.start_date, local, &rule.weekdays);
    let guess = u32::try_from(before).unwrap_or(u32::MAX);
    Ok(guess.saturating_sub(1))
}

fn startup_jump(
    rule: &Rule,
    occurrence_count: u32,
    now: DateTime<Utc>,
) -> Result<Step, &'static str> {
    let just_after = now + Duration::nanoseconds(1);
    if let Ok((index, _)) = first_from(rule, just_after) {
        let index = index.max(occurrence_count);
        let last = if index > occurrence_count {
            nth(rule, index - 1)?
        } else {
            None
        };
        let action = match last {
            Some(at) if at <= now => Action::Miss(at),
            _ => Action::Idle,
        };
        return Ok(Step {
            action,
            occurrence_count: index,
            next_run: nth(rule, index)?,
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
    let last_index = last_existing(rule, occurrence_count)?;
    let last_at = nth(rule, last_index)?.ok_or("schedule_has_no_occurrence")?;
    let next_index = last_index
        .checked_add(1)
        .ok_or("schedule_has_no_occurrence")?;
    Ok(Step {
        action: Action::Miss(last_at),
        occurrence_count: next_index,
        next_run: nth(rule, next_index)?,
    })
}

fn last_existing(rule: &Rule, from: u32) -> Result<u32, &'static str> {
    if nth(rule, from)?.is_none() {
        return Err("schedule_has_no_occurrence");
    }
    let mut lo = from;
    let mut hi = from;
    loop {
        if hi >= SCAN_LIMIT {
            break;
        }
        let next = hi.saturating_mul(2).saturating_add(1).min(SCAN_LIMIT);
        if nth(rule, next)?.is_some() {
            lo = next;
            if next == SCAN_LIMIT {
                break;
            }
            hi = next;
        } else {
            hi = next;
            break;
        }
    }
    while lo < hi {
        let mid = lo + (hi - lo) / 2 + 1;
        if nth(rule, mid)?.is_some() {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    Ok(lo)
}

fn nth_daily(rule: &Rule, index: u32) -> Result<Option<DateTime<Utc>>, &'static str> {
    for offset in (i64::from(index)..).take(6) {
        let Some(date) = rule.start_date.checked_add_signed(Duration::days(offset)) else {
            return Err("schedule_has_no_occurrence");
        };
        if let Some(at) = resolve(rule, date, rule.start_time) {
            return Ok(kept(rule, index, at));
        }
    }
    Err("schedule_has_no_occurrence")
}

fn nth_weekly(rule: &Rule, index: u32) -> Result<Option<DateTime<Utc>>, &'static str> {
    for raw in (index..).take(6) {
        let Some(date) = weekly_date(rule.start_date, &rule.weekdays, raw) else {
            return Err("schedule_has_no_occurrence");
        };
        if let Some(at) = resolve(rule, date, rule.start_time) {
            return Ok(kept(rule, index, at));
        }
    }
    Err("schedule_has_no_occurrence")
}

fn weekly_date(start: NaiveDate, days: &[Weekday], index: u32) -> Option<NaiveDate> {
    if days.is_empty() {
        return None;
    }
    let start_wd = start.weekday().num_days_from_monday();
    let nums: Vec<u32> = days.iter().map(Weekday::num_days_from_monday).collect();
    let first_week: Vec<u32> = nums
        .iter()
        .copied()
        .filter(|day| *day >= start_wd)
        .collect();
    let first_len = u32::try_from(first_week.len()).unwrap_or(0);
    if index < first_len {
        let delta = i64::from(first_week[index as usize] - start_wd);
        return start.checked_add_signed(Duration::days(delta));
    }
    let rest = index - first_len;
    let width = u32::try_from(nums.len()).unwrap_or(1).max(1);
    let week = i64::from(rest / width);
    let slot = (rest % width) as usize;
    let next_monday = start.checked_add_signed(Duration::days(i64::from(7 - start_wd)))?;
    next_monday.checked_add_signed(Duration::days(week * 7 + i64::from(nums[slot])))
}

fn count_selected(start: NaiveDate, end: NaiveDate, days: &[Weekday]) -> i64 {
    if end <= start || days.is_empty() {
        return 0;
    }
    let total = end.signed_duration_since(start).num_days();
    let weeks = total / 7;
    let extra = total % 7;
    let mut count = weeks * i64::try_from(days.len()).unwrap_or(0);
    let Some(partial) = start.checked_add_signed(Duration::days(weeks * 7)) else {
        return count;
    };
    for offset in 0..extra {
        let Some(date) = partial.checked_add_signed(Duration::days(offset)) else {
            break;
        };
        if days.contains(&date.weekday()) {
            count += 1;
        }
    }
    count
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
        chrono::LocalResult::None => shifted(rule, local),
    }
}

fn shifted(rule: &Rule, local: NaiveDateTime) -> Option<DateTime<Utc>> {
    for probe in 1..=168u32 {
        let earlier = local.checked_sub_signed(Duration::hours(i64::from(probe)))?;
        match rule.timezone.offset_from_local_datetime(&earlier) {
            chrono::LocalResult::Single(offset) | chrono::LocalResult::Ambiguous(offset, _) => {
                return Some(
                    Utc.from_utc_datetime(&local)
                        - Duration::seconds(i64::from(offset.fix().local_minus_utc())),
                );
            }
            chrono::LocalResult::None => {}
        }
    }
    None
}

fn parse_time(text: &str) -> Result<NaiveTime, &'static str> {
    NaiveTime::parse_from_str(text, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(text, "%H:%M:%S"))
        .map_err(|_| "schedule_time_is_invalid")
}

pub(crate) fn spawn(state: Shared) {
    tokio::spawn(async move {
        if let Err(err) = pass(&state, true).await {
            tracing::error!(error = %err, "schedule startup");
        }
        loop {
            tokio::time::sleep(StdDuration::from_secs(15)).await;
            if let Err(err) = pass(&state, false).await {
                tracing::error!(error = %err, "schedule");
            }
        }
    });
}

async fn pass(state: &Shared, startup: bool) -> AppResult<()> {
    let now = Utc::now();
    let due = {
        let db = state.store();
        db.due_schedules(&format_utc(now))?
    };
    for item in due {
        if let Err(err) = one(state, &item, now, startup).await {
            tracing::error!(schedule = %item.id, error = %err, "schedule");
        }
    }
    Ok(())
}

async fn one(state: &Shared, item: &Schedule, now: DateTime<Utc>, startup: bool) -> AppResult<()> {
    let rule = rule_from(item).map_err(ApiError::bad)?;
    let count = u32::try_from(item.occurrence_count).unwrap_or(0);
    let decided = step_blocking(rule, count, now, startup)
        .await
        .map_err(ApiError::bad)?;
    let next = decided.next_run.map(format_utc);
    let count = i64::from(decided.occurrence_count);
    match decided.action {
        Action::Idle => {
            if item.next_run_at.as_deref() != next.as_deref() || item.occurrence_count != count {
                let db = state.store();
                db.advance_schedule(&item.id, next.as_deref(), count)?;
            }
        }
        Action::Miss(at) => {
            let db = state.store();
            db.advance_schedule(&item.id, next.as_deref(), count)?;
            db.note_miss(&item.id, Some(&format_utc(at)))?;
        }
        Action::Fire(at) => {
            let actor = launch_actor(item);
            let allowed = {
                let db = state.store();
                actor_may_launch(db, &item.team_id, actor)?
            };
            if !allowed {
                tracing::warn!(
                    schedule = %item.id,
                    user = actor,
                    "scheduled run skipped because that user cannot start runs for this team"
                );
            }
            let started = if allowed {
                start(state, item, actor)
            } else {
                Err(ApiError::bad("schedule_needs_a_new_owner"))
            };
            if let Err(err) = &started {
                if allowed {
                    tracing::warn!(schedule = %item.id, error = %err, "schedule did not start");
                }
            }
            let db = state.store();
            db.advance_schedule(&item.id, next.as_deref(), count)?;
            if started.is_err() {
                db.note_miss(&item.id, Some(&format_utc(at)))?;
            }
            let block = if allowed {
                None
            } else {
                Some("schedule_needs_a_new_owner")
            };
            if item.launch_block.as_deref() != block {
                db.set_launch_block(&item.id, block)?;
            }
        }
    }
    Ok(())
}

fn actor_may_launch(
    store: &dyn crate::backend::Backend,
    team_id: &str,
    actor: &str,
) -> AppResult<bool> {
    let Some(user) = store.user_by_username(actor)? else {
        return Ok(false);
    };
    if user.disabled {
        return Ok(false);
    }
    match crate::features::identity::authorize(
        store,
        &user.into(),
        team_id,
        crate::features::identity::Role::Operator,
    ) {
        Ok(()) => Ok(true),
        Err(ApiError::Forbidden) => Ok(false),
        Err(err) => Err(err),
    }
}

/// The user who last saved the schedule. An older row with no editor falls
/// back to the creator, and a blank name stays the synthetic `schedule` actor.
fn launch_actor(item: &Schedule) -> &str {
    let name = if item.updated_by.is_empty() {
        item.created_by.as_str()
    } else {
        item.updated_by.as_str()
    };
    if name.is_empty() {
        "schedule"
    } else {
        name
    }
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
                    extra_vars: Value::Object(serde_json::Map::new()),
                    launched_by: actor.to_string(),
                    trigger: "schedule".to_string(),
                    workflow_job_id: None,
                    workflow_node_id: None,
                    parameters_override: None,
                    overrides: jobs::LaunchOverrides::default(),
                },
            )?;
        }
        "workflow" => {
            jobs::launch_workflow(
                state,
                &item.team_id,
                &item.target_id,
                Value::Object(serde_json::Map::new()),
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
#[path = "tests.rs"]
mod tests;
