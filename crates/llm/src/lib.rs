//! Model providers behind one trait.
//!
//! A provider turns a [`ChatRequest`] into a stream of [`Delta`]s sent over a channel. History is
//! provider neutral, so the next turn can use a different provider than the last one.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod anthropic;
pub mod http;
pub mod lines;
pub mod ollama;
pub mod openai;
pub mod price;
pub mod registry;
pub mod toolargs;

use async_trait::async_trait;
use tokio::sync::mpsc::Sender;
use workcraft_core::{ChatOptions, Delta, Message, Result, ToolSpec};

pub use registry::{
    ProviderConfig, Registry, is_local_model, model_supports_vision, supports_vision,
};

/// What a provider needs for one turn.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub options: ChatOptions,
    /// Whether the target model accepts image inputs. Non-vision models receive saved paths and
    /// pixel dimensions as plain text instead of inline base64 blocks.
    pub vision: bool,
}

#[async_trait]
pub trait Provider: Send + Sync {
    /// Provider id as used in `ModelRef`: `ollama`, `openai`, `anthropic`.
    fn id(&self) -> &str;

    /// Stream one answer. Deltas go to `tx` in order and the last one is `Delta::Done`.
    /// If the receiver is dropped the request stops quietly (that is how a turn is cancelled).
    async fn chat(&self, req: ChatRequest, tx: Sender<Delta>) -> Result<()>;

    /// Models this provider offers right now. Hosted providers may return a fixed list.
    async fn list_models(&self) -> Result<Vec<String>>;
}
