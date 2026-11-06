//! WorkCraft data model: messages, tools, events, errors and paths.
//!
//! Pure data and small helpers. No network, no async runtime.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod error;
pub mod event;
pub mod message;
pub mod paths;
pub mod team;
pub mod tool;

pub use error::{Error, Result};
pub use event::Event;
pub use message::{
    Attachment, ChatOptions, Delta, Message, ModelRef, Role, StopReason, Usage, format_usd,
};
pub use team::{AgentDef, TaskSpec, TaskStatus, TeamConfig};
pub use tool::{ImagePart, ToolCall, ToolResult, ToolSpec, looks_read_only};

/// A new random identifier, 12 hex characters. Short enough to read in a log, long enough to not collide
/// inside one data folder.
#[must_use]
pub fn new_id() -> String {
    let mut s = uuid::Uuid::new_v4().simple().to_string();
    s.truncate(12);
    s
}

/// Milliseconds since the Unix epoch.
#[must_use]
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Cut `s` to at most `max` bytes on a character boundary, adding a marker when something was removed.
#[must_use]
pub fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let cut = s.get(..end).unwrap_or("");
    format!("{cut}\n[truncated {} bytes]", s.len() - end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_short() {
        let a = new_id();
        let b = new_id();
        assert_eq!(a.len(), 12);
        assert_ne!(a, b);
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let s = "aä".repeat(10);
        let t = truncate_bytes(&s, 4);
        assert!(t.contains("[truncated"));
        assert!(t.starts_with("aä"));
        assert_eq!(truncate_bytes("abc", 10), "abc");
    }
}
