//! Local-time schedules for templates and workflows.
//!
//! A schedule stores a clock time and a time zone. The next run is the next
//! moment that clock time exists. A time that falls in a daylight-saving gap
//! is skipped. A repeated local time, when the clocks go back, is used once,
//! at the earlier offset. Hourly schedules step by one hour of absolute time.
//! Daily and weekly schedules step by the calendar, not by walking each day.

use std::time::Duration as StdDuration;

use chrono::{
    DateTime, Datelike, Duration, Months, NaiveDate, NaiveTime, Offset, TimeZone, Utc, Weekday,
};
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

/// A start more than one calendar year before today in `timezone` is rejected.
/// Stored schedules keep their original start and are not checked again.
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
    let gaps = i64::try_from(gap_dates(rule, rule.start_date, local).len()).unwrap_or(0);
    let guess = u32::try_from(calendar.saturating_sub(gaps)).unwrap_or(u32::MAX);
    Ok(guess.saturating_sub(1))
}

fn weekly_guess(rule: &Rule, not_before: DateTime<Utc>) -> Result<u32, &'static str> {
    let local = not_before.with_timezone(&rule.timezone).date_naive();
    if local <= rule.start_date {
        return Ok(0);
    }
    let before = count_selected(rule.start_date, local, &rule.weekdays);
    let gaps = i64::try_from(
        gap_dates(rule, rule.start_date, local)
            .into_iter()
            .filter(|day| rule.weekdays.contains(&day.weekday()))
            .count(),
    )
    .unwrap_or(0);
    let guess = u32::try_from(before.saturating_sub(gaps)).unwrap_or(u32::MAX);
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
    let mut offset = i64::from(index);
    for _ in 0..6 {
        let Some(date) = rule.start_date.checked_add_signed(Duration::days(offset)) else {
            return Err("schedule_has_no_occurrence");
        };
        let gaps = i64::try_from(gaps_through(rule, date)).unwrap_or(i64::MAX);
        let next = i64::from(index).saturating_add(gaps);
        if next == offset {
            let Some(at) = resolve(rule, date, rule.start_time) else {
                return Err("schedule_has_no_occurrence");
            };
            return Ok(kept(rule, index, at));
        }
        offset = next;
    }
    Err("schedule_has_no_occurrence")
}

fn nth_weekly(rule: &Rule, index: u32) -> Result<Option<DateTime<Utc>>, &'static str> {
    let mut raw = index;
    for _ in 0..6 {
        let Some(date) = weekly_date(rule.start_date, &rule.weekdays, raw) else {
            return Err("schedule_has_no_occurrence");
        };
        let gaps = selected_gaps_through(rule, date);
        let Some(next) = index.checked_add(gaps) else {
            return Err("schedule_has_no_occurrence");
        };
        if next == raw {
            let Some(at) = resolve(rule, date, rule.start_time) else {
                return Err("schedule_has_no_occurrence");
            };
            return Ok(kept(rule, index, at));
        }
        raw = next;
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

fn gaps_through(rule: &Rule, date: NaiveDate) -> usize {
    let Some(until) = date.checked_add_signed(Duration::days(1)) else {
        return gap_dates(rule, rule.start_date, date).len();
    };
    gap_dates(rule, rule.start_date, until).len()
}

fn selected_gaps_through(rule: &Rule, date: NaiveDate) -> u32 {
    let Some(until) = date.checked_add_signed(Duration::days(1)) else {
        return 0;
    };
    u32::try_from(
        gap_dates(rule, rule.start_date, until)
            .into_iter()
            .filter(|day| rule.weekdays.contains(&day.weekday()))
            .count(),
    )
    .unwrap_or(u32::MAX)
}

/// Local days in `[from, until)` whose clock time does not exist.
/// Only months whose noon offset changes are scanned.
fn gap_dates(rule: &Rule, from: NaiveDate, until: NaiveDate) -> Vec<NaiveDate> {
    if until <= from {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut year = from.year();
    let last_year = until.year();
    while year <= last_year {
        gaps_in_year(rule, year, from, until, &mut found);
        if year == i32::MAX {
            break;
        }
        year += 1;
    }
    found
}

fn gaps_in_year(
    rule: &Rule,
    year: i32,
    from: NaiveDate,
    until: NaiveDate,
    found: &mut Vec<NaiveDate>,
) {
    for month in 1..=12 {
        let Some(first) = NaiveDate::from_ymd_opt(year, month, 1) else {
            continue;
        };
        let next = if month == 12 {
            NaiveDate::from_ymd_opt(year.saturating_add(1), 1, 1)
        } else {
            NaiveDate::from_ymd_opt(year, month + 1, 1)
        };
        let Some(next) = next else {
            continue;
        };
        if next <= from || first >= until {
            continue;
        }
        let left = noon_offset(rule.timezone, first);
        let right = noon_offset(rule.timezone, next);
        let changed = match (left, right) {
            (Some(before), Some(after)) => before != after,
            _ => true,
        };
        if !changed {
            continue;
        }
        let mut day = first;
        while day < next {
            note_gap(rule, day, from, until, found);
            let Some(following) = day.checked_add_signed(Duration::days(1)) else {
                break;
            };
            day = following;
        }
        note_gap(rule, next, from, until, found);
    }
}

fn note_gap(
    rule: &Rule,
    day: NaiveDate,
    from: NaiveDate,
    until: NaiveDate,
    found: &mut Vec<NaiveDate>,
) {
    if day >= from
        && day < until
        && resolve(rule, day, rule.start_time).is_none()
        && !found.contains(&day)
    {
        found.push(day);
    }
}

fn noon_offset(timezone: Tz, date: NaiveDate) -> Option<i32> {
    let noon = NaiveTime::from_hms_opt(12, 0, 0)?;
    match timezone.offset_from_local_datetime(&date.and_time(noon)) {
        chrono::LocalResult::Single(offset) | chrono::LocalResult::Ambiguous(offset, _) => {
            Some(offset.fix().local_minus_utc())
        }
        chrono::LocalResult::None => None,
    }
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
        let db = state.db()?;
        db::due_schedules(&db, &format_utc(now))?
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
            db::insert_user(
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
            db::insert_template(
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
                    entrypoint: String::new(),
                    extra_vars: "{}".into(),
                    spec: json!({"script": ":"}).to_string(),
                    created_at: db::now(),
                    update_on_launch: false,
                    admin_saved: true,
                },
            )
            .expect("template");
            let now = db::now();
            db::insert_schedule(
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
                    created_at: now.clone(),
                    updated_at: now,
                },
            )
            .expect("schedule");
            id
        };
        let item = {
            let db = state.db().expect("db");
            db::schedule(&db, "operator-schedule")
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
                 SET occurrence_count = 0, missed_at = NULL, created_by = 'admin'
                 WHERE id = 'operator-schedule'",
                [],
            )
            .expect("reset");
        }
        let item = {
            let db = state.db().expect("db");
            db::schedule(&db, "operator-schedule")
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
            job_timeout_secs: 60,
            local_jobs: crate::config::LocalJobs::Admins,
            project_roots: vec![
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
            ],
            hsts: false,
            cookie_secure: crate::config::CookieSecure::Auto,
            trusted_proxy: false,
            ldap: None,
        }
    }
}
