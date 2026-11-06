//! The WorkCraft engine: tools, the agent loop, skills, team orchestration and the command bus.
//!
//! [`Engine`] is the one object the UI, the CLI, the control channel and the MCP server all talk to.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod agent;
pub mod checkpoints;
pub mod config;
pub mod context;
pub mod craft;
pub mod docs;
pub mod engine;
pub mod export;
pub mod mcp_server;
pub mod prompt;
pub mod shared;
pub mod skills;
pub mod team;
pub mod tools;

pub use engine::{Engine, EngineOptions};
pub use mcp_server::WorkCraftMcp;
