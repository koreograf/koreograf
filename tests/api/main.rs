#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod auth;
mod backup;
mod catalogs;
#[path = "../common/mod.rs"]
mod common;
mod credentials;
mod hooks;
mod inventories;
mod jobs;
mod projects;
mod schedules;
mod settings;
mod teams;
mod templates;
mod web_source;
mod workflows;
