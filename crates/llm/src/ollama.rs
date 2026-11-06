//! Ollama through the native `/api/chat` endpoint (NDJSON), which supports tools, thinking,
//! `num_ctx` and `keep_alive` directly.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::mpsc::Sender;
use workcraft_core::{Delta, Error, Message, Result, Role, StopReason, ToolCall, Usage, new_id};

use crate::http::{self, LOCAL_IDLE};
use crate::lines::LineBuffer;
use crate::{ChatRequest, Provider};

pub const DEFAULT_URL: &str = "http://127.0.0.1:11434";

pub struct Ollama {
    base: String,
    http: reqwest::Client,
    keep_alive: String,
}

impl Ollama {
    pub fn new(base_url: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| Error::Http(e.to_string()))?;
        Ok(Self {
            base: base_url.trim_end_matches('/').to_string(),
            http,
            keep_alive: "10m".to_string(),
        })
    }

    pub fn from_env() -> Result<Self> {
        let url = std::env::var("OLLAMA_HOST").unwrap_or_default();
        let url = if url.is_empty() {
            DEFAULT_URL.to_string()
        } else if url.starts_with("http") {
            url
        } else {
            format!("http://{url}")
        };
        Self::new(&url)
    }
}

/// Build the request body. Pure, so it is tested without a server.
/// The context window sent when the settings do not name one.
pub const DEFAULT_NUM_CTX: u32 = 16_384;

#[must_use]
pub fn build_body(req: &ChatRequest, keep_alive: &str) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    if let Some(sys) = req.system.as_deref().filter(|s| !s.is_empty()) {
        messages.push(json!({ "role": "system", "content": sys }));
    }
    // Ollama answers tool results by tool name; remember the name of every call id.
    let mut names: Vec<(String, String)> = Vec::new();
    let mut pending_tool_images: Vec<&str> = Vec::new();
    let flush_tool_images = |msgs: &mut Vec<Value>, imgs: &mut Vec<&str>| {
        if imgs.is_empty() {
            return;
        }
        let batch = std::mem::take(imgs);
        msgs.push(json!({
            "role": "user",
            "content": "[Tool image output]",
            "images": batch,
        }));
    };

    for m in &req.messages {
        if m.role != Role::Tool {
            flush_tool_images(&mut messages, &mut pending_tool_images);
        }
        for c in &m.tool_calls {
            names.push((c.id.clone(), c.name.clone()));
        }
        messages.push(message_json(m, &names, req.vision));
        if m.role == Role::Tool && req.vision {
            pending_tool_images.extend(
                m.attachments
                    .iter()
                    .filter(|a| a.mime.starts_with("image/"))
                    .map(|a| a.data.as_str()),
            );
        }
    }
    flush_tool_images(&mut messages, &mut pending_tool_images);

    let mut options = serde_json::Map::new();
    if let Some(t) = req.options.temperature {
        options.insert("temperature".into(), json!(t));
    }
    if let Some(n) = req.options.max_tokens {
        options.insert("num_predict".into(), json!(n));
    }
    // Ollama starts a model with 4,096 tokens and cuts the start of a longer prompt (the system prompt and the
    // tools). Every request states its window.
    options.insert(
        "num_ctx".into(),
        json!(req.options.num_ctx.unwrap_or(DEFAULT_NUM_CTX)),
    );

    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": true,
        "keep_alive": keep_alive,
        "options": Value::Object(options),
        // A prompt that does not fit is an error, not a silent cut of its start (the system prompt and tools).
        "truncate": false,
    });
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect();
        body["tools"] = Value::Array(tools);
    }
    if let Some(think) = req.options.think {
        body["think"] = json!(think);
    }
    body
}

fn message_json(m: &Message, names: &[(String, String)], vision: bool) -> Value {
    match m.role {
        Role::System => json!({ "role": "system", "content": m.content }),
        Role::User => {
            let mut v = json!({ "role": "user", "content": m.content });
            if vision {
                let images: Vec<&str> = m
                    .attachments
                    .iter()
                    .filter(|a| a.mime.starts_with("image/"))
                    .map(|a| a.data.as_str())
                    .collect();
                if !images.is_empty() {
                    v["images"] = json!(images);
                }
            }
            v
        }
        Role::Assistant => {
            let mut v = json!({ "role": "assistant", "content": m.content });
            if let Some(r) = m.reasoning.as_deref().filter(|s| !s.is_empty()) {
                v["thinking"] = json!(r);
            }
            if !m.tool_calls.is_empty() {
                let calls: Vec<Value> = m
                    .tool_calls
                    .iter()
                    .map(|c| json!({ "function": { "name": c.name, "arguments": c.arguments } }))
                    .collect();
                v["tool_calls"] = Value::Array(calls);
            }
            v
        }
        Role::Tool => {
            let name = m
                .tool_call_id
                .as_deref()
                .and_then(|id| names.iter().find(|(i, _)| i == id).map(|(_, n)| n.as_str()))
                .unwrap_or("tool");
            let trimmed = m.content.trim_start();
            let content = if trimmed.starts_with('{') || trimmed.starts_with('[') {
                format!("[Tool {name} result]:\n{}", m.content)
            } else {
                m.content.clone()
            };
            json!({ "role": "tool", "tool_name": name, "content": content })
        }
    }
}

/// Turn one NDJSON line into deltas. Errors in the line become `Err`.
pub fn parse_line(line: &str) -> Result<Vec<Delta>> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(Vec::new());
    }
    let v: Value =
        serde_json::from_str(line).map_err(|e| Error::Provider(format!("bad stream line: {e}")))?;
    if let Some(err) = v.get("error").and_then(Value::as_str) {
        return Err(Error::Provider(err.to_string()));
    }
    let mut out = Vec::new();
    if let Some(msg) = v.get("message") {
        if let Some(t) = msg
            .get("thinking")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            out.push(Delta::Reasoning(t.to_string()));
        }
        if let Some(t) = msg
            .get("content")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            out.push(Delta::Text(t.to_string()));
        }
        if let Some(calls) = msg.get("tool_calls").and_then(Value::as_array) {
            for c in calls {
                let f = c.get("function").unwrap_or(c);
                let name = f.get("name").and_then(Value::as_str).unwrap_or_default();
                if name.is_empty() {
                    continue;
                }
                let arguments = match f.get("arguments") {
                    Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
                    Some(a) => a.clone(),
                    None => Value::Null,
                };
                out.push(Delta::ToolCall(ToolCall {
                    id: format!("call_{}", new_id()),
                    name: name.to_string(),
                    arguments,
                }));
            }
        }
    }
    if v.get("done").and_then(Value::as_bool) == Some(true) {
        let input = v
            .get("prompt_eval_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let output = v.get("eval_count").and_then(Value::as_u64).unwrap_or(0);
        if input + output > 0 {
            out.push(Delta::Usage(Usage {
                input_tokens: input,
                output_tokens: output,
                ..Usage::default()
            }));
        }
        let stop = match v.get("done_reason").and_then(Value::as_str) {
            Some("length") => StopReason::Length,
            Some("stop") | None => StopReason::Stop,
            Some(_) => StopReason::Other,
        };
        out.push(Delta::Done(stop));
    }
    Ok(out)
}

#[async_trait]
impl Provider for Ollama {
    fn id(&self) -> &str {
        "ollama"
    }

    async fn chat(&self, req: ChatRequest, tx: Sender<Delta>) -> Result<()> {
        let body = build_body(&req, &self.keep_alive);
        let url = format!("{}/api/chat", self.base);
        let resp = match http::send_retrying("Ollama", &tx, || self.http.post(&url).json(&body))
            .await
        {
            Ok(r) => r,
            Err(Error::Cancelled) => return Ok(()),
            Err(Error::Http(_)) => {
                return Err(Error::Http(format!(
                    "Ollama is not running at {}. Start the Ollama app (or run `ollama serve`), then try again",
                    self.base
                )));
            }
            Err(e) => return Err(e),
        };

        let mut stream = resp.bytes_stream();
        let mut lines = LineBuffer::new();
        let mut saw_tool_call = false;
        let mut finished = false;
        while let Some(chunk) = http::next_chunk(&mut stream, LOCAL_IDLE, "Ollama").await? {
            for line in lines.push(&chunk) {
                for d in parse_line(&line)? {
                    if matches!(d, Delta::ToolCall(_)) {
                        saw_tool_call = true;
                    }
                    let d = match d {
                        Delta::Done(StopReason::Stop) if saw_tool_call => {
                            Delta::Done(StopReason::ToolUse)
                        }
                        other => other,
                    };
                    finished |= matches!(d, Delta::Done(_));
                    if tx.send(d).await.is_err() {
                        return Ok(());
                    }
                }
            }
        }
        if let Some(rest) = lines.finish() {
            for d in parse_line(&rest)? {
                finished |= matches!(d, Delta::Done(_));
                if tx.send(d).await.is_err() {
                    return Ok(());
                }
            }
        }
        if !finished {
            let _ = tx.send(Delta::Done(StopReason::Other)).await;
        }
        Ok(())
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        let v: Value = self
            .http
            .get(format!("{}/api/tags", self.base))
            .send()
            .await
            .map_err(|e| Error::Http(format!("cannot reach Ollama at {}: {e}", self.base)))?
            .json()
            .await
            .map_err(|e| Error::Provider(e.to_string()))?;
        let mut names: Vec<String> = v
            .get("models")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("name").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use workcraft_core::{ChatOptions, ToolSpec};

    fn request() -> ChatRequest {
        let mut assistant = Message::assistant("");
        assistant.tool_calls.push(ToolCall {
            id: "c1".into(),
            name: "read_file".into(),
            arguments: json!({ "path": "a.txt" }),
        });
        ChatRequest {
            model: "qwen3:4b".into(),
            system: Some("be brief".into()),
            messages: vec![
                Message::user("hi"),
                assistant,
                Message::tool("c1", "contents"),
            ],
            tools: vec![ToolSpec::new(
                "read_file",
                "read",
                json!({ "type": "object" }),
            )],
            options: ChatOptions {
                num_ctx: Some(16384),
                think: Some(false),
                ..ChatOptions::default()
            },
            vision: false,
        }
    }

    #[test]
    fn body_has_system_tools_and_tool_name() {
        let b = build_body(&request(), "10m");
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(b["messages"][3]["tool_name"], "read_file");
        assert_eq!(
            b["messages"][2]["tool_calls"][0]["function"]["name"],
            "read_file"
        );
        assert_eq!(b["tools"][0]["function"]["name"], "read_file");
        assert_eq!(b["options"]["num_ctx"], 16384);
        assert_eq!(b["think"], false);
        assert_eq!(b["stream"], true);
    }

    #[test]
    fn parses_text_thinking_and_done() {
        let d = parse_line(
            r#"{"message":{"role":"assistant","content":"Hel","thinking":"hm"},"done":false}"#,
        )
        .unwrap();
        assert_eq!(
            d,
            vec![Delta::Reasoning("hm".into()), Delta::Text("Hel".into())]
        );
        let d = parse_line(
            r#"{"done":true,"done_reason":"stop","prompt_eval_count":5,"eval_count":7}"#,
        )
        .unwrap();
        assert_eq!(
            d,
            vec![
                Delta::Usage(Usage {
                    input_tokens: 5,
                    output_tokens: 7,
                    ..Usage::default()
                }),
                Delta::Done(StopReason::Stop)
            ]
        );
    }

    #[test]
    fn parses_tool_calls_with_object_or_string_arguments() {
        let d = parse_line(r#"{"message":{"tool_calls":[{"function":{"name":"x","arguments":{"a":1}}}]},"done":false}"#).unwrap();
        assert!(matches!(&d[0], Delta::ToolCall(c) if c.name == "x" && c.arguments["a"] == 1));
        let d = parse_line(
            r#"{"message":{"tool_calls":[{"function":{"name":"y","arguments":"{\"b\":2}"}}]}}"#,
        )
        .unwrap();
        assert!(matches!(&d[0], Delta::ToolCall(c) if c.arguments["b"] == 2));
    }

    #[test]
    fn error_lines_and_garbage_are_errors_not_panics() {
        assert!(parse_line(r#"{"error":"model not found"}"#).is_err());
        assert!(parse_line("not json").is_err());
        assert!(parse_line("   ").unwrap().is_empty());
    }
}
