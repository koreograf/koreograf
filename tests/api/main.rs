#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod auth;
mod backup;
mod cache;
mod catalog;
mod catalogs;
#[path = "../common/mod.rs"]
mod common;
mod concurrency;
mod credentials;
mod hooks;
mod inventories;
mod jobs;
mod projects;
mod retention;
mod routes;
mod schedules;
mod service_accounts;
mod settings;
mod sources;
mod teams;
mod templates;
mod web_assets;
mod work;
mod workflows;
