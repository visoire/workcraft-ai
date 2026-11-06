//! State shared by everything that runs inside the engine.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::{Notify, broadcast, oneshot};
use workcraft_core::team::ApprovalMode;
use workcraft_core::{
    AgentDef, Error, Event, ModelRef, Result, TeamConfig, ToolCall, Usage, new_id,
};
use workcraft_llm::Registry;
use workcraft_store::Store;

use crate::config::Config;
use crate::tools::Tools;

/// How long a call waits for a person before it is denied.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Stops a running turn. Cheap to clone through `Arc`.
#[derive(Default)]
pub struct Cancel {
    flag: AtomicBool,
    notify: Notify,
}

impl Cancel {
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Resolves when cancelled.
    pub async fn cancelled(&self) {
        loop {
            let n = self.notify.notified();
            tokio::pin!(n);
            n.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            n.await;
        }
    }
}

pub struct PendingApproval {
    pub tx: oneshot::Sender<bool>,
    pub session: String,
    pub agent: Option<String>,
    pub call: ToolCall,
    pub reason: Option<String>,
    pub destructive: bool,
}

pub struct Shared {
    pub data_dir: PathBuf,
    pub config: RwLock<Config>,
    pub providers: RwLock<Registry>,
    pub store: Store,
    pub tools: RwLock<Arc<Tools>>,
    pub events: broadcast::Sender<Event>,
    pub approvals: Mutex<HashMap<String, PendingApproval>>,
    pub agents: RwLock<BTreeMap<String, AgentDef>>,
    pub teams: RwLock<BTreeMap<String, TeamConfig>>,
    pub cancels: Mutex<HashMap<String, Arc<Cancel>>>,
    /// Per-app async writer leases so concurrent Team specialists serialize mutating calls to the
    /// same Craft app while reads run in parallel (T5.2).
    pub app_leases: Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Token counts of the model calls of the turn that is running, by session and model.
    pub turn_usage: Mutex<HashMap<String, Vec<(ModelRef, Usage)>>>,
    /// Orders from the person to a running team plan, by session (`team.steer`).
    pub steers: Mutex<HashMap<String, Vec<Steer>>>,
    pub steer_wake: Notify,
}

/// What a person can tell a running plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Steer {
    /// Leave this task out (a running one is stopped); tasks that depend on it are skipped too.
    Skip { task: String },
    /// Run this task again, now, with this extra instruction (a running one is stopped first).
    Redo { task: String, note: String },
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn read<T>(l: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Shared {
    pub fn emit(&self, e: Event) {
        // No subscribers is fine.
        let _ = self.events.send(e);
    }

    #[must_use]
    pub fn config(&self) -> Config {
        read(&self.config).clone()
    }

    #[must_use]
    pub fn tools(&self) -> Arc<Tools> {
        Arc::clone(&read(&self.tools))
    }

    pub fn provider(&self, id: &str) -> Result<Arc<dyn workcraft_llm::Provider>> {
        read(&self.providers).get(id)
    }

    #[must_use]
    pub fn agent(&self, id: &str) -> Option<AgentDef> {
        read(&self.agents).get(id).cloned()
    }

    /// Return the per-app writer mutex for `app_id`.
    #[must_use]
    pub fn app_lease(&self, app_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut g = lock(&self.app_leases);
        Arc::clone(
            g.entry(app_id.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        )
    }

    /// Queue an order for the plan running in `session` and wake it. `false` when no turn is running there.
    pub fn steer(&self, session: &str, s: Steer) -> bool {
        if !lock(&self.cancels).contains_key(session) {
            return false;
        }
        lock(&self.steers)
            .entry(session.to_string())
            .or_default()
            .push(s);
        self.steer_wake.notify_one();
        true
    }

    /// The orders waiting for `session`, oldest first.
    pub fn take_steers(&self, session: &str) -> Vec<Steer> {
        lock(&self.steers).remove(session).unwrap_or_default()
    }

    /// Remember the tokens of one model call, so the cost of the turn uses each model's own price.
    pub fn record_usage(&self, session: &str, model: &ModelRef, usage: Usage) {
        if usage == Usage::default() {
            return;
        }
        let mut g = lock(&self.turn_usage);
        let list = g.entry(session.to_string()).or_default();
        match list.iter_mut().find(|(m, _)| m == model) {
            Some((_, u)) => u.add(usage),
            None => list.push((model.clone(), usage)),
        }
    }

    /// The cost in US dollars of the turn that just ended, or `None` when a model used has no known price.
    /// Clears the record for `session`.
    pub fn take_turn_cost(&self, session: &str) -> Option<f64> {
        let calls = lock(&self.turn_usage).remove(session).unwrap_or_default();
        if calls.is_empty() {
            return None;
        }
        let cfg = self.config();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let mut total = 0.0;
        for (m, u) in &calls {
            let local = workcraft_llm::is_local_model(m, &cfg.providers);
            total +=
                workcraft_llm::price::cost_usd(&m.provider, &m.model, u, &cfg.prices, local, now)?;
        }
        Some(total)
    }

    /// A cancel handle for `session`; a running turn registers it, `chat.cancel` fires it.
    pub fn begin_turn(&self, session: &str) -> Arc<Cancel> {
        let c = Arc::new(Cancel::default());
        lock(&self.cancels).insert(session.to_string(), Arc::clone(&c));
        c
    }

    pub fn end_turn(&self, session: &str) {
        lock(&self.cancels).remove(session);
    }

    pub fn cancel_turn(&self, session: &str) -> bool {
        let c = lock(&self.cancels).get(session).cloned();
        if let Some(c) = &c {
            c.cancel();
        }
        c.is_some()
    }

    /// Ask a person and wait. Denied on timeout or cancel. Answer with [`Shared::respond`].
    pub async fn request_approval(
        &self,
        session: &str,
        agent: Option<&str>,
        call: &ToolCall,
        cancel: &Cancel,
    ) -> bool {
        self.request_approval_with_reason(session, agent, call, None, false, cancel)
            .await
    }

    /// Ask a person with an explicit policy reason and destructive flag (D6, T3.3).
    pub async fn request_approval_with_reason(
        &self,
        session: &str,
        agent: Option<&str>,
        call: &ToolCall,
        reason: Option<String>,
        destructive: bool,
        cancel: &Cancel,
    ) -> bool {
        let id = new_id();
        let (tx, rx) = oneshot::channel();
        lock(&self.approvals).insert(
            id.clone(),
            PendingApproval {
                tx,
                session: session.to_string(),
                agent: agent.map(str::to_string),
                call: call.clone(),
                reason: reason.clone(),
                destructive,
            },
        );
        self.emit(Event::ApprovalRequested {
            id: id.clone(),
            session: session.to_string(),
            agent: agent.map(str::to_string),
            call: call.clone(),
            reason,
            destructive,
        });
        let allowed = tokio::select! {
            r = tokio::time::timeout(APPROVAL_TIMEOUT, rx) => matches!(r, Ok(Ok(true))),
            () = cancel.cancelled() => false,
        };
        lock(&self.approvals).remove(&id);
        allowed
    }

    pub fn respond(&self, id: &str, allow: bool) -> Result<()> {
        let p = lock(&self.approvals)
            .remove(id)
            .ok_or_else(|| Error::NotFound(format!("approval {id}")))?;
        let _ = p.tx.send(allow);
        Ok(())
    }

    #[must_use]
    pub fn pending_approvals(&self) -> Value {
        let g = lock(&self.approvals);
        let items: Vec<Value> = g
            .iter()
            .map(|(id, p)| {
                json!({
                    "id": id,
                    "session": p.session,
                    "agent": p.agent,
                    "tool": p.call.name,
                    "arguments": p.call.arguments,
                    "reason": p.reason,
                    "destructive": p.destructive,
                })
            })
            .collect();
        Value::Array(items)
    }

    /// Does this call need a person in the current mode?
    #[must_use]
    pub fn needs_approval(mode: ApprovalMode, read_only: bool) -> bool {
        mode == ApprovalMode::Manual && !read_only
    }

    /// Process any pending Craft Anything `--inbox` `*.request.json` files once, routing each through
    /// WorkCraft's approval system and writing `<id>.answer.json` (`T4.5`).
    pub async fn process_craft_anything_inbox_once(
        &self,
        inbox_dir: &Path,
        seen: &mut HashSet<String>,
        session: &str,
        agent: Option<&str>,
        cancel: &Cancel,
    ) {
        let Ok(rd) = std::fs::read_dir(inbox_dir) else {
            return;
        };
        for entry in rd.filter_map(std::result::Result::ok) {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let Some(req_id) = file_name.strip_suffix(".request.json") else {
                continue;
            };
            if !seen.insert(req_id.to_string()) {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(&raw) else {
                continue;
            };
            let kind = v.get("kind").and_then(Value::as_str).unwrap_or("approval");
            let title = v
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Craft Anything request");
            let detail = v.get("detail").and_then(Value::as_str).unwrap_or("");
            let first_choice = v
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                .map(str::to_string);

            let reason = if detail.is_empty() {
                format!("Craft Anything {kind}: {title}")
            } else {
                format!("Craft Anything {kind}: {title} — {detail}")
            };
            let tool_name = if kind == "question" {
                "craft-anything__core_person_ask"
            } else {
                "craft-anything__core_action_approve"
            };
            let call = ToolCall {
                id: format!("inbox-{req_id}"),
                name: tool_name.to_string(),
                arguments: v.clone(),
            };
            let allowed = self
                .request_approval_with_reason(
                    session,
                    agent,
                    &call,
                    Some(reason),
                    kind == "approval",
                    cancel,
                )
                .await;
            let answer = json!({
                "choice": if allowed { first_choice } else { None },
                "text": Value::Null,
                "allow": allowed,
                "always": false,
            });
            let ans_path = inbox_dir.join(format!("{req_id}.answer.json"));
            let _ = std::fs::write(ans_path, answer.to_string());
        }
    }

    /// Spawn a background watcher for Craft Anything's `--inbox` folder for the duration of a tool call.
    pub fn spawn_craft_anything_inbox_watcher(
        self: &Arc<Self>,
        inbox_dir: PathBuf,
        session: String,
        agent: Option<String>,
        cancel: Arc<Cancel>,
    ) -> tokio::task::JoinHandle<()> {
        let sh = Arc::clone(self);
        tokio::spawn(async move {
            let mut seen = HashSet::new();
            loop {
                if cancel.is_cancelled() {
                    break;
                }
                sh.process_craft_anything_inbox_once(
                    &inbox_dir,
                    &mut seen,
                    &session,
                    agent.as_deref(),
                    &cancel,
                )
                .await;
                tokio::time::sleep(Duration::from_millis(120)).await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancel_wakes_waiters_and_stays_cancelled() {
        let c = Arc::new(Cancel::default());
        let c2 = Arc::clone(&c);
        let waiter = tokio::spawn(async move { c2.cancelled().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished());
        c.cancel();
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .unwrap()
            .unwrap();
        // Already cancelled: resolves at once.
        tokio::time::timeout(Duration::from_millis(100), c.cancelled())
            .await
            .unwrap();
    }

    #[test]
    fn approval_policy() {
        assert!(Shared::needs_approval(ApprovalMode::Manual, false));
        assert!(!Shared::needs_approval(ApprovalMode::Manual, true));
        assert!(!Shared::needs_approval(ApprovalMode::Auto, false));
    }
}
