use serde::{Deserialize, Serialize};

use crate::tool::ToolCall;
use crate::{new_id, now_ms};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// Which provider and model answers. Written `provider:model`, for example `ollama:qwen3:4b`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelRef {
    pub provider: String,
    pub model: String,
}

impl ModelRef {
    #[must_use]
    pub fn new(provider: &str, model: &str) -> Self {
        Self {
            provider: provider.to_string(),
            model: model.to_string(),
        }
    }

    /// Parse `provider:model`. A bare model name means Ollama, the local default. Provider ids are the
    /// known ones below, or start with `openai-` or `custom-` (endpoints added in the settings).
    #[must_use]
    pub fn parse(s: &str) -> Self {
        const KNOWN: [&str; 13] = [
            "ollama",
            "openai",
            "anthropic",
            "google",
            "gemini",
            "deepseek",
            "xai",
            "zai",
            "mistral",
            "groq",
            "openrouter",
            "lmstudio",
            "together",
        ];
        if let Some((p, m)) = s.split_once(':')
            && (KNOWN.contains(&p) || p.starts_with("openai-") || p.starts_with("custom-"))
        {
            return Self::new(p, m);
        }
        Self::new("ollama", s)
    }
}

impl std::fmt::Display for ModelRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.provider, self.model)
    }
}

/// A file or image attached to a message. `data` is base64.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub data: String,
}

/// One message of a conversation. `parent` makes a tree: a branch is a message whose parent already
/// has another child.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub role: Role,
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// For `Role::Tool`: the call this answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelRef>,
    /// Which agent wrote it (team mode). Empty in single mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub ts: i64,
}

impl Message {
    fn base(role: Role, content: String) -> Self {
        Self {
            id: new_id(),
            parent: None,
            role,
            content,
            reasoning: None,
            tool_calls: Vec::new(),
            tool_call_id: None,
            attachments: Vec::new(),
            model: None,
            agent: None,
            ts: now_ms(),
        }
    }

    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self::base(Role::System, content.into())
    }

    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self::base(Role::User, content.into())
    }

    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::base(Role::Assistant, content.into())
    }

    #[must_use]
    pub fn tool(call_id: &str, content: impl Into<String>) -> Self {
        let mut m = Self::base(Role::Tool, content.into());
        m.tool_call_id = Some(call_id.to_string());
        m
    }

    #[must_use]
    pub fn with_parent(mut self, parent: Option<&str>) -> Self {
        self.parent = parent.map(str::to_string);
        self
    }
}

/// Token counts of one or more model calls.
///
/// `input_tokens` are the prompt tokens billed at the full input rate, so cache hits and cache writes are
/// counted separately and never twice. The prompt the model actually read is [`Usage::prompt_tokens`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Prompt tokens served from the provider's cache (cheaper).
    #[serde(default)]
    pub cache_read_tokens: u64,
    /// Prompt tokens written to the cache (Anthropic bills these at a premium).
    #[serde(default)]
    pub cache_write_tokens: u64,
    /// Output tokens spent on reasoning, when the provider reports them (already inside `output_tokens`).
    #[serde(default)]
    pub reasoning_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, other: Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_write_tokens += other.cache_write_tokens;
        self.reasoning_tokens += other.reasoning_tokens;
    }

    /// Everything the model read: uncached input plus cache reads and writes.
    #[must_use]
    pub fn prompt_tokens(&self) -> u64 {
        self.input_tokens + self.cache_read_tokens + self.cache_write_tokens
    }

    /// One line for people: `1840 in (72% cached), 95 out, $0.0012`. The cost is left out when it is unknown.
    #[must_use]
    pub fn summary(&self, cost_usd: Option<f64>) -> String {
        let mut s = format!("{} in", self.prompt_tokens());
        if self.cache_read_tokens > 0 {
            s.push_str(&format!(" ({:.0}% cached)", self.cache_hit_ratio() * 100.0));
        }
        s.push_str(&format!(", {} out", self.output_tokens));
        if self.reasoning_tokens > 0 {
            s.push_str(&format!(" ({} reasoning)", self.reasoning_tokens));
        }
        if let Some(c) = cost_usd {
            s.push_str(", ");
            s.push_str(&format_usd(c));
        }
        s
    }

    /// Share of the prompt served from cache, 0.0 to 1.0.
    #[must_use]
    pub fn cache_hit_ratio(&self) -> f64 {
        let p = self.prompt_tokens();
        if p == 0 {
            0.0
        } else {
            self.cache_read_tokens as f64 / p as f64
        }
    }
}

/// Dollars as a short string: `$0.0012`, `$1.20`, `< $0.0001`.
#[must_use]
pub fn format_usd(usd: f64) -> String {
    if usd <= 0.0 {
        "$0".to_string()
    } else if usd < 0.0001 {
        "< $0.0001".to_string()
    } else if usd < 1.0 {
        format!("${usd:.4}")
    } else {
        format!("${usd:.2}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Stop,
    ToolUse,
    Length,
    Other,
}

/// One streamed piece of an answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Delta {
    Text(String),
    Reasoning(String),
    ToolCall(ToolCall),
    Usage(Usage),
    /// Something the person should know while waiting: a retry after a rate limit, a slow start.
    Notice(String),
    Done(StopReason),
}

/// Per-request knobs. `None` means the provider default.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatOptions {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Context window for local models. `None` lets the adapter decide.
    pub num_ctx: Option<u32>,
    /// Reasoning on or off, where the model supports it.
    pub think: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_ref_parses_provider_prefix() {
        assert_eq!(
            ModelRef::parse("anthropic:claude-sonnet-5-5").provider,
            "anthropic"
        );
        assert_eq!(
            ModelRef::parse("deepseek:deepseek-chat").provider,
            "deepseek"
        );
        assert_eq!(ModelRef::parse("custom-my-box:llama").model, "llama");
        let m = ModelRef::parse("qwen3:4b");
        assert_eq!(
            (m.provider.as_str(), m.model.as_str()),
            ("ollama", "qwen3:4b")
        );
        assert_eq!(ModelRef::parse("ollama:qwen3:4b").model, "qwen3:4b");
        assert_eq!(
            ModelRef::parse("ollama:qwen3:4b").to_string(),
            "ollama:qwen3:4b"
        );
    }

    #[test]
    fn message_round_trips_and_skips_empty_fields() {
        let m = Message::user("hi").with_parent(Some("abc"));
        let s = serde_json::to_string(&m).unwrap_or_default();
        assert!(!s.contains("tool_calls"));
        let back: Message = serde_json::from_str(&s).unwrap();
        assert_eq!(back, m);
    }
}
