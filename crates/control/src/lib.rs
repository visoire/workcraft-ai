//! The command-id system and the control channel.
//!
//! The Craft apps (PhotoCraft and the others) are driven by stable command ids over an authenticated
//! loopback TCP channel: one JSON line per request, a first `auth` line carrying a 64 hex character token.
//! WorkCraft speaks the same protocol in both directions: its [`server`] lets other tools drive the chat,
//! and its [`client`] drives the Craft apps' own `--control` ports.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod client;
pub mod server;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;
use workcraft_core::{Event, Result};

/// A command the engine understands. The id is stable; tools and scripts depend on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandInfo {
    pub id: String,
    pub title: String,
    pub description: String,
    /// JSON Schema of `params`.
    pub params: Value,
}

/// Everything the engine offers, as one object. The UI, the CLI, the control channel and the MCP server
/// all go through this trait, so no action exists in only one of them.
#[async_trait]
pub trait CommandBus: Send + Sync {
    fn commands(&self) -> Vec<CommandInfo>;
    async fn execute(&self, id: &str, params: Value) -> Result<Value>;
    fn subscribe(&self) -> broadcast::Receiver<Event>;
}

/// A fresh 64 hex character token (two random UUIDs).
#[must_use]
pub fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Compare without stopping at the first difference.
#[must_use]
pub fn tokens_equal(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        let t = new_token();
        assert_eq!(t.len(), 64);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(tokens_equal(&t, &t.clone()));
        assert!(!tokens_equal(&t, &new_token()));
        assert!(!tokens_equal("abc", "abcd"));
        assert!(!tokens_equal("", "x"));
    }
}
