//! Anthropic Messages API with SSE streaming, tool use and images.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::mpsc::Sender;
use workcraft_core::{Delta, Error, Result, Role, StopReason, ToolCall, Usage};

use crate::http::{self, CLOUD_IDLE};
use crate::lines::{LineBuffer, SseParser};
use crate::{ChatRequest, Provider};

pub const DEFAULT_URL: &str = "https://api.anthropic.com";
const VERSION: &str = "2023-06-01";

pub struct Anthropic {
    base: String,
    key: String,
    http: reqwest::Client,
}

impl Anthropic {
    pub fn new(base_url: &str, api_key: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| Error::Http(e.to_string()))?;
        Ok(Self {
            base: base_url.trim_end_matches('/').to_string(),
            key: api_key.to_string(),
            http,
        })
    }
}

#[must_use]
pub fn build_body(req: &ChatRequest) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    for m in &req.messages {
        match m.role {
            // System messages in the history are folded into the system prompt by the caller.
            Role::System => {}
            Role::User => {
                let mut blocks: Vec<Value> = Vec::new();
                if req.vision {
                    for a in m
                        .attachments
                        .iter()
                        .filter(|a| a.mime.starts_with("image/"))
                    {
                        blocks.push(json!({
                            "type": "image",
                            "source": { "type": "base64", "media_type": a.mime, "data": a.data }
                        }));
                    }
                }
                blocks.push(json!({ "type": "text", "text": m.content }));
                messages.push(json!({ "role": "user", "content": blocks }));
            }
            Role::Assistant => {
                let mut blocks: Vec<Value> = Vec::new();
                if !m.content.is_empty() {
                    blocks.push(json!({ "type": "text", "text": m.content }));
                }
                for c in &m.tool_calls {
                    blocks.push(json!({ "type": "tool_use", "id": c.id, "name": c.name, "input": c.arguments }));
                }
                if blocks.is_empty() {
                    blocks.push(json!({ "type": "text", "text": "(no output)" }));
                }
                messages.push(json!({ "role": "assistant", "content": blocks }));
            }
            Role::Tool => {
                let images: Vec<_> = if req.vision {
                    m.attachments
                        .iter()
                        .filter(|a| a.mime.starts_with("image/"))
                        .collect()
                } else {
                    Vec::new()
                };
                let content = if images.is_empty() {
                    json!(m.content)
                } else {
                    let mut arr = vec![json!({ "type": "text", "text": m.content })];
                    for a in images {
                        arr.push(json!({
                            "type": "image",
                            "source": { "type": "base64", "media_type": a.mime, "data": a.data }
                        }));
                    }
                    Value::Array(arr)
                };
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": m.tool_call_id.clone().unwrap_or_default(),
                    "content": content,
                });
                // Results of one assistant turn must share one user message.
                let merged = messages
                    .last_mut()
                    .filter(|l| {
                        l["role"] == "user"
                            && l["content"]
                                .as_array()
                                .is_some_and(|a| a.iter().all(|b| b["type"] == "tool_result"))
                    })
                    .and_then(|l| l["content"].as_array_mut());
                match merged {
                    Some(arr) => arr.push(block),
                    None => messages.push(json!({ "role": "user", "content": [block] })),
                }
            }
        }
    }
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.options.max_tokens.unwrap_or(8192),
        "messages": messages,
        "stream": true,
    });
    // Prompt caching: the system prompt, the tool list and the end of the history are the parts that repeat
    // on every round of a tool loop. Cache hits cost a tenth of the input price. Parts below the minimum
    // size (1,024 tokens, 2,048 on small models) are simply not cached, which is not an error.
    if let Some(sys) = req.system.as_deref().filter(|s| !s.is_empty()) {
        body["system"] =
            json!([{ "type": "text", "text": sys, "cache_control": { "type": "ephemeral" } }]);
    }
    if let Some(t) = req.options.temperature {
        body["temperature"] = json!(t);
    }
    if !req.tools.is_empty() {
        let mut tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.input_schema }))
            .collect();
        if let Some(last) = tools.last_mut() {
            last["cache_control"] = json!({ "type": "ephemeral" });
        }
        body["tools"] = Value::Array(tools);
    }
    mark_history_for_cache(&mut body["messages"]);
    body
}

/// Put a cache breakpoint on the last block of the last message, so everything before the newest turn is
/// read from the cache on the next round.
fn mark_history_for_cache(messages: &mut Value) {
    let Some(last) = messages.as_array_mut().and_then(|a| a.last_mut()) else {
        return;
    };
    if let Some(block) = last["content"].as_array_mut().and_then(|b| b.last_mut()) {
        block["cache_control"] = json!({ "type": "ephemeral" });
    }
}

#[derive(Debug, Default)]
pub struct StreamState {
    /// Open tool_use blocks by index: (id, name, partial json).
    tools: BTreeMap<u64, (String, String, String)>,
    input_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    stop: Option<StopReason>,
}

impl StreamState {
    pub fn feed(&mut self, event: Option<&str>, data: &str) -> Result<Vec<Delta>> {
        let data = data.trim();
        if data.is_empty() {
            return Ok(Vec::new());
        }
        let v: Value = serde_json::from_str(data)
            .map_err(|e| Error::Provider(format!("bad stream data: {e}")))?;
        let kind = v
            .get("type")
            .and_then(Value::as_str)
            .or(event)
            .unwrap_or_default();
        let mut out = Vec::new();
        match kind {
            "error" => {
                let msg = v["error"]["message"].as_str().unwrap_or("provider error");
                let text = match v["error"]["type"].as_str() {
                    Some("overloaded_error") => {
                        format!("Anthropic is overloaded right now. Try again in a minute ({msg})")
                    }
                    Some("rate_limit_error") => {
                        format!("Anthropic: too many requests. Wait a moment ({msg})")
                    }
                    Some("authentication_error" | "permission_error") => {
                        "Anthropic: the API key was not accepted. Check it in Settings, Providers"
                            .to_string()
                    }
                    _ => format!("Anthropic: {msg}"),
                };
                return Err(Error::Provider(text));
            }
            "message_start" => {
                let u = &v["message"]["usage"];
                self.input_tokens = u["input_tokens"].as_u64().unwrap_or(0);
                self.cache_read_tokens = u["cache_read_input_tokens"].as_u64().unwrap_or(0);
                self.cache_write_tokens = u["cache_creation_input_tokens"].as_u64().unwrap_or(0);
            }
            "content_block_start" => {
                let idx = v["index"].as_u64().unwrap_or(0);
                let b = &v["content_block"];
                if b["type"] == "tool_use" {
                    self.tools.insert(
                        idx,
                        (
                            b["id"].as_str().unwrap_or_default().to_string(),
                            b["name"].as_str().unwrap_or_default().to_string(),
                            String::new(),
                        ),
                    );
                }
            }
            "content_block_delta" => {
                let idx = v["index"].as_u64().unwrap_or(0);
                let d = &v["delta"];
                match d["type"].as_str().unwrap_or_default() {
                    "text_delta" => {
                        if let Some(t) = d["text"].as_str().filter(|s| !s.is_empty()) {
                            out.push(Delta::Text(t.to_string()));
                        }
                    }
                    "thinking_delta" => {
                        if let Some(t) = d["thinking"].as_str().filter(|s| !s.is_empty()) {
                            out.push(Delta::Reasoning(t.to_string()));
                        }
                    }
                    "input_json_delta" => {
                        if let (Some(e), Some(p)) =
                            (self.tools.get_mut(&idx), d["partial_json"].as_str())
                        {
                            e.2.push_str(p);
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let idx = v["index"].as_u64().unwrap_or(0);
                if let Some((id, name, args)) = self.tools.remove(&idx) {
                    let arguments = if args.trim().is_empty() {
                        json!({})
                    } else {
                        crate::toolargs::parse_tool_args(&args)
                    };
                    out.push(Delta::ToolCall(ToolCall {
                        id,
                        name,
                        arguments,
                    }));
                }
            }
            "message_delta" => {
                self.stop = Some(match v["delta"]["stop_reason"].as_str() {
                    Some("tool_use") => StopReason::ToolUse,
                    Some("max_tokens") => StopReason::Length,
                    Some("end_turn" | "stop_sequence") | None => StopReason::Stop,
                    Some(_) => StopReason::Other,
                });
                let output = v["usage"]["output_tokens"].as_u64().unwrap_or(0);
                out.push(Delta::Usage(Usage {
                    input_tokens: self.input_tokens,
                    output_tokens: output,
                    cache_read_tokens: self.cache_read_tokens,
                    cache_write_tokens: self.cache_write_tokens,
                    reasoning_tokens: 0,
                }));
            }
            "message_stop" => out.push(Delta::Done(self.stop.unwrap_or(StopReason::Stop))),
            _ => {}
        }
        Ok(out)
    }
}

#[async_trait]
impl Provider for Anthropic {
    fn id(&self) -> &str {
        "anthropic"
    }

    async fn chat(&self, req: ChatRequest, tx: Sender<Delta>) -> Result<()> {
        let body = build_body(&req);
        let url = format!("{}/v1/messages", self.base);
        let resp = match http::send_retrying("Anthropic", &tx, || {
            self.http
                .post(&url)
                .header("x-api-key", &self.key)
                .header("anthropic-version", VERSION)
                .json(&body)
        })
        .await
        {
            Ok(r) => r,
            Err(Error::Cancelled) => return Ok(()),
            Err(e) => return Err(e),
        };
        let mut stream = resp.bytes_stream();
        let mut lines = LineBuffer::new();
        let mut sse = SseParser::new();
        let mut state = StreamState::default();
        let mut finished = false;
        while let Some(chunk) = http::next_chunk(&mut stream, CLOUD_IDLE, "Anthropic").await? {
            for line in lines.push(&chunk) {
                if let Some(ev) = sse.push_line(&line) {
                    for d in state.feed(ev.event.as_deref(), &ev.data)? {
                        finished |= matches!(d, Delta::Done(_));
                        if tx.send(d).await.is_err() {
                            return Ok(());
                        }
                    }
                }
            }
        }
        if !finished {
            let _ = tx.send(Delta::Done(StopReason::Other)).await;
        }
        Ok(())
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        let resp = self
            .http
            .get(format!("{}/v1/models", self.base))
            .header("x-api-key", &self.key)
            .header("anthropic-version", VERSION)
            .send()
            .await
            .map_err(|e| Error::Http(e.to_string()))?;
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(Error::Provider(crate::openai::list_error(
                status.as_u16(),
                &v,
            )));
        }
        Ok(v.get("data")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|m| m["id"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use workcraft_core::Message;

    #[test]
    fn tool_results_merge_into_one_user_message() {
        let mut a = Message::assistant("");
        a.tool_calls.push(ToolCall {
            id: "1".into(),
            name: "f".into(),
            arguments: json!({}),
        });
        a.tool_calls.push(ToolCall {
            id: "2".into(),
            name: "g".into(),
            arguments: json!({}),
        });
        let req = ChatRequest {
            model: "m".into(),
            system: Some("s".into()),
            messages: vec![
                Message::user("go"),
                a,
                Message::tool("1", "r1"),
                Message::tool("2", "r2"),
            ],
            tools: vec![],
            options: Default::default(),
            vision: true,
        };
        let b = build_body(&req);
        let msgs = b["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2]["content"].as_array().unwrap().len(), 2);
        assert_eq!(b["system"][0]["text"], "s");
    }

    #[test]
    fn prompt_caching_marks_system_last_tool_and_end_of_history() {
        let req = ChatRequest {
            model: "m".into(),
            system: Some("rules".into()),
            messages: vec![
                Message::user("one"),
                Message::assistant("two"),
                Message::user("three"),
            ],
            tools: vec![
                workcraft_core::ToolSpec::new("a", "first", json!({"type": "object"})),
                workcraft_core::ToolSpec::new("b", "second", json!({"type": "object"})),
            ],
            options: Default::default(),
            vision: false,
        };
        let b = build_body(&req);
        assert_eq!(b["system"][0]["cache_control"]["type"], "ephemeral");
        assert!(
            b["tools"][0].get("cache_control").is_none(),
            "only the last tool is marked"
        );
        assert_eq!(b["tools"][1]["cache_control"]["type"], "ephemeral");
        let msgs = b["messages"].as_array().unwrap();
        assert!(msgs[0]["content"][0].get("cache_control").is_none());
        assert_eq!(msgs[2]["content"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn cache_usage_is_reported_separately() {
        let mut s = StreamState::default();
        s.feed(
            Some("message_start"),
            r#"{"type":"message_start","message":{"usage":{"input_tokens":12,"cache_read_input_tokens":3000,"cache_creation_input_tokens":500}}}"#,
        )
        .unwrap();
        let d = s
            .feed(
                Some("message_delta"),
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":40}}"#,
            )
            .unwrap();
        let Delta::Usage(u) = &d[0] else {
            panic!("no usage")
        };
        assert_eq!(
            (
                u.input_tokens,
                u.cache_read_tokens,
                u.cache_write_tokens,
                u.output_tokens
            ),
            (12, 3000, 500, 40)
        );
        assert_eq!(u.prompt_tokens(), 3512);
    }

    #[test]
    fn overloaded_error_reads_plainly() {
        let mut s = StreamState::default();
        let e = s
            .feed(
                Some("error"),
                r#"{"type":"error","error":{"type":"overloaded_error","message":"busy"}}"#,
            )
            .unwrap_err();
        assert!(e.to_string().contains("overloaded"), "{e}");
    }

    #[test]
    fn tool_result_images_follow_vision_flag() {
        let mut t = Message::tool("1", "rendered [saved image: out.png (64x64)]");
        t.attachments.push(workcraft_core::Attachment {
            name: "out.png".into(),
            mime: "image/png".into(),
            data: "QUJD".into(),
        });
        let mut a = Message::assistant("");
        a.tool_calls.push(ToolCall {
            id: "1".into(),
            name: "render".into(),
            arguments: json!({}),
        });
        let req_vis = ChatRequest {
            model: "claude-sonnet-4-5".into(),
            system: None,
            messages: vec![Message::user("render"), a.clone(), t.clone()],
            tools: vec![],
            options: Default::default(),
            vision: true,
        };
        let b_vis = build_body(&req_vis);
        assert_eq!(
            b_vis["messages"][2]["content"][0]["content"][1]["type"],
            "image"
        );

        let req_txt = ChatRequest {
            vision: false,
            ..req_vis
        };
        let b_txt = build_body(&req_txt);
        assert!(b_txt["messages"][2]["content"][0]["content"].is_string());
    }

    #[test]
    fn stream_text_tool_and_done() {
        let mut s = StreamState::default();
        let mut all = Vec::new();
        for (ev, data) in [
            (
                "message_start",
                r#"{"type":"message_start","message":{"usage":{"input_tokens":11}}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#,
            ),
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tu1","name":"read"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"p\":"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"1}"}}"#,
            ),
            (
                "content_block_stop",
                r#"{"type":"content_block_stop","index":1}"#,
            ),
            (
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":9}}"#,
            ),
            ("message_stop", r#"{"type":"message_stop"}"#),
        ] {
            all.extend(s.feed(Some(ev), data).unwrap());
        }
        assert_eq!(all[0], Delta::Text("Hi".into()));
        assert!(matches!(&all[1], Delta::ToolCall(c) if c.id == "tu1" && c.arguments["p"] == 1));
        assert_eq!(
            all[2],
            Delta::Usage(Usage {
                input_tokens: 11,
                output_tokens: 9,
                ..Usage::default()
            })
        );
        assert_eq!(all[3], Delta::Done(StopReason::ToolUse));
    }

    #[test]
    fn error_event_is_an_error() {
        let mut s = StreamState::default();
        assert!(
            s.feed(None, r#"{"type":"error","error":{"message":"overloaded"}}"#)
                .is_err()
        );
    }
}
