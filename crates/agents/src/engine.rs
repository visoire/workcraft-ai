//! The engine: owns all state and implements the command bus.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use workcraft_control::{CommandBus, CommandInfo};
use workcraft_core::team::ApprovalMode;
use workcraft_core::{
    AgentDef, Attachment, ChatOptions, Error, Event, Message, ModelRef, Result, TeamConfig, paths,
};
use workcraft_llm::Registry;
use workcraft_mcp::{McpConfig, McpHub};
use workcraft_store::Store;

use crate::agent::{AgentRun, run_agent};
use crate::config::{self, Config};
use crate::prompt::system_prompt;
use crate::shared::Shared;
use crate::skills::Skills;
use crate::team::{TeamRequest, run_team};
use crate::tools::{Memory, Tools};

#[derive(Default)]
pub struct EngineOptions {
    /// Use these providers instead of the ones built from the environment and `config.json`.
    pub providers: Option<Registry>,
    /// Start the MCP servers listed in `mcp.json`.
    pub connect_mcp: bool,
    /// Override the approval mode from `config.json` (the CLI `--auto` flag).
    pub approval: Option<ApprovalMode>,
}

#[derive(Clone)]
pub struct Engine {
    pub shared: Arc<Shared>,
    /// Warnings found while loading (damaged definition files, MCP servers that did not start).
    pub warnings: Arc<Mutex<Vec<String>>>,
    /// The providers were given by the caller (tests): settings changes do not rebuild them.
    fixed_providers: bool,
}

/// Config as JSON with every saved API key replaced by `has_key` and a short hint.
fn masked_config(c: &Config) -> Result<Value> {
    let mut v = serde_json::to_value(c)?;
    if let Some(list) = v.get_mut("providers").and_then(Value::as_array_mut) {
        for p in list {
            if let Some(o) = p.as_object_mut() {
                let key = o.remove("api_key");
                let hint = key
                    .as_ref()
                    .and_then(Value::as_str)
                    .filter(|k| !k.trim().is_empty())
                    .map(workcraft_llm::registry::key_hint);
                o.insert("has_key".into(), json!(hint.is_some()));
                o.insert("key_hint".into(), json!(hint));
            }
        }
    }
    Ok(v)
}

/// Provider ids the settings screen may create: letters, digits and `-`.
fn valid_provider_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn rd<T>(l: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn wr<T>(l: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn str_arg<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
    p.get(k).and_then(Value::as_str)
}

fn need<'a>(p: &'a Value, k: &str) -> Result<&'a str> {
    str_arg(p, k)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::Invalid(format!("missing parameter '{k}'")))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn scan_skills(data_dir: &std::path::Path) -> Arc<Skills> {
    let mut skill_dirs = vec![data_dir.join("skills")];
    if let Some(extra) = std::env::var_os("WORKCRAFT_SKILLS") {
        skill_dirs.extend(std::env::split_paths(&extra));
    }
    skill_dirs.push(paths::home_dir().join(".agents").join("skills"));
    skill_dirs.push(paths::home_dir().join(".claude").join("skills"));
    Arc::new(Skills::scan(&skill_dirs))
}

/// Merge saved agents into the default `craft` team so custom or edited specialists participate in Team runs.
fn build_default_craft_team(agents: &BTreeMap<String, AgentDef>, cfg: &Config) -> TeamConfig {
    let mut craft = config::craft_team();
    if crate::craft::available_on_demand("craft-anything", &cfg.craft_apps, &cfg.craft_modes)
        && !craft.workers.iter().any(|w| w.id == "explorer")
    {
        craft.workers.push(config::explorer_agent());
    }
    if let Some(leader) = agents.get("leader") {
        craft.leader = leader.clone();
    }
    for w in &mut craft.workers {
        if let Some(updated) = agents.get(&w.id) {
            *w = updated.clone();
        }
    }
    for (id, def) in agents {
        if id != "leader" && !config::is_app_agent(id) && !craft.workers.iter().any(|w| &w.id == id)
        {
            craft.workers.push(def.clone());
        }
    }
    craft
}

/// The `apps` team: one agent per Craft app that is installed and switched on (all of them when none is), plus the explorer
/// when Craft Anything is on. Saved edits of an agent (`agents/<id>.json`) win over the built-in definition.
fn build_apps_team(agents: &BTreeMap<String, AgentDef>, cfg: &Config) -> TeamConfig {
    let pick = |a: AgentDef| agents.get(&a.id).cloned().unwrap_or(a);
    let on: Vec<AgentDef> = crate::craft::specs()
        .iter()
        .filter(|s| s.id != "craft-anything")
        .filter(|s| crate::craft::available_on_demand(s.id, &cfg.craft_apps, &cfg.craft_modes))
        .filter_map(|s| {
            config::app_agents()
                .into_iter()
                .find(|a| a.id == config::app_agent_id(s.id))
        })
        .map(pick)
        .collect();
    let mut workers = if on.is_empty() {
        config::app_agents().into_iter().map(pick).collect()
    } else {
        on
    };
    if crate::craft::available_on_demand("craft-anything", &cfg.craft_apps, &cfg.craft_modes) {
        workers.push(
            agents
                .get("explorer")
                .cloned()
                .unwrap_or_else(config::explorer_agent),
        );
    }
    config::apps_team(workers)
}

impl Engine {
    pub async fn open(data_dir: PathBuf, opts: EngineOptions) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)?;
        let mut warnings = Vec::new();
        let mut config = match Config::load(&data_dir) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(e.to_string());
                Config::default()
            }
        };
        if let Some(a) = opts.approval {
            config.approval = a;
        }
        let fixed_providers = opts.providers.is_some();
        let providers = match opts.providers {
            Some(p) => p,
            None => {
                let (r, w) = Registry::build(&config.providers)?;
                warnings.extend(w);
                r
            }
        };
        let workspace = config
            .workspace
            .clone()
            .unwrap_or_else(|| data_dir.join("workspace"));
        std::fs::create_dir_all(&workspace)?;

        // The Craft app skills ship inside WorkCraft; they are written once and then belong to the user.
        crate::skills::install_bundled(&data_dir.join("skills"));
        let skills = scan_skills(&data_dir);

        let roots = config.roots_context(workspace.clone(), data_dir.clone());
        let hub = McpHub::new();
        if opts.connect_mcp {
            let (found, ov_errs) = crate::craft::discover_with_errors();
            for (app_id, msg) in ov_errs {
                warnings.push(format!("craft '{app_id}': {msg}"));
            }
            match McpConfig::load(&data_dir.join("mcp.json")) {
                Ok(c) => {
                    let c = crate::craft::merge_with_roots(
                        c,
                        &found,
                        &config.craft_apps,
                        &config.craft_modes,
                        &roots,
                    );
                    for r in hub.connect_all(&c).await {
                        if let Some(e) = r.error {
                            warnings.push(format!("mcp '{}': {e}", r.server));
                        }
                    }
                }
                Err(e) => warnings.push(e.to_string()),
            }
        }
        let controls = config::load_control_targets(&data_dir).unwrap_or_else(|e| {
            warnings.push(e.to_string());
            BTreeMap::new()
        });

        let mut agents = config::builtin_agents();
        for a in config::app_agents() {
            agents.insert(a.id.clone(), a);
        }
        if crate::craft::available_on_demand(
            "craft-anything",
            &config.craft_apps,
            &config.craft_modes,
        ) {
            let exp = config::explorer_agent();
            agents.insert(exp.id.clone(), exp);
        }
        let (defs, bad) = config::load_dir::<AgentDef>(&data_dir.join("agents"));
        for a in defs {
            agents.insert(a.id.clone(), a);
        }
        warnings.extend(
            bad.into_iter()
                .map(|b| format!("could not read agent file {b}")),
        );
        let mut teams = BTreeMap::new();
        let craft = build_default_craft_team(&agents, &config);
        teams.insert(craft.name.clone(), craft);
        let apps = build_apps_team(&agents, &config);
        teams.insert(apps.name.clone(), apps);
        let (defs, bad) = config::load_dir::<TeamConfig>(&data_dir.join("teams"));
        for t in defs {
            teams.insert(t.name.clone(), t);
        }
        warnings.extend(
            bad.into_iter()
                .map(|b| format!("could not read team file {b}")),
        );

        let checkpoints = Arc::new(crate::checkpoints::Checkpoints::new(
            data_dir.join("checkpoints"),
        ));
        let tools_obj = Tools::from_config(
            workspace,
            skills,
            hub,
            controls,
            data_dir.clone(),
            checkpoints,
            &config,
            opts.connect_mcp,
        );

        let (events, _) = broadcast::channel(1024);
        let shared = Arc::new(Shared {
            store: Store::open(data_dir.join("sessions"))?,
            tools: RwLock::new(Arc::new(tools_obj)),
            data_dir,
            config: RwLock::new(config),
            providers: RwLock::new(providers),
            events,
            approvals: Mutex::default(),
            agents: RwLock::new(agents),
            teams: RwLock::new(teams),
            cancels: Mutex::default(),
            app_leases: Mutex::default(),
            turn_usage: Mutex::default(),
            steers: Mutex::default(),
            steer_wake: tokio::sync::Notify::new(),
        });
        Ok(Self {
            shared,
            warnings: Arc::new(Mutex::new(warnings)),
            fixed_providers,
        })
    }

    fn sync_tools(&self, new_skills: Option<Arc<Skills>>, new_hub: Option<McpHub>) {
        let sh = &self.shared;
        let old = sh.tools();
        let cfg = sh.config();
        let ws = cfg
            .workspace
            .clone()
            .unwrap_or_else(|| old.workspace.clone());
        let updated = Tools::from_config(
            ws,
            new_skills.unwrap_or_else(|| Arc::clone(&old.skills)),
            new_hub.unwrap_or_else(|| old.hub.clone()),
            old.controls.clone(),
            old.data_dir.clone(),
            Arc::clone(&old.checkpoints),
            &cfg,
            old.allow_craft_on_demand,
        );
        *wr(&sh.tools) = Arc::new(updated);
    }

    /// What every model request carries before the conversation: the system prompt and the tool definitions,
    /// in estimated tokens, so a person (or a test) can see where the room goes.
    async fn token_report(&self, question: Option<&str>) -> Value {
        let sh = &self.shared;
        let system = crate::prompt::system_prompt(sh, None);
        let mut specs = sh.tools().specs().await;
        let all_tools = specs.len();
        if let Some(q) = question.filter(|_| sh.config().tool_routing) {
            specs = crate::craft::route(specs, &[], q).tools;
        }
        let mut groups: std::collections::BTreeMap<String, (usize, usize)> =
            std::collections::BTreeMap::new();
        for t in &specs {
            let group = workcraft_mcp::hub::split_name(&t.name).map_or("built-in", |(s, _)| s);
            let e = groups.entry(group.to_string()).or_default();
            e.0 += 1;
            e.1 += crate::craft::estimate_tool_tokens(t);
        }
        let tool_tokens: usize = groups.values().map(|g| g.1).sum();
        let system_tokens = crate::context::estimate_tokens(system.len());
        json!({
            "system_tokens": system_tokens,
            "tool_count": specs.len(),
            "tools_before_routing": all_tools,
            "tool_tokens": tool_tokens,
            "total_tokens": system_tokens + tool_tokens,
            "groups": groups.into_iter().map(|(k, (n, tok))| json!({ "group": k, "tools": n, "tokens": tok })).collect::<Vec<_>>(),
        })
    }
    async fn craft_report(&self) -> Value {
        let sh = &self.shared;
        let tools = sh.tools();
        let cfg = sh.config();
        let (found, ov_errs) = crate::craft::discover_with_errors();
        let servers = tools.hub.servers().await;
        let mut counts = BTreeMap::new();
        let mut last_errors = BTreeMap::new();
        for sp in crate::craft::specs() {
            if let Some(list) = tools.hub.server_tools(sp.id).await {
                counts.insert(sp.id.to_string(), list.len());
            }
            if let Some(err) = tools.hub.last_error(sp.id).await {
                last_errors.insert(sp.id.to_string(), err);
            }
        }
        crate::craft::report_full(
            &found,
            &ov_errs,
            &cfg.craft_apps,
            &cfg.craft_modes,
            &servers,
            &counts,
            &last_errors,
        )
    }

    #[must_use]
    pub fn warnings(&self) -> Vec<String> {
        self.warnings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    #[must_use]
    pub fn subscribe_events(&self) -> broadcast::Receiver<Event> {
        self.shared.events.subscribe()
    }

    // ---------------------------------------------------------------- chat

    fn session_model(&self, header_model: Option<ModelRef>, override_: Option<&str>) -> ModelRef {
        override_
            .map(ModelRef::parse)
            .or(header_model)
            .unwrap_or_else(|| self.shared.config().model_ref())
    }

    pub async fn chat_send(
        &self,
        session: &str,
        text: &str,
        leaf: Option<&str>,
        model: Option<&str>,
        attachments: Vec<Attachment>,
        team: Option<&str>,
    ) -> Result<Value> {
        let sh = &self.shared;
        let s = sh.store.load(session)?;
        if sh
            .cancels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(session)
        {
            return Err(Error::Invalid(format!(
                "session {session} is already answering"
            )));
        }
        let model = self.session_model(s.header.model.clone(), model);
        let parent = match leaf {
            Some(l) => {
                if !s.messages.iter().any(|m| m.id == l) {
                    return Err(Error::NotFound(format!("message {l}")));
                }
                Some(l.to_string())
            }
            None => s.tip().map(|m| m.id.clone()),
        };
        let mut user = Message::user(text).with_parent(parent.as_deref());
        user.attachments = attachments;
        sh.store.append(session, &user)?;
        sh.emit(Event::MessageAdded {
            session: session.to_string(),
            message: user.clone(),
        });
        if s.header.title == "New chat" && !text.trim().is_empty() {
            let title: String = text
                .trim()
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(48)
                .collect();
            let _ = sh.store.set_meta(session, Some(title), None, None);
        }

        let mut history: Vec<Message> = s
            .path_to(parent.as_deref().unwrap_or(""))
            .into_iter()
            .cloned()
            .collect();
        history.push(user.clone());

        // Auto-activate `@app` mentions on demand and disconnect idle non-default servers (`T5.6`, `D4`).
        {
            let tools = sh.tools();
            if tools.allow_craft_on_demand {
                let cfg = sh.config();
                let mentions = crate::craft::extract_mentions(text);
                let mut keep = std::collections::BTreeSet::new();
                for id in crate::craft::ON_BY_DEFAULT {
                    keep.insert((*id).to_string());
                }
                for (id, &on) in &cfg.craft_apps {
                    if on {
                        keep.insert(id.clone());
                    }
                }
                for &m in &mentions {
                    keep.insert(m.to_string());
                }
                let _ = tools
                    .hub
                    .disconnect_idle(std::time::Duration::from_secs(600), &keep)
                    .await;

                let roots = tools.roots_context();
                for &app_id in &mentions {
                    if crate::craft::available_on_demand(app_id, &cfg.craft_apps, &cfg.craft_modes)
                        && !tools.hub.is_connected(app_id).await
                    {
                        let mode =
                            crate::craft::mode_for(app_id, &cfg.craft_apps, &cfg.craft_modes);
                        match mode {
                            crate::craft::CraftMode::Live => {
                                if let Some(sp) = crate::craft::find_spec(app_id)
                                    && crate::craft::live_port_for(sp.id, sp.name).is_some()
                                {
                                    let _ = crate::craft::ensure_live_with_roots(
                                        app_id, None, &roots, &tools.hub,
                                    )
                                    .await;
                                }
                            }
                            crate::craft::CraftMode::Headless => {
                                let _ =
                                    crate::craft::ensure_headless(app_id, &roots, &tools.hub).await;
                            }
                            crate::craft::CraftMode::Off => {}
                        }
                    }
                }
            }
        }

        let cancel = sh.begin_turn(session);
        sh.emit(Event::TurnStarted {
            session: session.to_string(),
        });
        let team_name = team
            .map(str::to_string)
            .or_else(|| (s.header.mode == "team").then(|| "craft".to_string()));
        let result: Result<Value> = async {
            if let Some(name) = team_name {
                let mut team = rd(&sh.teams).get(&name).cloned().ok_or_else(|| Error::NotFound(format!("team '{name}'")))?;
                // The apps team follows the Craft apps that are installed and on right now, unless it was saved by hand.
                if name == "apps" && !sh.data_dir.join("teams").join("apps.json").exists() {
                    team = build_apps_team(&rd(&sh.agents), &sh.config());
                }
                // The global switch wins in one direction: auto approval turns the team's prompts off.
                if sh.config().approval == ApprovalMode::Auto {
                    team.approval = ApprovalMode::Auto;
                    team.plan_approval = None;
                    for w in &mut team.workers {
                        w.approval = None;
                    }
                }
                let o = run_team(sh, TeamRequest { session: session.to_string(), goal: text.to_string(), team, model: model.clone(), parent: Some(user.id.clone()), cancel: Arc::clone(&cancel) }).await?;
                let cost = sh.take_turn_cost(session);
                sh.emit(Event::TurnFinished { session: session.to_string(), usage: o.usage, cost_usd: cost });
                let tasks: Vec<Value> = o.tasks.iter().map(|t| json!({ "id": t.spec.id, "title": t.spec.title, "agent": t.spec.agent, "status": t.status, "result": t.result })).collect();
                Ok(json!({ "session": session, "text": o.text, "message_id": o.last_id, "usage": o.usage, "cost_usd": cost, "stopped": o.stopped, "run": o.run, "tasks": tasks }))
            } else {
                let cfg = sh.config();
                let o = run_agent(sh, AgentRun {
                    session: session.to_string(),
                    agent: None,
                    model: model.clone(),
                    system: system_prompt(sh, None),
                    history,
                    persist: true,
                    memory: Arc::new(Memory::default()),
                    cancel: Arc::clone(&cancel),
                    approval: cfg.approval,
                    max_steps: cfg.max_steps,
                    options: ChatOptions { num_ctx: cfg.num_ctx, think: cfg.think, ..ChatOptions::default() },
                })
                .await?;
                let cost = sh.take_turn_cost(session);
                sh.emit(Event::TurnFinished { session: session.to_string(), usage: o.usage, cost_usd: cost });
                Ok(json!({ "session": session, "text": o.text, "message_id": o.last_id, "usage": o.usage, "cost_usd": cost, "stopped": o.stopped }))
            }
        }
        .await;
        sh.end_turn(session);
        if let Err(e) = &result {
            sh.emit(Event::Error {
                session: session.to_string(),
                message: e.to_string(),
            });
        }
        result
    }

    fn new_session(&self, p: &Value) -> Result<Value> {
        let model = str_arg(p, "model").map(ModelRef::parse);
        let mode = str_arg(p, "mode").unwrap_or("single");
        if !matches!(mode, "single" | "team") {
            return Err(Error::Invalid("mode must be 'single' or 'team'".into()));
        }
        let h = self
            .shared
            .store
            .create(str_arg(p, "title").unwrap_or(""), model, mode)?;
        Ok(json!({ "session": h.id, "title": h.title, "mode": h.mode }))
    }

    /// Every provider the engine can use, with where its key comes from. Never the key itself.
    fn provider_info(&self) -> Value {
        let sh = &self.shared;
        let ids = rd(&sh.providers).ids();
        let cfg = sh.config();
        let env = |v: &str| std::env::var(v).is_ok_and(|k| !k.trim().is_empty());
        let list: Vec<Value> = ids
            .iter()
            .map(|id| match cfg.providers.iter().find(|c| &c.id == id) {
                Some(c) => {
                    let saved = c.api_key.as_deref().filter(|k| !k.trim().is_empty());
                    let from_env = c.key_env.as_deref().is_some_and(env);
                    json!({
                        "id": id, "kind": c.kind, "name": c.name, "base_url": c.base_url,
                        "key_env": c.key_env, "source": "settings",
                        "has_key": saved.is_some() || from_env,
                        "key_from": if saved.is_some() { "saved" } else if from_env { "env" } else { "none" },
                        "key_hint": saved.map(workcraft_llm::registry::key_hint),
                    })
                }
                None => {
                    let var = match id.as_str() {
                        "openai" => Some("OPENAI_API_KEY"),
                        "anthropic" => Some("ANTHROPIC_API_KEY"),
                        _ => None,
                    };
                    json!({
                        "id": id,
                        "kind": if id == "anthropic" { "anthropic" } else if id == "ollama" { "ollama" } else { "openai" },
                        "source": "built-in", "key_env": var,
                        "has_key": var.is_some_and(env),
                        "key_from": if var.is_some_and(env) { "env" } else { "none" },
                    })
                }
            })
            .collect();
        // Saved entries that failed to build still show, so they can be fixed or removed.
        let broken: Vec<Value> = cfg
            .providers
            .iter()
            .filter(|c| !ids.contains(&c.id))
            .map(|c| json!({ "id": c.id, "kind": c.kind, "name": c.name, "base_url": c.base_url, "source": "settings", "broken": true }))
            .collect();
        json!(list.into_iter().chain(broken).collect::<Vec<_>>())
    }

    fn rebuild_providers(&self) -> Result<()> {
        if self.fixed_providers {
            return Ok(());
        }
        let (r, warnings) = Registry::build(&self.shared.config().providers)?;
        *wr(&self.shared.providers) = r;
        if !warnings.is_empty() {
            self.warnings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend(warnings);
        }
        Ok(())
    }

    fn session_json(&self, p: &Value) -> Result<Value> {
        let s = self.shared.store.load(need(p, "session")?)?;
        let leaf = str_arg(p, "leaf")
            .map(str::to_string)
            .or_else(|| s.tip().map(|m| m.id.clone()));
        let msgs: Vec<&Message> = leaf.as_deref().map(|l| s.path_to(l)).unwrap_or_default();
        Ok(json!({ "header": s.header, "messages": msgs, "skipped_lines": s.skipped }))
    }

    // ---------------------------------------------------------------- commands

    async fn run_command(&self, id: &str, p: Value) -> Result<Value> {
        let sh = &self.shared;
        match id {
            "chat.new" => self.new_session(&p),
            "chat.list" => {
                let list: Vec<Value> = sh.store.list()?.into_iter().map(|s| json!({ "id": s.id, "title": s.title, "created": s.created, "updated": s.updated, "messages": s.messages })).collect();
                Ok(Value::Array(list))
            }
            "chat.get" => self.session_json(&p),
            "chat.tree" => {
                let s = sh.store.load(need(&p, "session")?)?;
                Ok(
                    json!({ "header": s.header, "messages": s.messages, "skipped_lines": s.skipped }),
                )
            }
            "chat.children" => {
                let s = sh.store.load(need(&p, "session")?)?;
                let kids: Vec<&Message> = s.children_of(need(&p, "message")?);
                Ok(json!(kids))
            }
            "chat.send" => {
                let session = match str_arg(&p, "session") {
                    Some(s) if !s.is_empty() => s.to_string(),
                    _ => self.new_session(&p)?["session"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                };
                let atts: Vec<Attachment> = p
                    .get("attachments")
                    .map(|a| serde_json::from_value(a.clone()))
                    .transpose()?
                    .unwrap_or_default();
                self.chat_send(
                    &session,
                    need(&p, "text")?,
                    str_arg(&p, "leaf"),
                    str_arg(&p, "model"),
                    atts,
                    str_arg(&p, "team"),
                )
                .await
            }
            "chat.cancel" => Ok(json!({ "cancelled": sh.cancel_turn(need(&p, "session")?) })),
            "chat.delete" => {
                sh.store.delete(need(&p, "session")?)?;
                Ok(json!({ "deleted": true }))
            }
            "chat.rename" => {
                sh.store.set_meta(
                    need(&p, "session")?,
                    Some(need(&p, "title")?.to_string()),
                    None,
                    None,
                )?;
                Ok(json!({ "renamed": true }))
            }
            "model.get" => Ok(json!({ "model": sh.config().model })),
            "model.set" => {
                let m = need(&p, "model")?.to_string();
                // With a session the choice belongs to that chat (switch local/cloud mid-conversation).
                if let Some(session) = str_arg(&p, "session") {
                    sh.store
                        .set_meta(session, None, Some(ModelRef::parse(&m)), None)?;
                } else {
                    let mut c = wr(&sh.config);
                    c.model = m.clone();
                    let _ = c.save(&sh.data_dir);
                }
                Ok(json!({ "model": m }))
            }
            "model.list" => {
                let ids = rd(&sh.providers).ids();
                let want = str_arg(&p, "provider");
                let mut out = BTreeMap::new();
                for id in ids.into_iter().filter(|i| want.is_none_or(|w| w == i)) {
                    let models = match sh.provider(&id)?.list_models().await {
                        Ok(m) => json!(m),
                        Err(e) => json!({ "error": e.to_string() }),
                    };
                    out.insert(id, models);
                }
                Ok(json!(out))
            }
            "provider.list" => Ok(json!(rd(&sh.providers).ids())),
            "provider.info" => Ok(self.provider_info()),
            "provider.save" => {
                let id = need(&p, "id")?.to_string();
                if !valid_provider_id(&id) {
                    return Err(Error::Invalid(
                        "provider id must be lowercase letters, digits or '-'".into(),
                    ));
                }
                let kind = str_arg(&p, "kind").unwrap_or("openai").to_string();
                if !matches!(kind.as_str(), "ollama" | "openai" | "anthropic") {
                    return Err(Error::Invalid(
                        "kind must be 'ollama', 'openai' or 'anthropic'".into(),
                    ));
                }
                let text = |k: &str| {
                    str_arg(&p, k)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                };
                {
                    let mut c = wr(&sh.config);
                    let old = c.providers.iter().position(|x| x.id == id);
                    let prev_key = old.and_then(|i| c.providers[i].api_key.clone());
                    // api_key: a string sets it, null removes it, leaving it out keeps the saved one.
                    let api_key = match p.get("api_key") {
                        None => prev_key,
                        Some(Value::Null) => None,
                        Some(v) => v
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .or(prev_key),
                    };
                    let prev_vision = old.and_then(|i| c.providers[i].vision);
                    let vision = match p.get("vision") {
                        None => prev_vision,
                        Some(Value::Null) => None,
                        Some(v) => v.as_bool().or(prev_vision),
                    };
                    let entry = workcraft_llm::ProviderConfig {
                        id: id.clone(),
                        kind,
                        name: text("name"),
                        base_url: text("base_url"),
                        key_env: text("key_env"),
                        api_key,
                        vision,
                    };
                    match old {
                        Some(i) => c.providers[i] = entry,
                        None => c.providers.push(entry),
                    }
                    c.save(&sh.data_dir)?;
                }
                self.rebuild_providers()?;
                Ok(self.provider_info())
            }
            "provider.remove" => {
                let id = need(&p, "id")?;
                let removed = {
                    let mut c = wr(&sh.config);
                    let n = c.providers.len();
                    c.providers.retain(|x| x.id != id);
                    let removed = c.providers.len() != n;
                    if removed {
                        c.save(&sh.data_dir)?;
                    }
                    removed
                };
                self.rebuild_providers()?;
                Ok(json!({ "removed": removed }))
            }
            "mode.set" => {
                let mode = need(&p, "mode")?;
                if !matches!(mode, "single" | "team") {
                    return Err(Error::Invalid("mode must be 'single' or 'team'".into()));
                }
                sh.store
                    .set_meta(need(&p, "session")?, None, None, Some(mode.to_string()))?;
                Ok(json!({ "mode": mode }))
            }
            "approval.mode" => {
                if let Some(m) = str_arg(&p, "mode") {
                    let mode = match m {
                        "manual" => ApprovalMode::Manual,
                        "auto" => ApprovalMode::Auto,
                        _ => return Err(Error::Invalid("mode must be 'manual' or 'auto'".into())),
                    };
                    let mut c = wr(&sh.config);
                    c.approval = mode;
                    let _ = c.save(&sh.data_dir);
                }
                Ok(json!({ "mode": sh.config().approval }))
            }
            "approval.list" => Ok(sh.pending_approvals()),
            "approval.respond" => {
                let allow = p
                    .get("allow")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| Error::Invalid("missing parameter 'allow'".into()))?;
                sh.respond(need(&p, "id")?, allow)?;
                Ok(json!({ "ok": true }))
            }
            "agent.list" => Ok(json!(rd(&sh.agents).values().collect::<Vec<_>>())),
            "agent.define" => {
                let def: AgentDef =
                    serde_json::from_value(p.get("agent").cloned().unwrap_or(p.clone()))?;
                if !valid_id(&def.id) {
                    return Err(Error::Invalid(
                        "agent id must be letters, digits, '-' or '_'".into(),
                    ));
                }
                let dir = sh.data_dir.join("agents");
                std::fs::create_dir_all(&dir)?;
                std::fs::write(
                    dir.join(format!("{}.json", def.id)),
                    serde_json::to_string_pretty(&def)?,
                )?;
                {
                    let mut agents = wr(&sh.agents);
                    agents.insert(def.id.clone(), def.clone());
                    if !sh.data_dir.join("teams").join("craft.json").exists() {
                        wr(&sh.teams).insert(
                            "craft".to_string(),
                            build_default_craft_team(&agents, &sh.config()),
                        );
                    } else if let Some(craft) = wr(&sh.teams).get_mut("craft") {
                        if def.id == "leader" {
                            craft.leader = def.clone();
                        } else if let Some(w) = craft.workers.iter_mut().find(|w| w.id == def.id) {
                            *w = def.clone();
                        } else {
                            craft.workers.push(def.clone());
                        }
                    }
                }
                Ok(json!(def))
            }
            "agent.remove" => {
                let id = need(&p, "id")?;
                let _ = std::fs::remove_file(sh.data_dir.join("agents").join(format!("{id}.json")));
                let removed = {
                    let mut agents = wr(&sh.agents);
                    let r = agents.remove(id).is_some();
                    // If a built-in specialist was customized and then removed, restore the built-in default.
                    if let Some(builtin) = config::builtin_agents().remove(id) {
                        agents.insert(id.to_string(), builtin);
                    }
                    if !sh.data_dir.join("teams").join("craft.json").exists() {
                        wr(&sh.teams).insert(
                            "craft".to_string(),
                            build_default_craft_team(&agents, &sh.config()),
                        );
                    } else if let Some(craft) = wr(&sh.teams).get_mut("craft") {
                        if let Some(updated) = agents.get(id) {
                            if let Some(w) = craft.workers.iter_mut().find(|w| w.id == id) {
                                *w = updated.clone();
                            }
                        } else {
                            craft.workers.retain(|w| w.id != id);
                        }
                    }
                    r
                };
                Ok(json!({ "removed": removed }))
            }
            "team.list" => Ok(json!(rd(&sh.teams).values().collect::<Vec<_>>())),
            "team.define" => {
                let team: TeamConfig =
                    serde_json::from_value(p.get("team").cloned().unwrap_or(p.clone()))?;
                if !valid_id(&team.name) {
                    return Err(Error::Invalid(
                        "team name must be letters, digits, '-' or '_'".into(),
                    ));
                }
                let dir = sh.data_dir.join("teams");
                std::fs::create_dir_all(&dir)?;
                std::fs::write(
                    dir.join(format!("{}.json", team.name)),
                    serde_json::to_string_pretty(&team)?,
                )?;
                wr(&sh.teams).insert(team.name.clone(), team.clone());
                Ok(json!(team))
            }
            "team.remove" => {
                let name = need(&p, "name")?;
                let _ =
                    std::fs::remove_file(sh.data_dir.join("teams").join(format!("{name}.json")));
                let removed = wr(&sh.teams).remove(name).is_some();
                if name == "craft" {
                    let craft = build_default_craft_team(&rd(&sh.agents), &sh.config());
                    wr(&sh.teams).insert("craft".to_string(), craft);
                }
                Ok(json!({ "removed": removed }))
            }
            "team.run" => {
                let session = match str_arg(&p, "session") {
                    Some(s) if !s.is_empty() => s.to_string(),
                    _ => self.new_session(&json!({ "title": "Team run", "mode": "team" }))?["session"].as_str().unwrap_or_default().to_string(),
                };
                let team = str_arg(&p, "team").unwrap_or("craft");
                self.chat_send(
                    &session,
                    need(&p, "goal")?,
                    None,
                    str_arg(&p, "model"),
                    Vec::new(),
                    Some(team),
                )
                .await
            }
            "mcp.status" => {
                let hub = sh.tools().hub.clone();
                let tools = hub.tools().await;
                let tool_names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
                Ok(
                    json!({ "servers": hub.servers().await, "tools": tools.len(), "tool_names": tool_names, "warnings": self.warnings() }),
                )
            }
            "mcp.reload" => {
                let mcp_cfg = McpConfig::load(&sh.data_dir.join("mcp.json"))?;
                let cfg = sh.config();
                let old = sh.tools();
                let roots = cfg.roots_context(old.workspace.clone(), sh.data_dir.clone());
                let (found, ov_errs) = crate::craft::discover_with_errors();
                let merged = crate::craft::merge_with_roots(
                    mcp_cfg,
                    &found,
                    &cfg.craft_apps,
                    &cfg.craft_modes,
                    &roots,
                );
                let hub = McpHub::new();
                let reports = hub.connect_all(&merged).await;
                self.sync_tools(None, Some(hub));
                {
                    let mut w = self
                        .warnings
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    w.retain(|x| !x.starts_with("mcp '") && !x.starts_with("craft '"));
                    for (app_id, msg) in ov_errs {
                        w.push(format!("craft '{app_id}': {msg}"));
                    }
                    w.extend(reports.iter().filter_map(|r| {
                        r.error.as_ref().map(|e| format!("mcp '{}': {e}", r.server))
                    }));
                }
                Ok(json!(
                    reports
                        .iter()
                        .map(|r| json!({ "server": r.server, "tools": r.tools, "error": r.error }))
                        .collect::<Vec<_>>()
                ))
            }
            "craft.list" => Ok(self.craft_report().await),
            "team.steer" => {
                let session = need(&p, "session")?;
                let task = need(&p, "task")?.to_string();
                let order = match need(&p, "action")? {
                    "skip" => crate::shared::Steer::Skip { task },
                    "redo" => crate::shared::Steer::Redo {
                        task,
                        note: p
                            .get("note")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .trim()
                            .to_string(),
                    },
                    other => {
                        return Err(Error::Invalid(format!(
                            "unknown steer action '{other}' (skip or redo)"
                        )));
                    }
                };
                if !sh.steer(session, order) {
                    return Err(Error::NotFound(format!(
                        "a running plan in session '{session}'"
                    )));
                }
                Ok(json!({ "ok": true }))
            }
            "tools.stats" => Ok(self
                .token_report(p.get("text").and_then(Value::as_str))
                .await),
            "craft.set" => {
                let raw_app = need(&p, "app")?;
                let spec = crate::craft::find_spec(raw_app)
                    .ok_or_else(|| Error::NotFound(format!("Craft app '{raw_app}'")))?;
                let on = p
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| Error::Invalid("missing parameter 'enabled'".into()))?;
                {
                    let mut c = wr(&sh.config);
                    c.craft_apps.insert(spec.id.to_string(), on);
                    if !on {
                        c.craft_modes
                            .insert(spec.id.to_string(), crate::craft::CraftMode::Off);
                    } else if c.craft_modes.get(spec.id) == Some(&crate::craft::CraftMode::Off) {
                        c.craft_modes
                            .insert(spec.id.to_string(), crate::craft::CraftMode::Headless);
                    }
                    c.save(&sh.data_dir)?;
                }
                self.sync_tools(None, None);
                if spec.id == "craft-anything" {
                    let mut agents = wr(&sh.agents);
                    if on {
                        let exp = config::explorer_agent();
                        agents.insert(exp.id.clone(), exp);
                    } else if !sh.data_dir.join("agents").join("explorer.json").exists() {
                        agents.remove("explorer");
                    }
                    if !sh.data_dir.join("teams").join("craft.json").exists() {
                        let craft = build_default_craft_team(&agents, &sh.config());
                        wr(&sh.teams).insert("craft".to_string(), craft);
                    }
                }
                if !on {
                    let _ = sh.tools().hub.remove(spec.id).await;
                } else if sh.tools().allow_craft_on_demand {
                    let tools = sh.tools();
                    let _ =
                        crate::craft::ensure_headless(spec.id, &tools.roots_context(), &tools.hub)
                            .await;
                }
                Ok(json!({ "app": spec.id, "enabled": on }))
            }
            "craft.mode" => {
                let raw_app = need(&p, "app")?;
                let spec = crate::craft::find_spec(raw_app)
                    .ok_or_else(|| Error::NotFound(format!("Craft app '{raw_app}'")))?;
                let raw_mode = need(&p, "mode")?;
                let mode = crate::craft::CraftMode::parse(raw_mode).ok_or_else(|| {
                    Error::Invalid("mode must be 'headless', 'live', or 'off'".into())
                })?;
                if mode == crate::craft::CraftMode::Live && !spec.supports_live() {
                    return Err(Error::Invalid(format!(
                        "{} supports headless MCP only and cannot run in live bridge mode",
                        spec.name
                    )));
                }
                {
                    let mut c = wr(&sh.config);
                    c.craft_modes.insert(spec.id.to_string(), mode);
                    if mode == crate::craft::CraftMode::Off {
                        c.craft_apps.insert(spec.id.to_string(), false);
                    } else if spec.off_by_default || c.craft_apps.get(spec.id) == Some(&false) {
                        c.craft_apps.insert(spec.id.to_string(), true);
                    }
                    c.save(&sh.data_dir)?;
                }
                self.sync_tools(None, None);
                if spec.id == "craft-anything"
                    && !sh.data_dir.join("teams").join("craft.json").exists()
                {
                    let mut agents = wr(&sh.agents);
                    if mode != crate::craft::CraftMode::Off {
                        let exp = config::explorer_agent();
                        agents.insert(exp.id.clone(), exp);
                    } else if !sh.data_dir.join("agents").join("explorer.json").exists() {
                        agents.remove("explorer");
                    }
                    let craft = build_default_craft_team(&agents, &sh.config());
                    wr(&sh.teams).insert("craft".to_string(), craft);
                }
                let tools = sh.tools();
                match mode {
                    crate::craft::CraftMode::Off => {
                        let _ = tools.hub.remove(spec.id).await;
                    }
                    crate::craft::CraftMode::Headless if tools.allow_craft_on_demand => {
                        let _ = crate::craft::ensure_headless(
                            spec.id,
                            &tools.roots_context(),
                            &tools.hub,
                        )
                        .await;
                    }
                    crate::craft::CraftMode::Live if tools.allow_craft_on_demand => {
                        if crate::craft::live_port_for(spec.id, spec.name).is_some() {
                            let _ = crate::craft::ensure_live_with_roots(
                                spec.id,
                                None,
                                &tools.roots_context(),
                                &tools.hub,
                            )
                            .await;
                        } else {
                            let _ = tools.hub.remove(spec.id).await;
                        }
                    }
                    _ => {}
                }
                Ok(json!({ "app": spec.id, "mode": mode.as_str() }))
            }
            "craft.open" => {
                let raw_app = need(&p, "app")?;
                let spec = crate::craft::find_spec(raw_app)
                    .ok_or_else(|| Error::NotFound(format!("Craft app '{raw_app}'")))?;
                let tools = sh.tools();
                let open_path = str_arg(&p, "path")
                    .filter(|s| !s.trim().is_empty())
                    .and_then(|r| tools.resolve(r).ok().map(|(x, _)| x));
                let status = crate::craft::ensure_live_with_roots(
                    spec.id,
                    open_path.as_deref(),
                    &tools.roots_context(),
                    &tools.hub,
                )
                .await
                .map_err(Error::Mcp)?;
                Ok(json!({ "app": spec.id, "status": status }))
            }
            "craft.test" => {
                let raw_app = need(&p, "app")?;
                let spec = crate::craft::find_spec(raw_app)
                    .ok_or_else(|| Error::NotFound(format!("Craft app '{raw_app}'")))?;
                let tools = sh.tools();
                let cfg = sh.config();
                let mode = crate::craft::mode_for(spec.id, &cfg.craft_apps, &cfg.craft_modes);
                let roots = tools.roots_context();
                let status = if mode == crate::craft::CraftMode::Live
                    && crate::craft::live_port_for(spec.id, spec.name).is_some()
                {
                    crate::craft::ensure_live_with_roots(spec.id, None, &roots, &tools.hub)
                        .await
                        .map_err(Error::Mcp)?
                } else {
                    crate::craft::ensure_headless(spec.id, &roots, &tools.hub)
                        .await
                        .map_err(Error::Mcp)?
                };
                let count = tools.hub.server_tools(spec.id).await.map_or(0, |v| v.len());
                let (found, _) = crate::craft::discover_with_errors();
                let version = found
                    .into_iter()
                    .find(|f| f.id == spec.id)
                    .and_then(|f| f.version);
                Ok(json!({
                    "ok": true,
                    "app": spec.id,
                    "name": spec.name,
                    "version": version,
                    "mode": mode.as_str(),
                    "tools": count,
                    "status": status,
                }))
            }
            "checkpoint.list" => Ok(sh.tools().checkpoints.list(need(&p, "session")?)),
            "checkpoint.restore" => sh
                .tools()
                .checkpoints
                .restore(need(&p, "session")?, need(&p, "turn")?),
            "chat.export" => {
                let s = sh.store.load(need(&p, "session")?)?;
                let format = crate::export::Format::parse(str_arg(&p, "format").unwrap_or("md"))?;
                let leaf = str_arg(&p, "leaf")
                    .map(str::to_string)
                    .or_else(|| s.tip().map(|m| m.id.clone()));
                let msgs: Vec<Message> = leaf
                    .as_deref()
                    .map(|l| s.path_to(l).into_iter().cloned().collect())
                    .unwrap_or_default();
                let title = if s.header.title.trim().is_empty() {
                    "Chat".to_string()
                } else {
                    s.header.title.clone()
                };
                Ok(json!({
                    "text": crate::export::export(&title, &msgs, format)?,
                    "file_name": crate::export::file_name(&title, format),
                }))
            }
            "chat.export_all" => {
                let mut all = Vec::new();
                for sum in sh.store.list()? {
                    if let Ok(s) = sh.store.load(&sum.id) {
                        let msgs: Vec<Message> = s
                            .tip()
                            .map(|t| s.path_to(&t.id).into_iter().cloned().collect())
                            .unwrap_or_default();
                        all.push((s.header.title.clone(), msgs));
                    }
                }
                Ok(json!({
                    "text": serde_json::to_string_pretty(&crate::export::bundle(all))?,
                    "file_name": format!("workcraft-chats-{}.json", chrono::Local::now().format("%Y-%m-%d")),
                }))
            }
            "skill.list" => Ok(json!(
                sh.tools()
                    .skills
                    .list()
                    .iter()
                    .map(|s| json!({ "name": s.name, "description": s.description }))
                    .collect::<Vec<_>>()
            )),
            "skill.read" => {
                let name = need(&p, "name")?;
                let file = str_arg(&p, "file");
                let text = sh.tools().skills.read(name, file)?;
                let desc = sh
                    .tools()
                    .skills
                    .list()
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case(name))
                    .map(|s| s.description.clone())
                    .unwrap_or_default();
                Ok(json!({ "text": text, "description": desc }))
            }
            "skill.save" => {
                let name = need(&p, "name")?.trim();
                if !valid_id(name) {
                    return Err(Error::Invalid(
                        "skill name must be letters, digits, '-' or '_'".into(),
                    ));
                }
                let description = str_arg(&p, "description").unwrap_or("").trim();
                let body = str_arg(&p, "body").unwrap_or("").trim();
                let folder = sh.data_dir.join("skills").join(name);
                std::fs::create_dir_all(&folder)?;
                let desc_clean = description.replace('\r', "").replace('\n', " ");
                let md = format!(
                    "---\nname: {name}\ndescription: >\n  {}\n---\n\n{}\n",
                    desc_clean.trim(),
                    body
                );
                std::fs::write(folder.join("SKILL.md"), md)?;
                self.sync_tools(Some(scan_skills(&sh.data_dir)), None);
                Ok(json!(
                    sh.tools()
                        .skills
                        .list()
                        .iter()
                        .map(|s| json!({ "name": s.name, "description": s.description }))
                        .collect::<Vec<_>>()
                ))
            }
            "skill.remove" => {
                let name = need(&p, "name")?.trim();
                if !valid_id(name) {
                    return Err(Error::Invalid(
                        "skill name must be letters, digits, '-' or '_'".into(),
                    ));
                }
                let folder = sh.data_dir.join("skills").join(name);
                let removed = folder.exists() && std::fs::remove_dir_all(&folder).is_ok();
                self.sync_tools(Some(scan_skills(&sh.data_dir)), None);
                Ok(json!({ "removed": removed }))
            }
            "config.get" => masked_config(&sh.config()),
            "config.set" => {
                let saved = {
                    let mut c = wr(&sh.config);
                    if let Some(v) = p.get("think") {
                        c.think = v.as_bool();
                    }
                    if let Some(v) = p.get("num_ctx") {
                        c.num_ctx = v.as_u64().map(|n| n.clamp(512, 1_048_576) as u32);
                    }
                    if let Some(v) = p.get("max_steps").and_then(Value::as_u64) {
                        c.max_steps = v.clamp(1, 200) as usize;
                    }
                    if let Some(v) = p.get("system_prompt") {
                        c.system_prompt = v
                            .as_str()
                            .map(str::to_string)
                            .filter(|s| !s.trim().is_empty());
                    }
                    if let Some(arr) = p.get("extra_roots").and_then(Value::as_array) {
                        c.extra_roots = arr
                            .iter()
                            .filter_map(Value::as_str)
                            .filter(|s| !s.trim().is_empty())
                            .map(PathBuf::from)
                            .collect();
                    }
                    if let Some(b) = p.get("full_tool_catalog").and_then(Value::as_bool) {
                        c.full_tool_catalog = b;
                    }
                    if let Some(b) = p.get("tool_routing").and_then(Value::as_bool) {
                        c.tool_routing = b;
                    }
                    if let Some(v) = p.get("fallback_model") {
                        c.fallback_model = v
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string);
                    }
                    if let Some(v) = p.get("context_window") {
                        c.context_window = v.as_u64().map(|n| n.clamp(2048, 2_000_000) as u32);
                    }
                    if let Some(b) = p.get("craft_anything_deep").and_then(Value::as_bool) {
                        c.craft_anything_deep = b;
                    }
                    if let Some(b) = p.get("craft_anything_allow_run").and_then(Value::as_bool) {
                        c.craft_anything_allow_run = b;
                    }
                    c.save(&sh.data_dir)?;
                    c.clone()
                };
                self.sync_tools(None, None);
                masked_config(&saved)
            }
            other => Err(Error::NotFound(format!("command '{other}'"))),
        }
    }
}

fn cmd(id: &str, title: &str, description: &str, props: Value, required: &[&str]) -> CommandInfo {
    CommandInfo {
        id: id.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        params: json!({ "type": "object", "properties": props, "required": required }),
    }
}

#[async_trait]
impl CommandBus for Engine {
    fn commands(&self) -> Vec<CommandInfo> {
        let s = |d: &str| json!({ "type": "string", "description": d });
        vec![
            cmd(
                "chat.new",
                "New chat",
                "Create a session.",
                json!({ "title": s("Title"), "model": s("provider:model"), "mode": { "type": "string", "enum": ["single", "team"] } }),
                &[],
            ),
            cmd(
                "chat.list",
                "List chats",
                "All sessions, newest first.",
                json!({}),
                &[],
            ),
            cmd(
                "chat.get",
                "Get chat",
                "Messages of a session along one branch (default: the latest).",
                json!({ "session": s("Session id"), "leaf": s("Message id that ends the branch") }),
                &["session"],
            ),
            cmd(
                "chat.tree",
                "Chat tree",
                "Every message of a session with its parent, for branch browsing.",
                json!({ "session": s("Session id") }),
                &["session"],
            ),
            cmd(
                "chat.children",
                "Branches",
                "The replies below one message; more than one means a branch.",
                json!({ "session": s("Session id"), "message": s("Message id") }),
                &["session", "message"],
            ),
            cmd(
                "chat.send",
                "Send message",
                "Send a message and wait for the answer. Pass leaf to branch from an earlier message, team to run a team.",
                json!({ "session": s("Session id; omitted creates one"), "text": s("Message"), "leaf": s("Message to branch from"), "model": s("provider:model for this turn"), "team": s("Team name"), "attachments": { "type": "array", "items": { "type": "object" } } }),
                &["text"],
            ),
            cmd(
                "chat.cancel",
                "Stop",
                "Stop the running answer of a session.",
                json!({ "session": s("Session id") }),
                &["session"],
            ),
            cmd(
                "chat.delete",
                "Delete chat",
                "Delete a session.",
                json!({ "session": s("Session id") }),
                &["session"],
            ),
            cmd(
                "chat.rename",
                "Rename chat",
                "Change the title of a session.",
                json!({ "session": s("Session id"), "title": s("Title") }),
                &["session", "title"],
            ),
            cmd(
                "model.get",
                "Current model",
                "The default model.",
                json!({}),
                &[],
            ),
            cmd(
                "model.set",
                "Set model",
                "Set the default model, or the model of one session (switch local and cloud mid-chat).",
                json!({ "model": s("provider:model"), "session": s("Session id") }),
                &["model"],
            ),
            cmd(
                "model.list",
                "List models",
                "Models offered by each provider.",
                json!({ "provider": s("Only this provider") }),
                &[],
            ),
            cmd(
                "provider.list",
                "List providers",
                "Configured provider ids.",
                json!({}),
                &[],
            ),
            cmd(
                "craft.list",
                "Craft apps",
                "All 17 ArtCraft apps with installed status, version, mode, port, tool count, and health.",
                json!({}),
                &[],
            ),
            cmd(
                "team.steer",
                "Steer a running plan",
                "Skip a task of the plan that is running, or run it again now with an extra instruction.",
                json!({ "session": s("Session id"), "task": s("Task id, for example t2"), "action": { "type": "string", "enum": ["skip", "redo"] }, "note": s("For redo: what to do differently") }),
                &["session", "task", "action"],
            ),
            cmd(
                "tools.stats",
                "Request size",
                "How many tokens the system prompt and each tool group add to every model request.",
                json!({ "text": s("Optional question: show the size the router would send for it") }),
                &[],
            ),
            cmd(
                "craft.set",
                "Switch a Craft app",
                "Turn a Craft app on or off as a startup MCP server.",
                json!({ "app": s("App id, for example wordcraft"), "enabled": { "type": "boolean" } }),
                &["app", "enabled"],
            ),
            cmd(
                "craft.mode",
                "Set Craft app mode",
                "Set a Craft app's execution mode: headless (default), live (desktop window), or off.",
                json!({ "app": s("App id, for example photocraft"), "mode": { "type": "string", "enum": ["headless", "live", "off"] } }),
                &["app", "mode"],
            ),
            cmd(
                "craft.open",
                "Open Craft app window",
                "Launch or attach to a Craft desktop app window on its control port.",
                json!({ "app": s("App id"), "path": s("Optional file path") }),
                &["app"],
            ),
            cmd(
                "craft.test",
                "Test Craft app MCP",
                "Run a connection and tool-list health check against an installed Craft app.",
                json!({ "app": s("App id") }),
                &["app"],
            ),
            cmd(
                "checkpoint.list",
                "Undo history",
                "The answers of a session that changed files, newest first, with the files they changed.",
                json!({ "session": s("Session id") }),
                &["session"],
            ),
            cmd(
                "checkpoint.restore",
                "Undo changes",
                "Put back every file one answer changed (created files are removed).",
                json!({ "session": s("Session id"), "turn": s("Turn id from checkpoint.list") }),
                &["session", "turn"],
            ),
            cmd(
                "chat.export",
                "Export chat",
                "A conversation as Markdown, JSON, HTML or plain text.",
                json!({ "session": s("Session id"), "format": { "type": "string", "enum": ["md", "json", "html", "txt"] }, "leaf": s("Branch tip, default the newest") }),
                &["session"],
            ),
            cmd(
                "chat.export_all",
                "Export all chats",
                "Every chat in one JSON document, for a backup.",
                json!({}),
                &[],
            ),
            cmd(
                "provider.info",
                "Provider details",
                "Every provider with its kind, address and whether it has a key (keys are never returned).",
                json!({}),
                &[],
            ),
            cmd(
                "provider.save",
                "Save provider",
                "Add or change a provider: an OpenAI-compatible server, Anthropic or Ollama. api_key is saved in config.json (null removes it); key_env names an environment variable instead.",
                json!({
                    "id": s("Provider id, lowercase, for example 'deepseek' or 'custom-lab'"),
                    "kind": { "type": "string", "enum": ["openai", "anthropic", "ollama"] },
                    "name": s("Display name"),
                    "base_url": s("API address, for example https://api.deepseek.com/v1"),
                    "key_env": s("Environment variable holding the key"),
                    "api_key": { "type": ["string", "null"], "description": "Key to save; null removes it" }
                }),
                &["id"],
            ),
            cmd(
                "provider.remove",
                "Remove provider",
                "Remove a provider added in the settings.",
                json!({ "id": s("Provider id") }),
                &["id"],
            ),
            cmd(
                "mode.set",
                "Set mode",
                "Switch a session between single and team mode.",
                json!({ "session": s("Session id"), "mode": { "type": "string", "enum": ["single", "team"] } }),
                &["session", "mode"],
            ),
            cmd(
                "approval.mode",
                "Approval mode",
                "Read or set manual or auto approval of tool calls.",
                json!({ "mode": { "type": "string", "enum": ["manual", "auto"] } }),
                &[],
            ),
            cmd(
                "approval.list",
                "Pending approvals",
                "Calls waiting for a person.",
                json!({}),
                &[],
            ),
            cmd(
                "approval.respond",
                "Answer approval",
                "Allow or deny a waiting call.",
                json!({ "id": s("Approval id"), "allow": { "type": "boolean" } }),
                &["id", "allow"],
            ),
            cmd(
                "agent.list",
                "List agents",
                "Agent definitions.",
                json!({}),
                &[],
            ),
            cmd(
                "agent.define",
                "Define agent",
                "Create or replace an agent: id, name, scope, prompt, model, tools (patterns), skills.",
                json!({ "agent": { "type": "object" } }),
                &["agent"],
            ),
            cmd(
                "agent.remove",
                "Remove agent",
                "Delete an agent definition.",
                json!({ "id": s("Agent id") }),
                &["id"],
            ),
            cmd("team.list", "List teams", "Team presets.", json!({}), &[]),
            cmd(
                "team.define",
                "Define team",
                "Create or replace a team preset (leader, workers, concurrency, review).",
                json!({ "team": { "type": "object" } }),
                &["team"],
            ),
            cmd(
                "team.remove",
                "Remove team",
                "Delete a custom team preset (restores default if 'craft').",
                json!({ "name": s("Team name") }),
                &["name"],
            ),
            cmd(
                "team.run",
                "Run team",
                "Give a goal to a team: the leader plans, workers run in parallel, the leader reviews and answers.",
                json!({ "goal": s("Goal"), "team": s("Team name, default craft"), "session": s("Session id"), "model": s("provider:model") }),
                &["goal"],
            ),
            cmd(
                "mcp.status",
                "MCP status",
                "Connected MCP servers and tool count.",
                json!({}),
                &[],
            ),
            cmd(
                "mcp.reload",
                "Reload MCP",
                "Reconnect the servers in mcp.json.",
                json!({}),
                &[],
            ),
            cmd(
                "skill.list",
                "List skills",
                "Available skills.",
                json!({}),
                &[],
            ),
            cmd(
                "skill.read",
                "Read skill",
                "Body of a skill or one of its files.",
                json!({ "name": s("Skill"), "file": s("File under the skill") }),
                &["name"],
            ),
            cmd(
                "skill.save",
                "Save skill",
                "Create or update a skill (SKILL.md with name, description and Markdown instructions).",
                json!({ "name": s("Skill id"), "description": s("Short description"), "body": s("Markdown instructions") }),
                &["name", "body"],
            ),
            cmd(
                "skill.remove",
                "Remove skill",
                "Delete a custom skill folder.",
                json!({ "name": s("Skill id") }),
                &["name"],
            ),
            cmd("config.get", "Config", "Current settings.", json!({}), &[]),
            cmd(
                "config.set",
                "Change config",
                "Set think, num_ctx, max_steps, system_prompt, extra_roots, full_tool_catalog, craft_anything_deep, or craft_anything_allow_run.",
                json!({
                    "think": { "type": ["boolean", "null"] },
                    "num_ctx": { "type": "integer" },
                    "max_steps": { "type": "integer" },
                    "system_prompt": s("Extra system prompt"),
                    "extra_roots": { "type": "array", "items": { "type": "string" } },
                    "full_tool_catalog": { "type": "boolean" },
                    "craft_anything_deep": { "type": "boolean" },
                    "craft_anything_allow_run": { "type": "boolean" }
                }),
                &[],
            ),
        ]
    }

    async fn execute(&self, id: &str, params: Value) -> Result<Value> {
        let params = if params.is_null() { json!({}) } else { params };
        self.run_command(id, params).await
    }

    fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.shared.events.subscribe()
    }
}

/// For tests and tools: a map of command id to title.
#[must_use]
pub fn command_index(e: &Engine) -> HashMap<String, String> {
    e.commands().into_iter().map(|c| (c.id, c.title)).collect()
}
