//! Keeping a request inside the model's context window.
//!
//! The stored chat is never changed. Before each model call the history is copied and, only when the whole
//! request (system prompt, tool definitions, messages) would not fit, shortened in steps that keep the
//! newest turn whole and keep every tool call next to its result:
//!
//! 1. old tool results are cut, old screenshots and attachments are dropped;
//! 2. old results are cut much harder and old answers are shortened;
//! 3. whole old turns are dropped, oldest first, with a note saying so.
//!
//! The same shortening is the first level of the token economy: even when it fits, a long tool result from
//! ten steps ago is not sent again in full.

use workcraft_core::{Message, Role, truncate_bytes};

/// Messages at the end that are never shortened (the current step and a few before it).
pub const KEEP_RECENT: usize = 8;
/// A tool result older than [`KEEP_RECENT`] keeps this many bytes at level 0.
pub const OLD_RESULT_BYTES: usize = 2000;
/// Characters per token for the estimate. Code and JSON run lower than prose, so this errs high.
const CHARS_PER_TOKEN: f64 = 3.4;
/// Room kept free for the answer.
pub const DEFAULT_ANSWER_TOKENS: usize = 4096;
/// A hosted conversation is trimmed above this size even when the window is larger, which keeps cost and
/// latency in check on million-token models.
pub const CLOUD_WORKING_TOKENS: usize = 200_000;
/// Ollama starts models with 4,096 tokens, which silently cuts off the start of the prompt (the system
/// prompt and the tools). This is used when the settings do not say otherwise.
pub const OLLAMA_DEFAULT_CTX: usize = workcraft_llm::ollama::DEFAULT_NUM_CTX as usize;

/// How much had to be given up to fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Shortened {
    /// Nothing beyond the routine cut of old tool results.
    None,
    /// Old results were cut harder and old answers shortened.
    Hard,
    /// Whole old turns were left out.
    Dropped(usize),
}

/// Estimated tokens of a piece of text.
#[must_use]
pub fn estimate_tokens(chars: usize) -> usize {
    (chars as f64 / CHARS_PER_TOKEN).ceil() as usize
}

/// Estimated tokens of a message list.
#[must_use]
pub fn estimate_messages(messages: &[Message]) -> usize {
    estimate_tokens(messages.iter().map(message_chars).sum())
}

fn message_chars(m: &Message) -> usize {
    let calls: usize = m
        .tool_calls
        .iter()
        .map(|c| c.name.len() + c.arguments.to_string().len() + 16)
        .sum();
    // An image costs a few hundred tokens whatever its bytes; count it as ~1,200 characters.
    let images = m
        .attachments
        .iter()
        .filter(|a| a.mime.starts_with("image/"))
        .count()
        * 1200;
    m.content.len() + m.reasoning.as_ref().map_or(0, String::len) + calls + images + 12
}

/// The context window to plan with, in tokens.
///
/// `configured` is the number from the settings (`num_ctx` for Ollama). Hosted models use their real window
/// capped at [`CLOUD_WORKING_TOKENS`].
#[must_use]
pub fn window_for(provider_kind: &str, model: &str, configured: Option<u32>) -> usize {
    if let Some(n) = configured {
        return n as usize;
    }
    if provider_kind == "ollama" {
        return OLLAMA_DEFAULT_CTX;
    }
    let m = model.to_ascii_lowercase();
    let leaf = m.rsplit('/').next().unwrap_or(&m);
    let real = if leaf.starts_with("claude") {
        200_000
    } else if leaf.starts_with("gemini") || leaf.starts_with("deepseek") {
        1_000_000
    } else if leaf.starts_with("gpt-5") {
        400_000
    } else if leaf.starts_with("gpt-4.1") {
        1_000_000
    } else if leaf.starts_with("o1") || leaf.starts_with("o3") || leaf.starts_with("o4") {
        200_000
    } else if leaf.starts_with("grok") {
        256_000
    } else {
        // LM Studio and unknown servers: the window of a typical local setup.
        32_768
    };
    real.min(CLOUD_WORKING_TOKENS)
}

fn cut(m: &mut Message, bytes: usize) {
    if m.content.len() > bytes {
        m.content = truncate_bytes(&m.content, bytes);
    }
}

fn shorten_old(
    history: &[Message],
    recent: usize,
    tool_bytes: usize,
    answer_bytes: Option<usize>,
    drop_reasoning: bool,
) -> Vec<Message> {
    let keep_from = history.len().saturating_sub(recent);
    history
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let mut m = m.clone();
            if i < keep_from {
                match m.role {
                    Role::Tool => {
                        cut(&mut m, tool_bytes);
                        m.attachments.clear();
                    }
                    Role::Assistant => {
                        if let Some(b) = answer_bytes
                            && m.tool_calls.is_empty()
                        {
                            cut(&mut m, b);
                        }
                        if drop_reasoning {
                            m.reasoning = None;
                        }
                    }
                    Role::User => {
                        // Files attached long ago are not sent again; the text of the question stays.
                        if answer_bytes.is_some() {
                            m.attachments.clear();
                        }
                    }
                    Role::System => {}
                }
            }
            m
        })
        .collect()
}

/// Start index of each turn: a user message and everything until the next one.
fn turn_starts(history: &[Message]) -> Vec<usize> {
    history
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == Role::User)
        .map(|(i, _)| i)
        .collect()
}

/// The history as the model should see it, and what had to be given up.
///
/// `fixed_tokens` is the size of everything that is not history (system prompt and tool definitions).
/// `keep_reasoning` is true for providers that need earlier reasoning sent back during tool chains
/// (DeepSeek); other providers ignore it, so it is dropped from old messages to save tokens.
#[must_use]
pub fn fit(
    history: &[Message],
    fixed_tokens: usize,
    window: usize,
    answer_tokens: usize,
    keep_reasoning: bool,
) -> (Vec<Message>, Shortened) {
    let budget = window
        .saturating_sub(answer_tokens)
        .saturating_sub(fixed_tokens)
        .max(1024);
    let drop_reasoning = !keep_reasoning;

    // Level 0: the routine cut every request gets.
    let mut v = shorten_old(history, KEEP_RECENT, OLD_RESULT_BYTES, None, drop_reasoning);
    if estimate_messages(&v) <= budget {
        return (v, Shortened::None);
    }
    // Level 1: much harder on everything but the last few messages.
    v = shorten_old(history, KEEP_RECENT / 2, 300, Some(1200), true);
    if estimate_messages(&v) <= budget {
        return (v, Shortened::Hard);
    }
    // Level 2: leave out whole old turns, oldest first. The newest turn always stays.
    let mut chosen: Option<(usize, usize)> = None; // (turns dropped, index the history now starts at)
    for (k, &start) in turn_starts(&v).iter().enumerate().skip(1) {
        chosen = Some((k, start));
        if estimate_messages(&v[start..]) + 80 <= budget {
            break;
        }
    }
    if let Some((dropped, cut_at)) = chosen {
        let mut kept = vec![Message::user(format!(
            "(Earlier messages of this chat, {dropped} turn(s), were left out to fit the context window. Ask the user or call a tool again if you need something from them.)"
        ))];
        kept.extend(v.drain(cut_at..));
        // The note is a user message, so a following user message would be two in a row; merge them.
        if kept.get(1).is_some_and(|m| m.role == Role::User) {
            let first = kept.remove(1);
            kept[0].content.push_str("\n\n");
            kept[0].content.push_str(&first.content);
            kept[0].attachments = first.attachments;
        }
        return (kept, Shortened::Dropped(dropped));
    }
    // One enormous turn: nothing older to drop. Send it as it is and let the provider say so.
    (v, Shortened::Hard)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use workcraft_core::ToolCall;

    fn turn(i: usize, result_bytes: usize) -> Vec<Message> {
        let mut a = Message::assistant("");
        a.tool_calls.push(ToolCall {
            id: format!("c{i}"),
            name: "read".into(),
            arguments: json!({"n": i}),
        });
        a.reasoning = Some("thinking about it".into());
        vec![
            Message::user(format!("question {i}")),
            a,
            Message::tool(&format!("c{i}"), "x".repeat(result_bytes)),
            Message::assistant(format!("answer {i}")),
        ]
    }

    fn convo(turns: usize, result_bytes: usize) -> Vec<Message> {
        (0..turns).flat_map(|i| turn(i, result_bytes)).collect()
    }

    /// Every tool message has its call in the message list, and the list starts with a user message.
    fn well_formed(h: &[Message]) -> bool {
        h.first().is_some_and(|m| m.role == Role::User)
            && h.iter().enumerate().all(|(i, m)| {
                m.role != Role::Tool
                    || h[..i].iter().rev().any(|p| {
                        p.tool_calls
                            .iter()
                            .any(|c| Some(&c.id) == m.tool_call_id.as_ref())
                    })
            })
    }

    #[test]
    fn a_short_chat_is_untouched_apart_from_old_big_results() {
        let h = convo(2, 100);
        let (v, s) = fit(&h, 1000, 32_000, 4096, false);
        assert_eq!(s, Shortened::None);
        assert_eq!(v.len(), h.len());
        let big = convo(6, 20_000);
        let (v, s) = fit(&big, 1000, 200_000, 4096, false);
        assert_eq!(s, Shortened::None);
        assert!(
            v[2].content.len() < 2300,
            "an old result is cut even when everything fits"
        );
        assert_eq!(
            v[v.len() - 2].content.len(),
            20_000,
            "the newest result stays whole"
        );
    }

    #[test]
    fn reasoning_is_kept_only_where_the_provider_needs_it() {
        let h = convo(5, 10);
        let (kept, _) = fit(&h, 100, 32_000, 4096, true);
        assert!(kept[1].reasoning.is_some());
        let (dropped, _) = fit(&h, 100, 32_000, 4096, false);
        assert!(dropped[1].reasoning.is_none());
        assert!(dropped[dropped.len() - 3].reasoning.is_some() || dropped.len() <= KEEP_RECENT);
    }

    #[test]
    fn a_tight_window_shortens_then_drops_old_turns_and_stays_valid() {
        let h = convo(40, 6_000);
        let full = estimate_messages(&h);
        let window = 5_000;
        assert!(full > window * 5);
        let (mid, s_mid) = fit(&h, 1_000, 12_000, 1_000, false);
        assert_eq!(
            s_mid,
            Shortened::Hard,
            "cutting old results is enough for a roomier window"
        );
        assert!(estimate_messages(&mid) < 9_000 && well_formed(&mid));
        let (v, s) = fit(&h, 1_000, window, 1_000, false);
        assert!(matches!(s, Shortened::Dropped(n) if n > 0), "{s:?}");
        assert!(
            estimate_messages(&v) + 1_000 + 1_000 <= window,
            "{} tokens",
            estimate_messages(&v)
        );
        assert!(well_formed(&v));
        assert!(v[0].content.contains("left out to fit"));
        // The newest turn is complete.
        let last = v.last().unwrap();
        assert_eq!(last.content, "answer 39");
        assert!(v.iter().any(|m| m.content == "question 39"));
    }

    #[test]
    fn one_giant_turn_is_sent_as_is() {
        let h = convo(1, 500_000);
        let (v, _) = fit(&h, 0, 8_000, 1_000, false);
        assert_eq!(v.len(), h.len());
    }

    #[test]
    fn windows() {
        assert_eq!(window_for("ollama", "qwen3.5:4b", None), OLLAMA_DEFAULT_CTX);
        assert_eq!(window_for("ollama", "qwen3.5:4b", Some(65_536)), 65_536);
        assert_eq!(
            window_for("openai", "deepseek-flash", None),
            CLOUD_WORKING_TOKENS
        );
        assert_eq!(window_for("anthropic", "claude-sonnet-5-5", None), 200_000);
        assert_eq!(window_for("openai", "mystery", None), 32_768);
    }
}
