//! MCP server over stdio. WorkCraft uses it to expose its own command bus, so other agents and the
//! Craft apps can drive the chat. Anything that implements [`ToolHandler`] can be served.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use workcraft_core::{Result, ToolResult, ToolSpec};

use crate::jsonrpc::{self, Incoming};

#[async_trait]
pub trait ToolHandler: Send + Sync {
    /// Name and version reported in `initialize`.
    fn info(&self) -> (String, String);
    /// Short guidance for the client model.
    fn instructions(&self) -> Option<String> {
        None
    }
    async fn list_tools(&self) -> Vec<ToolSpec>;
    async fn call_tool(&self, name: &str, arguments: Value) -> ToolResult;
}

/// Answer one JSON-RPC message. Notifications and responses return `None`.
pub async fn handle_message(handler: &dyn ToolHandler, msg: Value) -> Option<Value> {
    match jsonrpc::classify(&msg) {
        Incoming::Request { id, method, params } => {
            Some(handle_request(handler, &id, &method, params).await)
        }
        Incoming::Notification { .. } | Incoming::Response { .. } => None,
        Incoming::Invalid => Some(jsonrpc::response_err(
            &Value::Null,
            jsonrpc::INVALID_REQUEST,
            "invalid request",
        )),
    }
}

pub(crate) async fn handle_request(
    handler: &dyn ToolHandler,
    id: &Value,
    method: &str,
    params: Value,
) -> Value {
    match method {
        "initialize" => {
            let (name, version) = handler.info();
            let mut result = json!({
                "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or(jsonrpc::PROTOCOL),
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": name, "version": version },
            });
            if let Some(i) = handler.instructions() {
                result["instructions"] = json!(i);
            }
            jsonrpc::response_ok(id, result)
        }
        "ping" => jsonrpc::response_ok(id, json!({})),
        "tools/list" => {
            let tools: Vec<Value> = handler
                .list_tools()
                .await
                .into_iter()
                .map(|t| {
                    let mut obj = json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": t.input_schema,
                    });
                    if t.read_only_hint.is_some() || t.destructive {
                        obj["annotations"] = json!({
                            "readOnlyHint": t.read_only_hint.unwrap_or(t.read_only),
                            "destructiveHint": t.destructive,
                        });
                    }
                    obj
                })
                .collect();
            jsonrpc::response_ok(id, json!({ "tools": tools }))
        }
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return jsonrpc::response_err(id, jsonrpc::INVALID_PARAMS, "missing tool name");
            };
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let r = handler.call_tool(name, args).await;
            let mut content = vec![json!({ "type": "text", "text": r.text })];
            for img in r.images {
                content.push(json!({
                    "type": "image",
                    "data": img.data,
                    "mimeType": img.mime_type,
                }));
            }
            jsonrpc::response_ok(id, json!({ "content": content, "isError": !r.ok }))
        }
        other => jsonrpc::response_err(
            id,
            jsonrpc::METHOD_NOT_FOUND,
            &format!("method not found: {other}"),
        ),
    }
}

/// Serve over any line reader and writer. Requests run concurrently, so a long `chat_send` never blocks
/// a `ping`; replies are written one line at a time.
pub async fn serve<R, W>(handler: Arc<dyn ToolHandler>, input: R, mut output: W) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut rx) = mpsc::channel::<Value>(64);
    let writer = tokio::spawn(async move {
        while let Some(v) = rx.recv().await {
            let mut line = v.to_string();
            line.push('\n');
            if output.write_all(line.as_bytes()).await.is_err() || output.flush().await.is_err() {
                break;
            }
        }
    });
    let mut lines = BufReader::new(input).lines();
    let mut tasks = tokio::task::JoinSet::new();
    while let Ok(Some(line)) = lines.next_line().await {
        // Some Windows shells put a UTF-8 byte order mark in front of the first line.
        let line = line.trim_start_matches('\u{feff}').to_string();
        if line.trim().is_empty() {
            continue;
        }
        let tx = tx.clone();
        match serde_json::from_str::<Value>(&line) {
            Ok(msg) => {
                let h = Arc::clone(&handler);
                tasks.spawn(async move {
                    if let Some(reply) = handle_message(h.as_ref(), msg).await {
                        let _ = tx.send(reply).await;
                    }
                });
            }
            Err(e) => {
                let _ = tx
                    .send(jsonrpc::response_err(
                        &Value::Null,
                        jsonrpc::PARSE_ERROR,
                        &format!("parse error: {e}"),
                    ))
                    .await;
            }
        }
    }
    // Input closed: let running calls finish, then flush the writer.
    while tasks.join_next().await.is_some() {}
    drop(tx);
    let _ = writer.await;
    Ok(())
}

/// Serve on this process's stdin and stdout.
pub async fn serve_stdio(handler: Arc<dyn ToolHandler>) -> Result<()> {
    serve(handler, tokio::io::stdin(), tokio::io::stdout()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    #[async_trait]
    impl ToolHandler for Echo {
        fn info(&self) -> (String, String) {
            ("echo".into(), "0.0.1".into())
        }
        async fn list_tools(&self) -> Vec<ToolSpec> {
            vec![ToolSpec::new("say", "say it", json!({"type":"object"}))]
        }
        async fn call_tool(&self, name: &str, arguments: Value) -> ToolResult {
            if name == "say" {
                ToolResult::ok(arguments.to_string())
            } else {
                ToolResult::err("no such tool")
            }
        }
    }

    #[tokio::test]
    async fn handshake_list_call_and_errors() {
        let h = Echo;
        let init = handle_message(&h, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}})).await.unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2024-11-05");
        assert!(
            handle_message(
                &h,
                json!({"jsonrpc":"2.0","method":"notifications/initialized"})
            )
            .await
            .is_none()
        );
        let list = handle_message(&h, json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
            .await
            .unwrap();
        assert_eq!(list["result"]["tools"][0]["name"], "say");
        let call = handle_message(&h, json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"say","arguments":{"a":1}}})).await.unwrap();
        assert_eq!(call["result"]["isError"], false);
        let bad = handle_message(
            &h,
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"x"}}),
        )
        .await
        .unwrap();
        assert_eq!(bad["result"]["isError"], true);
        let nometh = handle_message(&h, json!({"jsonrpc":"2.0","id":5,"method":"nope"}))
            .await
            .unwrap();
        assert_eq!(nometh["error"]["code"], jsonrpc::METHOD_NOT_FOUND);
        let noname = handle_message(
            &h,
            json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{}}),
        )
        .await
        .unwrap();
        assert_eq!(noname["error"]["code"], jsonrpc::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn serves_over_pipes_and_survives_garbage() {
        let (client_in, server_in) = tokio::io::duplex(4096);
        let (server_out, client_out) = tokio::io::duplex(4096);
        let handler: Arc<dyn ToolHandler> = Arc::new(Echo);
        let srv = tokio::spawn(serve(handler, server_in, server_out));
        let mut w = client_in;
        // The BOM before the first request must not break it.
        w.write_all("\u{feff}{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\nnot json\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n".as_bytes()).await.unwrap();
        drop(w);
        let mut lines = BufReader::new(client_out).lines();
        let mut got = Vec::new();
        while let Ok(Some(l)) = lines.next_line().await {
            got.push(serde_json::from_str::<Value>(&l).unwrap());
        }
        srv.await.unwrap().unwrap();
        assert_eq!(got.len(), 3);
        assert!(
            got.iter().any(|v| v["id"] == 7 && v["result"].is_object()),
            "the BOM line was answered"
        );
        assert!(
            got.iter()
                .any(|v| v["error"]["code"] == jsonrpc::PARSE_ERROR)
        );
        assert!(got.iter().any(|v| v["id"] == 1 && v["result"].is_object()));
    }
}
