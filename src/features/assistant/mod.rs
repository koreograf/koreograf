//! The assistant chat, its tool calls and redaction, and the MCP bridge.

mod model;
mod routes;
mod service;

#[cfg(test)]
mod tests;

pub use model::{
    ChatBody, ChatMessage, Error, JobQuery, LlmConfig, LlmConfigError, LlmError, LlmSetup,
    PendingActionResponse,
};
pub(crate) use routes::routes;
pub use service::{complete, config_for, from_env, parse, requested, run, Client, Server};
