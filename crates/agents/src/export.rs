//! Export a conversation: Markdown (readable, re-importable as notes), JSON (everything, for tools),
//! HTML (one self-contained page, light and dark) and plain text.

use serde_json::{Value, json};
use workcraft_core::{Error, Message, Result, Role};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Markdown,
    Json,
    Html,
    Text,
}

impl Format {
    pub fn parse(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "md" | "markdown" => Ok(Self::Markdown),
            "json" => Ok(Self::Json),
            "html" | "htm" => Ok(Self::Html),
            "txt" | "text" => Ok(Self::Text),
            other => Err(Error::Invalid(format!(
                "unknown export format '{other}' (md, json, html, txt)"
            ))),
        }
    }

    #[must_use]
    pub fn ext(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Json => "json",
            Self::Html => "html",
            Self::Text => "txt",
        }
    }
}

fn who(m: &Message) -> String {
    match (&m.role, &m.agent) {
        (Role::User, _) => "You".into(),
        (_, Some(a)) => a.clone(),
        (Role::Assistant, None) => "WorkCraft".into(),
        (Role::Tool, _) => "Tool".into(),
        (Role::System, _) => "System".into(),
    }
}

fn when(ts: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ts)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// A safe file name from a chat title.
#[must_use]
pub fn file_name(title: &str, f: Format) -> String {
    let base: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                ' '
            }
        })
        .collect();
    let base = base.split_whitespace().collect::<Vec<_>>().join(" ");
    let base: String = base.chars().take(60).collect();
    format!(
        "{}.{}",
        if base.is_empty() { "chat" } else { &base },
        f.ext()
    )
}

/// The conversation in `f`. `messages` is the visible path, root first. Tool results are folded into
/// the call that asked for them.
pub fn export(title: &str, messages: &[Message], f: Format) -> Result<String> {
    let results: std::collections::HashMap<&str, &str> = messages
        .iter()
        .filter(|m| m.role == Role::Tool)
        .filter_map(|m| m.tool_call_id.as_deref().map(|id| (id, m.content.as_str())))
        .collect();
    let shown: Vec<&Message> = messages
        .iter()
        .filter(|m| matches!(m.role, Role::User | Role::Assistant))
        .collect();
    Ok(match f {
        Format::Json => serde_json::to_string_pretty(&json!({
            "title": title,
            "exported": chrono::Local::now().to_rfc3339(),
            "app": "WorkCraft",
            "messages": messages,
        }))?,
        Format::Markdown => {
            let mut s = format!("# {title}\n\n");
            for m in &shown {
                let model = m
                    .model
                    .as_ref()
                    .map(|x| format!(" ({})", x.model))
                    .unwrap_or_default();
                s.push_str(&format!("## {}{model} \u{00b7} {}\n\n", who(m), when(m.ts)));
                if !m.content.trim().is_empty() {
                    s.push_str(m.content.trim());
                    s.push_str("\n\n");
                }
                for a in &m.attachments {
                    s.push_str(&format!("*Attached: {}*\n\n", a.name));
                }
                for c in &m.tool_calls {
                    s.push_str(&format!(
                        "<details><summary>Tool: {}</summary>\n\n```json\n{}\n```\n",
                        c.name,
                        serde_json::to_string_pretty(&c.arguments).unwrap_or_default()
                    ));
                    if let Some(r) = results.get(c.id.as_str()) {
                        s.push_str(&format!("\n```\n{}\n```\n", truncate(r, 4000)));
                    }
                    s.push_str("\n</details>\n\n");
                }
            }
            s
        }
        Format::Text => {
            let mut s = format!("{title}\n{}\n\n", "=".repeat(title.chars().count().max(4)));
            for m in &shown {
                s.push_str(&format!(
                    "{} ({}):\n{}\n\n",
                    who(m),
                    when(m.ts),
                    m.content.trim()
                ));
            }
            s
        }
        Format::Html => {
            let mut body = String::new();
            for m in &shown {
                let cls = if m.role == Role::User { "user" } else { "bot" };
                body.push_str(&format!(
                    "<article class=\"{cls}\"><header><b>{}</b><span>{}</span></header>{}",
                    esc(&who(m)),
                    esc(&when(m.ts)),
                    md_html(&m.content)
                ));
                for c in &m.tool_calls {
                    body.push_str(&format!(
                        "<details><summary>Tool: {}</summary><pre>{}</pre>{}</details>",
                        esc(&c.name),
                        esc(&serde_json::to_string_pretty(&c.arguments).unwrap_or_default()),
                        results
                            .get(c.id.as_str())
                            .map(|r| format!("<pre>{}</pre>", esc(&truncate(r, 4000))))
                            .unwrap_or_default()
                    ));
                }
                body.push_str("</article>");
            }
            format!(
                "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{t}</title><style>{CSS}</style></head><body><main><h1>{t}</h1><p class=\"meta\">Exported from WorkCraft, {d}</p>{body}</main></body></html>",
                t = esc(title),
                d = esc(&chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()),
            )
        }
    })
}

const CSS: &str = ":root{--bg:#fff;--fg:#0d0d0d;--muted:#5d5d5d;--bubble:#f3f3f3;--code:#f6f6f6;--line:#e5e5e5}@media (prefers-color-scheme:dark){:root{--bg:#212121;--fg:#ececec;--muted:#a0a0a0;--bubble:#2f2f2f;--code:#181818;--line:#343434}}body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.6 system-ui,-apple-system,Segoe UI,Inter,sans-serif}main{max-width:780px;margin:0 auto;padding:32px 16px}h1{font-size:24px;margin:0}.meta{color:var(--muted);margin:4px 0 28px}article{margin:0 0 24px}article header{display:flex;gap:10px;align-items:baseline;margin-bottom:4px}article header span{color:var(--muted);font-size:12px}article.user{background:var(--bubble);border-radius:16px;padding:10px 16px;margin-left:15%}pre{background:var(--code);border:1px solid var(--line);border-radius:10px;padding:12px;overflow:auto;font:13px/1.5 ui-monospace,Consolas,monospace}code{background:var(--code);border-radius:4px;padding:0 4px;font:13px ui-monospace,Consolas,monospace}pre code{padding:0;background:none}details{border:1px solid var(--line);border-radius:10px;padding:6px 12px;margin:8px 0;color:var(--muted)}table{border-collapse:collapse}td,th{border:1px solid var(--line);padding:4px 8px}";

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}\u{2026}", s.chars().take(max).collect::<String>())
    }
}

fn inline(s: &str) -> String {
    let mut out = String::new();
    let mut parts = s.split('`');
    let mut code = false;
    for (i, p) in parts.by_ref().enumerate() {
        if i > 0 {
            code = !code;
        }
        if code {
            out.push_str(&format!("<code>{}</code>", esc(p)));
        } else {
            let mut t = esc(p);
            // **bold** then *italic*, pairwise.
            for (mark, tag) in [("**", "b"), ("*", "i")] {
                let pieces: Vec<&str> = t.split(mark).collect();
                if pieces.len() >= 3 {
                    // An odd number of pieces means every mark has a partner.
                    let closed = pieces.len() % 2 == 1;
                    let mut r = String::new();
                    for (j, piece) in pieces.iter().enumerate() {
                        if j > 0 {
                            if !closed && j == pieces.len() - 1 {
                                r.push_str(mark);
                            } else if j % 2 == 1 {
                                r.push_str(&format!("<{tag}>"));
                            } else {
                                r.push_str(&format!("</{tag}>"));
                            }
                        }
                        r.push_str(piece);
                    }
                    t = r;
                }
            }
            out.push_str(&t);
        }
    }
    out
}

/// Small Markdown to HTML: fenced code, headings, lists, paragraphs, inline styles.
fn md_html(md: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    let mut in_list = false;
    for line in md.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            if in_list {
                out.push_str("</ul>");
                in_list = false;
            }
            out.push_str(if in_code {
                "</code></pre>"
            } else {
                "<pre><code>"
            });
            in_code = !in_code;
            continue;
        }
        if in_code {
            out.push_str(&esc(line));
            out.push('\n');
            continue;
        }
        let item = t.strip_prefix("- ").or_else(|| t.strip_prefix("* "));
        if item.is_none() && in_list {
            out.push_str("</ul>");
            in_list = false;
        }
        if let Some(i) = item {
            if !in_list {
                out.push_str("<ul>");
                in_list = true;
            }
            out.push_str(&format!("<li>{}</li>", inline(i)));
        } else if let Some(h) = t.strip_prefix("### ") {
            out.push_str(&format!("<h4>{}</h4>", inline(h)));
        } else if let Some(h) = t.strip_prefix("## ") {
            out.push_str(&format!("<h3>{}</h3>", inline(h)));
        } else if let Some(h) = t.strip_prefix("# ") {
            out.push_str(&format!("<h2>{}</h2>", inline(h)));
        } else if !t.is_empty() {
            out.push_str(&format!("<p>{}</p>", inline(t)));
        }
    }
    if in_code {
        out.push_str("</code></pre>");
    }
    if in_list {
        out.push_str("</ul>");
    }
    out
}

/// Every session as one JSON document, for a full backup.
#[must_use]
pub fn bundle(sessions: Vec<(String, Vec<Message>)>) -> Value {
    json!({
        "app": "WorkCraft",
        "exported": chrono::Local::now().to_rfc3339(),
        "chats": sessions.into_iter().map(|(t, m)| json!({ "title": t, "messages": m })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat() -> Vec<Message> {
        let u = Message::user("What is **2+2**? Use `code`.");
        let mut a = Message::assistant("It is 4.\n\n```rust\nlet x = 4;\n```\n- one <b>\n- two");
        a.parent = Some(u.id.clone());
        vec![u, a]
    }

    #[test]
    fn every_format_has_the_conversation() {
        for f in ["md", "json", "html", "txt"] {
            let s = export("Math & more", &chat(), Format::parse(f).unwrap()).unwrap();
            assert!(s.contains("It is 4."), "{f}: {s}");
        }
        let html = export("Math & more", &chat(), Format::Html).unwrap();
        assert!(html.contains("<title>Math &amp; more</title>"));
        assert!(html.contains("<b>2+2</b>") && html.contains("<code>code</code>"));
        assert!(html.contains("<pre><code>let x = 4;"));
        assert!(
            html.contains("one &lt;b&gt;"),
            "markup in messages is escaped"
        );
        let md = export("t", &chat(), Format::Markdown).unwrap();
        assert!(md.starts_with("# t\n") && md.contains("## You"));
        assert!(Format::parse("docx").is_err());
        assert_eq!(file_name("Plan: Q3/Q4?", Format::Markdown), "Plan Q3 Q4.md");
        assert_eq!(file_name("***", Format::Json), "chat.json");
    }
}
