//! MCP client: stdio child process or Streamable HTTP.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex as AsyncMutex, oneshot};
use workcraft_core::{
    Error, ImagePart, Result, ToolResult, ToolSpec, looks_read_only, truncate_bytes,
};

use crate::config::ServerEntry;
use crate::jsonrpc::{self, Incoming};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(300);
/// Tool output beyond this many bytes is cut before it reaches the model.
const MAX_RESULT_BYTES: usize = 64 * 1024;
/// Maximum decoded size per MCP image part (4 MiB).
pub const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
/// Maximum images kept from a single `tools/call` response.
pub const MAX_IMAGES_PER_CALL: usize = 8;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<std::result::Result<Value, String>>>>>;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

enum Transport {
    Stdio {
        stdin: AsyncMutex<ChildStdin>,
        pending: Pending,
        alive: Arc<AtomicBool>,
        tools_dirty: Arc<AtomicBool>,
        _child: Box<Child>,
    },
    Http {
        http: reqwest::Client,
        url: String,
        headers: Vec<(String, String)>,
        session: Mutex<Option<String>>,
    },
    /// The 2024-11 transport: a long-lived `GET` event stream for replies, a `POST` endpoint (announced by the
    /// stream) for requests. Many servers still speak only this one.
    Sse {
        http: reqwest::Client,
        post_url: String,
        headers: Vec<(String, String)>,
        pending: Pending,
        alive: Arc<AtomicBool>,
        tools_dirty: Arc<AtomicBool>,
    },
    InMemory(Arc<dyn crate::server::ToolHandler>),
}

/// Why an HTTP request failed, so the caller can decide what to do.
enum HttpFail {
    /// Worth another try: the server could not be reached, or said "busy".
    Transient(String),
    /// The session id is no longer known to the server (it restarted): start a new session.
    SessionGone,
    /// A real answer that will not change by asking again.
    Fatal { message: String },
}

pub struct McpClient {
    name: String,
    args: Vec<String>,
    next_id: AtomicU64,
    transport: Transport,
    server_info: Mutex<Option<Value>>,
    tools_cache: AsyncMutex<Option<Vec<ToolSpec>>>,
}

impl McpClient {
    /// Start the server described by `entry`, run the `initialize` handshake and return the client.
    pub async fn connect(name: &str, entry: &ServerEntry) -> Result<Self> {
        if let Some(url) = entry.url.as_deref() {
            return Self::connect_http(name, entry, url).await;
        }
        let transport = if let Some(cmd) = entry.command.as_deref() {
            spawn_stdio(name, cmd, entry)?
        } else {
            return Err(Error::Invalid(format!(
                "mcp server '{name}' needs a command or a url"
            )));
        };
        let client = Self {
            name: name.to_string(),
            args: entry.args.clone(),
            next_id: AtomicU64::new(1),
            transport,
            server_info: Mutex::new(None),
            tools_cache: AsyncMutex::new(None),
        };
        client.handshake().await?;
        Ok(client)
    }

    /// `initialize` and `notifications/initialized`.
    async fn handshake(&self) -> Result<()> {
        let info = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": jsonrpc::PROTOCOL,
                    "capabilities": {},
                    "clientInfo": { "name": "workcraft", "version": env!("CARGO_PKG_VERSION") },
                }),
                REQUEST_TIMEOUT,
            )
            .await?;
        *lock(&self.server_info) = Some(info);
        self.notify("notifications/initialized", json!({})).await
    }

    /// A server given as a URL: Streamable HTTP first; when the server does not know that transport (HTTP
    /// 400, 404, 405 or 406 on the first POST) the older HTTP+SSE transport. A URL ending in `/sse` goes
    /// straight to the older one.
    async fn connect_http(name: &str, entry: &ServerEntry, url: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| Error::Mcp(e.to_string()))?;
        let headers: Vec<(String, String)> = entry
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let make = |transport: Transport| Self {
            name: name.to_string(),
            args: entry.args.clone(),
            next_id: AtomicU64::new(1),
            transport,
            server_info: Mutex::new(None),
            tools_cache: AsyncMutex::new(None),
        };
        let prefer_sse = url.trim_end_matches('/').ends_with("/sse");
        let mut streamable_error = None;
        if !prefer_sse {
            let client = make(Transport::Http {
                http: http.clone(),
                url: url.to_string(),
                headers: headers.clone(),
                session: Mutex::new(None),
            });
            match client.handshake().await {
                Ok(()) => return Ok(client),
                Err(e) if is_old_transport_signal(&e) => streamable_error = Some(e),
                Err(e) => return Err(e),
            }
        }
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let dirty = Arc::new(AtomicBool::new(false));
        let post_url = open_sse(
            name,
            &http,
            url,
            &headers,
            Arc::clone(&pending),
            Arc::clone(&alive),
            Arc::clone(&dirty),
        )
        .await
        .map_err(|e| match &streamable_error {
            Some(first) => Error::Mcp(format!(
                "{name}: Streamable HTTP failed ({first}) and so did the older SSE transport ({e})"
            )),
            None => e,
        })?;
        let client = make(Transport::Sse {
            http,
            post_url,
            headers,
            pending,
            alive,
            tools_dirty: dirty,
        });
        client.handshake().await?;
        Ok(client)
    }

    /// Connect an in-process `ToolHandler` directly without spawning a subprocess.
    pub async fn from_handler(
        name: &str,
        args: Vec<String>,
        handler: Arc<dyn crate::server::ToolHandler>,
    ) -> Result<Self> {
        let client = Self {
            name: name.to_string(),
            args,
            next_id: AtomicU64::new(1),
            transport: Transport::InMemory(handler),
            server_info: Mutex::new(None),
            tools_cache: AsyncMutex::new(None),
        };
        let info = client
            .request(
                "initialize",
                json!({
                    "protocolVersion": jsonrpc::PROTOCOL,
                    "capabilities": {},
                    "clientInfo": { "name": "workcraft", "version": env!("CARGO_PKG_VERSION") },
                }),
                REQUEST_TIMEOUT,
            )
            .await?;
        *lock(&client.server_info) = Some(info);
        Ok(client)
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    #[must_use]
    pub fn server_info(&self) -> Option<Value> {
        lock(&self.server_info).clone()
    }

    /// True while a stdio child process is still running (always true for HTTP).
    #[must_use]
    pub fn is_alive(&self) -> bool {
        match &self.transport {
            Transport::Stdio { alive, .. } | Transport::Sse { alive, .. } => {
                alive.load(Ordering::SeqCst)
            }
            Transport::Http { .. } | Transport::InMemory(_) => true,
        }
    }

    /// OS PID of a spawned stdio MCP server child, if any.
    #[must_use]
    pub fn child_pid(&self) -> Option<u32> {
        match &self.transport {
            Transport::Stdio { _child, .. } => _child.id(),
            Transport::Http { .. } | Transport::Sse { .. } | Transport::InMemory(_) => None,
        }
    }

    /// Force the next `list_tools` call to re-query `tools/list`.
    pub async fn invalidate_tools_cache(&self) {
        *self.tools_cache.lock().await = None;
        if let Transport::Stdio { tools_dirty, .. } | Transport::Sse { tools_dirty, .. } =
            &self.transport
        {
            tools_dirty.store(true, Ordering::SeqCst);
        }
    }

    pub async fn list_tools_raw(&self) -> Result<Vec<Value>> {
        let mut raw_tools = Vec::new();
        let mut cursor: Option<String> = None;
        // Bounded: a broken server that always returns a cursor cannot loop forever.
        for _ in 0..50 {
            let params = cursor
                .as_ref()
                .map_or_else(|| json!({}), |c| json!({ "cursor": c }));
            let r = self.request("tools/list", params, REQUEST_TIMEOUT).await?;
            if let Some(arr) = r.get("tools").and_then(Value::as_array) {
                raw_tools.extend(arr.iter().cloned());
            }
            cursor = r
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(raw_tools)
    }

    pub async fn list_tools(&self) -> Result<Vec<ToolSpec>> {
        let dirty = match &self.transport {
            Transport::Stdio { tools_dirty, .. } | Transport::Sse { tools_dirty, .. } => {
                tools_dirty.swap(false, Ordering::SeqCst)
            }
            Transport::Http { .. } | Transport::InMemory(_) => false,
        };
        if !dirty && let Some(cached) = self.tools_cache.lock().await.clone() {
            return Ok(cached);
        }
        let raw_tools = self.list_tools_raw().await?;
        let mut tools = Vec::with_capacity(raw_tools.len());
        for t in raw_tools {
            let Some(name) = t.get("name").and_then(Value::as_str) else {
                continue;
            };
            let hint = t
                .pointer("/annotations/readOnlyHint")
                .and_then(Value::as_bool);
            let destructive = t
                .pointer("/annotations/destructiveHint")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            tools.push(ToolSpec {
                name: name.to_string(),
                description: t
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                input_schema: t
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
                read_only: hint.unwrap_or_else(|| looks_read_only(name)),
                read_only_hint: hint,
                destructive,
            });
        }
        *self.tools_cache.lock().await = Some(tools.clone());
        Ok(tools)
    }

    pub async fn call_tool(&self, tool: &str, arguments: Value) -> Result<ToolResult> {
        self.call_tool_with_timeout(tool, arguments, DEFAULT_CALL_TIMEOUT)
            .await
    }

    pub async fn call_tool_with_timeout(
        &self,
        tool: &str,
        arguments: Value,
        timeout: Duration,
    ) -> Result<ToolResult> {
        let arguments = if arguments.is_null() {
            json!({})
        } else {
            arguments
        };
        let r = self
            .request(
                "tools/call",
                json!({ "name": tool, "arguments": arguments }),
                timeout,
            )
            .await?;
        Ok(parse_tool_result(&r))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.send(&jsonrpc::notification(method, params)).await
    }

    async fn send(&self, msg: &Value) -> Result<()> {
        match &self.transport {
            Transport::Stdio { stdin, alive, .. } => {
                let mut line = msg.to_string();
                line.push('\n');
                let mut w = stdin.lock().await;
                if let Err(e) = w.write_all(line.as_bytes()).await {
                    alive.store(false, Ordering::SeqCst);
                    return Err(Error::Mcp(format!("{}: server closed: {e}", self.name)));
                }
                if let Err(e) = w.flush().await {
                    alive.store(false, Ordering::SeqCst);
                    return Err(Error::Mcp(format!("{}: server closed: {e}", self.name)));
                }
                Ok(())
            }
            Transport::Http { .. } => self
                .http_post(msg)
                .await
                .map(|_| ())
                .map_err(|f| self.fail(f)),
            Transport::Sse {
                http,
                post_url,
                headers,
                alive,
                ..
            } => {
                let mut rb = http.post(post_url).json(msg);
                for (k, v) in headers {
                    rb = rb.header(k, v);
                }
                match rb.send().await {
                    Ok(r) if r.status().is_success() => Ok(()),
                    Ok(r) => Err(self.fail(http_status_fail(r).await)),
                    Err(e) => {
                        alive.store(false, Ordering::SeqCst);
                        Err(Error::Mcp(format!(
                            "{}: cannot reach the server ({})",
                            self.name,
                            short_cause(&e)
                        )))
                    }
                }
            }
            Transport::InMemory(_) => Ok(()),
        }
    }

    /// An [`HttpFail`] as the error a caller sees.
    fn fail(&self, f: HttpFail) -> Error {
        match f {
            HttpFail::Transient(m) | HttpFail::Fatal { message: m, .. } => {
                Error::Mcp(format!("{}: {m}", self.name))
            }
            HttpFail::SessionGone => Error::Mcp(format!(
                "{}: the server no longer knows this session (it may have restarted)",
                self.name
            )),
        }
    }

    /// Send a request whose reply arrives on a channel the reader task feeds (stdio and SSE).
    async fn roundtrip(
        &self,
        pending: &Pending,
        alive: &AtomicBool,
        id: u64,
        msg: &Value,
        method: &str,
        timeout: Duration,
    ) -> Result<Value> {
        if !alive.load(Ordering::SeqCst) {
            return Err(Error::Mcp(format!("{}: server closed", self.name)));
        }
        let (tx, rx) = oneshot::channel();
        lock(pending).insert(id, tx);
        if let Err(e) = self.send(msg).await {
            lock(pending).remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(m))) => Err(Error::Mcp(format!("{}: {m}", self.name))),
            Ok(Err(_)) => {
                alive.store(false, Ordering::SeqCst);
                Err(Error::Mcp(format!("{}: server closed", self.name)))
            }
            Err(_) => {
                lock(pending).remove(&id);
                let _ = self
                    .notify(
                        "notifications/cancelled",
                        json!({ "requestId": id, "reason": "timeout" }),
                    )
                    .await;
                Err(Error::Timeout(format!(
                    "{} {method} timed out after {}s",
                    self.name,
                    timeout.as_secs()
                )))
            }
        }
    }

    async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        if let Transport::InMemory(handler) = &self.transport {
            let resp = tokio::time::timeout(
                timeout,
                crate::server::handle_request(handler.as_ref(), &json!(id), method, params),
            )
            .await
            .map_err(|_| {
                Error::Timeout(format!(
                    "{} {method} timed out after {}s",
                    self.name,
                    timeout.as_secs()
                ))
            })?;
            if let Some(err) = resp.get("error") {
                let msg = err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("mcp error");
                return Err(Error::Mcp(format!("{}: {msg}", self.name)));
            }
            return Ok(resp.get("result").cloned().unwrap_or(Value::Null));
        }
        let msg = jsonrpc::request(id, method, params);
        match &self.transport {
            Transport::Stdio { pending, alive, .. } | Transport::Sse { pending, alive, .. } => {
                self.roundtrip(pending, alive, id, &msg, method, timeout)
                    .await
            }
            Transport::Http { .. } => self.http_request(&msg, id, method, timeout).await,
            Transport::InMemory(_) => Ok(Value::Null),
        }
    }

    /// A request over Streamable HTTP. A busy or unreachable server is asked again (twice, with a growing
    /// pause), a session the server forgot is started anew once, and everything else is reported as it is.
    async fn http_request(
        &self,
        msg: &Value,
        id: u64,
        method: &str,
        timeout: Duration,
    ) -> Result<Value> {
        let mut retries = 0u32;
        let mut renewed = false;
        loop {
            let attempt = tokio::time::timeout(timeout, self.http_post_for(msg, id))
                .await
                .map_err(|_| {
                    Error::Timeout(format!(
                        "{} {method} timed out after {}s",
                        self.name,
                        timeout.as_secs()
                    ))
                })?;
            match attempt {
                Ok(reply) => return reply.map_err(|m| Error::Mcp(format!("{}: {m}", self.name))),
                Err(HttpFail::SessionGone) if !renewed && method != "initialize" => {
                    renewed = true;
                    if let Transport::Http { session, .. } = &self.transport {
                        *lock(session) = None;
                    }
                    Box::pin(self.handshake()).await?;
                }
                // Only calls that cannot have run twice are repeated: a request that never reached the server
                // (`tools/call` included), or one answered "busy".
                Err(HttpFail::Transient(m)) if retries < 2 => {
                    retries += 1;
                    tokio::time::sleep(Duration::from_millis(600 * u64::from(retries))).await;
                    let _ = m;
                }
                Err(f) => return Err(self.fail(f)),
            }
        }
    }

    async fn http_post(
        &self,
        msg: &Value,
    ) -> std::result::Result<Option<reqwest::Response>, HttpFail> {
        let Transport::Http {
            http,
            url,
            headers,
            session,
        } = &self.transport
        else {
            return Ok(None);
        };
        let mut rb = http
            .post(url)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", jsonrpc::PROTOCOL)
            .json(msg);
        for (k, v) in headers {
            rb = rb.header(k, v);
        }
        let had_session = lock(session).clone();
        if let Some(s) = &had_session {
            rb = rb.header("Mcp-Session-Id", s);
        }
        let resp = rb.send().await.map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                HttpFail::Transient(format!("cannot reach {url} ({})", short_cause(&e)))
            } else {
                HttpFail::Fatal {
                    message: format!("request failed ({})", short_cause(&e)),
                }
            }
        })?;
        if let Some(s) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            *lock(session) = Some(s.to_string());
        }
        if resp.status().is_success() {
            return Ok(Some(resp));
        }
        if resp.status().as_u16() == 404 && had_session.is_some() {
            return Err(HttpFail::SessionGone);
        }
        Err(http_status_fail(resp).await)
    }

    /// POST a request and read the reply with the matching id, from a JSON body or an SSE stream.
    async fn http_post_for(
        &self,
        msg: &Value,
        id: u64,
    ) -> std::result::Result<std::result::Result<Value, String>, HttpFail> {
        let Some(resp) = self.http_post(msg).await? else {
            return Err(HttpFail::Fatal {
                message: "not an http transport".into(),
            });
        };
        let broke = |e: reqwest::Error| HttpFail::Fatal {
            message: format!("the reply broke off ({})", short_cause(&e)),
        };
        let is_sse = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.contains("text/event-stream"));
        if !is_sse {
            let v: Value = resp.json().await.map_err(broke)?;
            return Ok(find_reply(&v, id).unwrap_or_else(|| Err("no reply for request".into())));
        }
        let mut stream = resp.bytes_stream();
        let mut lines = LineSplitter::default();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(broke)?;
            for line in lines.push(&chunk) {
                if let Some(data) = line.strip_prefix("data:")
                    && let Ok(v) = serde_json::from_str::<Value>(data.trim())
                    && let Some(r) = find_reply(&v, id)
                {
                    return Ok(r);
                }
            }
        }
        Err(HttpFail::Fatal {
            message: "the stream ended without a reply".into(),
        })
    }
}

/// Turn a non-success HTTP answer into a failure with a message a person can act on.
async fn http_status_fail(resp: reqwest::Response) -> HttpFail {
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    let detail = truncate_bytes(body.trim(), 300);
    let hint = match status {
        401 | 403 => " The server wants credentials: add them under `headers` in mcp.json",
        404 => " The address is wrong, or the server does not use this transport",
        _ => "",
    };
    let message = format!(
        "HTTP {status}{}{hint}",
        if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}.")
        }
    );
    if matches!(status, 429 | 502 | 503 | 504) {
        HttpFail::Transient(message)
    } else {
        HttpFail::Fatal { message }
    }
}

/// The root cause of a request error, without the URL (which can carry a key in its query string).
fn short_cause(e: &reqwest::Error) -> String {
    let mut msg = String::new();
    let mut src: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
    while let Some(s) = src {
        msg = s.to_string();
        src = s.source();
    }
    if msg.is_empty() {
        if e.is_timeout() {
            "timed out".to_string()
        } else {
            "connection failed".to_string()
        }
    } else {
        msg.chars().take(160).collect()
    }
}

/// Whether a failed first request means "this server does not speak Streamable HTTP".
fn is_old_transport_signal(e: &Error) -> bool {
    let s = e.to_string();
    ["HTTP 400", "HTTP 404", "HTTP 405", "HTTP 406"]
        .iter()
        .any(|m| s.contains(m))
}

/// Open the older SSE transport: `GET` the event stream, wait for the `endpoint` event, and keep reading
/// replies on a background task. Returns the URL to `POST` requests to.
async fn open_sse(
    name: &str,
    http: &reqwest::Client,
    url: &str,
    headers: &[(String, String)],
    pending: Pending,
    alive: Arc<AtomicBool>,
    tools_dirty: Arc<AtomicBool>,
) -> Result<String> {
    let mut rb = http.get(url).header("Accept", "text/event-stream");
    for (k, v) in headers {
        rb = rb.header(k, v);
    }
    let resp = rb
        .send()
        .await
        .map_err(|e| Error::Mcp(format!("{name}: cannot reach {url} ({})", short_cause(&e))))?;
    if !resp.status().is_success() {
        let f = http_status_fail(resp).await;
        return Err(match f {
            HttpFail::Transient(m) | HttpFail::Fatal { message: m, .. } => {
                Error::Mcp(format!("{name}: {m}"))
            }
            HttpFail::SessionGone => Error::Mcp(format!("{name}: session gone")),
        });
    }
    let (ep_tx, ep_rx) = oneshot::channel::<String>();
    let base = url.to_string();
    tokio::spawn(async move {
        let mut ep_tx = Some(ep_tx);
        let mut stream = resp.bytes_stream();
        let mut lines = LineSplitter::default();
        let mut event = String::new();
        let mut data = String::new();
        while let Some(Ok(chunk)) = stream.next().await {
            for line in lines.push(&chunk) {
                if line.is_empty() {
                    let ev = std::mem::take(&mut event);
                    let d = std::mem::take(&mut data);
                    if d.is_empty() {
                        continue;
                    }
                    if ev == "endpoint" {
                        if let Some(tx) = ep_tx.take() {
                            let _ = tx.send(resolve_endpoint(&base, d.trim()));
                        }
                    } else if let Ok(v) = serde_json::from_str::<Value>(&d) {
                        match jsonrpc::classify(&v) {
                            Incoming::Response { id, result } => {
                                if let Some(tx) = lock(&pending).remove(&id) {
                                    let _ = tx.send(result);
                                }
                            }
                            Incoming::Notification { method, .. }
                                if method == "notifications/tools/list_changed" =>
                            {
                                tools_dirty.store(true, Ordering::SeqCst);
                            }
                            _ => {}
                        }
                    }
                } else if let Some(v) = line.strip_prefix("event:") {
                    event = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("data:") {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(v.strip_prefix(' ').unwrap_or(v));
                }
            }
        }
        alive.store(false, Ordering::SeqCst);
        let waiting: Vec<_> = lock(&pending).drain().collect();
        for (_, tx) in waiting {
            let _ = tx.send(Err("server exited".to_string()));
        }
    });
    match tokio::time::timeout(Duration::from_secs(10), ep_rx).await {
        Ok(Ok(u)) => Ok(u),
        Ok(Err(_)) => Err(Error::Mcp(format!(
            "{name}: the event stream closed before it named a message endpoint"
        ))),
        Err(_) => Err(Error::Timeout(format!(
            "{name}: the event stream did not name a message endpoint within 10 s"
        ))),
    }
}

/// The endpoint an SSE server announces is a path (`/messages?id=1`) or a full URL.
fn resolve_endpoint(base: &str, endpoint: &str) -> String {
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        return endpoint.to_string();
    }
    let scheme_end = base.find("://").map_or(0, |i| i + 3);
    let rest = &base[scheme_end..];
    let origin_end = scheme_end + rest.find('/').unwrap_or(rest.len());
    let origin = &base[..origin_end];
    if endpoint.starts_with('/') {
        format!("{origin}{endpoint}")
    } else {
        let dir = base[origin_end..].rsplit_once('/').map_or("", |(d, _)| d);
        format!("{origin}{dir}/{endpoint}")
    }
}
fn find_reply(v: &Value, id: u64) -> Option<std::result::Result<Value, String>> {
    let items: Vec<&Value> = match v {
        Value::Array(a) => a.iter().collect(),
        other => vec![other],
    };
    items
        .into_iter()
        .find_map(|item| match jsonrpc::classify(item) {
            Incoming::Response { id: rid, result } if rid == id => Some(result),
            _ => None,
        })
}

#[derive(Default)]
struct LineSplitter(Vec<u8>);

impl LineSplitter {
    fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.0.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.0.iter().position(|b| *b == b'\n') {
            let rest = self.0.split_off(pos + 1);
            let line = std::mem::replace(&mut self.0, rest);
            out.push(String::from_utf8_lossy(&line).trim_end().to_string());
        }
        out
    }
}

fn spawn_stdio(name: &str, cmd: &str, entry: &ServerEntry) -> Result<Transport> {
    let mut child = match spawn(cmd, entry) {
        Ok(c) => c,
        // npx, uvx and friends are .cmd files on Windows; Rust only resolves .exe on its own.
        Err(e)
            if cfg!(windows) && e.kind() == std::io::ErrorKind::NotFound && !cmd.contains('.') =>
        {
            spawn(&format!("{cmd}.cmd"), entry)
                .map_err(|e2| Error::Mcp(format!("{name}: cannot start '{cmd}': {e2}")))?
        }
        Err(e) => return Err(Error::Mcp(format!("{name}: cannot start '{cmd}': {e}"))),
    };
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::Mcp(format!("{name}: no stdin")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Mcp(format!("{name}: no stdout")))?;
    let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
    let alive = Arc::new(AtomicBool::new(true));
    let tools_dirty = Arc::new(AtomicBool::new(false));
    let p2 = Arc::clone(&pending);
    let alive2 = Arc::clone(&alive);
    let dirty2 = Arc::clone(&tools_dirty);
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            match jsonrpc::classify(&v) {
                Incoming::Response { id, result } => {
                    if let Some(tx) = lock(&p2).remove(&id) {
                        let _ = tx.send(result);
                    }
                }
                Incoming::Notification { method, .. }
                    if method == "notifications/tools/list_changed" =>
                {
                    dirty2.store(true, Ordering::SeqCst);
                }
                _ => {}
            }
        }
        alive2.store(false, Ordering::SeqCst);
        // The server went away: fail every request still waiting.
        let waiting: Vec<_> = lock(&p2).drain().collect();
        for (_, tx) in waiting {
            let _ = tx.send(Err("server exited".to_string()));
        }
    });
    Ok(Transport::Stdio {
        stdin: AsyncMutex::new(stdin),
        pending,
        alive,
        tools_dirty,
        _child: Box::new(child),
    })
}

fn spawn(cmd: &str, entry: &ServerEntry) -> std::io::Result<Child> {
    let mut c = Command::new(cmd);
    c.args(&entry.args)
        .envs(&entry.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // No console window flashing up for every Craft app server.
        c.creation_flags(0x0800_0000);
    }
    c.spawn()
}

/// Turn a `tools/call` result into text and retained image parts.
#[must_use]
pub fn parse_tool_result(r: &Value) -> ToolResult {
    let mut parts: Vec<String> = Vec::new();
    let mut images: Vec<ImagePart> = Vec::new();
    if let Some(content) = r.get("content").and_then(Value::as_array) {
        for c in content {
            match c.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(t) = c.get("text").and_then(Value::as_str) {
                        parts.push(t.to_string());
                    }
                }
                Some("image") => {
                    let mime = c
                        .get("mimeType")
                        .or_else(|| c.get("mime_type"))
                        .and_then(Value::as_str)
                        .unwrap_or("image/png");
                    let data = c.get("data").and_then(Value::as_str).unwrap_or_default();
                    let approx_bytes = data.len().saturating_mul(3) / 4;
                    if !data.is_empty()
                        && images.len() < MAX_IMAGES_PER_CALL
                        && approx_bytes <= MAX_IMAGE_BYTES
                    {
                        images.push(ImagePart {
                            mime_type: mime.to_string(),
                            data: data.to_string(),
                        });
                        parts.push(format!("[image: {mime}]"));
                    } else if approx_bytes > MAX_IMAGE_BYTES {
                        parts.push(format!(
                            "[image omitted: {mime} exceeds 4 MiB ({approx_bytes} bytes)]"
                        ));
                    } else if images.len() >= MAX_IMAGES_PER_CALL {
                        parts.push(format!(
                            "[image omitted: {mime} exceeds {MAX_IMAGES_PER_CALL} images per call]"
                        ));
                    } else {
                        parts.push(format!("[image: {mime}]"));
                    }
                }
                Some("resource") => {
                    let res = &c["resource"];
                    match res.get("text").and_then(Value::as_str) {
                        Some(t) => parts.push(t.to_string()),
                        None => parts.push(format!(
                            "[resource: {}]",
                            res.get("uri").and_then(Value::as_str).unwrap_or("?")
                        )),
                    }
                }
                _ => {}
            }
        }
    }
    if parts.is_empty()
        && let Some(s) = r.get("structuredContent")
    {
        parts.push(s.to_string());
    }
    let text = truncate_bytes(&parts.join("\n"), MAX_RESULT_BYTES);
    let res = if r.get("isError").and_then(Value::as_bool) == Some(true) {
        ToolResult::err(text)
    } else {
        ToolResult::ok(text)
    };
    res.with_images(images)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_results_become_text_and_images() {
        // 1x1 transparent PNG in base64.
        let png_1x1 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        let r = parse_tool_result(
            &json!({"content":[{"type":"text","text":"a"},{"type":"image","mimeType":"image/png","data":png_1x1},{"type":"text","text":"b"}]}),
        );
        assert!(r.ok);
        assert_eq!(r.text, "a\n[image: image/png]\nb");
        assert_eq!(r.images.len(), 1);
        assert_eq!(r.images[0].mime_type, "image/png");
        assert_eq!(r.images[0].data, png_1x1);

        let e =
            parse_tool_result(&json!({"content":[{"type":"text","text":"bad"}],"isError":true}));
        assert!(!e.ok);
        let s = parse_tool_result(&json!({"structuredContent":{"n":1}}));
        assert_eq!(s.text, "{\"n\":1}");
    }

    #[test]
    fn parse_tool_result_caps_image_count_and_size() {
        let mut items = Vec::new();
        for _ in 0..10 {
            items.push(json!({"type": "image", "mimeType": "image/png", "data": "QUJDRA=="}));
        }
        let r = parse_tool_result(&json!({"content": items}));
        assert_eq!(r.images.len(), MAX_IMAGES_PER_CALL);
        assert!(r.text.contains("image omitted"));

        let huge = "A".repeat((MAX_IMAGE_BYTES * 4 / 3) + 128);
        let r2 = parse_tool_result(
            &json!({"content": [{"type": "image", "mimeType": "image/png", "data": huge}]}),
        );
        assert!(r2.images.is_empty());
        assert!(r2.text.contains("exceeds 4 MiB"));
    }

    #[test]
    fn finds_reply_in_batches() {
        let v = json!([{"jsonrpc":"2.0","method":"notifications/progress"},{"jsonrpc":"2.0","id":3,"result":{"ok":true}}]);
        assert_eq!(find_reply(&v, 3), Some(Ok(json!({"ok":true}))));
        assert_eq!(find_reply(&v, 4), None);
    }

    #[tokio::test]
    async fn bad_entry_is_an_error() {
        assert!(
            McpClient::connect("x", &ServerEntry::default())
                .await
                .is_err()
        );
        let e = ServerEntry {
            command: Some("definitely-not-a-real-binary-xyz".into()),
            ..ServerEntry::default()
        };
        assert!(McpClient::connect("x", &e).await.is_err());
    }

    // ---- a tiny HTTP server for the transport tests -------------------------------------------------

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    struct Req {
        method: String,
        path: String,
        headers: Vec<(String, String)>,
        body: String,
    }

    async fn read_req(sock: &mut TcpStream) -> Option<Req> {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let head_end = loop {
            let n = sock.read(&mut tmp).await.ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i;
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let mut lines = head.lines();
        let mut first = lines.next()?.split_whitespace();
        let (method, path) = (first.next()?.to_string(), first.next()?.to_string());
        let headers: Vec<(String, String)> = lines
            .filter_map(|l| {
                l.split_once(':')
                    .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            })
            .collect();
        let len: usize = headers
            .iter()
            .find(|(k, _)| k == "content-length")
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);
        let mut body = buf[head_end + 4..].to_vec();
        while body.len() < len {
            let n = sock.read(&mut tmp).await.ok()?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
        Some(Req {
            method,
            path,
            headers,
            body: String::from_utf8_lossy(&body).to_string(),
        })
    }

    /// The reply a mock MCP server gives to a JSON-RPC message, if it is a request.
    fn mock_reply(body: &str) -> Option<String> {
        let v: Value = serde_json::from_str(body).ok()?;
        let id = v.get("id")?.clone();
        let result = match v["method"].as_str()? {
            "initialize" => {
                json!({"protocolVersion": "2025-03-26", "capabilities": {}, "serverInfo": {"name": "mock", "version": "1"}})
            }
            "tools/list" => {
                json!({"tools": [{"name": "echo", "description": "d", "inputSchema": {"type": "object"}}]})
            }
            "tools/call" => json!({"content": [{"type": "text", "text": "pong"}]}),
            _ => json!({}),
        };
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string())
    }

    /// An old-style SSE server: any `GET` is the event stream, `POST /messages` takes requests.
    async fn sse_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let rx = Arc::new(tokio::sync::Mutex::new(rx));
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let (tx, rx) = (tx.clone(), Arc::clone(&rx));
                tokio::spawn(async move {
                    let Some(req) = read_req(&mut sock).await else {
                        return;
                    };
                    if req.method == "GET" {
                        let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\nevent: endpoint\ndata: /messages\n\n";
                        let _ = sock.write_all(head.as_bytes()).await;
                        let mut rx = rx.lock().await;
                        while let Some(m) = rx.recv().await {
                            let ev = format!("event: message\ndata: {m}\n\n");
                            if sock.write_all(ev.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                    } else if req.method == "POST" && req.path == "/messages" {
                        let _ = sock.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                        if let Some(r) = mock_reply(&req.body) {
                            let _ = tx.send(r);
                        }
                    } else {
                        let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    }
                });
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn the_older_sse_transport_works_by_url_suffix() {
        let base = sse_server().await;
        let e = ServerEntry {
            url: Some(format!("{base}/sse")),
            ..ServerEntry::default()
        };
        let c = McpClient::connect("old", &e).await.unwrap();
        let tools = c.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
        let r = c.call_tool("echo", json!({})).await.unwrap();
        assert_eq!(r.text, "pong");
        assert!(c.is_alive());
    }

    #[tokio::test]
    async fn a_url_that_refuses_streamable_http_falls_back_to_sse() {
        let base = sse_server().await;
        // POST /mcp is a 404 on this server, which is how a server that only knows SSE answers.
        let e = ServerEntry {
            url: Some(format!("{base}/mcp")),
            ..ServerEntry::default()
        };
        let c = McpClient::connect("old2", &e).await.unwrap();
        assert_eq!(c.call_tool("echo", json!({})).await.unwrap().text, "pong");
    }

    #[test]
    fn endpoints_resolve_against_the_stream_url() {
        assert_eq!(
            resolve_endpoint("http://h:1/sse", "/messages?s=1"),
            "http://h:1/messages?s=1"
        );
        assert_eq!(
            resolve_endpoint("http://h:1/a/sse", "msg"),
            "http://h:1/a/msg"
        );
        assert_eq!(resolve_endpoint("http://h", "/m"), "http://h/m");
        assert_eq!(
            resolve_endpoint("http://h/sse", "https://x/y"),
            "https://x/y"
        );
    }

    /// A Streamable HTTP server that is busy once and forgets its session once, like a restarted service.
    async fn flaky_http_server() -> (String, Arc<std::sync::atomic::AtomicU32>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let (calls, s2) = (
            Arc::new(std::sync::atomic::AtomicU32::new(0)),
            Arc::clone(&state),
        );
        let calls2 = Arc::clone(&calls);
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let (calls, inits) = (Arc::clone(&calls2), Arc::clone(&s2));
                tokio::spawn(async move {
                    let Some(req) = read_req(&mut sock).await else {
                        return;
                    };
                    let n = calls.fetch_add(1, Ordering::SeqCst);
                    let session = req
                        .headers
                        .iter()
                        .find(|(k, _)| k == "mcp-session-id")
                        .map(|(_, v)| v.clone());
                    let method = serde_json::from_str::<Value>(&req.body)
                        .ok()
                        .and_then(|v| v["method"].as_str().map(str::to_string));
                    let reply = |status: &str, extra: &str, body: &str| {
                        format!(
                            "HTTP/1.1 {status}\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    };
                    let out = if n == 0 {
                        // The very first request meets a busy server.
                        reply("503 Service Unavailable", "", "")
                    } else if method.as_deref() == Some("initialize") {
                        let k = inits.fetch_add(1, Ordering::SeqCst) + 1;
                        reply(
                            "200 OK",
                            &format!("Mcp-Session-Id: s{k}\r\n"),
                            &mock_reply(&req.body).unwrap_or_default(),
                        )
                    } else if method.as_deref() == Some("tools/list")
                        && session.as_deref() == Some("s1")
                    {
                        // The first session has been forgotten.
                        reply("404 Not Found", "", "session unknown")
                    } else if let Some(r) = mock_reply(&req.body) {
                        reply("200 OK", "", &r)
                    } else {
                        reply("202 Accepted", "", "")
                    };
                    let _ = sock.write_all(out.as_bytes()).await;
                });
            }
        });
        (format!("http://{addr}/mcp"), state)
    }

    #[tokio::test]
    async fn streamable_http_retries_a_busy_server_and_renews_a_forgotten_session() {
        let (url, inits) = flaky_http_server().await;
        let e = ServerEntry {
            url: Some(url),
            ..ServerEntry::default()
        };
        let c = McpClient::connect("flaky", &e).await.unwrap();
        // tools/list is answered 404 for session s1: the client starts session s2 and asks again.
        let tools = c.list_tools().await.unwrap();
        assert_eq!(tools[0].name, "echo");
        assert_eq!(inits.load(Ordering::SeqCst), 2, "one new session");
    }
}
