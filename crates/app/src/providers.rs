//! The providers the settings screen offers, as in the WorkCraft workspace: local Ollama and LM Studio,
//! the big cloud APIs, and any OpenAI-compatible endpoint. Every cloud API below speaks the OpenAI
//! Chat Completions format except Anthropic, which has its own adapter.

pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    /// Engine adapter: `openai`, `anthropic` or `ollama`.
    pub kind: &'static str,
    pub base_url: &'static str,
    /// Environment variable that can hold the key instead of the settings.
    pub key_env: &'static str,
    pub local: bool,
    /// Where to get a key.
    pub docs: &'static str,
    pub blurb: &'static str,
}

pub const PRESETS: [Preset; 12] = [
    Preset {
        id: "ollama",
        name: "Ollama",
        kind: "ollama",
        base_url: "http://127.0.0.1:11434",
        key_env: "",
        local: true,
        docs: "ollama.com",
        blurb: "Models on this computer. Private, works offline.",
    },
    Preset {
        id: "lmstudio",
        name: "LM Studio",
        kind: "openai",
        base_url: "http://127.0.0.1:1234/v1",
        key_env: "",
        local: true,
        docs: "lmstudio.ai",
        blurb: "Local models served by LM Studio.",
    },
    Preset {
        id: "anthropic",
        name: "Claude",
        kind: "anthropic",
        base_url: "https://api.anthropic.com",
        key_env: "ANTHROPIC_API_KEY",
        local: false,
        docs: "console.anthropic.com",
        blurb: "Anthropic. Strong at writing, code and careful tool use.",
    },
    Preset {
        id: "openai",
        name: "ChatGPT",
        kind: "openai",
        base_url: "https://api.openai.com/v1",
        key_env: "OPENAI_API_KEY",
        local: false,
        docs: "platform.openai.com",
        blurb: "OpenAI GPT models.",
    },
    Preset {
        id: "gemini",
        name: "Gemini",
        kind: "openai",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        key_env: "GEMINI_API_KEY",
        local: false,
        docs: "aistudio.google.com",
        blurb: "Google. Long context, through its OpenAI-compatible endpoint.",
    },
    Preset {
        id: "deepseek",
        name: "DeepSeek",
        kind: "openai",
        base_url: "https://api.deepseek.com/v1",
        key_env: "DEEPSEEK_API_KEY",
        local: false,
        docs: "platform.deepseek.com",
        blurb: "Low cost chat and reasoning models.",
    },
    Preset {
        id: "xai",
        name: "Grok",
        kind: "openai",
        base_url: "https://api.x.ai/v1",
        key_env: "XAI_API_KEY",
        local: false,
        docs: "console.x.ai",
        blurb: "xAI Grok models.",
    },
    Preset {
        id: "zai",
        name: "GLM",
        kind: "openai",
        base_url: "https://api.z.ai/api/paas/v4",
        key_env: "ZAI_API_KEY",
        local: false,
        docs: "z.ai",
        blurb: "Z.ai GLM models with a long context.",
    },
    Preset {
        id: "mistral",
        name: "Mistral",
        kind: "openai",
        base_url: "https://api.mistral.ai/v1",
        key_env: "MISTRAL_API_KEY",
        local: false,
        docs: "console.mistral.ai",
        blurb: "Mistral AI models.",
    },
    Preset {
        id: "groq",
        name: "Groq",
        kind: "openai",
        base_url: "https://api.groq.com/openai/v1",
        key_env: "GROQ_API_KEY",
        local: false,
        docs: "console.groq.com",
        blurb: "Very fast open models.",
    },
    Preset {
        id: "openrouter",
        name: "OpenRouter",
        kind: "openai",
        base_url: "https://openrouter.ai/api/v1",
        key_env: "OPENROUTER_API_KEY",
        local: false,
        docs: "openrouter.ai",
        blurb: "Many providers behind one key.",
    },
    Preset {
        id: "together",
        name: "Together AI",
        kind: "openai",
        base_url: "https://api.together.xyz/v1",
        key_env: "TOGETHER_API_KEY",
        local: false,
        docs: "api.together.ai",
        blurb: "Open models in the cloud.",
    },
];

#[must_use]
pub fn preset(id: &str) -> Option<&'static Preset> {
    let id = if id == "google" { "gemini" } else { id };
    PRESETS.iter().find(|p| p.id == id)
}

/// Display name of a provider id.
#[must_use]
pub fn label(id: &str, saved_name: Option<&str>) -> String {
    if let Some(n) = saved_name.filter(|n| !n.trim().is_empty()) {
        return n.to_string();
    }
    if let Some(p) = preset(id) {
        return p.name.to_string();
    }
    let raw = id.strip_prefix("custom-").unwrap_or(id).replace('-', " ");
    let mut c = raw.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

/// A provider error in plain words: "not reachable" instead of a transport message.
#[must_use]
pub fn friendly_error(id: &str, err: &str) -> String {
    let unreachable = [
        "error sending request",
        "connection refused",
        "dns error",
        "timed out",
    ];
    if unreachable.iter().any(|u| err.to_lowercase().contains(u)) {
        return if is_local(id) {
            format!("Not running. Start {} and check again.", label(id, None))
        } else {
            "Not reachable. Check the address and your internet connection.".to_string()
        };
    }
    err.strip_prefix("provider error: ")
        .unwrap_or(err)
        .to_string()
}

#[must_use]
pub fn is_local(id: &str) -> bool {
    preset(id).is_some_and(|p| p.local)
}

/// `custom-` plus a slug of the name, for an endpoint the user adds.
#[must_use]
pub fn custom_id(name: &str) -> String {
    let slug: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug: String = slug.chars().take(30).collect();
    format!(
        "custom-{}",
        if slug.is_empty() { "endpoint" } else { &slug }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_unique_and_labels_read_well() {
        let mut ids: Vec<&str> = PRESETS.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), PRESETS.len());
        assert_eq!(label("google", None), "Gemini");
        assert_eq!(label("custom-my-lab", None), "My lab");
        assert_eq!(label("deepseek", Some("DS")), "DS");
        assert!(is_local("ollama") && !is_local("openai"));
        for p in &PRESETS {
            // Every preset id parses as a provider in a model id.
            let m = workcraft_core::ModelRef::parse(&format!("{}:x", p.id));
            assert_eq!(m.provider, p.id);
        }
    }

    #[test]
    fn errors_read_well() {
        let e = "http error: error sending request for url (http://127.0.0.1:1234/v1/models)";
        assert_eq!(
            friendly_error("lmstudio", e),
            "Not running. Start LM Studio and check again."
        );
        assert!(friendly_error("openai", e).starts_with("Not reachable"));
        assert_eq!(
            friendly_error("openai", "provider error: bad key"),
            "bad key"
        );
    }

    #[test]
    fn custom_ids_are_slugs() {
        assert_eq!(custom_id("My Lab Server!"), "custom-my-lab-server");
        assert_eq!(custom_id("  "), "custom-endpoint");
    }
}
