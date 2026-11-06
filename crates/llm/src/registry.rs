//! The set of providers an engine can use, built from a small config.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use workcraft_core::{Error, ModelRef, Result};

use crate::Provider;
use crate::anthropic::{self, Anthropic};
use crate::ollama::{self, Ollama};
use crate::openai::OpenAiCompat;

/// One configured provider. The key is `api_key` (saved in `config.json` from the settings screen) or
/// else the environment variable named in `key_env`. Keys never go into session files, and the engine
/// masks them in everything it returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    /// `ollama`, `openai` (any compatible server) or `anthropic`.
    pub kind: String,
    /// Display name, for example "DeepSeek".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Optional explicit override for whether models on this provider accept vision inputs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision: Option<bool>,
}

impl ProviderConfig {
    /// The key to use: the saved one, else the environment variable.
    #[must_use]
    pub fn key(&self) -> Option<String> {
        self.api_key
            .clone()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| {
                self.key_env
                    .as_deref()
                    .and_then(|v| std::env::var(v).ok())
                    .filter(|k| !k.trim().is_empty())
            })
    }
}

/// Whether `(provider, model)` natively accepts image inputs.
#[must_use]
pub fn model_supports_vision(provider: &str, model: &str) -> bool {
    let p = provider.to_ascii_lowercase();
    let m = model.to_ascii_lowercase();
    // Strip optional routing prefix like `openai/gpt-4o` on OpenRouter.
    let leaf = m.rsplit('/').next().unwrap_or(&m);

    if p == "anthropic" || m.starts_with("anthropic/") {
        return leaf.starts_with("claude-3")
            || leaf.starts_with("claude-4")
            || leaf.starts_with("claude-sonnet")
            || leaf.starts_with("claude-opus")
            || leaf.starts_with("claude-haiku");
    }
    if p == "google" || p == "gemini" || leaf.starts_with("gemini-") {
        return true;
    }
    if leaf.starts_with("gpt-4o")
        || leaf.starts_with("chatgpt-4o")
        || leaf.starts_with("gpt-4.1")
        || leaf.starts_with("gpt-4.5")
        || leaf.starts_with("gpt-5")
        || leaf.starts_with("o4")
        || (leaf.starts_with("o1")
            && !leaf.starts_with("o1-mini")
            && !leaf.starts_with("o1-preview"))
        || (leaf.starts_with("o3") && !leaf.starts_with("o3-mini"))
    {
        return true;
    }
    if leaf.starts_with("pixtral")
        || leaf.starts_with("mistral-small-3")
        || leaf.starts_with("mistral-medium-3")
        || leaf.contains("vision")
        || leaf.starts_with("deepseek-vl")
        || leaf.starts_with("grok-2-vision")
        || leaf.starts_with("grok-3")
        || leaf.starts_with("grok-4")
    {
        return true;
    }
    // Local / Ollama multimodal families (e.g. qwen3.5, qwen2.5vl, qwen3-vl, llava, gemma3/4, minicpm-v, moondream).
    leaf.starts_with("qwen3.5")
        || leaf.starts_with("qwen2.5vl")
        || leaf.starts_with("qwen2-vl")
        || leaf.starts_with("qwen3-vl")
        || leaf.contains("-vl")
        || leaf.contains(":vl")
        || leaf.starts_with("llava")
        || leaf.starts_with("bakllava")
        || leaf.starts_with("gemma3")
        || leaf.starts_with("gemma4")
        || leaf.starts_with("minicpm-v")
        || leaf.starts_with("moondream")
}

/// Resolve vision support for `model_ref`, honoring any explicit `ProviderConfig.vision` override.
#[must_use]
pub fn supports_vision(model_ref: &ModelRef, configs: &[ProviderConfig]) -> bool {
    if let Some(cfg) = configs.iter().find(|c| c.id == model_ref.provider)
        && let Some(explicit) = cfg.vision
    {
        return explicit;
    }
    model_supports_vision(&model_ref.provider, &model_ref.model)
}

/// True for local runtimes (Ollama, LM Studio, or localhost endpoints) so tool budgets stay compact.
#[must_use]
pub fn is_local_model(model_ref: &ModelRef, configs: &[ProviderConfig]) -> bool {
    let p = model_ref.provider.to_ascii_lowercase();
    if p == "ollama" || p == "lmstudio" {
        return true;
    }
    configs
        .iter()
        .find(|c| c.id == model_ref.provider)
        .is_some_and(|c| {
            c.kind == "ollama"
                || c.base_url.as_deref().is_some_and(|u| {
                    u.contains("127.0.0.1") || u.contains("localhost") || u.contains("[::1]")
                })
        })
}

/// `sk-...wxyz`: enough to recognise a key, not enough to use it.
#[must_use]
pub fn key_hint(key: &str) -> String {
    let k: Vec<char> = key.trim().chars().collect();
    if k.len() <= 8 {
        return "\u{2022}\u{2022}\u{2022}\u{2022}".to_string();
    }
    let head: String = k[..3].iter().collect();
    let tail: String = k[k.len() - 4..].iter().collect();
    format!("{head}\u{2026}{tail}")
}

#[derive(Default, Clone)]
pub struct Registry {
    providers: HashMap<String, Arc<dyn Provider>>,
}

impl Registry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ollama (local), plus OpenAI and Anthropic when their key variables are set.
    pub fn from_env() -> Result<Self> {
        let mut r = Self::new();
        r.add(Arc::new(Ollama::from_env()?));
        if let Ok(k) = std::env::var("OPENAI_API_KEY") {
            r.add(Arc::new(OpenAiCompat::new(
                "openai",
                "https://api.openai.com/v1",
                Some(k),
            )?));
        }
        if let Ok(k) = std::env::var("ANTHROPIC_API_KEY") {
            r.add(Arc::new(Anthropic::new(anthropic::DEFAULT_URL, &k)?));
        }
        Ok(r)
    }

    /// The environment providers plus every configured one. A broken entry is left out and reported,
    /// so one typo does not take the other providers down.
    pub fn build(configs: &[ProviderConfig]) -> Result<(Self, Vec<String>)> {
        let mut r = Self::from_env()?;
        let mut warnings = Vec::new();
        for c in configs {
            match Self::from_configs(std::slice::from_ref(c)) {
                Ok(one) => r.merge(one),
                Err(e) => warnings.push(format!("provider '{}': {e}", c.id)),
            }
        }
        Ok((r, warnings))
    }

    pub fn from_configs(configs: &[ProviderConfig]) -> Result<Self> {
        let mut r = Self::new();
        for c in configs {
            let key = c.key();
            let p: Arc<dyn Provider> = match c.kind.as_str() {
                "ollama" => Arc::new(Ollama::new(
                    c.base_url.as_deref().unwrap_or(ollama::DEFAULT_URL),
                )?),
                "openai" => Arc::new(OpenAiCompat::new(
                    &c.id,
                    c.base_url.as_deref().unwrap_or("https://api.openai.com/v1"),
                    key,
                )?),
                "anthropic" => Arc::new(Anthropic::new(
                    c.base_url.as_deref().unwrap_or(anthropic::DEFAULT_URL),
                    &key.unwrap_or_default(),
                )?),
                other => return Err(Error::Invalid(format!("unknown provider kind '{other}'"))),
            };
            r.providers.insert(c.id.clone(), p);
        }
        Ok(r)
    }

    /// Take every provider of `other`; ids already present are replaced.
    pub fn merge(&mut self, other: Registry) {
        self.providers.extend(other.providers);
    }

    pub fn add(&mut self, p: Arc<dyn Provider>) {
        self.providers.insert(p.id().to_string(), p);
    }

    pub fn get(&self, id: &str) -> Result<Arc<dyn Provider>> {
        self.providers
            .get(id)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("provider '{id}' is not configured")))
    }

    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        let mut v: Vec<String> = self.providers.keys().cloned().collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_provider_is_an_error() {
        let r = Registry::new();
        assert!(r.get("nope").is_err());
        let bad = ProviderConfig {
            id: "x".into(),
            kind: "wat".into(),
            name: None,
            base_url: None,
            key_env: None,
            api_key: None,
            vision: None,
        };
        assert!(Registry::from_configs(std::slice::from_ref(&bad)).is_err());
        // build() keeps going past a bad entry.
        let good = ProviderConfig {
            id: "lmstudio".into(),
            kind: "openai".into(),
            name: None,
            base_url: Some("http://127.0.0.1:1234/v1".into()),
            key_env: None,
            api_key: None,
            vision: None,
        };
        let (r, warn) = Registry::build(&[bad, good]).unwrap();
        assert!(r.ids().contains(&"lmstudio".to_string()));
        assert_eq!(warn.len(), 1);
    }

    #[test]
    fn saved_key_wins_and_hints_hide_the_key() {
        let c = ProviderConfig {
            id: "deepseek".into(),
            kind: "openai".into(),
            name: None,
            base_url: None,
            key_env: Some("WC_TEST_SURELY_UNSET_VAR".into()),
            api_key: Some("sk-1234567890abcd".into()),
            vision: None,
        };
        assert_eq!(c.key().as_deref(), Some("sk-1234567890abcd"));
        assert_eq!(key_hint("sk-1234567890abcd"), "sk-\u{2026}abcd");
        assert!(!key_hint("short").contains('s'));
        let none = ProviderConfig {
            api_key: Some("  ".into()),
            ..c
        };
        assert!(none.key().is_none());
    }

    #[test]
    fn builds_from_configs() {
        let cfg = vec![
            ProviderConfig {
                id: "ollama".into(),
                kind: "ollama".into(),
                name: None,
                base_url: None,
                key_env: None,
                api_key: None,
                vision: None,
            },
            ProviderConfig {
                id: "deepseek".into(),
                kind: "openai".into(),
                name: None,
                base_url: Some("https://api.deepseek.com/v1".into()),
                key_env: Some("DEEPSEEK_API_KEY".into()),
                api_key: None,
                vision: None,
            },
        ];
        let r = Registry::from_configs(&cfg).unwrap();
        assert_eq!(r.ids(), vec!["deepseek", "ollama"]);
    }

    #[test]
    fn vision_and_locality_detection() {
        assert!(model_supports_vision("anthropic", "claude-sonnet-4-5"));
        assert!(model_supports_vision("openai", "gpt-4o-mini"));
        assert!(model_supports_vision("ollama", "qwen3.5:4b"));
        assert!(model_supports_vision("ollama", "llava:7b"));
        assert!(!model_supports_vision("ollama", "qwen3:4b"));
        assert!(!model_supports_vision("deepseek", "deepseek-chat"));

        let cfg = ProviderConfig {
            id: "custom-vlm".into(),
            kind: "openai".into(),
            name: None,
            base_url: Some("http://127.0.0.1:8080/v1".into()),
            key_env: None,
            api_key: None,
            vision: Some(true),
        };
        let mr = ModelRef::new("custom-vlm", "my-model");
        assert!(supports_vision(&mr, std::slice::from_ref(&cfg)));
        assert!(is_local_model(&mr, std::slice::from_ref(&cfg)));
        assert!(!is_local_model(
            &ModelRef::new("anthropic", "claude-sonnet-4-5"),
            &[]
        ));
    }
}
