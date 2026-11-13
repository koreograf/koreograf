//! Credential shapes and the files a job receives.
//!
//! Secrets stay in files with mode 0600 or in process environment variables.
//! They are not placed on the command line.

mod model;
mod routes;
mod service;
pub(crate) mod store;

pub(crate) use model::AssignedSet;
#[cfg(test)]
pub(crate) use model::KINDS;
pub(crate) use routes::routes;
pub use service::{askpass, askpass_kind, git_ssh, git_ssh_requested, AskpassKind};
pub(crate) use service::{
    assign_loaded, backfill_netbox_urls, check, is_secret_field, kind_fits_tool, prepare,
    reserved_env_name,
};
