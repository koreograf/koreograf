use chrono::{DateTime, NaiveDate, NaiveTime, Utc, Weekday};
use chrono_tz::Tz;

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
