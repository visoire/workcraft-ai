//! The WorkCraft window: chats on the left, the conversation in the middle, agent activity (team,
//! plan, tool calls) on the right. Everything it does is an engine command, so anything you can click
//! can also be done over the control channel or MCP.
//!
//! This file holds the state and the event handling; the screens are drawn in the `view_*` modules.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use eframe::egui;
use serde_json::{Value, json};
use workcraft_core::{Event, Message, Role, TaskStatus, paths};

use crate::assets::Assets;
use crate::bridge::{Bridge, UiMsg};
use crate::theme::{self, ThemePref};

pub(crate) struct SessionItem {
    pub id: String,
    pub title: String,
    pub updated: i64,
}

pub(crate) struct ToolRow {
    pub id: String,
    pub agent: Option<String>,
    pub name: String,
    pub args: String,
    pub result: Option<(bool, String)>,
    pub artifacts: Vec<String>,
    pub mode: Option<String>,
    pub started: f64,
    pub finished: Option<f64>,
}

pub(crate) struct PendingApproval {
    pub id: String,
    /// The chat the call belongs to (it may not be the open one).
    pub session: String,
    pub agent: Option<String>,
    pub tool: String,
    pub args: String,
    pub reason: Option<String>,
    pub destructive: bool,
}

pub(crate) struct TaskRow {
    pub id: String,
    pub title: String,
    pub agent: String,
    pub status: TaskStatus,
    pub note: Option<String>,
    pub depends: Vec<String>,
}

/// What an agent is doing right now, as the activity panel shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentStatus {
    Idle,
    Thinking,
    Writing,
    Tool,
    Waiting,
    Reviewing,
    Done,
    Failed,
}

pub(crate) struct AgentLive {
    pub status: AgentStatus,
    /// The tool or task the status is about.
    pub detail: String,
    /// The tail of what the agent is writing.
    pub text: String,
}

#[derive(Clone)]
pub(crate) struct AgentInfo {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub prompt: String,
    pub skills: Vec<String>,
    pub tools: Vec<String>,
    pub model: Option<String>,
    /// `manual` or `auto` when the agent has its own approval; `None` follows the team.
    pub approval: Option<String>,
}

/// An agent/role being created or edited in Settings > Agents.
#[derive(Clone, Default)]
pub(crate) struct AgentEdit {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub prompt: String,
    pub model: String,
    pub skills: String,
    pub tools: String,
    /// "" follows the team, otherwise `manual` or `auto`.
    pub approval: String,
    pub is_new: bool,
}

/// Team settings being edited in Settings > Agents.
#[derive(Clone, Default)]
pub(crate) struct TeamEdit {
    pub name: String,
    pub leader_name: String,
    pub leader_scope: String,
    pub leader_prompt: String,
    pub concurrency: u32,
    pub review: bool,
    /// `None` asks for the plan only when the team asks for approvals.
    pub plan_approval: Option<bool>,
}

/// A custom or edited skill in Settings > Skills.
#[derive(Clone, Default)]
pub(crate) struct SkillEdit {
    pub name: String,
    pub description: String,
    pub body: String,
    pub is_new: bool,
}

/// Things a click asks for. Collected while drawing, applied afterwards, so drawing never needs `&mut self`.
pub(crate) enum Action {
    Choose {
        parent_key: String,
        id: String,
    },
    Branch {
        from: Option<String>,
        text: String,
    },
    /// Copy `text`; `key` names the button so it can say "Copied".
    Copy {
        key: String,
        text: String,
    },
    Prompt(String),
    /// Ask the same question again as a new version (branch) of the answer.
    Regenerate {
        from: Option<String>,
        text: String,
    },
    Approve {
        id: String,
        allow: bool,
    },
    AutoApprove,
    /// Put back the files one answer changed (turn = the user message it answered).
    Undo(String),
    /// Open a local artifact/image path in the OS viewer.
    OpenPath(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsTab {
    General,
    Models,
    Providers,
    Agents,
    Tools,
    Skills,
}

/// A provider being set up in Settings > Providers.
#[derive(Clone, Default)]
pub(crate) struct ProviderEdit {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub base_url: String,
    pub key_env: String,
    /// What the person typed; empty keeps the saved key.
    pub key: String,
    pub show_key: bool,
    /// A new OpenAI-compatible endpoint (the id comes from the name).
    pub custom: bool,
}

/// Settings being edited, applied on Save.
#[derive(Default)]
pub(crate) struct Draft {
    pub system_prompt: String,
    pub num_ctx: String,
    pub max_steps: u32,
    /// `None` = model default.
    pub think: Option<bool>,
    pub default_model: String,
}

pub struct App {
    pub(crate) bridge: Bridge,
    rx: Receiver<UiMsg>,
    files_tx: Sender<Vec<std::path::PathBuf>>,
    files_rx: Receiver<Vec<std::path::PathBuf>>,
    /// `provider.info`: every provider with its key status (never the key).
    pub(crate) provider_info: Vec<Value>,
    /// The provider open in the editor in Settings > Providers.
    pub(crate) provider_edit: Option<ProviderEdit>,
    pub(crate) settings_error: Option<String>,
    /// Test hook (`WORKCRAFT_UI_OPEN`), see `logic`.
    ui_open: Option<String>,
    pub(crate) open_picker: bool,
    /// Keep repainting until this time (see `logic`).
    pub(crate) anim_until: f64,
    pub(crate) settings_scroll: Option<f32>,
    pub(crate) assets: Assets,
    pub(crate) sessions: Vec<SessionItem>,
    pub(crate) current: Option<String>,
    pub(crate) mode_team: bool,
    /// The team Team mode runs: `craft` (five specialists) or `apps` (one agent for every Craft app).
    pub(crate) team_preset: String,
    pub(crate) messages: Vec<Message>,
    pub(crate) choice: HashMap<String, String>,
    pub(crate) stream: BTreeMap<String, String>,
    pub(crate) reasoning: String,
    pub(crate) busy: bool,
    /// Scroll mode (middle click): the point where it started.
    pub(crate) autoscroll: std::cell::Cell<Option<egui::Pos2>>,
    /// Chats with a reply or a team run in progress: the open one and the ones left running in the background.
    pub(crate) running: HashSet<String>,
    /// Chats that finished while another chat was open (the list shows a dot until the chat is opened).
    pub(crate) unseen_done: HashSet<String>,
    /// A message was sent from a new chat that has no id yet: the first turn that starts is its turn.
    pub(crate) awaiting_first: bool,
    pub(crate) turn_started: Option<f64>,
    /// Tokens and cost of the last finished reply, for the toolbar ("1840 in (72% cached), 95 out, $0.0012").
    pub(crate) last_usage: Option<String>,
    /// The side panel shows 0 the activity of the reply, 1 the artifacts of the chat.
    pub(crate) activity_tab: u8,
    /// Pictures and files the tools of this chat made: ((session, message count) it was built for, list).
    pub(crate) artifacts: (
        (Option<String>, usize),
        Vec<crate::view_artifacts::Artifact>,
    ),
    /// The picture shown full size, and its texture.
    pub(crate) lightbox: Option<String>,
    pub(crate) lightbox_tex: Option<(String, egui::TextureHandle)>,
    pub(crate) tools: Vec<ToolRow>,
    pub(crate) approvals: Vec<PendingApproval>,
    pub(crate) tasks: Vec<TaskRow>,
    pub(crate) live: BTreeMap<String, AgentLive>,
    pub(crate) roster: Vec<AgentInfo>,
    pub(crate) agent_names: HashMap<String, String>,
    pub(crate) input: String,
    pub(crate) chips: Vec<crate::attach::Attached>,
    attach_tx: Sender<crate::attach::Attached>,
    attach_rx: Receiver<crate::attach::Attached>,
    next_attach: u64,
    /// Where an export was saved (or why it failed), from the save dialog thread.
    saved_tx: Sender<Result<String, String>>,
    saved_rx: Receiver<Result<String, String>>,
    /// Undo history of the open chat: turn id (user message) -> changed files.
    pub(crate) checkpoints: HashMap<String, Value>,
    /// A short message that is not an error ("Saved to ...").
    pub(crate) notice: Option<(String, f64)>,
    /// Text to put on the clipboard on the next frame (results of commands).
    to_copy: Option<String>,
    /// `craft.list`: the Craft apps on this computer.
    pub(crate) craft: Vec<Value>,
    /// Where the composer was drawn last frame (the drop zone).
    pub(crate) composer_rect: Option<egui::Rect>,
    /// Highlighted row of the @ / slash popup.
    pub(crate) suggest_sel: usize,
    /// Tools of the connected MCP servers, for the slash popup and Settings.
    pub(crate) mcp_tools: Vec<String>,
    pub(crate) branch_from: Option<String>,
    pub(crate) model: String,
    pub(crate) models: Vec<String>,
    pub(crate) providers: Vec<String>,
    pub(crate) model_errors: BTreeMap<String, String>,
    pub(crate) model_filter: String,
    pub(crate) auto_approve: bool,
    pub(crate) think_off: bool,
    pub(crate) config: Value,
    pub(crate) banner: Option<String>,
    pub(crate) theme: ThemePref,
    pub(crate) show_sidebar: bool,
    pub(crate) show_activity: bool,
    pub(crate) show_settings: bool,
    pub(crate) settings_tab: SettingsTab,
    pub(crate) draft: Draft,
    pub(crate) search: String,
    pub(crate) pending_delete: Option<String>,
    pub(crate) copied: Option<(String, f64)>,
    pub(crate) now: f64,
    pub(crate) mcp_status: Option<Value>,
    pub(crate) skills: Vec<(String, String)>,
    pub(crate) blip: crate::blip::Blip,
    pub(crate) voice: crate::voice::VoiceController,
    pub(crate) agent_edit: Option<AgentEdit>,
    pub(crate) team_edit: Option<TeamEdit>,
    pub(crate) skill_edit: Option<SkillEdit>,
    pub(crate) team_raw: Value,
    pub(crate) extra_root_input: String,
    pub(crate) tool_thumbs: std::cell::RefCell<HashMap<String, Option<(egui::TextureHandle, f32)>>>,
}

/// Window preferences live next to the engine's files, in `ui.json`.
fn prefs_file() -> std::path::PathBuf {
    paths::data_dir().join("ui.json")
}

fn load_prefs() -> Value {
    std::fs::read_to_string(prefs_file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        rt: Arc<tokio::runtime::Runtime>,
        engine: workcraft_agents::Engine,
    ) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let bridge = Bridge::new(rt, engine, tx, cc.egui_ctx.clone());
        let (files_tx, files_rx) = std::sync::mpsc::channel();
        let (attach_tx, attach_rx) = std::sync::mpsc::channel();
        let (saved_tx, saved_rx) = std::sync::mpsc::channel();
        let prefs = load_prefs();
        let theme = ThemePref::parse(prefs["theme"].as_str().unwrap_or("system"));
        theme::install(&cc.egui_ctx, theme);
        let app = Self {
            bridge,
            rx,
            files_tx,
            files_rx,
            provider_info: Vec::new(),
            provider_edit: None,
            settings_error: None,
            ui_open: std::env::var("WORKCRAFT_UI_OPEN")
                .ok()
                .filter(|s| !s.is_empty()),
            open_picker: false,
            anim_until: 0.0,
            settings_scroll: None,
            assets: Assets::load(&cc.egui_ctx),
            sessions: Vec::new(),
            current: None,
            mode_team: false,
            messages: Vec::new(),
            choice: HashMap::new(),
            stream: BTreeMap::new(),
            reasoning: String::new(),
            busy: false,
            autoscroll: std::cell::Cell::new(None),
            running: HashSet::new(),
            unseen_done: HashSet::new(),
            awaiting_first: false,
            turn_started: None,
            last_usage: None,
            // `WORKCRAFT_UI_TAB=artifacts` opens the side panel on the artifacts, for window captures.
            activity_tab: u8::from(
                std::env::var("WORKCRAFT_UI_TAB").is_ok_and(|v| v == "artifacts"),
            ),
            artifacts: ((None, 0), Vec::new()),
            lightbox: None,
            lightbox_tex: None,
            tools: Vec::new(),
            approvals: Vec::new(),
            tasks: Vec::new(),
            live: BTreeMap::new(),
            roster: Vec::new(),
            agent_names: HashMap::new(),
            input: String::new(),
            chips: Vec::new(),
            attach_tx,
            attach_rx,
            next_attach: 1,
            saved_tx,
            saved_rx,
            checkpoints: HashMap::new(),
            notice: None,
            to_copy: None,
            craft: Vec::new(),
            composer_rect: None,
            suggest_sel: 0,
            mcp_tools: Vec::new(),
            branch_from: None,
            model: String::new(),
            models: Vec::new(),
            providers: Vec::new(),
            model_errors: BTreeMap::new(),
            model_filter: String::new(),
            auto_approve: false,
            think_off: false,
            config: Value::Null,
            banner: None,
            theme,
            show_sidebar: prefs["sidebar"].as_bool().unwrap_or(true),
            show_activity: prefs["activity"].as_bool().unwrap_or(true),
            show_settings: false,
            settings_tab: SettingsTab::General,
            draft: Draft::default(),
            search: String::new(),
            pending_delete: None,
            copied: None,
            now: 0.0,
            mcp_status: None,
            skills: Vec::new(),
            blip: {
                let mut b = crate::blip::Blip::default();
                if let Some(t) = prefs["blip_theme"].as_str() {
                    b.theme = crate::blip::BlipTheme::parse(t);
                }
                if let Some(a) = prefs["blip_accessory"].as_str() {
                    b.accessory = crate::blip::BlipAccessory::parse(a);
                }
                if let Some(a) = prefs["blip_anim"].as_str() {
                    b.anim = crate::blip::BlipAnim::parse(a);
                }
                if let Some(v) = prefs["blip_bubbles"].as_bool() {
                    b.bubbles = v;
                }
                if let Some(z) = prefs["blip_size"].as_str() {
                    b.size = crate::blip::BlipSize::parse(z);
                }
                crate::theme::set_accent(&cc.egui_ctx, b.theme.accent());
                b
            },
            voice: {
                let mut v = crate::voice::VoiceController::new();
                if let Some(eng) = prefs["voice_engine"].as_str() {
                    v.engine_pref = match eng {
                        "handy" => crate::voice::VoiceEnginePref::HandyNemotron,
                        "windows" => crate::voice::VoiceEnginePref::WindowsSpeech,
                        "webspeech" => crate::voice::VoiceEnginePref::WebSpeechApi,
                        _ => crate::voice::VoiceEnginePref::Auto,
                    };
                }
                if let Some(auto_stop) = prefs["voice_auto_stop"].as_bool() {
                    v.auto_stop_on_silence = auto_stop;
                }
                if let Some(send_cmd) = prefs["voice_send_cmd"].as_bool() {
                    v.voice_send_command = send_cmd;
                }
                v
            },
            team_preset: prefs["team_preset"]
                .as_str()
                .filter(|t| matches!(*t, "craft" | "apps"))
                .unwrap_or("craft")
                .to_string(),
            agent_edit: None,
            team_edit: None,
            skill_edit: None,
            team_raw: Value::Null,
            extra_root_input: String::new(),
            tool_thumbs: std::cell::RefCell::new(HashMap::new()),
        };
        app.bridge.run("sessions", "chat.list", Value::Null);
        app.bridge.run("config", "config.get", Value::Null);
        app.bridge.run("models", "model.list", Value::Null);
        app.bridge.run("teams", "team.list", Value::Null);
        app.bridge.run("agents", "agent.list", Value::Null);
        // The MCP servers start in the background so the window opens at once.
        app.bridge.run("mcp", "mcp.reload", Value::Null);
        app.bridge.run("craft", "craft.list", Value::Null);
        app.bridge.run("skills", "skill.list", Value::Null);
        app.bridge
            .run("provider-info", "provider.info", Value::Null);
        app
    }

    pub(crate) fn save_prefs(&self) {
        let voice_engine = match self.voice.engine_pref {
            crate::voice::VoiceEnginePref::Auto => "auto",
            crate::voice::VoiceEnginePref::HandyNemotron => "handy",
            crate::voice::VoiceEnginePref::WindowsSpeech => "windows",
            crate::voice::VoiceEnginePref::WebSpeechApi => "webspeech",
        };
        let v = json!({
            "theme": self.theme.as_str(),
            "sidebar": self.show_sidebar,
            "activity": self.show_activity,
            "blip_theme": self.blip.theme.id(),
            "blip_accessory": self.blip.accessory.id(),
            "blip_anim": self.blip.anim.id(),
            "blip_bubbles": self.blip.bubbles,
            "blip_size": self.blip.size.id(),
            "voice_engine": voice_engine,
            "voice_auto_stop": self.voice.auto_stop_on_silence,
            "voice_send_cmd": self.voice.voice_send_command,
            "team_preset": self.team_preset,
        });
        let _ = std::fs::create_dir_all(paths::data_dir());
        let _ = std::fs::write(
            prefs_file(),
            serde_json::to_string_pretty(&v).unwrap_or_default(),
        );
    }

    pub(crate) fn set_theme(&mut self, ctx: &egui::Context, t: ThemePref) {
        self.theme = t;
        theme::set_theme(ctx, t);
        self.save_prefs();
    }

    /// Display name of an agent id (`None` is the single-mode assistant).
    pub(crate) fn agent_name(&self, id: Option<&str>) -> String {
        match id {
            None | Some("") => "WorkCraft".to_string(),
            Some(a) => self
                .agent_names
                .get(a)
                .cloned()
                .unwrap_or_else(|| a.to_string()),
        }
    }

    pub(crate) fn agent_skills(&self, id: &str) -> &[String] {
        self.roster
            .iter()
            .find(|a| a.id == id)
            .map_or(&[], |a| a.skills.as_slice())
    }

    // ------------------------------------------------------------------ incoming

    fn drain(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                UiMsg::Event(e) => self.on_event(*e),
                UiMsg::Done(tag, r) => self.on_done(tag, r),
            }
        }
    }

    fn mine(&mut self, session: &str) -> bool {
        match &self.current {
            Some(c) => c == session,
            // A chat started from here has no id until its first event arrives.
            None if self.awaiting_first => {
                self.current = Some(session.to_string());
                self.awaiting_first = false;
                true
            }
            None => false,
        }
    }

    /// Key of the live map for an agent: workers by id, the single assistant as "".
    fn live_key(&self, agent: Option<&str>) -> String {
        match agent {
            Some(a) => a.to_string(),
            None if self.mode_team => "leader".to_string(),
            None => String::new(),
        }
    }

    fn set_live(&mut self, key: String, status: AgentStatus, detail: Option<String>) {
        let e = self.live.entry(key).or_insert(AgentLive {
            status,
            detail: String::new(),
            text: String::new(),
        });
        e.status = status;
        if let Some(d) = detail {
            e.detail = d;
        }
    }

    fn on_event(&mut self, e: Event) {
        match e {
            Event::MessageAdded { session, message } => {
                if self.mine(&session) {
                    if message.role == Role::Assistant
                        && message.agent.as_deref().is_none_or(|a| a == "leader")
                    {
                        self.stream.clear();
                        self.reasoning.clear();
                    }
                    if !self.messages.iter().any(|m| m.id == message.id) {
                        self.messages.push(message);
                    }
                }
            }
            Event::Text {
                session,
                agent,
                delta,
            } => {
                if self.mine(&session) {
                    let key = self.live_key(agent.as_deref());
                    if self.mode_team || !key.is_empty() {
                        self.set_live(key.clone(), AgentStatus::Writing, None);
                        if let Some(l) = self.live.get_mut(&key) {
                            l.text.push_str(&delta);
                            let n = l.text.chars().count();
                            if n > 600 {
                                l.text = l.text.chars().skip(n - 400).collect();
                            }
                        }
                    }
                    self.stream
                        .entry(agent.unwrap_or_default())
                        .or_default()
                        .push_str(&delta);
                }
            }
            Event::Reasoning {
                session,
                agent,
                delta,
            } => {
                if self.mine(&session) {
                    let key = self.live_key(agent.as_deref());
                    if self.mode_team || !key.is_empty() {
                        let st = self.live.get(&key).map(|l| l.status);
                        if st != Some(AgentStatus::Reviewing) {
                            self.set_live(key, AgentStatus::Thinking, None);
                        }
                    }
                    if agent.is_none() {
                        self.reasoning.push_str(&delta);
                    }
                }
            }
            Event::TurnStarted { session } => {
                self.running.insert(session.clone());
                // An agent or script started a chat through the control channel: follow it when we are idle.
                if self.current.is_none() && !self.busy && !self.awaiting_first {
                    self.current = Some(session.clone());
                    self.messages.clear();
                    self.busy = true;
                    self.bridge
                        .run("tree", "chat.tree", json!({ "session": session.clone() }));
                    self.bridge.run(
                        "checkpoints",
                        "checkpoint.list",
                        json!({ "session": session }),
                    );
                }
                if self.mine(&session) {
                    self.busy = true;
                    self.turn_started = Some(self.now);
                    self.blip.set_state(crate::blip::BlipState::Thinking);
                    self.tools.clear();
                    self.tasks.clear();
                    self.live.clear();
                    if self.mode_team {
                        self.set_live(
                            "leader".into(),
                            AgentStatus::Thinking,
                            Some("Planning".into()),
                        );
                    }
                }
            }
            Event::TurnFinished {
                session,
                usage,
                cost_usd,
            } => {
                self.running.remove(&session);
                if !self.mine(&session) {
                    // Another chat finished while this one is open: say so quietly and mark it in the list.
                    self.unseen_done.insert(session.clone());
                    let title = self
                        .sessions
                        .iter()
                        .find(|s| s.id == session)
                        .map(|s| s.title.clone())
                        .filter(|t| !t.trim().is_empty())
                        .unwrap_or_else(|| "A chat".into());
                    self.notify(format!(
                        "{} finished in the background.",
                        crate::widgets::short(&title, 40)
                    ));
                }
                if self.mine(&session) {
                    self.last_usage = (usage.prompt_tokens() + usage.output_tokens > 0)
                        .then(|| usage.summary(cost_usd));
                    self.busy = false;
                    self.turn_started = None;
                    self.blip.set_state(crate::blip::BlipState::Idle);
                    let cost = cost_usd.map(workcraft_core::format_usd);
                    self.blip.on_turn_done(cost, self.now);
                    self.stream.clear();
                    self.reasoning.clear();
                    for (k, l) in self.live.iter_mut() {
                        if l.status != AgentStatus::Failed {
                            l.status = AgentStatus::Done;
                        }
                        // The leader's last step ("Reviewing ...") is over once the answer is in.
                        if k == "leader" {
                            l.detail = "Wrote the final answer".into();
                        }
                        l.text.clear();
                    }
                    self.bridge
                        .run("tree", "chat.tree", json!({ "session": session.clone() }));
                    self.bridge.run(
                        "checkpoints",
                        "checkpoint.list",
                        json!({ "session": session }),
                    );
                }
                self.bridge.run("sessions", "chat.list", Value::Null);
                // Settings may have been changed from outside (control channel, MCP).
                self.bridge.run("config", "config.get", Value::Null);
            }
            Event::ToolStarted {
                session,
                agent,
                call,
            } => {
                if !self.mine(&session) {
                    return;
                }
                self.blip.on_tool_start(&call.name, self.now);
                let key = self.live_key(agent.as_deref());
                self.set_live(key, AgentStatus::Tool, Some(call.name.clone()));
                self.tools.push(ToolRow {
                    id: call.id,
                    agent,
                    name: call.name,
                    args: serde_json::to_string_pretty(&call.arguments).unwrap_or_default(),
                    result: None,
                    artifacts: Vec::new(),
                    mode: None,
                    started: self.now,
                    finished: None,
                });
            }
            Event::ToolFinished {
                session,
                call_id,
                ok,
                output,
                agent,
                artifacts,
                mode,
            } => {
                if !self.mine(&session) {
                    return;
                }
                let key = self.live_key(agent.as_deref());
                self.set_live(key, AgentStatus::Thinking, None);
                let now = self.now;
                let mut tool_name = String::new();
                let has_art = !artifacts.is_empty();
                if let Some(t) = self.tools.iter_mut().find(|t| t.id == call_id) {
                    tool_name = t.name.clone();
                    t.result = Some((ok, output.clone()));
                    t.artifacts = artifacts;
                    t.mode = mode;
                    t.finished = Some(now);
                }
                if ok && has_art {
                    self.blip.on_render(&tool_name, now);
                }
                self.blip.on_tool_finished(&tool_name, ok, &output, now);
            }
            Event::ApprovalRequested {
                id,
                session,
                agent,
                call,
                reason,
                destructive,
            } => {
                // An approval is shown wherever you are (a chat left running still needs your answer).
                let here = self.current.as_deref() == Some(session.as_str());
                if here {
                    self.blip.on_approval_requested(&call.name, self.now);
                    let key = self.live_key(agent.as_deref());
                    self.set_live(key, AgentStatus::Waiting, Some(call.name.clone()));
                } else {
                    self.notify("Another chat is waiting for your OK.".to_string());
                }
                self.approvals.push(PendingApproval {
                    id,
                    session,
                    agent,
                    tool: call.name,
                    args: serde_json::to_string_pretty(&call.arguments).unwrap_or_default(),
                    reason,
                    destructive,
                });
            }
            Event::TeamPlan { session, tasks, .. } => {
                if !self.mine(&session) {
                    return;
                }
                self.tasks = tasks
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|t| TaskRow {
                        id: t["id"].as_str().unwrap_or("?").to_string(),
                        title: t["title"].as_str().unwrap_or("").to_string(),
                        agent: t["agent"].as_str().unwrap_or("?").to_string(),
                        status: TaskStatus::Pending,
                        note: None,
                        depends: t["depends_on"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect(),
                    })
                    .collect();
                self.set_live(
                    "leader".into(),
                    AgentStatus::Thinking,
                    Some("Assigning tasks".into()),
                );
            }
            Event::TaskUpdate {
                session,
                task,
                status,
                note,
                ..
            } => {
                if !self.mine(&session) {
                    return;
                }
                let (agent, title) = match self.tasks.iter_mut().find(|t| t.id == task) {
                    Some(t) => {
                        t.status = status;
                        t.note = note;
                        (t.agent.clone(), t.title.clone())
                    }
                    // A task added mid-run (hand-off or swarm).
                    None => {
                        self.tasks.push(TaskRow {
                            id: task.clone(),
                            title: String::new(),
                            agent: String::new(),
                            status,
                            note,
                            depends: Vec::new(),
                        });
                        (String::new(), task)
                    }
                };
                match status {
                    TaskStatus::Running if !agent.is_empty() => {
                        self.set_live(agent, AgentStatus::Thinking, Some(title));
                    }
                    TaskStatus::Reviewing => {
                        self.set_live(
                            "leader".into(),
                            AgentStatus::Reviewing,
                            Some(format!("Reviewing {title}")),
                        );
                        if !agent.is_empty() {
                            self.set_live(agent, AgentStatus::Done, Some(title));
                        }
                    }
                    TaskStatus::Done if !agent.is_empty() => {
                        self.set_live(agent, AgentStatus::Done, Some(title));
                    }
                    TaskStatus::Failed if !agent.is_empty() => {
                        self.set_live(agent, AgentStatus::Failed, Some(title));
                    }
                    _ => {}
                }
            }
            Event::Notice {
                session, message, ..
            } => {
                // A retry or a slow start: the companion says it while the request waits.
                if self.mine(&session) {
                    self.blip.on_notice(&message, self.now);
                    // Also in the chat, where it can be read: the toast above the composer.
                    self.notify(message);
                }
            }
            Event::Error { session, message } => {
                self.running.remove(&session);
                if self.current.as_deref() == Some(session.as_str()) || self.awaiting_first {
                    self.awaiting_first = false;
                    self.busy = false;
                    self.turn_started = None;
                    self.blip.on_error(&message, self.now);
                    self.banner = Some(message);
                } else {
                    self.notify(format!(
                        "Another chat stopped: {}",
                        crate::widgets::short(&message, 80)
                    ));
                }
            }
        }
    }

    fn on_done(&mut self, tag: &str, r: workcraft_core::Result<Value>) {
        let v = match r {
            Ok(v) => v,
            Err(e) => {
                if matches!(
                    tag,
                    "provider-saved"
                        | "provider-removed"
                        | "agent-saved"
                        | "agent-removed"
                        | "team-saved"
                        | "team-removed"
                        | "skill-saved"
                        | "skill-removed"
                        | "skill-read"
                ) {
                    // Shown inside the settings window, where the person is looking.
                    self.settings_error = Some(e.to_string());
                } else if !matches!(
                    tag,
                    "mcp" | "teams" | "agents" | "provider-info" | "craft" | "checkpoints"
                ) {
                    self.banner = Some(e.to_string());
                }
                if tag == "send" && self.awaiting_first {
                    self.awaiting_first = false;
                    self.busy = false;
                    self.turn_started = None;
                    self.blip.set_state(crate::blip::BlipState::Idle);
                    self.blip
                        .trigger(crate::blip::BlipState::Startled, 3.0, self.now);
                }
                return;
            }
        };
        match tag {
            "sessions" => {
                self.sessions = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|s| SessionItem {
                        id: s["id"].as_str().unwrap_or_default().to_string(),
                        title: s["title"].as_str().unwrap_or_default().to_string(),
                        updated: s["updated"].as_i64().unwrap_or(0),
                    })
                    .collect();
                self.sessions.sort_by_key(|s| std::cmp::Reverse(s.updated));
            }
            "tree" => {
                if let Ok(msgs) = serde_json::from_value::<Vec<Message>>(v["messages"].clone()) {
                    self.messages = msgs;
                }
                self.mode_team = v["header"]["mode"] == "team";
                if let Some(m) = v["header"]["model"].as_object() {
                    let p = m
                        .get("provider")
                        .and_then(Value::as_str)
                        .unwrap_or("ollama");
                    let n = m.get("model").and_then(Value::as_str).unwrap_or("");
                    self.model = format!("{p}:{n}");
                }
            }
            "send" => {
                // This answers one chat, which may no longer be the open one: never move the person back to it.
                if let Some(s) = v["session"].as_str() {
                    self.running.remove(s);
                    if self.current.as_deref() == Some(s) {
                        self.busy = false;
                        self.turn_started = None;
                        self.bridge
                            .run("checkpoints", "checkpoint.list", json!({ "session": s }));
                    } else if self.current.is_none() && self.awaiting_first {
                        self.current = Some(s.to_string());
                        self.awaiting_first = false;
                        self.busy = false;
                        self.turn_started = None;
                    }
                }
                self.bridge.run("sessions", "chat.list", Value::Null);
            }
            "models" => {
                self.models.clear();
                self.model_errors.clear();
                self.providers.clear();
                for (p, list) in v.as_object().into_iter().flatten() {
                    self.providers.push(p.clone());
                    match list.as_array() {
                        Some(a) => self.models.extend(
                            a.iter()
                                .filter_map(Value::as_str)
                                .map(|m| format!("{p}:{m}")),
                        ),
                        None => {
                            let e = list["error"].as_str().unwrap_or("not reachable");
                            self.model_errors.insert(p.clone(), e.to_string());
                        }
                    }
                }
                // The built-in default may not be installed; use a local model that is.
                if !self.models.contains(&self.model)
                    && let Some(first) = self.models.iter().find(|m| m.starts_with("ollama:"))
                {
                    self.model = first.clone();
                }
            }
            "config" => {
                if self.model.is_empty() {
                    self.model = v["model"].as_str().unwrap_or_default().to_string();
                }
                self.auto_approve = v["approval"] == "auto";
                self.think_off = v["think"] == false;
                self.config = v;
            }
            "config-saved" => {
                self.config = v;
                self.think_off = self.config["think"] == false;
            }
            "mcp" => {
                self.bridge.run("mcpstatus", "mcp.status", Value::Null);
                self.bridge.run("craft", "craft.list", Value::Null);
            }
            "mcpstatus" => {
                self.mcp_tools = v["tool_names"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                self.mcp_status = Some(v);
            }
            "craft" => {
                self.craft = v.as_array().cloned().unwrap_or_default();
            }
            "craft-set" | "craft-mode" => {
                self.mcp_status = None;
                self.bridge.run("craft", "craft.list", Value::Null);
                self.bridge.run("mcpstatus", "mcp.status", Value::Null);
            }
            "craft-test" => {
                let app = v["app"].as_str().unwrap_or("app");
                if v["ok"] == true {
                    let count = v["tool_count"].as_u64().unwrap_or(0);
                    let mode = v["mode"].as_str().unwrap_or("headless");
                    self.notify(format!("{app} ({mode}): {count} tools verified"));
                } else {
                    let err = v["error"].as_str().unwrap_or("health check failed");
                    self.banner = Some(format!("{app}: {err}"));
                }
                self.bridge.run("craft", "craft.list", Value::Null);
                self.bridge.run("mcpstatus", "mcp.status", Value::Null);
            }
            "craft-open" => {
                if let Some(st) = v["status"].as_str() {
                    self.notify(st);
                    self.blip
                        .trigger(crate::blip::BlipState::Happy, 2.2, self.now);
                    self.blip.say(st, 2.6, self.now);
                }
                self.bridge.run("craft", "craft.list", Value::Null);
                self.bridge.run("mcpstatus", "mcp.status", Value::Null);
            }
            "checkpoints" => {
                self.checkpoints = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c["turn"].as_str().map(|t| (t.to_string(), c.clone())))
                    .collect();
            }
            "undo" => {
                let n = v["restored"].as_array().map_or(0, Vec::len);
                let failed: Vec<&str> = v["failed"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                if !failed.is_empty() {
                    self.banner = Some(format!(
                        "Some files could not be restored: {}",
                        failed.join("; ")
                    ));
                } else {
                    self.notify(if n == 1 {
                        "Restored 1 file.".to_string()
                    } else {
                        format!("Restored {n} files.")
                    });
                }
                if let Some(s) = &self.current {
                    self.bridge
                        .run("checkpoints", "checkpoint.list", json!({ "session": s }));
                }
            }
            "export" => {
                let name = v["file_name"].as_str().unwrap_or("chat.md").to_string();
                let text = v["text"].as_str().unwrap_or_default().to_string();
                self.save_with_dialog(name, text);
            }
            "export-copy" => {
                let text = v["text"].as_str().unwrap_or_default().to_string();
                self.to_copy = Some(text);
                self.notify("Copied chat to clipboard.");
            }
            "provider-info" => {
                self.provider_info = v.as_array().cloned().unwrap_or_default();
            }
            "provider-saved" => {
                self.provider_info = v.as_array().cloned().unwrap_or_default();
                self.provider_edit = None;
                self.copied = Some(("provider-saved".into(), self.now));
                self.bridge.run("models", "model.list", Value::Null);
            }
            "provider-removed" => {
                self.provider_edit = None;
                self.bridge
                    .run("provider-info", "provider.info", Value::Null);
                self.bridge.run("models", "model.list", Value::Null);
            }
            "skills" => {
                self.skills = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|s| {
                        (
                            s["name"].as_str().unwrap_or_default().to_string(),
                            s["description"].as_str().unwrap_or_default().to_string(),
                        )
                    })
                    .collect();
            }
            "teams" => {
                let teams: Vec<&Value> = v.as_array().into_iter().flatten().collect();
                let team = teams
                    .iter()
                    .find(|t| t["name"] == "craft")
                    .or_else(|| teams.first());
                if let Some(t) = team {
                    self.team_raw = (*t).clone();
                    self.roster = std::iter::once(&t["leader"])
                        .chain(t["workers"].as_array().into_iter().flatten())
                        .map(agent_info)
                        .collect();
                    // The per-app agents of the `apps` team are listed too, so each one can be opened and edited.
                    if let Some(apps) = teams.iter().find(|t| t["name"] == "apps") {
                        for w in apps["workers"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(agent_info)
                        {
                            if !self.roster.iter().any(|a| a.id == w.id) {
                                self.roster.push(w);
                            }
                        }
                    }
                    for a in &self.roster {
                        self.agent_names.insert(a.id.clone(), a.name.clone());
                    }
                }
            }
            "agents" => {
                for a in v.as_array().into_iter().flatten().map(agent_info) {
                    self.agent_names.entry(a.id.clone()).or_insert(a.name);
                }
            }
            "agent-saved" | "agent-removed" => {
                self.agent_edit = None;
                self.settings_error = None;
                self.bridge.run("teams", "team.list", Value::Null);
                self.bridge.run("agents", "agent.list", Value::Null);
            }
            "team-saved" | "team-removed" => {
                self.team_edit = None;
                self.settings_error = None;
                self.bridge.run("teams", "team.list", Value::Null);
                self.bridge.run("agents", "agent.list", Value::Null);
            }
            "skill-read" => {
                if let Some(ed) = self.skill_edit.as_mut() {
                    let raw = v["text"].as_str().unwrap_or_default();
                    let body = raw
                        .split("\n\n[More files, read with read_skill")
                        .next()
                        .unwrap_or(raw)
                        .to_string();
                    ed.body = body;
                    if ed.description.is_empty() {
                        ed.description = v["description"].as_str().unwrap_or_default().to_string();
                    }
                }
            }
            "skill-saved" => {
                self.skill_edit = None;
                self.settings_error = None;
                self.skills = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|s| {
                        (
                            s["name"].as_str().unwrap_or_default().to_string(),
                            s["description"].as_str().unwrap_or_default().to_string(),
                        )
                    })
                    .collect();
            }
            "skill-removed" => {
                self.skill_edit = None;
                self.settings_error = None;
                self.bridge.run("skills", "skill.list", Value::Null);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ outgoing

    pub(crate) fn new_chat(&mut self) {
        // A reply that is still running keeps running in its own chat; this window just stops showing it.
        self.current = None;
        self.busy = false;
        self.turn_started = None;
        self.awaiting_first = false;
        self.approvals
            .retain(|a| !a.session.is_empty() && self.running.contains(&a.session));
        self.messages.clear();
        self.choice.clear();
        self.stream.clear();
        self.tools.clear();
        self.tasks.clear();
        self.live.clear();
        self.checkpoints.clear();
        self.branch_from = None;
        self.banner = None;
        self.notice = None;
        self.blip
            .trigger(crate::blip::BlipState::Spawn, 1.3, self.now);
    }

    pub(crate) fn open(&mut self, id: &str) {
        self.new_chat();
        self.current = Some(id.to_string());
        self.unseen_done.remove(id);
        // Back to a chat that is still working: show it as working again.
        if self.running.contains(id) {
            self.busy = true;
            self.turn_started = Some(self.now);
            self.blip.set_state(crate::blip::BlipState::Thinking);
        } else {
            self.blip.set_state(crate::blip::BlipState::Idle);
        }
        self.bridge
            .run("tree", "chat.tree", json!({ "session": id }));
        self.bridge
            .run("checkpoints", "checkpoint.list", json!({ "session": id }));
    }

    pub(crate) fn set_mode(&mut self, team: bool) {
        self.mode_team = team;
        if let Some(s) = &self.current {
            self.bridge.run(
                "mode",
                "mode.set",
                json!({ "session": s, "mode": if team { "team" } else { "single" } }),
            );
        }
    }

    pub(crate) fn set_model(&mut self, m: String) {
        // Switching models mid-conversation applies from the next message.
        self.bridge.run(
            "model",
            "model.set",
            json!({ "model": m, "session": self.current }),
        );
        self.model = m;
    }

    pub(crate) fn set_auto_approve(&mut self, auto: bool) {
        self.auto_approve = auto;
        self.bridge.run(
            "approval",
            "approval.mode",
            json!({ "mode": if auto { "auto" } else { "manual" } }),
        );
    }

    pub(crate) fn set_think_off(&mut self, off: bool) {
        self.think_off = off;
        let v = if off { json!(false) } else { Value::Null };
        self.bridge
            .run("config-saved", "config.set", json!({ "think": v }));
    }

    pub(crate) fn cancel(&self) {
        if let Some(s) = &self.current {
            self.bridge
                .run("cancel", "chat.cancel", json!({ "session": s }));
        }
    }

    /// Runs a typed slash command (`/new`, `/stop`, `/team apps`, ...). `true` when the text was a command, so it is not sent.
    pub(crate) fn run_typed_command(&mut self, text: &str) -> bool {
        let mut parts = text.trim().splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("").to_ascii_lowercase();
        let arg = parts.next().unwrap_or("").trim().to_string();
        match cmd.as_str() {
            "/new" | "/clear" => self.new_chat(),
            "/stop" => {
                if self.busy {
                    self.cancel();
                } else {
                    self.notify("Nothing is running in this chat.");
                }
            }
            "/undo" => self.undo_last(),
            "/retry" | "/regenerate" => self.retry_last(),
            "/copy" => self.export("md", true),
            "/export" => {
                let f = match arg.as_str() {
                    "html" => "html",
                    "json" => "json",
                    _ => "md",
                };
                self.export(f, false);
            }
            "/team" => match arg.as_str() {
                "apps" | "craft" => self.set_team_preset(&arg),
                "off" | "single" => self.set_mode(false),
                _ => self.set_mode(true),
            },
            "/single" => self.set_mode(false),
            "/auto" => self.set_auto_approve(true),
            "/ask" => self.set_auto_approve(false),
            "/think" => {
                self.set_think_off(arg.eq_ignore_ascii_case("off"));
                self.notify(if arg.eq_ignore_ascii_case("off") { "Reasoning is off." } else { "Reasoning is on." });
            }
            "/model" => {
                if arg.is_empty() {
                    self.open_settings(SettingsTab::Models);
                } else {
                    self.set_model(arg);
                }
            }
            "/rename" => match (&self.current, arg.is_empty()) {
                (Some(s), false) => {
                    self.bridge.run("renamed", "chat.rename", json!({ "session": s, "title": arg }));
                    self.bridge.run("sessions", "chat.list", Value::Null);
                }
                (None, _) => self.notify("Send a message first, then rename the chat."),
                (_, true) => self.notify("Use /rename New title"),
            },
            "/data" => open_folder(&paths::data_dir()),
            "/rescan" => {
                self.bridge.run("craft", "craft.list", json!({}));
                self.notify("Looking for Craft apps again...");
            }
            "/mcp" | "/apps" => self.open_settings(SettingsTab::Tools),
            "/skills" => self.open_settings(SettingsTab::Skills),
            "/agents" => self.open_settings(SettingsTab::Agents),
            "/settings" => self.open_settings(SettingsTab::General),
            "/help" => self.notify("/new /stop /retry /undo /team [apps|craft] /single /auto /ask /think on|off /model /rename /export /copy /data /rescan /apps /skills /agents /settings"),
            _ => return false,
        }
        true
    }

    /// Ask the last question again as a new version of the answer.
    pub(crate) fn retry_last(&mut self) {
        if self.busy {
            self.notify("Wait for the reply to finish, or press Esc to stop it.");
            return;
        }
        let q = self
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .cloned();
        match q {
            Some(q) => {
                self.branch_from = q.parent.clone();
                self.input = crate::attach::split_message(&q.content).0;
                self.send();
            }
            None => self.notify("There is nothing to retry yet."),
        }
    }

    /// Team mode with one of the two teams: `craft` (five specialists) or `apps` (one agent per Craft app).
    pub(crate) fn set_team_preset(&mut self, preset: &str) {
        self.team_preset = if preset == "apps" { "apps" } else { "craft" }.to_string();
        self.set_mode(true);
        self.save_prefs();
        self.notify(if self.team_preset == "apps" {
            "Team mode: one agent per Craft app."
        } else {
            "Team mode: the Craft team."
        });
    }

    pub(crate) fn send(&mut self) {
        if self.input.trim_start().starts_with('/') && self.run_typed_command(&self.input.clone()) {
            self.input.clear();
            return;
        }
        let mut text = self.input.trim().to_string();
        let ready: Vec<&crate::attach::Attached> = self
            .chips
            .iter()
            .filter(|c| !c.pending && c.kind != crate::attach::Kind::Failed)
            .collect();
        if (text.is_empty() && ready.is_empty()) || self.busy {
            return;
        }
        if self.chips.iter().any(|c| c.pending) {
            self.banner = Some("Wait until the attached files are read.".into());
            return;
        }
        let mut atts = Vec::new();
        for c in self.chips.drain(..) {
            if let Some(a) = c.image.clone() {
                atts.push(a);
            }
            if let Some(block) = c.message_block() {
                text.push_str("\n\n");
                text.push_str(&block);
            }
        }
        // Expand @mentions of Craft apps, folders and skills into explicit tool/skill guidance.
        let mut hints = Vec::new();
        let lower = text.to_ascii_lowercase();
        for spec in workcraft_agents::craft::specs() {
            let id = spec.id;
            let name = spec.name;
            if lower.contains(&format!("@{id}")) {
                hints.push(format!(
                    "The user mentioned @{id} ({name}): read the '{id}' skill with read_skill and use {id}__* MCP tools."
                ));
                if self
                    .craft
                    .iter()
                    .any(|c| c["id"] == id && c["installed"] == true && c["mode"] == "off")
                {
                    self.bridge.run(
                        "craft-mode",
                        "craft.mode",
                        json!({ "app": id, "mode": "headless" }),
                    );
                }
            }
        }
        for (tag, sub) in [
            ("@workspace", paths::data_dir().join("workspace")),
            ("@documents", paths::home_dir().join("Documents")),
            ("@desktop", paths::home_dir().join("Desktop")),
            ("@downloads", paths::home_dir().join("Downloads")),
            ("@pictures", paths::home_dir().join("Pictures")),
            ("@videos", paths::home_dir().join("Videos")),
            ("@music", paths::home_dir().join("Music")),
            (
                "@artifacts",
                paths::data_dir()
                    .join("workspace")
                    .join(".workcraft")
                    .join("artifacts"),
            ),
            ("@home", paths::home_dir()),
        ] {
            if lower
                .split_whitespace()
                .any(|w| w.trim_end_matches([',', '.', ':', ';', '?', '!']) == tag)
            {
                hints.push(format!("{} refers to folder {}", tag, sub.display()));
            }
        }
        // Agents: in Team mode the leader sends the task to them, in single mode they describe the role to follow.
        for a in &self.roster {
            if a.id != "leader" && lower.contains(&format!("@{}", a.id.to_ascii_lowercase())) {
                hints.push(format!(
                    "The user mentioned agent @{} ({}): {}. In Team mode assign the matching tasks to this agent.",
                    a.id, a.name, a.scope
                ));
            }
        }
        // Skills: say exactly how to read them.
        for (name, _) in &self.skills {
            if lower.contains(&format!("@skill:{}", name.to_ascii_lowercase())) {
                hints.push(format!(
                    "The user mentioned the skill @skill:{name}: read it first with read_skill and the argument {{\"name\": \"{name}\"}}."
                ));
            }
        }
        if lower
            .split_whitespace()
            .any(|w| w.trim_end_matches([',', '.', ':', ';', '?', '!']) == "@all")
        {
            hints.push("The user mentioned @all: any installed Craft app may be used for this task; use find_tools or craft_status to pick them.".to_string());
        }
        if lower
            .split_whitespace()
            .any(|w| w.trim_end_matches([',', '.', ':', ';', '?', '!']) == "@web")
        {
            hints.push("The user mentioned @web: fetch web pages with fetch_url when the answer needs them.".to_string());
        }
        // Team presets inside the message switch the team for this chat.
        if lower.contains("@team:apps") {
            self.set_team_preset("apps");
        } else if lower.contains("@team:craft") {
            self.set_team_preset("craft");
        }
        if !hints.is_empty() {
            text.push_str(&format!(
                "\n\n<attachment name=\"mentions\" type=\"NOTE\" size=\"0 B\" note=\"{}\"/>",
                hints.join(" ")
            ));
        }
        let mut p = json!({ "text": text, "attachments": atts });
        match &self.current {
            Some(s) => p["session"] = json!(s),
            None if self.mode_team => p["mode"] = json!("team"),
            None => {}
        }
        if self.mode_team && self.team_preset != "craft" {
            p["team"] = json!(self.team_preset);
        }
        if let Some(l) = self.branch_from.take() {
            p["leaf"] = json!(l);
        }
        if !self.model.trim().is_empty() {
            p["model"] = json!(self.model.trim());
        }
        self.input.clear();
        self.busy = true;
        match &self.current {
            Some(s) => {
                self.running.insert(s.clone());
            }
            None => self.awaiting_first = true,
        }
        self.turn_started = Some(self.now);
        self.banner = None;
        self.stream.clear();
        self.tools.clear();
        self.live.clear();
        self.bridge.run("send", "chat.send", p);
    }

    pub(crate) fn respond(&mut self, id: &str, allow: bool) {
        if let Some(a) = self.approvals.iter().find(|a| a.id == id) {
            let key = self.live_key(a.agent.as_deref());
            let tool = a.tool.clone();
            self.set_live(
                key,
                if allow {
                    AgentStatus::Tool
                } else {
                    AgentStatus::Thinking
                },
                Some(tool),
            );
        }
        self.approvals.retain(|a| a.id != id);
        self.bridge.run(
            "approval",
            "approval.respond",
            json!({ "id": id, "allow": allow }),
        );
    }

    pub(crate) fn delete_session(&mut self, id: &str) {
        if self.current.as_deref() == Some(id) {
            self.new_chat();
        }
        self.bridge
            .run("deleted", "chat.delete", json!({ "session": id }));
        self.sessions.retain(|s| s.id != id);
        self.bridge.run("sessions", "chat.list", Value::Null);
    }

    fn pick_dropped(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            let files = ctx.input(|i| i.raw.dropped_files.clone());
            for f in files {
                let name = f
                    .path()
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let path = f.path().to_path_buf();
                if path.as_os_str().is_empty() {
                    // No path (rare on desktop): use the bytes.
                    if let Ok(bytes) = f.bytes() {
                        self.attach_bytes(ctx, name, bytes.to_vec());
                    }
                } else {
                    self.attach_path(ctx, path);
                }
            }
        }
        // Files chosen in the file dialog.
        while let Ok(paths) = self.files_rx.try_recv() {
            for path in paths {
                self.attach_path(ctx, path);
            }
        }
        // Files that finished loading replace their placeholder.
        while let Ok(a) = self.attach_rx.try_recv() {
            if let Some(slot) = self.chips.iter_mut().find(|c| c.id == a.id) {
                *slot = a;
            }
        }
        while let Ok(r) = self.saved_rx.try_recv() {
            match r {
                Ok(p) => self.notify(format!("Saved to {p}")),
                Err(e) => self.banner = Some(e),
            }
        }
        if let Some(t) = self.to_copy.take() {
            ctx.copy_text(t);
        }
    }

    /// Show a placeholder at once and read the file on a worker thread.
    pub(crate) fn attach_path(&mut self, ctx: &egui::Context, path: std::path::PathBuf) {
        if self
            .chips
            .iter()
            .any(|c| c.path.as_deref() == Some(path.as_path()))
        {
            return;
        }
        let id = self.next_attach;
        self.next_attach += 1;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        self.chips
            .push(crate::attach::pending(id, &name, Some(path.clone())));
        let (tx, ctx) = (self.attach_tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let a = crate::attach::from_path(&ctx, id, &path);
            let _ = tx.send(a);
            ctx.request_repaint();
        });
    }

    fn attach_bytes(&mut self, ctx: &egui::Context, name: String, bytes: Vec<u8>) {
        let id = self.next_attach;
        self.next_attach += 1;
        self.chips.push(crate::attach::pending(id, &name, None));
        let (tx, ctx) = (self.attach_tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let a = crate::attach::from_bytes(&ctx, id, &name, &bytes);
            let _ = tx.send(a);
            ctx.request_repaint();
        });
    }

    /// Show a short notice under the composer for a few seconds.
    pub(crate) fn notify(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), self.now));
    }

    /// Export the open chat: save it with the system dialog, or copy it (`copy`).
    pub(crate) fn export(&mut self, format: &str, copy: bool) {
        let Some(s) = self.current.clone() else {
            self.notify("Open a chat to export it.");
            return;
        };
        let tag = if copy { "export-copy" } else { "export" };
        self.bridge.run(
            tag,
            "chat.export",
            json!({ "session": s, "format": format }),
        );
    }

    pub(crate) fn export_all(&mut self) {
        self.bridge.run("export", "chat.export_all", Value::Null);
    }

    fn save_with_dialog(&self, file_name: String, text: String) {
        let ext = std::path::Path::new(&file_name)
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default();
        let saved = self.saved_tx.clone();
        std::thread::spawn(move || {
            let mut d = rfd::FileDialog::new()
                .set_title("Export chat")
                .set_file_name(&file_name);
            if !ext.is_empty() {
                d = d.add_filter(ext.to_uppercase(), &[ext.as_str()]);
            }
            if let Some(p) = d.save_file() {
                let r = std::fs::write(&p, text.as_bytes())
                    .map(|()| p.display().to_string())
                    .map_err(|e| e.to_string());
                let _ = saved.send(r);
            }
        });
    }

    /// Undo every file change of one answer.
    pub(crate) fn undo_turn(&mut self, turn: &str) {
        if let Some(s) = self.current.clone() {
            self.bridge.run(
                "undo",
                "checkpoint.restore",
                json!({ "session": s, "turn": turn }),
            );
        }
    }

    /// Undo the newest answer that changed files and is not undone yet.
    pub(crate) fn undo_last(&mut self) {
        let mut turns: Vec<(&String, &Value)> = self
            .checkpoints
            .iter()
            .filter(|(_, v)| v["restored"] != true)
            .collect();
        turns.sort_by_key(|(_, v)| std::cmp::Reverse(v["ts"].as_i64().unwrap_or(0)));
        match turns.first().map(|(t, _)| (*t).clone()) {
            Some(t) => self.undo_turn(&t),
            None => self.notify("Nothing to undo in this chat."),
        }
    }

    /// Open the system file dialog without blocking the window; the files arrive in `files_rx`.
    pub(crate) fn pick_files(&self, ctx: &egui::Context) {
        let (tx, ctx) = (self.files_tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let picked = rfd::FileDialog::new()
                .set_title("Attach files")
                .add_filter("All files", &["*"])
                .add_filter(
                    "Documents",
                    &[
                        "pdf", "docx", "doc", "odt", "rtf", "txt", "md", "html", "pptx", "odp",
                        "xlsx", "xls", "ods", "csv", "tsv", "json",
                    ],
                )
                .add_filter("Images", &["png", "jpg", "jpeg", "webp", "gif", "bmp"])
                .add_filter(
                    "Code",
                    &[
                        "rs", "py", "js", "ts", "tsx", "jsx", "css", "toml", "yaml", "yml", "xml",
                        "sql", "sh", "ps1", "go", "java", "c", "cpp", "h", "log",
                    ],
                )
                .pick_files();
            if let Some(p) = picked {
                let _ = tx.send(p);
                ctx.request_repaint();
            }
        });
    }

    /// Open the system file dialog filtered to images/photos.
    pub(crate) fn pick_photos(&self, ctx: &egui::Context) {
        let (tx, ctx) = (self.files_tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let picked = rfd::FileDialog::new()
                .set_title("Attach photos & images")
                .add_filter("Images", &["png", "jpg", "jpeg", "webp", "gif", "bmp"])
                .add_filter("All files", &["*"])
                .pick_files();
            if let Some(p) = picked {
                let _ = tx.send(p);
                ctx.request_repaint();
            }
        });
    }

    /// Open the system folder dialog to attach a folder path.
    pub(crate) fn pick_folder(&self, ctx: &egui::Context) {
        let (tx, ctx) = (self.files_tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            if let Some(p) = rfd::FileDialog::new()
                .set_title("Attach folder")
                .pick_folder()
            {
                let _ = tx.send(vec![p]);
                ctx.request_repaint();
            }
        });
    }

    pub(crate) fn apply(&mut self, actions: Vec<Action>, ctx: &egui::Context) {
        for a in actions {
            match a {
                Action::Choose { parent_key, id } => {
                    self.choice.insert(parent_key, id);
                }
                Action::Branch { from, text } => {
                    self.branch_from = from;
                    self.input = text;
                }
                Action::Copy { key, text } => {
                    ctx.copy_text(text);
                    self.copied = Some((key, self.now));
                }
                Action::Prompt(t) => self.input = t,
                Action::Regenerate { from, text } => {
                    if !self.busy {
                        self.branch_from = from;
                        self.input = text;
                        self.send();
                    }
                }
                Action::Approve { id, allow } => self.respond(&id, allow),
                Action::AutoApprove => {
                    self.set_auto_approve(true);
                    let ids: Vec<String> = self.approvals.iter().map(|a| a.id.clone()).collect();
                    for id in ids {
                        self.respond(&id, true);
                    }
                }
                Action::Undo(turn) => self.undo_turn(&turn),
                Action::OpenPath(path) => open_in_os(&path),
            }
        }
    }

    pub(crate) fn thumb_for_path(
        &self,
        ctx: &egui::Context,
        path: &str,
    ) -> Option<(egui::TextureHandle, f32)> {
        let key = format!("path:{path}");
        if let Some(cached) = self.tool_thumbs.borrow().get(&key) {
            return cached.clone();
        }
        let bytes = std::fs::read(path).ok();
        let loaded = bytes.as_deref().and_then(|b| decode_thumb(ctx, &key, b));
        let mut map = self.tool_thumbs.borrow_mut();
        if map.len() >= 64 {
            map.clear();
        }
        map.insert(key, loaded.clone());
        loaded
    }

    pub(crate) fn thumb_for_b64(
        &self,
        ctx: &egui::Context,
        key_id: &str,
        b64: &str,
    ) -> Option<(egui::TextureHandle, f32)> {
        let key = format!("b64:{key_id}");
        if let Some(cached) = self.tool_thumbs.borrow().get(&key) {
            return cached.clone();
        }
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok();
        let loaded = bytes.as_deref().and_then(|b| decode_thumb(ctx, &key, b));
        let mut map = self.tool_thumbs.borrow_mut();
        if map.len() >= 64 {
            map.clear();
        }
        map.insert(key, loaded.clone());
        loaded
    }

    /// True for a second and a half after the copy button `key` was clicked.
    pub(crate) fn just_copied(&self, key: &str) -> bool {
        self.copied
            .as_ref()
            .is_some_and(|(k, t)| k == key && self.now - t < 1.6)
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let new_chat = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::O);
        let sidebar = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::S);
        let settings = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);
        let voice_toggle = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::M);
        if ctx.input_mut(|i| i.consume_shortcut(&new_chat)) {
            self.new_chat();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sidebar)) {
            self.show_sidebar = !self.show_sidebar;
            self.save_prefs();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&settings)) {
            self.open_settings(SettingsTab::General);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&voice_toggle)) {
            self.toggle_voice(ctx);
        }
        if matches!(
            self.voice.state,
            crate::voice::VoiceState::Listening | crate::voice::VoiceState::Processing
        ) && !self.show_settings
            && ctx.input(|i| i.key_pressed(Key::Escape))
        {
            self.voice.cancel();
            self.input = self.voice.input_prefix.trim_end().to_string();
            self.blip.set_state(crate::blip::BlipState::Idle);
            self.blip.say("Voice cancelled", 1.8, self.now);
            return;
        }
        if self.busy && !self.show_settings && ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.cancel();
        }
    }

    pub(crate) fn toggle_voice(&mut self, ctx: &egui::Context) {
        let mut extra_words: Vec<String> = Vec::new();
        for a in &self.roster {
            extra_words.push(a.id.clone());
            extra_words.push(a.name.clone());
        }
        let current = self.input.clone();
        self.voice.toggle(&current, extra_words);
        match &self.voice.state {
            crate::voice::VoiceState::Listening => {
                self.blip.set_state(crate::blip::BlipState::Listening);
                self.blip.say("Listening... speak now!", 2.6, self.now);
                ctx.request_repaint();
            }
            crate::voice::VoiceState::Processing => {
                self.blip.set_state(crate::blip::BlipState::Thinking);
                self.blip.say("Transcribing speech...", 3.0, self.now);
                ctx.request_repaint();
            }
            crate::voice::VoiceState::Error(err) => {
                self.blip
                    .trigger(crate::blip::BlipState::Error, 2.8, self.now);
                self.blip.say(&err.message, 3.5, self.now);
                ctx.request_repaint();
            }
            crate::voice::VoiceState::Idle => {}
        }
    }

    fn poll_voice(&mut self, ctx: &egui::Context) {
        if matches!(
            self.voice.state,
            crate::voice::VoiceState::Listening | crate::voice::VoiceState::Processing
        ) {
            ctx.request_repaint_after(std::time::Duration::from_millis(28));
        }
        let events = self.voice.poll();
        let mut should_send = false;
        for ev in events {
            match ev {
                crate::voice::VoiceEvent::Partial(partial) => {
                    self.input = format!("{}{}", self.voice.input_prefix, partial);
                    ctx.request_repaint();
                }
                crate::voice::VoiceEvent::ProcessingStarted => {
                    self.blip.set_state(crate::blip::BlipState::Thinking);
                    self.blip.say("Transcribing speech...", 2.8, self.now);
                    ctx.request_repaint();
                }
                crate::voice::VoiceEvent::Final {
                    text,
                    engine,
                    duration_ms,
                    auto_send,
                    ..
                } => {
                    self.input = format!("{}{}", self.voice.input_prefix, text);
                    ctx.memory_mut(|m| m.request_focus(egui::Id::unique("composer-input")));
                    self.blip.set_state(crate::blip::BlipState::Idle);
                    self.blip
                        .trigger(crate::blip::BlipState::Success, 1.8, self.now);
                    self.blip.say(
                        format!("Heard you! ({engine}, {duration_ms}ms)"),
                        2.8,
                        self.now,
                    );
                    if auto_send {
                        should_send = true;
                    }
                    ctx.request_repaint();
                }
                crate::voice::VoiceEvent::Error(err) => {
                    self.input = self.voice.input_prefix.trim_end().to_string();
                    self.blip.set_state(crate::blip::BlipState::Idle);
                    self.blip
                        .trigger(crate::blip::BlipState::Error, 2.8, self.now);
                    // Say what was wrong (too short, too quiet, nothing recognised), not just that something was
                    self.blip.say_kind(err.message.clone(), 4.5, self.now, true);
                    ctx.request_repaint();
                }
            }
        }
        if should_send && !self.busy {
            self.send();
        }
    }

    pub(crate) fn open_settings(&mut self, tab: SettingsTab) {
        let c = &self.config;
        self.draft = Draft {
            system_prompt: c["system_prompt"].as_str().unwrap_or_default().to_string(),
            num_ctx: c["num_ctx"]
                .as_u64()
                .map(|n| n.to_string())
                .unwrap_or_default(),
            max_steps: c["max_steps"].as_u64().unwrap_or(24) as u32,
            think: c["think"].as_bool(),
            default_model: c["model"].as_str().unwrap_or_default().to_string(),
        };
        self.settings_tab = tab;
        self.anim_until = self.now + 0.8;
        self.settings_scroll = Some(0.0);
        self.settings_error = None;
        self.provider_edit = None;
        self.agent_edit = None;
        self.team_edit = None;
        self.skill_edit = None;
        self.show_settings = true;
        self.bridge
            .run("provider-info", "provider.info", Value::Null);
    }

    pub(crate) fn save_agent(&mut self) {
        let Some(e) = self.agent_edit.clone() else {
            return;
        };
        let id = if e.is_new && e.id.trim().is_empty() {
            crate::providers::custom_id(&e.name)
        } else {
            e.id.trim().to_string()
        };
        if id.is_empty() || e.name.trim().is_empty() {
            self.settings_error = Some("Enter a name and ID for the agent.".into());
            return;
        }
        let split_csv = |s: &str| -> Vec<String> {
            s.split([',', '\n'])
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(str::to_string)
                .collect()
        };
        let model = {
            let m = e.model.trim();
            if m.is_empty() {
                Value::Null
            } else {
                let r = workcraft_core::ModelRef::parse(m);
                json!({ "provider": r.provider, "model": r.model })
            }
        };
        let def = json!({
            "id": id,
            "name": e.name.trim(),
            "scope": e.scope.trim(),
            "prompt": e.prompt.trim(),
            "model": model,
            "skills": split_csv(&e.skills),
            "tools": split_csv(&e.tools),
            "approval": if e.approval.is_empty() { Value::Null } else { json!(e.approval) },
        });
        self.settings_error = None;
        self.bridge
            .run("agent-saved", "agent.define", json!({ "agent": def }));
    }

    pub(crate) fn remove_agent(&mut self, id: &str) {
        self.settings_error = None;
        self.bridge
            .run("agent-removed", "agent.remove", json!({ "id": id }));
    }

    pub(crate) fn save_team(&mut self) {
        let Some(e) = self.team_edit.clone() else {
            return;
        };
        let mut team = if self.team_raw.is_object() {
            self.team_raw.clone()
        } else {
            serde_json::to_value(workcraft_agents::config::craft_team()).unwrap_or_default()
        };
        team["name"] = json!(if e.name.trim().is_empty() {
            "craft"
        } else {
            e.name.trim()
        });
        team["leader"]["name"] = json!(e.leader_name.trim());
        team["leader"]["scope"] = json!(e.leader_scope.trim());
        team["leader"]["prompt"] = json!(e.leader_prompt.trim());
        team["concurrency"] = json!(e.concurrency.clamp(1, 8));
        team["review"] = json!(e.review);
        team["plan_approval"] = json!(e.plan_approval);
        self.settings_error = None;
        self.bridge
            .run("team-saved", "team.define", json!({ "team": team }));
    }

    pub(crate) fn reset_team(&mut self) {
        self.settings_error = None;
        self.bridge
            .run("team-removed", "team.remove", json!({ "name": "craft" }));
    }

    pub(crate) fn open_skill_edit(&mut self, name: &str, description: &str) {
        self.settings_error = None;
        self.skill_edit = Some(SkillEdit {
            name: name.to_string(),
            description: description.to_string(),
            body: String::new(),
            is_new: false,
        });
        self.bridge
            .run("skill-read", "skill.read", json!({ "name": name }));
    }

    pub(crate) fn save_skill(&mut self) {
        let Some(e) = self.skill_edit.clone() else {
            return;
        };
        let name = e.name.trim();
        if name.is_empty() {
            self.settings_error = Some("Enter a skill name (letters, digits, '-' or '_').".into());
            return;
        }
        if e.body.trim().is_empty() {
            self.settings_error = Some("Enter Markdown instructions for the skill.".into());
            return;
        }
        self.settings_error = None;
        self.bridge.run(
            "skill-saved",
            "skill.save",
            json!({
                "name": name,
                "description": e.description.trim(),
                "body": e.body.trim(),
            }),
        );
    }

    pub(crate) fn remove_skill(&mut self, name: &str) {
        self.settings_error = None;
        self.bridge
            .run("skill-removed", "skill.remove", json!({ "name": name }));
    }

    /// Flip between light and dark (from the sidebar switch). "System" stays a choice in the settings.
    pub(crate) fn toggle_theme(&mut self, ctx: &egui::Context) {
        let dark = ctx.theme() == egui::Theme::Dark;
        self.set_theme(
            ctx,
            if dark {
                ThemePref::Light
            } else {
                ThemePref::Dark
            },
        );
    }

    pub(crate) fn save_provider(&mut self) {
        let Some(e) = self.provider_edit.clone() else {
            return;
        };
        let id = if e.custom {
            crate::providers::custom_id(&e.name)
        } else {
            e.id.clone()
        };
        if e.custom && e.base_url.trim().is_empty() {
            self.settings_error = Some("Enter the address of the endpoint.".into());
            return;
        }
        self.settings_error = None;
        let mut p = json!({
            "id": id, "kind": e.kind, "name": e.name.trim(),
            "base_url": e.base_url.trim(), "key_env": e.key_env.trim(),
        });
        if !e.key.trim().is_empty() {
            p["api_key"] = json!(e.key.trim());
        }
        self.bridge.run("provider-saved", "provider.save", p);
        if let Some(ed) = self.provider_edit.as_mut() {
            // Do not keep the typed key in memory longer than needed.
            ed.key.clear();
        }
    }

    pub(crate) fn forget_key(&mut self, id: &str, kind: &str) {
        self.bridge.run(
            "provider-saved",
            "provider.save",
            json!({ "id": id, "kind": kind, "api_key": null,
                    "name": self.provider_edit.as_ref().map(|e| e.name.clone()),
                    "base_url": self.provider_edit.as_ref().map(|e| e.base_url.clone()),
                    "key_env": self.provider_edit.as_ref().map(|e| e.key_env.clone()) }),
        );
    }
}

fn agent_info(v: &Value) -> AgentInfo {
    let strs = |k: &str| -> Vec<String> {
        v[k].as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    };
    AgentInfo {
        id: v["id"].as_str().unwrap_or_default().to_string(),
        name: v["name"].as_str().unwrap_or_default().to_string(),
        scope: v["scope"].as_str().unwrap_or_default().to_string(),
        prompt: v["prompt"].as_str().unwrap_or_default().to_string(),
        skills: strs("skills"),
        tools: strs("tools"),
        model: v["model"].as_object().map(|m| {
            format!(
                "{}:{}",
                m.get("provider")
                    .and_then(Value::as_str)
                    .unwrap_or("ollama"),
                m.get("model").and_then(Value::as_str).unwrap_or("")
            )
        }),
        approval: v["approval"].as_str().map(str::to_string),
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.now = ctx.input(|i| i.time);
        self.drain();
        if self.busy
            && let Some(t0) = self.turn_started
        {
            self.blip.on_slow(self.now - t0, self.now);
            // Keep drawing so the nudge appears even when nothing else happens.
            ctx.request_repaint_after(std::time::Duration::from_secs(5));
        }
        self.poll_voice(ctx);
        // egui only draws when something happens. After a click or an open, keep drawing for a moment
        // so fades and slides (the settings window, switches) finish instead of freezing half-way.
        if ctx
            .input(|i| i.pointer.any_pressed() || i.pointer.any_released() || !i.events.is_empty())
        {
            self.anim_until = self.anim_until.max(self.now + 0.6);
        }
        if self.ui_open.is_some() || self.now < self.anim_until {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        // Test hook: WORKCRAFT_UI_OPEN opens one screen once the first data is in, so the window can
        // be checked from a capture without clicking. Values: settings:<tab>[@scroll], provider:<id>,
        // custom, picker, blip:<state>.
        if self.now > 1.5
            && let Some(what) = self.ui_open.take()
        {
            let (what, scroll) = match what.split_once('@') {
                Some((w, s)) => (w.to_string(), s.parse::<f32>().ok()),
                None => (what, None),
            };
            let tab = |t: &str| match t {
                "models" => SettingsTab::Models,
                "providers" => SettingsTab::Providers,
                "agents" => SettingsTab::Agents,
                "tools" => SettingsTab::Tools,
                "skills" => SettingsTab::Skills,
                _ => SettingsTab::General,
            };
            match what.split_once(':') {
                Some(("settings", t)) => self.open_settings(tab(t)),
                Some(("chat", id)) => self.open(id),
                // `blip:<state>@<scroll>`: Settings with Blip held in one state (for window captures).
                Some(("blip", st)) => {
                    self.open_settings(SettingsTab::General);
                    if let Some(s) = crate::blip::BlipState::ALL
                        .iter()
                        .copied()
                        .find(|s| s.label().eq_ignore_ascii_case(st))
                    {
                        self.blip.trigger(s, 600.0, self.now);
                    }
                }
                // The same path the file dialog takes.
                Some(("attach", path)) => {
                    for f in path.split('|') {
                        let pb = std::path::PathBuf::from(f);
                        let _ = self.files_tx.send(vec![pb]);
                    }
                }
                Some(("provider", id)) => {
                    self.open_settings(SettingsTab::Providers);
                    let pre = crate::providers::preset(id);
                    self.provider_edit = Some(ProviderEdit {
                        id: id.to_string(),
                        kind: pre.map_or("openai", |x| x.kind).to_string(),
                        name: pre.map_or("", |x| x.name).to_string(),
                        base_url: pre.map_or("", |x| x.base_url).to_string(),
                        key_env: pre.map_or("", |x| x.key_env).to_string(),
                        ..ProviderEdit::default()
                    });
                }
                _ if what == "custom" => {
                    self.open_settings(SettingsTab::Providers);
                    self.provider_edit = Some(ProviderEdit {
                        id: "custom".into(),
                        kind: "openai".into(),
                        custom: true,
                        ..ProviderEdit::default()
                    });
                }
                _ if what == "picker" => self.open_picker = true,
                _ => {}
            }
            self.settings_scroll = scroll;
        }
        self.pick_dropped(ctx);
        self.shortcuts(ctx);
        if self.busy {
            // The elapsed timer and the live dots move even when no event arrives.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        if self.copied.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, t)| self.now - *t < 4.0)
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
        } else {
            self.notice = None;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Messages that arrive while drawing (theme change, a finished command) start animations too.
        if ui.ctx().has_requested_repaint() {
            self.anim_until = self.anim_until.max(self.now + 0.3);
        }
        let p = theme::pal(ui);
        if self.show_sidebar {
            egui::Panel::left("sessions")
                .default_size(264.0)
                .size_range(220.0..=360.0)
                .frame(
                    egui::Frame::new()
                        .fill(p.sidebar)
                        .inner_margin(egui::Margin::symmetric(10, 10)),
                )
                .show_separator_line(false)
                .show(ui, |ui| self.sidebar(ui));
        }
        self.refresh_artifacts();
        if self.show_activity {
            egui::Panel::right("activity")
                .default_size(320.0)
                .size_range(260.0..=460.0)
                .frame(
                    egui::Frame::new()
                        .fill(p.sidebar)
                        .inner_margin(egui::Margin::symmetric(14, 12)),
                )
                .show_separator_line(false)
                .show(ui, |ui| self.activity(ui));
        }
        egui::Panel::top("chat-top")
            .frame(
                egui::Frame::new()
                    .fill(p.bg)
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show_separator_line(false)
            .show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("composer")
            .frame(
                egui::Frame::new()
                    .fill(p.bg)
                    .inner_margin(egui::Margin::symmetric(16, 6)),
            )
            .show_separator_line(false)
            .resizable(false)
            .show(ui, |ui| self.composer_area(ui));
        egui::CentralPanel::no_frame()
            .frame(egui::Frame::new().fill(p.bg))
            .show(ui, |ui| self.chat(ui));
        let ctx = ui.ctx().clone();
        if !self.show_settings {
            self.floating_chat_overlays(&ctx);
            self.blip_dock(&ctx);
        }
        self.lightbox_view(&ctx);
        if self.show_settings {
            self.settings(&ctx);
        }
        self.drop_overlay(&ctx);
    }
}

fn decode_thumb(
    ctx: &egui::Context,
    key: &str,
    bytes: &[u8],
) -> Option<(egui::TextureHandle, f32)> {
    let dyn_img = image::load_from_memory(bytes).ok()?;
    let (w, h) = (dyn_img.width().max(1), dyn_img.height().max(1));
    let aspect = (w as f32 / h as f32).clamp(0.2, 5.0);
    let scaled = if w > 512 || h > 512 {
        dyn_img.thumbnail(512, 512)
    } else {
        dyn_img
    };
    let rgba = scaled.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    let tex = ctx.load_texture(key, color, egui::TextureOptions::LINEAR);
    Some((tex, aspect))
}

/// Shows a folder in the file manager (Explorer, Finder, the default file manager). The folder is created first, so the
/// button never does nothing on a fresh install.
pub(crate) fn open_folder(path: &std::path::Path) {
    let _ = std::fs::create_dir_all(path);
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

fn open_in_os(path: &str) {
    let p = std::path::Path::new(path);
    if !p.exists() {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", path])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}
