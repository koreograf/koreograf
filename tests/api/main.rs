#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod auth;
mod backup;
mod catalog;
mod catalogs;
#[path = "../common/mod.rs"]
mod common;
mod credentials;
mod hooks;
mod inventories;
mod jobs;
mod projects;
mod routes;
mod schedules;
mod settings;
mod sources;
mod teams;
mod templates;
mod web_assets;
mod work;
mod workflows;
