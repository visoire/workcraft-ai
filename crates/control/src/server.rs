//! Control channel server: authenticated loopback TCP, one JSON line per request.
//!
//! ```text
//! -> {"id":"auth","method":"auth","params":{"token":"<64 hex>"}}
//! <- {"id":"auth","result":{"ok":true}}
//! -> {"id":1,"method":"engine.execute","params":{"command":"chat.new","params":{}}}
//! <- {"id":1,"result":{...}}
//! -> {"id":2,"method":"engine.commands"}
//! -> {"id":3,"method":"events.subscribe"}      then {"event":{...}} lines follow
//! ```

use std::net::SocketAddr;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use workcraft_core::{Error, Result};

use crate::{CommandBus, tokens_equal};

/// One request line may not be longer than this.
const MAX_LINE: u64 = 1024 * 1024;

pub struct ControlServer {
    pub addr: SocketAddr,
    pub token: String,
    task: tokio::task::JoinHandle<()>,
}

impl ControlServer {
    /// Listen on `127.0.0.1:port` (0 picks a free port). Only loopback is ever bound.
    pub async fn start(bus: Arc<dyn CommandBus>, port: u16, token: String) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|e| Error::Io(format!("control port {port}: {e}")))?;
        let addr = listener.local_addr()?;
        let tok = token.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, peer)) = listener.accept().await else {
                    continue;
                };
                if !peer.ip().is_loopback() {
                    continue;
                }
                let bus = Arc::clone(&bus);
                let tok = tok.clone();
                tokio::spawn(async move {
                    let _ = handle_connection(stream, bus, &tok).await;
                });
            }
        });
        Ok(Self { addr, token, task })
    }

    pub fn stop(&self) {
        self.task.abort();
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn reply(id: &Value, result: Result<Value>) -> Value {
    match result {
        Ok(v) => json!({ "id": id, "result": v }),
        Err(e) => json!({ "id": id, "error": e.to_string() }),
    }
}

async fn handle_connection(
    stream: TcpStream,
    bus: Arc<dyn CommandBus>,
    token: &str,
) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let (tx, mut rx) = mpsc::channel::<Value>(256);
    let writer = tokio::spawn(async move {
        while let Some(v) = rx.recv().await {
            let mut line = v.to_string();
            line.push('\n');
            if write.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    });

    let mut reader = BufReader::new(read.take(MAX_LINE));
    let mut authed = false;
    let mut buf = String::new();
    loop {
        buf.clear();
        reader.get_mut().set_limit(MAX_LINE);
        let n = reader.read_line(&mut buf).await?;
        if n == 0 || (n as u64 >= MAX_LINE && !buf.ends_with('\n')) {
            break;
        }
        let Ok(req) = serde_json::from_str::<Value>(buf.trim().trim_start_matches('\u{feff}'))
        else {
            let _ = tx
                .send(json!({ "id": null, "error": "invalid json" }))
                .await;
            continue;
        };
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let params = req.get("params").cloned().unwrap_or(Value::Null);

        if !authed {
            let given = params
                .get("token")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if method == "auth" && tokens_equal(given, token) {
                authed = true;
                let _ = tx.send(json!({ "id": id, "result": { "ok": true } })).await;
                continue;
            }
            // Wrong or missing token: say so once and hang up.
            let _ = tx.send(json!({ "id": id, "error": "unauthorized" })).await;
            break;
        }

        match method.as_str() {
            "auth" => {
                let _ = tx.send(json!({ "id": id, "result": { "ok": true } })).await;
            }
            "engine.commands" => {
                let list = serde_json::to_value(bus.commands()).map_err(Error::from);
                let _ = tx.send(reply(&id, list)).await;
            }
            "events.subscribe" => {
                let mut events = bus.subscribe();
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    while let Ok(ev) = events.recv().await {
                        let Ok(v) = serde_json::to_value(&ev) else {
                            continue;
                        };
                        if tx2.send(json!({ "event": v })).await.is_err() {
                            break;
                        }
                    }
                });
                let _ = tx
                    .send(json!({ "id": id, "result": { "subscribed": true } }))
                    .await;
            }
            "engine.execute" => {
                let command = params
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let cmd_params = params.get("params").cloned().unwrap_or(Value::Null);
                // Commands such as chat.send take a while; run them without blocking the connection.
                let bus = Arc::clone(&bus);
                let tx = tx.clone();
                tokio::spawn(async move {
                    let r = if command.is_empty() {
                        Err(Error::Invalid("missing command".into()))
                    } else {
                        bus.execute(&command, cmd_params).await
                    };
                    let _ = tx.send(reply(&id, r)).await;
                });
            }
            other => {
                let _ = tx
                    .send(json!({ "id": id, "error": format!("unknown method '{other}'") }))
                    .await;
            }
        }
    }
    drop(tx);
    let _ = writer.await;
    Ok(())
}
