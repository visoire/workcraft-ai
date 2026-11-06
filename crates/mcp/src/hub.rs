//! All connected MCP servers behind one namespace: tools are named `<server>__<tool>`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::RwLock;
use workcraft_core::{Error, Result, ToolResult, ToolSpec};

use crate::client::{DEFAULT_CALL_TIMEOUT, McpClient};
use crate::config::{McpConfig, ServerEntry};

pub const SEPARATOR: &str = "__";

/// Server names become part of tool names, so keep them to characters every model API accepts.
#[must_use]
pub fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    s.replace(SEPARATOR, "_")
}

#[must_use]
pub fn split_name(full: &str) -> Option<(&str, &str)> {
    full.split_once(SEPARATOR)
}

#[derive(Default, Clone)]
pub struct McpHub {
    clients: Arc<RwLock<BTreeMap<String, Arc<McpClient>>>>,
    entries: Arc<RwLock<BTreeMap<String, ServerEntry>>>,
    last_used: Arc<RwLock<BTreeMap<String, Instant>>>,
    last_error: Arc<RwLock<BTreeMap<String, String>>>,
    /// Servers that failed to reconnect: when to try again and how many times it failed in a row. Without
    /// this a dead server is restarted (and waited for) on every model step.
    cooldown: Arc<RwLock<BTreeMap<String, (Instant, u32)>>>,
}

/// A server that cannot be started is left alone for this long, doubling per failure up to the cap.
const COOLDOWN_BASE: Duration = Duration::from_secs(5);
const COOLDOWN_CAP: Duration = Duration::from_secs(300);
/// Longest a server may take to start, and to list its tools.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const LIST_TIMEOUT: Duration = Duration::from_secs(15);

/// Start a server with a time limit, so a server that hangs in its handshake cannot hold anything up.
async fn connect_within(name: &str, entry: &ServerEntry) -> Result<McpClient> {
    match tokio::time::timeout(CONNECT_TIMEOUT, McpClient::connect(name, entry)).await {
        Ok(r) => r,
        Err(_) => Err(Error::Timeout(format!(
            "{name} did not answer the handshake within {} s",
            CONNECT_TIMEOUT.as_secs()
        ))),
    }
}
/// How one server fared at startup.
#[derive(Debug, Clone)]
pub struct ConnectReport {
    pub server: String,
    pub error: Option<String>,
    pub tools: usize,
}

impl McpHub {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Connect every enabled server in `config`, in parallel. One failing server never blocks the others.
    pub async fn connect_all(&self, config: &McpConfig) -> Vec<ConnectReport> {
        let mut tasks = Vec::new();
        for (name, entry) in config.servers.iter().filter(|(_, e)| !e.disabled) {
            let name = name.clone();
            let entry = entry.clone();
            tasks.push(tokio::spawn(async move {
                let r = connect_within(&sanitize(&name), &entry).await;
                (name, entry, r)
            }));
        }
        let mut reports = Vec::new();
        for t in tasks {
            let Ok((name, entry, result)) = t.await else {
                continue;
            };
            let key = sanitize(&name);
            match result {
                Ok(client) => {
                    let tools = client.list_tools().await.map(|t| t.len()).unwrap_or(0);
                    self.clients
                        .write()
                        .await
                        .insert(key.clone(), Arc::new(client));
                    self.entries.write().await.insert(key.clone(), entry);
                    self.last_used
                        .write()
                        .await
                        .insert(key.clone(), Instant::now());
                    self.last_error.write().await.remove(&key);
                    reports.push(ConnectReport {
                        server: name,
                        error: None,
                        tools,
                    });
                }
                Err(e) => {
                    let msg = e.to_string();
                    self.entries.write().await.insert(key.clone(), entry);
                    self.last_error.write().await.insert(key, msg.clone());
                    reports.push(ConnectReport {
                        server: name,
                        error: Some(msg),
                        tools: 0,
                    });
                }
            }
        }
        reports
    }

    pub async fn add(&self, client: McpClient) {
        let key = client.name().to_string();
        self.last_used
            .write()
            .await
            .insert(key.clone(), Instant::now());
        self.last_error.write().await.remove(&key);
        self.clients.write().await.insert(key, Arc::new(client));
    }

    /// Connect or replace a single server in place and return its tool count.
    pub async fn connect_one(&self, name: &str, entry: &ServerEntry) -> Result<usize> {
        let key = sanitize(name);
        self.entries
            .write()
            .await
            .insert(key.clone(), entry.clone());
        match connect_within(&key, entry).await {
            Ok(client) => {
                let tools = client.list_tools().await.map(|t| t.len()).unwrap_or(0);
                self.cooldown.write().await.remove(&key);
                self.clients
                    .write()
                    .await
                    .insert(key.clone(), Arc::new(client));
                self.last_used
                    .write()
                    .await
                    .insert(key.clone(), Instant::now());
                self.last_error.write().await.remove(&key);
                Ok(tools)
            }
            Err(e) => {
                self.last_error.write().await.insert(key, e.to_string());
                Err(e)
            }
        }
    }

    pub async fn remove(&self, name: &str) -> bool {
        let key = sanitize(name);
        self.last_used.write().await.remove(&key);
        self.clients.write().await.remove(&key).is_some()
    }

    /// Disconnect and drop every active server in this hub.
    pub async fn clear(&self) {
        self.clients.write().await.clear();
        self.last_used.write().await.clear();
        self.last_error.write().await.clear();
    }

    /// OS PID of the stdio child process backing `name`, if connected via stdio.
    pub async fn child_pid(&self, name: &str) -> Option<u32> {
        self.clients
            .read()
            .await
            .get(&sanitize(name))
            .and_then(|c| c.child_pid())
    }

    pub async fn is_connected(&self, name: &str) -> bool {
        let key = sanitize(name);
        self.clients
            .read()
            .await
            .get(&key)
            .is_some_and(|c| c.is_alive())
    }

    pub async fn last_error(&self, name: &str) -> Option<String> {
        self.last_error.read().await.get(&sanitize(name)).cloned()
    }

    pub async fn record_error(&self, name: &str, err: impl Into<String>) {
        self.last_error
            .write()
            .await
            .insert(sanitize(name), err.into());
    }

    pub async fn client_args(&self, name: &str) -> Option<Vec<String>> {
        self.clients
            .read()
            .await
            .get(&sanitize(name))
            .map(|c| c.args().to_vec())
    }

    pub async fn server_tools(&self, name: &str) -> Option<Vec<ToolSpec>> {
        let client = self.clients.read().await.get(&sanitize(name)).cloned()?;
        client.list_tools().await.ok()
    }

    pub async fn servers(&self) -> Vec<String> {
        self.clients
            .read()
            .await
            .iter()
            .filter(|(_, c)| c.is_alive())
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// The same without waiting, for synchronous code (prompt building). Empty while a writer holds
    /// the list, which only happens for the moment a server is added.
    #[must_use]
    pub fn server_names(&self) -> Vec<String> {
        self.clients
            .try_read()
            .map(|g| {
                g.iter()
                    .filter(|(_, c)| c.is_alive())
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Disconnect on-demand servers idle longer than `max_idle` (excluding `keep`).
    pub async fn disconnect_idle(
        &self,
        max_idle: Duration,
        keep: &BTreeSet<String>,
    ) -> Vec<String> {
        let now = Instant::now();
        let stale: Vec<String> = {
            let used = self.last_used.read().await;
            let clients = self.clients.read().await;
            clients
                .keys()
                .filter(|k| !keep.contains(*k))
                .filter(|k| {
                    used.get(*k)
                        .is_some_and(|ts| now.saturating_duration_since(*ts) >= max_idle)
                })
                .cloned()
                .collect()
        };
        if !stale.is_empty() {
            let mut clients = self.clients.write().await;
            let mut used = self.last_used.write().await;
            for k in &stale {
                clients.remove(k);
                used.remove(k);
            }
        }
        stale
    }

    /// Every tool of every server, with the server name in front.
    pub async fn tools(&self) -> Vec<ToolSpec> {
        let clients: Vec<(String, Arc<McpClient>)> = self
            .clients
            .read()
            .await
            .iter()
            .map(|(k, c)| (k.clone(), Arc::clone(c)))
            .collect();
        // Servers are asked at the same time, each with a time limit: one slow or dead server must not hold up
        // the model's next step. A server that fails to list is skipped, not fatal.
        let listed =
            futures_util::future::join_all(clients.into_iter().map(|(key, mut c)| async move {
                if !c.is_alive()
                    && !self.cooling_down(&key).await
                    && let Ok(Some(fresh)) = self.try_reconnect(&key).await
                {
                    c = fresh;
                }
                if !c.is_alive() {
                    return Vec::new();
                }
                match tokio::time::timeout(LIST_TIMEOUT, c.list_tools()).await {
                    Ok(Ok(tools)) => tools
                        .into_iter()
                        .map(|mut t| {
                            t.name = format!("{}{SEPARATOR}{}", c.name(), t.name);
                            t
                        })
                        .collect(),
                    Ok(Err(e)) => {
                        self.last_error.write().await.insert(key, e.to_string());
                        Vec::new()
                    }
                    Err(_) => {
                        self.last_error.write().await.insert(
                            key,
                            format!("did not list its tools within {} s", LIST_TIMEOUT.as_secs()),
                        );
                        Vec::new()
                    }
                }
            }))
            .await;
        listed.into_iter().flatten().collect()
    }

    /// True while a server that failed to restart is being left alone.
    async fn cooling_down(&self, key: &str) -> bool {
        self.cooldown
            .read()
            .await
            .get(key)
            .is_some_and(|(until, _)| Instant::now() < *until)
    }

    /// Seconds until the server is tried again, for messages.
    async fn cooldown_left(&self, key: &str) -> u64 {
        self.cooldown.read().await.get(key).map_or(0, |(until, _)| {
            until.saturating_duration_since(Instant::now()).as_secs() + 1
        })
    }

    pub async fn call(&self, full_name: &str, arguments: Value) -> Result<ToolResult> {
        self.call_with_timeout(full_name, arguments, DEFAULT_CALL_TIMEOUT)
            .await
    }

    pub async fn call_with_timeout(
        &self,
        full_name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> Result<ToolResult> {
        let (server, tool) = split_name(full_name)
            .ok_or_else(|| Error::NotFound(format!("'{full_name}' is not an MCP tool name")))?;
        let key = sanitize(server);
        let mut client = self
            .clients
            .read()
            .await
            .get(&key)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("mcp server '{server}' is not connected")))?;

        if !client.is_alive() {
            if self.cooling_down(&key).await {
                let why = self.last_error(&key).await.unwrap_or_default();
                return Err(Error::Mcp(format!(
                    "{server} is not responding{}. It is tried again in {} s",
                    if why.is_empty() {
                        String::new()
                    } else {
                        format!(" ({why})")
                    },
                    self.cooldown_left(&key).await
                )));
            }
            if let Some(reconnected) = self.try_reconnect(&key).await? {
                client = reconnected;
            }
        }

        self.last_used
            .write()
            .await
            .insert(key.clone(), Instant::now());
        match client
            .call_tool_with_timeout(tool, arguments.clone(), timeout)
            .await
        {
            Ok(res) => Ok(res),
            Err(err) if !client.is_alive() => {
                // Child died mid-call: attempt one automatic reconnect and retry once.
                if let Some(reconnected) = self.try_reconnect(&key).await? {
                    self.last_used.write().await.insert(key, Instant::now());
                    reconnected
                        .call_tool_with_timeout(tool, arguments, timeout)
                        .await
                } else {
                    Err(err)
                }
            }
            Err(err) => Err(err),
        }
    }

    async fn try_reconnect(&self, key: &str) -> Result<Option<Arc<McpClient>>> {
        let Some(entry) = self.entries.read().await.get(key).cloned() else {
            return Ok(None);
        };
        match connect_within(key, &entry).await {
            Ok(fresh) => {
                let arc = Arc::new(fresh);
                self.clients
                    .write()
                    .await
                    .insert(key.to_string(), Arc::clone(&arc));
                self.last_error.write().await.remove(key);
                self.cooldown.write().await.remove(key);
                Ok(Some(arc))
            }
            Err(e) => {
                let mut cd = self.cooldown.write().await;
                let fails = cd.get(key).map_or(1, |(_, n)| n + 1);
                let wait = COOLDOWN_BASE
                    .saturating_mul(1u32 << fails.min(6))
                    .min(COOLDOWN_CAP);
                cd.insert(key.to_string(), (Instant::now() + wait, fails));
                drop(cd);
                self.last_error
                    .write()
                    .await
                    .insert(key.to_string(), e.to_string());
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(sanitize("Photo Craft!"), "Photo_Craft_");
        assert_eq!(sanitize("a__b"), "a_b");
        assert_eq!(
            split_name("photocraft__command_run"),
            Some(("photocraft", "command_run"))
        );
        assert_eq!(split_name("plain"), None);
    }

    #[tokio::test]
    async fn unknown_server_is_an_error() {
        let hub = McpHub::new();
        assert!(hub.call("nope__x", Value::Null).await.is_err());
        assert!(hub.call("noseparator", Value::Null).await.is_err());
        assert!(hub.tools().await.is_empty());
    }

    #[tokio::test]
    async fn a_server_that_cannot_start_is_left_alone_for_a_while() {
        let hub = McpHub::new();
        let entry = ServerEntry {
            command: Some(
                std::env::temp_dir()
                    .join("no-such-mcp-server.exe")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ..ServerEntry::default()
        };
        hub.entries.write().await.insert("ghost".into(), entry);
        assert!(!hub.cooling_down("ghost").await);
        assert!(hub.try_reconnect("ghost").await.is_err());
        assert!(
            hub.cooling_down("ghost").await,
            "no new start attempt right after a failure"
        );
        let first = hub.cooldown_left("ghost").await;
        assert!((1..=11).contains(&first), "{first}");
        assert!(hub.last_error("ghost").await.is_some());
        // Failures in a row back off further, up to the cap.
        for _ in 0..8 {
            let _ = hub.try_reconnect("ghost").await;
        }
        assert!(hub.cooldown_left("ghost").await <= COOLDOWN_CAP.as_secs() + 1);
        assert!(hub.cooldown_left("ghost").await > first);
    }
}
