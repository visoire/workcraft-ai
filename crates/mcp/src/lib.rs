//! Model Context Protocol for WorkCraft.
//!
//! JSON-RPC 2.0 written by hand, like the ArtCraft MCP server: one small implementation we can read and
//! test, no macro framework. The client talks to Craft app servers (`photocraft-cli mcp`) over stdio or
//! Streamable HTTP. The server side exposes any [`server::ToolHandler`] over stdio.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod client;
pub mod config;
pub mod hub;
pub mod jsonrpc;
pub mod server;

pub use client::McpClient;
pub use config::{McpConfig, ServerEntry};
pub use hub::McpHub;
pub use server::ToolHandler;
