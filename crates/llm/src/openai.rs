//! OpenAI-compatible `/chat/completions` with SSE streaming. Works with OpenAI, DeepSeek, LM Studio,
//! vLLM, OpenRouter and anything else that speaks the same wire format.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::mpsc::Sender;
use workcraft_core::{Delta, Error, Message, Result, Role, StopReason, ToolCall, Usage, new_id};

use crate::http::{self, CLOUD_IDLE, LOCAL_IDLE};
use crate::lines::{LineBuffer, SseParser};
use crate::{ChatRequest, Provider};

/// Provider-specific rules of the OpenAI wire format. Servers that speak it differ in what they accept:
/// OpenAI's newer models reject `max_tokens` and `temperature`, DeepSeek needs its reasoning sent back
/// during tool chains, and DeepSeek and Z.ai switch thinking with a `thinking` object.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Quirks {
    /// Send `max_completion_tokens` instead of `max_tokens` and leave `temperature` out.
    pub reasoning_model_params: bool,
    /// Send each assistant message's reasoning back as `reasoning_content`.
    pub replay_reasoning: bool,
    /// Switch thinking with `{"thinking": {"type": "enabled" | "disabled"}}`.
    pub thinking_object: bool,
}

impl Quirks {
    /// Rules for this provider and model, from the provider id, the base URL and the model name.
    #[must_use]
    pub fn detect(provider_id: &str, base_url: &str, model: &str) -> Self {
        let id = provider_id.to_ascii_lowercase();
        let base = base_url.to_ascii_lowercase();
        let m = model.to_ascii_lowercase();
        let leaf = m.rsplit('/').next().unwrap_or(&m);
        let deepseek = id == "deepseek" || base.contains("deepseek.com");
        let zai =
            id == "glm" || id == "zai" || base.contains("z.ai") || base.contains("bigmodel.cn");
        let openai = id == "openai" || base.contains("api.openai.com");
        let o_series = leaf.starts_with("o1") || leaf.starts_with("o3") || leaf.starts_with("o4");
        Self {
            reasoning_model_params: openai && (o_series || leaf.starts_with("gpt-5")),
            replay_reasoning: deepseek,
            thinking_object: deepseek || zai,
        }
    }
}

pub struct OpenAiCompat {
    id: String,
    base: String,
    key: Option<String>,
    http: reqwest::Client,
}

impl OpenAiCompat {
    /// `base_url` includes the version path, for example `https://api.openai.com/v1`.
    pub fn new(id: &str, base_url: &str, api_key: Option<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| Error::Http(e.to_string()))?;
        Ok(Self {
            id: id.to_string(),
            base: base_url.trim_end_matches('/').to_string(),
            key: api_key.filter(|k| !k.is_empty()),
            http,
        })
    }

    /// Name shown in messages: the provider id with a capital letter, for example "Deepseek".
    fn label(&self) -> String {
        let mut c = self.id.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => "Provider".to_string(),
        }
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.key {
            Some(k) => rb.bearer_auth(k),
            None => rb,
        }
    }
}

#[must_use]
pub fn build_body(req: &ChatRequest) -> Value {
    build_body_with(req, Quirks::default())
}

#[must_use]
pub fn build_body_with(req: &ChatRequest, q: Quirks) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    if let Some(sys) = req.system.as_deref().filter(|s| !s.is_empty()) {
        messages.push(json!({ "role": "system", "content": sys }));
    }
    let mut pending_tool_images: Vec<&workcraft_core::Attachment> = Vec::new();
    let flush_tool_images = |msgs: &mut Vec<Value>, imgs: &mut Vec<&workcraft_core::Attachment>| {
        if imgs.is_empty() {
            return;
        }
        let mut parts = vec![json!({ "type": "text", "text": "[Tool image output]" })];
        for a in imgs.drain(..) {
            parts.push(json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{};base64,{}", a.mime, a.data) }
            }));
        }
        msgs.push(json!({ "role": "user", "content": parts }));
    };

    for m in &req.messages {
        if m.role != Role::Tool {
            flush_tool_images(&mut messages, &mut pending_tool_images);
        }
        messages.push(message_json(m, req.vision, q.replay_reasoning));
        if m.role == Role::Tool && req.vision {
            pending_tool_images.extend(
                m.attachments
                    .iter()
                    .filter(|a| a.mime.starts_with("image/")),
            );
        }
    }
    flush_tool_images(&mut messages, &mut pending_tool_images);

    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if let Some(t) = req.options.temperature
        && !q.reasoning_model_params
    {
        body["temperature"] = json!(t);
    }
    if let Some(n) = req.options.max_tokens {
        let key = if q.reasoning_model_params {
            "max_completion_tokens"
        } else {
            "max_tokens"
        };
        body[key] = json!(n);
    }
    if q.thinking_object
        && let Some(on) = req.options.think
    {
        body["thinking"] = json!({ "type": if on { "enabled" } else { "disabled" } });
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": { "name": t.name, "description": t.description, "parameters": t.input_schema }
                })
            })
            .collect();
        body["tools"] = Value::Array(tools);
    }
    body
}

fn message_json(m: &Message, vision: bool, replay_reasoning: bool) -> Value {
    match m.role {
        Role::System => json!({ "role": "system", "content": m.content }),
        Role::User => {
            let images: Vec<&workcraft_core::Attachment> = if vision {
                m.attachments
                    .iter()
                    .filter(|a| a.mime.starts_with("image/"))
                    .collect()
            } else {
                Vec::new()
            };
            if images.is_empty() {
                json!({ "role": "user", "content": m.content })
            } else {
                let mut parts = vec![json!({ "type": "text", "text": m.content })];
                for a in images {
                    parts.push(json!({
                        "type": "image_url",
                        "image_url": { "url": format!("data:{};base64,{}", a.mime, a.data) }
                    }));
                }
                json!({ "role": "user", "content": parts })
            }
        }
        Role::Assistant => {
            let mut v = json!({ "role": "assistant", "content": if m.content.is_empty() { Value::Null } else { json!(m.content) } });
            if !m.tool_calls.is_empty() {
                let calls: Vec<Value> = m
                    .tool_calls
                    .iter()
                    .map(|c| {
                        json!({
                            "id": c.id,
                            "type": "function",
                            "function": { "name": c.name, "arguments": c.arguments.to_string() }
                        })
                    })
                    .collect();
                v["tool_calls"] = Value::Array(calls);
            }
            // DeepSeek answers 400 when a thinking-mode turn comes back without its reasoning.
            if replay_reasoning && let Some(r) = m.reasoning.as_deref().filter(|r| !r.is_empty()) {
                v["reasoning_content"] = json!(r);
            }
            v
        }
        Role::Tool => json!({
            "role": "tool",
            "tool_call_id": m.tool_call_id.clone().unwrap_or_default(),
            "content": m.content,
        }),
    }
}

/// Accumulates a streamed answer: tool calls arrive in pieces keyed by index.
#[derive(Debug, Default)]
pub struct StreamState {
    calls: BTreeMap<u64, (String, String, String)>,
    finish: Option<StopReason>,
    done: bool,
}

impl StreamState {
    /// Feed the `data:` payload of one SSE event.
    pub fn feed(&mut self, data: &str) -> Result<Vec<Delta>> {
        let data = data.trim();
        if data.is_empty() {
            return Ok(Vec::new());
        }
        if data == "[DONE]" {
            return Ok(self.finish());
        }
        let v: Value = serde_json::from_str(data)
            .map_err(|e| Error::Provider(format!("bad stream data: {e}")))?;
        if let Some(err) = v.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("provider error");
            return Err(Error::Provider(msg.to_string()));
        }
        let mut out = Vec::new();
        if let Some(choice) = v.get("choices").and_then(|c| c.get(0)) {
            if let Some(d) = choice.get("delta") {
                if let Some(t) = d
                    .get("reasoning_content")
                    .or_else(|| d.get("reasoning"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    out.push(Delta::Reasoning(t.to_string()));
                }
                if let Some(t) = d
                    .get("content")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    out.push(Delta::Text(t.to_string()));
                }
                if let Some(calls) = d.get("tool_calls").and_then(Value::as_array) {
                    for c in calls {
                        let idx = c.get("index").and_then(Value::as_u64).unwrap_or(0);
                        let e = self.calls.entry(idx).or_default();
                        if let Some(id) = c.get("id").and_then(Value::as_str) {
                            e.0 = id.to_string();
                        }
                        if let Some(f) = c.get("function") {
                            if let Some(n) = f.get("name").and_then(Value::as_str) {
                                e.1.push_str(n);
                            }
                            if let Some(a) = f.get("arguments").and_then(Value::as_str) {
                                e.2.push_str(a);
                            }
                        }
                    }
                }
            }
            if let Some(r) = choice.get("finish_reason").and_then(Value::as_str) {
                self.finish = Some(match r {
                    "tool_calls" | "function_call" => StopReason::ToolUse,
                    "length" => StopReason::Length,
                    "stop" => StopReason::Stop,
                    _ => StopReason::Other,
                });
            }
        }
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            out.push(Delta::Usage(parse_usage(u)));
        }
        Ok(out)
    }

    /// True when the stream ended because of the length limit, which can cut a tool call in half.
    #[must_use]
    pub fn hit_length_limit(&self) -> bool {
        self.finish == Some(StopReason::Length)
    }

    /// End of stream: emit the finished tool calls and the stop reason once.
    pub fn finish(&mut self) -> Vec<Delta> {
        if self.done {
            return Vec::new();
        }
        self.done = true;
        let mut out = Vec::new();
        for (_, (id, name, args)) in std::mem::take(&mut self.calls) {
            if name.is_empty() {
                continue;
            }
            let arguments = if args.trim().is_empty() {
                json!({})
            } else {
                crate::toolargs::parse_tool_args(&args)
            };
            let id = if id.is_empty() {
                format!("call_{}", new_id())
            } else {
                id
            };
            out.push(Delta::ToolCall(ToolCall {
                id,
                name,
                arguments,
            }));
        }
        let has_calls = out.iter().any(|d| matches!(d, Delta::ToolCall(_)));
        let stop = match (self.finish, has_calls) {
            (_, true) => StopReason::ToolUse,
            (Some(s), false) => s,
            (None, false) => StopReason::Stop,
        };
        out.push(Delta::Done(stop));
        out
    }
}

#[async_trait]
impl Provider for OpenAiCompat {
    fn id(&self) -> &str {
        &self.id
    }

    async fn chat(&self, req: ChatRequest, tx: Sender<Delta>) -> Result<()> {
        let body = build_body_with(&req, Quirks::detect(&self.id, &self.base, &req.model));
        let url = format!("{}/chat/completions", self.base);
        let label = self.label();
        let resp =
            match http::send_retrying(&label, &tx, || self.auth(self.http.post(&url)).json(&body))
                .await
            {
                Ok(r) => r,
                // The receiver is gone: the turn was cancelled.
                Err(Error::Cancelled) => return Ok(()),
                Err(e) => return Err(e),
            };
        let idle = if is_local_host(&self.base) {
            LOCAL_IDLE
        } else {
            CLOUD_IDLE
        };
        let mut stream = resp.bytes_stream();
        let mut lines = LineBuffer::new();
        let mut sse = SseParser::new();
        let mut state = StreamState::default();
        while let Some(chunk) = http::next_chunk(&mut stream, idle, &label).await? {
            for line in lines.push(&chunk) {
                if let Some(ev) = sse.push_line(&line) {
                    for d in state.feed(&ev.data)? {
                        if tx.send(d).await.is_err() {
                            return Ok(());
                        }
                    }
                }
            }
        }
        if let Some(rest) = lines.finish() {
            sse.push_line(&rest);
        }
        if let Some(ev) = sse.flush() {
            for d in state.feed(&ev.data)? {
                if tx.send(d).await.is_err() {
                    return Ok(());
                }
            }
        }
        for d in state.finish() {
            if tx.send(d).await.is_err() {
                return Ok(());
            }
        }
        Ok(())
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        let resp = self
            .auth(self.http.get(format!("{}/models", self.base)))
            .send()
            .await
            .map_err(|e| Error::Http(e.to_string()))?;
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        // A wrong key answers 401 with an error body: say so instead of listing no models.
        if !status.is_success() {
            return Err(Error::Provider(list_error(status.as_u16(), &v)));
        }
        let mut ids: Vec<String> = v
            .get("data")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        ids.sort();
        Ok(ids)
    }
}

/// Whether `base_url` points at this computer or the home network (long silences are normal there).
#[must_use]
pub fn is_local_host(base_url: &str) -> bool {
    let b = base_url.to_ascii_lowercase();
    let host = b
        .split("://")
        .nth(1)
        .unwrap_or(&b)
        .split(['/', ':'])
        .next()
        .unwrap_or("");
    host == "localhost"
        || host.starts_with("127.")
        || host.starts_with("192.168.")
        || host.starts_with("10.")
        || host.ends_with(".local")
}

/// Token counts of an OpenAI-style `usage` object. `prompt_tokens` includes the cached part, so the full
/// rate input is what is left after it. Reads DeepSeek''s hit counter and OpenAI''s `cached_tokens`.
#[must_use]
pub fn parse_usage(u: &Value) -> Usage {
    let n = |ptr: &str| u.pointer(ptr).and_then(Value::as_u64);
    let prompt = n("/prompt_tokens").unwrap_or(0);
    let cached = n("/prompt_cache_hit_tokens")
        .or_else(|| n("/prompt_tokens_details/cached_tokens"))
        .unwrap_or(0)
        .min(prompt);
    Usage {
        input_tokens: prompt - cached,
        output_tokens: n("/completion_tokens").unwrap_or(0),
        cache_read_tokens: cached,
        cache_write_tokens: 0,
        reasoning_tokens: n("/completion_tokens_details/reasoning_tokens").unwrap_or(0),
    }
}

/// A readable message for a failed model list, from the status and the error body if there is one.
pub(crate) fn list_error(status: u16, body: &Value) -> String {
    let msg = body
        .pointer("/error/message")
        .or_else(|| body.get("error"))
        .or_else(|| body.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let what = match status {
        401 | 403 => "the API key was not accepted",
        404 => "the address has no model list",
        429 => "too many requests or no credit left",
        _ => "the server answered with an error",
    };
    if msg.is_empty() {
        format!("{what} (HTTP {status})")
    } else {
        format!("{what} (HTTP {status}): {msg}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req_with(history: Vec<Message>, tools: bool) -> ChatRequest {
        ChatRequest {
            model: "deepseek-flash".into(),
            system: Some("s".into()),
            messages: history,
            tools: if tools {
                vec![workcraft_core::ToolSpec::new(
                    "t",
                    "d",
                    json!({"type": "object"}),
                )]
            } else {
                vec![]
            },
            options: workcraft_core::ChatOptions {
                temperature: Some(0.2),
                max_tokens: Some(100),
                ..Default::default()
            },
            vision: false,
        }
    }

    #[test]
    fn deepseek_gets_its_reasoning_back_and_the_thinking_switch() {
        let mut a = Message::assistant("");
        a.reasoning = Some("I should call t".into());
        a.tool_calls.push(ToolCall {
            id: "1".into(),
            name: "t".into(),
            arguments: json!({}),
        });
        let mut req = req_with(vec![Message::user("go"), a, Message::tool("1", "r")], true);
        let q = Quirks::detect("deepseek", "https://api.deepseek.com/v1", "deepseek-flash");
        assert!(q.replay_reasoning && q.thinking_object);
        let b = build_body_with(&req, q);
        assert_eq!(b["messages"][2]["reasoning_content"], "I should call t");
        assert!(b.get("thinking").is_none(), "no switch unless asked");
        req.options.think = Some(false);
        assert_eq!(build_body_with(&req, q)["thinking"]["type"], "disabled");
        // Other servers never see the field.
        let other = build_body_with(
            &req,
            Quirks::detect("lmstudio", "http://127.0.0.1:1234/v1", "x"),
        );
        assert!(other["messages"][2].get("reasoning_content").is_none());
        assert!(other.get("thinking").is_none());
    }

    #[test]
    fn openai_reasoning_models_use_max_completion_tokens_and_no_temperature() {
        let req = req_with(vec![Message::user("hi")], false);
        let q = Quirks::detect("openai", "https://api.openai.com/v1", "gpt-5");
        let b = build_body_with(&req, q);
        assert_eq!(b["max_completion_tokens"], 100);
        assert!(b.get("max_tokens").is_none() && b.get("temperature").is_none());
        let old = build_body_with(
            &req,
            Quirks::detect("openai", "https://api.openai.com/v1", "gpt-4o"),
        );
        assert_eq!(old["max_tokens"], 100);
        assert!(old.get("temperature").is_some());
        // A local server that happens to serve a model called o3 is left alone.
        assert!(
            !Quirks::detect("lmstudio", "http://localhost:1234/v1", "o3-mini")
                .reasoning_model_params
        );
    }

    #[test]
    fn usage_separates_cache_hits_for_deepseek_and_openai() {
        let d = parse_usage(&json!({"prompt_tokens": 1000, "completion_tokens": 20,
            "prompt_cache_hit_tokens": 900, "prompt_cache_miss_tokens": 100,
            "completion_tokens_details": {"reasoning_tokens": 7}}));
        assert_eq!(
            (
                d.input_tokens,
                d.cache_read_tokens,
                d.output_tokens,
                d.reasoning_tokens
            ),
            (100, 900, 20, 7)
        );
        let o = parse_usage(&json!({"prompt_tokens": 50, "completion_tokens": 5,
            "prompt_tokens_details": {"cached_tokens": 40}}));
        assert_eq!((o.input_tokens, o.cache_read_tokens), (10, 40));
        // A server that reports more cached than total never makes the count go negative.
        let odd = parse_usage(&json!({"prompt_tokens": 5, "prompt_cache_hit_tokens": 9}));
        assert_eq!((odd.input_tokens, odd.cache_read_tokens), (0, 5));
        assert!((d.cache_hit_ratio() - 0.9).abs() < 1e-9);
    }

    #[test]
    fn local_hosts_are_recognised() {
        for u in [
            "http://localhost:1234/v1",
            "http://127.0.0.1:11434",
            "http://192.168.1.5:8080/v1",
            "http://box.local/v1",
        ] {
            assert!(is_local_host(u), "{u}");
        }
        assert!(!is_local_host("https://api.deepseek.com/v1"));
    }

    #[test]
    fn failed_model_lists_read_well() {
        let b = serde_json::json!({ "error": { "message": "Authentication Fails" } });
        assert_eq!(
            list_error(401, &b),
            "the API key was not accepted (HTTP 401): Authentication Fails"
        );
        assert_eq!(
            list_error(500, &Value::Null),
            "the server answered with an error (HTTP 500)"
        );
    }

    #[test]
    fn tool_calls_assemble_across_chunks() {
        let mut s = StreamState::default();
        let mut all = Vec::new();
        for data in [
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c9","function":{"name":"read_","arguments":"{\"pa"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"file","arguments":"th\":\"a\"}"}}]},"finish_reason":"tool_calls"}]}"#,
            "[DONE]",
        ] {
            all.extend(s.feed(data).unwrap());
        }
        assert_eq!(all.len(), 2);
        assert!(
            matches!(&all[0], Delta::ToolCall(c) if c.id == "c9" && c.name == "read_file" && c.arguments["path"] == "a")
        );
        assert_eq!(all[1], Delta::Done(StopReason::ToolUse));
    }

    #[test]
    fn text_usage_and_single_done() {
        let mut s = StreamState::default();
        let mut all = s
            .feed(r#"{"choices":[{"delta":{"content":"Hi"}}]}"#)
            .unwrap();
        all.extend(s.feed(r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1}}"#).unwrap());
        all.extend(s.feed("[DONE]").unwrap());
        all.extend(s.finish());
        assert_eq!(all[0], Delta::Text("Hi".into()));
        assert!(matches!(
            all[1],
            Delta::Usage(Usage {
                input_tokens: 3,
                output_tokens: 1,
                ..
            })
        ));
        assert_eq!(
            all.iter().filter(|d| matches!(d, Delta::Done(_))).count(),
            1
        );
    }

    #[test]
    fn bad_json_and_error_objects_are_errors() {
        let mut s = StreamState::default();
        assert!(s.feed("{oops").is_err());
        assert!(s.feed(r#"{"error":{"message":"quota"}}"#).is_err());
    }

    #[test]
    fn body_maps_roles() {
        let mut a = Message::assistant("");
        a.tool_calls.push(ToolCall {
            id: "c1".into(),
            name: "f".into(),
            arguments: json!({"x":1}),
        });
        let req = ChatRequest {
            model: "m".into(),
            system: Some("sys".into()),
            messages: vec![Message::user("u"), a, Message::tool("c1", "res")],
            tools: vec![],
            options: Default::default(),
            vision: true,
        };
        let b = build_body(&req);
        assert_eq!(
            b["messages"][2]["tool_calls"][0]["function"]["arguments"],
            "{\"x\":1}"
        );
        assert_eq!(b["messages"][3]["tool_call_id"], "c1");
        assert!(b["messages"][2]["content"].is_null());
    }

    #[test]
    fn tool_images_emit_followup_user_image_url_only_when_vision_enabled() {
        let mut a = Message::assistant("");
        a.tool_calls.push(ToolCall {
            id: "c1".into(),
            name: "render".into(),
            arguments: json!({}),
        });
        let mut tool = Message::tool("c1", "ok");
        tool.attachments.push(workcraft_core::Attachment {
            name: "r.png".into(),
            mime: "image/png".into(),
            data: "QUJD".into(),
        });
        let req = ChatRequest {
            model: "gpt-4o".into(),
            system: None,
            messages: vec![Message::user("go"), a, tool],
            tools: vec![],
            options: Default::default(),
            vision: true,
        };
        let b = build_body(&req);
        let msgs = b["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[2]["role"], "tool");
        assert_eq!(msgs[3]["role"], "user");
        assert_eq!(msgs[3]["content"][1]["type"], "image_url");

        let b_no_vis = build_body(&ChatRequest {
            vision: false,
            ..req
        });
        assert_eq!(b_no_vis["messages"].as_array().unwrap().len(), 3);
    }
}
