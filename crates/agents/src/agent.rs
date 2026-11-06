//! The loop of one agent: ask the model, run the tools it asks for, feed the results back, repeat.

use std::collections::{BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use workcraft_core::team::ApprovalMode;
use workcraft_core::{
    AgentDef, Attachment, ChatOptions, Delta, Error, Event, Message, ModelRef, Result, Role,
    StopReason, ToolCall, ToolSpec, Usage, truncate_bytes,
};
use workcraft_llm::ChatRequest;

use crate::context;
use crate::craft::{
    ApprovalLevel, apply_tool_budget, classify_tool_call, extract_mentions, find_spec,
};
use crate::shared::{Cancel, Shared};
use crate::tools::{CallOutcome, Memory, ToolCtx};

/// After this many identical failing calls in a row the loop stops instead of burning tokens.
const LOOP_GUARD: u32 = 3;
/// The same call, with the same arguments, this many times in a row also stops the loop even when every
/// call succeeds: a small model that cannot finish otherwise repeats one export fifty times.
const SAME_CALL_LIMIT: u32 = 6;

pub struct AgentRun {
    pub session: String,
    /// `None` is the default single-mode assistant: every tool, the session model.
    pub agent: Option<AgentDef>,
    pub model: ModelRef,
    pub system: String,
    /// The conversation so far, root to leaf, ending with the message to answer.
    pub history: Vec<Message>,
    /// Write every new message to the session file.
    pub persist: bool,
    pub memory: Arc<Memory>,
    pub cancel: Arc<Cancel>,
    pub approval: ApprovalMode,
    pub max_steps: usize,
    pub options: ChatOptions,
}

#[derive(Debug, Default)]
pub struct AgentOutcome {
    /// The last assistant text.
    pub text: String,
    /// Id of the last message written.
    pub last_id: Option<String>,
    pub usage: Usage,
    /// Set when the loop ended for a reason other than a finished answer.
    pub stopped: Option<String>,
    /// Messages added by this run, in order.
    pub added: Vec<Message>,
    /// Tool calls that succeeded and tool calls that failed during this run (a worker that did real work
    /// and met no error does not need a second opinion).
    pub tools_ok: u32,
    pub tools_failed: u32,
}

/// Failures a second model may fix: the service is unreachable, overloaded, rate limited, out of credit or silent.
/// A wrong key, a bad request or a too-long conversation would fail the same way again, so they are not.
fn fallback_worthy(e: &Error) -> bool {
    match e {
        Error::Http(_) | Error::Timeout(_) => true,
        Error::Provider(m) => ["HTTP 408", "HTTP 429", "HTTP 402", "HTTP 5", "overloaded"]
            .iter()
            .any(|k| m.contains(k)),
        _ => false,
    }
}

fn call_key(c: &ToolCall) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    c.name.hash(&mut h);
    c.arguments.to_string().hash(&mut h);
    h.finish()
}

/// What the request needs to know about the provider to stay inside the model's context window.
struct Window {
    tokens: usize,
    /// DeepSeek wants earlier reasoning sent back during tool chains.
    keep_reasoning: bool,
    ollama: bool,
}

fn window_of(model: &ModelRef, cfg: &crate::config::Config) -> Window {
    let pc = cfg.providers.iter().find(|p| p.id == model.provider);
    let kind = if model.provider == "ollama" {
        "ollama"
    } else {
        pc.map_or("openai", |p| p.kind.as_str())
    };
    let base = pc.and_then(|p| p.base_url.as_deref()).unwrap_or("");
    let configured = if kind == "ollama" {
        cfg.num_ctx
    } else {
        cfg.context_window
    };
    Window {
        tokens: context::window_for(kind, &model.model, configured),
        keep_reasoning: workcraft_llm::openai::Quirks::detect(&model.provider, base, &model.model)
            .replay_reasoning,
        ollama: kind == "ollama",
    }
}

/// Token estimate of the parts of a request that are not history.
fn fixed_tokens(system: &str, tools: &[ToolSpec]) -> usize {
    let tool_chars: usize = tools
        .iter()
        .map(|t| t.name.len() + t.description.len() + t.input_schema.to_string().len())
        .sum();
    context::estimate_tokens(system.len() + tool_chars)
}

pub async fn run_agent(sh: &Arc<Shared>, mut run: AgentRun) -> Result<AgentOutcome> {
    let agent_id = run.agent.as_ref().map(|a| a.id.clone());
    let mut provider = sh.provider(&run.model.provider)?;
    let cfg = sh.config();
    let mut vision = workcraft_llm::supports_vision(&run.model, &cfg.providers);
    let mut is_local = workcraft_llm::is_local_model(&run.model, &cfg.providers);

    let last_user_msg = run.history.iter().rev().find(|m| m.role == Role::User);
    let last_user_text = last_user_msg.map_or("", |m| m.content.as_str()).to_string();
    let mentioned_apps: BTreeSet<String> = last_user_msg
        .map(|m| {
            extract_mentions(&m.content)
                .into_iter()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let ctx = ToolCtx {
        session: run.session.clone(),
        agent: agent_id.clone(),
        memory: Arc::clone(&run.memory),
        // File changes are grouped under the user message being answered, for undo.
        turn: last_user_msg.map(|m| m.id.clone()).unwrap_or_default(),
    };

    let mut out = AgentOutcome::default();
    let mut failures: HashMap<u64, u32> = HashMap::new();
    let mut same_call: (u64, u32) = (0, 0);
    let mut win = window_of(&run.model, &cfg);
    // The model to switch to once, when the main one cannot be reached or is overloaded (config `fallback_model`).
    let fallback = cfg
        .fallback_model
        .as_deref()
        .map(ModelRef::parse)
        .filter(|m| *m != run.model);
    let mut switched = false;
    let mut last_shortened = context::Shortened::None;

    for _step in 0..run.max_steps {
        if run.cancel.is_cancelled() {
            out.stopped = Some("cancelled".into());
            return Ok(out);
        }

        let tools = sh.tools();
        // Apps the question is about must be connected before their tools can be listed.
        if run.agent.is_none() && cfg.tool_routing {
            let wanted = {
                let mut w = crate::craft::apps_named_in(&last_user_text);
                w.extend(crate::craft::apps_in_history(&run.history));
                w
            };
            tools.connect_on_demand(&wanted).await;
        }
        let all_specs = tools.specs().await;
        let scoped_specs: Vec<ToolSpec> = match &run.agent {
            Some(a) => all_specs
                .into_iter()
                .filter(|t| a.allows(&t.name))
                .collect(),
            // The single assistant gets the tools of the apps this chat is about; `find_tools` reaches the rest.
            None if cfg.tool_routing => {
                crate::craft::route(all_specs, &run.history, &last_user_text).tools
            }
            None => all_specs,
        };
        let (specs, budget_note) = apply_tool_budget(scoped_specs, is_local, None, &mentioned_apps);
        let mut step_system = run.system.clone();
        if let Some(note) = budget_note {
            step_system.push_str("\n\n");
            step_system.push_str(&note);
        }

        let (messages, shortened) = context::fit(
            &run.history,
            fixed_tokens(&step_system, &specs),
            win.tokens,
            run.options
                .max_tokens
                .map_or(context::DEFAULT_ANSWER_TOKENS, |n| n as usize),
            win.keep_reasoning,
        );
        if shortened > last_shortened {
            last_shortened = shortened;
            let message = match shortened {
                context::Shortened::Dropped(n) => format!(
                    "Long chat: {n} earlier turn(s) were left out of this request to fit the {}k-token context window. The saved chat is unchanged.",
                    win.tokens / 1000
                ),
                _ => {
                    "Long chat: older tool results were cut to fit the context window.".to_string()
                }
            };
            sh.emit(Event::Notice {
                session: run.session.clone(),
                agent: agent_id.clone(),
                message,
            });
        }
        let mut options = run.options.clone();
        if win.ollama {
            // Ollama starts models with 4,096 tokens and cuts the start of the prompt without a word.
            options.num_ctx = Some(win.tokens as u32);
        }
        let req = ChatRequest {
            model: run.model.model.clone(),
            system: Some(step_system),
            messages,
            tools: specs.clone(),
            options,
            vision,
        };

        // One model call, streamed.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Delta>(128);
        let p = Arc::clone(&provider);
        let call = tokio::spawn(async move { p.chat(req, tx).await });
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut calls: Vec<ToolCall> = Vec::new();
        let mut stop = StopReason::Other;
        loop {
            tokio::select! {
                d = rx.recv() => match d {
                    Some(Delta::Text(t)) => {
                        sh.emit(Event::Text { session: run.session.clone(), agent: agent_id.clone(), delta: t.clone() });
                        text.push_str(&t);
                    }
                    Some(Delta::Reasoning(t)) => {
                        sh.emit(Event::Reasoning { session: run.session.clone(), agent: agent_id.clone(), delta: t.clone() });
                        reasoning.push_str(&t);
                    }
                    Some(Delta::ToolCall(c)) => calls.push(c),
                    Some(Delta::Notice(message)) => {
                        sh.emit(Event::Notice { session: run.session.clone(), agent: agent_id.clone(), message });
                    }
                    Some(Delta::Usage(u)) => {
                        out.usage.add(u);
                        sh.record_usage(&run.session, &run.model, u);
                    }
                    Some(Delta::Done(s)) => stop = s,
                    None => break,
                },
                () = run.cancel.cancelled() => {
                    call.abort();
                    out.stopped = Some("cancelled".into());
                    break;
                }
            }
        }
        if out.stopped.is_some() {
            // Keep what was said so far so the transcript matches what the person saw.
            if !text.is_empty() {
                let partial = assistant_message(&run, &agent_id, &text, &reasoning, &[]);
                out.last_id = Some(add_message(sh, &mut run, &mut out, partial));
            }
            return Ok(out);
        }
        match call.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                if !switched
                    && text.is_empty()
                    && calls.is_empty()
                    && fallback_worthy(&e)
                    && let Some(fb) = fallback.clone()
                    && let Ok(p) = sh.provider(&fb.provider)
                {
                    switched = true;
                    sh.emit(Event::Notice {
                        session: run.session.clone(),
                        agent: agent_id.clone(),
                        message: format!(
                            "{} failed ({}). Continuing with {}.",
                            run.model.model,
                            e.to_string().chars().take(80).collect::<String>(),
                            fb.model
                        ),
                    });
                    provider = p;
                    vision = workcraft_llm::supports_vision(&fb, &cfg.providers);
                    is_local = workcraft_llm::is_local_model(&fb, &cfg.providers);
                    run.model = fb;
                    win = window_of(&run.model, &cfg);
                    continue;
                }
                return Err(e);
            }
            Err(e) => return Err(Error::Provider(format!("model task failed: {e}"))),
        }

        // Small reasoning models sometimes end inside their thinking and never answer. Retry once with
        // thinking off instead of handing the user an empty reply.
        if text.is_empty()
            && calls.is_empty()
            && !reasoning.is_empty()
            && run.options.think != Some(false)
        {
            run.options.think = Some(false);
            sh.emit(Event::Reasoning {
                session: run.session.clone(),
                agent: agent_id.clone(),
                delta: "\n[no answer after thinking, retrying with thinking off]\n".into(),
            });
            continue;
        }

        let msg = assistant_message(&run, &agent_id, &text, &reasoning, &calls);
        out.last_id = Some(add_message(sh, &mut run, &mut out, msg));
        if !text.is_empty() {
            out.text = text;
        }
        if calls.is_empty() {
            if stop == StopReason::Length {
                out.stopped = Some("the answer hit the length limit".into());
            } else if out.text.is_empty() {
                out.stopped = Some("the model returned no answer".into());
            }
            return Ok(out);
        }

        // Tools, one after the other: writes must not race.
        for c in calls {
            let outcome = run_one_call(sh, &run, &ctx, &specs, &c, &agent_id).await;
            if outcome.result.ok {
                out.tools_ok += 1;
            } else {
                out.tools_failed += 1;
            }
            let key = call_key(&c);
            same_call = if same_call.0 == key {
                (key, same_call.1 + 1)
            } else {
                (key, 1)
            };
            let repeated = same_call.1 >= SAME_CALL_LIMIT;
            let failed_repeatedly = if outcome.result.ok {
                failures.remove(&key);
                false
            } else {
                let n = failures.entry(key).or_insert(0);
                *n += 1;
                *n >= LOOP_GUARD
            };
            let tripped = failed_repeatedly || repeated;
            sh.emit(Event::ToolFinished {
                session: run.session.clone(),
                agent: agent_id.clone(),
                call_id: c.id.clone(),
                ok: outcome.result.ok,
                output: truncate_bytes(&outcome.result.text, 4000),
                artifacts: outcome.artifacts.clone(),
                mode: outcome.mode.clone(),
            });
            let mut text = outcome.result.text;
            if tripped {
                text.push_str(if repeated {
                    "\n[stopped: the same call ran six times in a row]"
                } else {
                    "\n[stopped: the same call failed three times in a row]"
                });
            }
            let mut tm = Message::tool(&c.id, text);
            tm.agent = agent_id.clone();
            for (idx, img) in outcome.result.images.iter().enumerate() {
                tm.attachments.push(Attachment {
                    name: outcome
                        .artifacts
                        .get(idx)
                        .cloned()
                        .unwrap_or_else(|| format!("{}-{idx}.png", c.id)),
                    mime: img.mime_type.clone(),
                    data: img.data.clone(),
                });
            }
            let id = add_message(sh, &mut run, &mut out, tm);
            out.last_id = Some(id);
            if tripped {
                out.stopped = Some(if repeated {
                    format!(
                        "{} was called six times in a row with the same arguments",
                        c.name
                    )
                } else {
                    format!("{} failed three times in a row", c.name)
                });
                return Ok(out);
            }
            if run.cancel.is_cancelled() {
                out.stopped = Some("cancelled".into());
                return Ok(out);
            }
        }
    }
    if !run.cancel.is_cancelled() && run.history.last().is_some_and(|m| m.role == Role::Tool) {
        let mut final_system = run.system.clone();
        final_system.push_str(
            "\n\nYou have reached the tool step limit for this turn. Do not call any more tools; briefly summarize what you have completed so far and what the user can ask next if anything remains.",
        );
        let req = ChatRequest {
            model: run.model.model.clone(),
            system: Some(final_system.clone()),
            messages: context::fit(
                &run.history,
                fixed_tokens(&final_system, &[]),
                win.tokens,
                context::DEFAULT_ANSWER_TOKENS,
                win.keep_reasoning,
            )
            .0,
            tools: Vec::new(),
            options: run.options.clone(),
            vision,
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Delta>(128);
        let p = Arc::clone(&provider);
        let call = tokio::spawn(async move { p.chat(req, tx).await });
        let mut text = String::new();
        let mut reasoning = String::new();
        while let Some(d) = rx.recv().await {
            match d {
                Delta::Text(t) => {
                    sh.emit(Event::Text {
                        session: run.session.clone(),
                        agent: agent_id.clone(),
                        delta: t.clone(),
                    });
                    text.push_str(&t);
                }
                Delta::Reasoning(t) => {
                    sh.emit(Event::Reasoning {
                        session: run.session.clone(),
                        agent: agent_id.clone(),
                        delta: t.clone(),
                    });
                    reasoning.push_str(&t);
                }
                Delta::Usage(u) => {
                    out.usage.add(u);
                    sh.record_usage(&run.session, &run.model, u);
                }
                Delta::Notice(message) => {
                    sh.emit(Event::Notice {
                        session: run.session.clone(),
                        agent: agent_id.clone(),
                        message,
                    });
                }
                Delta::ToolCall(_) | Delta::Done(_) => {}
            }
        }
        let _ = call.await;
        if !text.trim().is_empty() {
            let msg = assistant_message(&run, &agent_id, &text, &reasoning, &[]);
            out.last_id = Some(add_message(sh, &mut run, &mut out, msg));
            out.text = text;
            return Ok(out);
        }
    }
    out.stopped = Some(format!("reached the limit of {} steps", run.max_steps));
    Ok(out)
}

fn assistant_message(
    run: &AgentRun,
    agent: &Option<String>,
    text: &str,
    reasoning: &str,
    calls: &[ToolCall],
) -> Message {
    let mut m = Message::assistant(text);
    m.reasoning = (!reasoning.is_empty()).then(|| reasoning.to_string());
    m.tool_calls = calls.to_vec();
    m.model = Some(run.model.clone());
    m.agent = agent.clone();
    m
}

/// Chain the message onto the history, persist it, announce it. Returns its id.
fn add_message(
    sh: &Arc<Shared>,
    run: &mut AgentRun,
    out: &mut AgentOutcome,
    mut m: Message,
) -> String {
    m.parent = run.history.last().map(|l| l.id.clone());
    if run.persist
        && let Err(e) = sh.store.append(&run.session, &m)
    {
        sh.emit(Event::Error {
            session: run.session.clone(),
            message: format!("could not save message: {e}"),
        });
    }
    sh.emit(Event::MessageAdded {
        session: run.session.clone(),
        message: m.clone(),
    });
    let id = m.id.clone();
    run.history.push(m.clone());
    out.added.push(m);
    id
}

/// Identify which Craft app a call mutates so Team specialists acquire per-app writer leases (`T5.2`).
fn target_craft_app_for_lease(call: &ToolCall) -> Option<&'static str> {
    if let Some((srv, _)) = workcraft_mcp::hub::split_name(&call.name) {
        return find_spec(srv).map(|s| s.id);
    }
    if call.name.starts_with("craft_") || call.name == "open_craft_app" {
        let raw = call
            .arguments
            .get("app")
            .and_then(serde_json::Value::as_str)?;
        return find_spec(raw).map(|s| s.id);
    }
    None
}

async fn run_one_call(
    sh: &Arc<Shared>,
    run: &AgentRun,
    ctx: &ToolCtx,
    specs: &[ToolSpec],
    c: &ToolCall,
    agent_id: &Option<String>,
) -> CallOutcome {
    use workcraft_core::ToolResult;

    if let Some(a) = &run.agent
        && !a.allows(&c.name)
    {
        return CallOutcome {
            result: ToolResult::err(format!(
                "tool '{}' is outside specialist '{}' scope",
                c.name, a.id
            )),
            artifacts: Vec::new(),
            mode: None,
        };
    }

    let tools = sh.tools();
    let spec = specs.iter().find(|t| t.name == c.name);
    if spec.is_none() {
        let can_dispatch_mcp = if let Some((srv, _)) = workcraft_mcp::hub::split_name(&c.name) {
            tools.hub.is_connected(srv).await
                || (tools.allow_craft_on_demand && find_spec(srv).is_some())
        } else {
            false
        };
        if !can_dispatch_mcp {
            return CallOutcome {
                result: ToolResult::err(format!("tool '{}' is not available", c.name)),
                artifacts: Vec::new(),
                mode: None,
            };
        }
    }

    sh.emit(Event::ToolStarted {
        session: run.session.clone(),
        agent: agent_id.clone(),
        call: c.clone(),
    });

    // Compute approval level (`D6`, `T3.3`, `T3.4`):
    // - Built-in workspace/file tools keep their workspace-boundary read check.
    // - MCP and `craft_*` tools use `classify_tool_call` (gated tools ask EVEN in `Auto`!).
    let is_mcp_or_craft = workcraft_mcp::hub::split_name(&c.name).is_some()
        || c.name.starts_with("craft_")
        || c.name == "open_craft_app";

    let (must_ask, reason, destructive, is_write) = if is_mcp_or_craft {
        match classify_tool_call(&c.name, &c.arguments, spec) {
            ApprovalLevel::Free => (false, None, false, false),
            ApprovalLevel::AskInManual { destructive } => (
                run.approval == ApprovalMode::Manual,
                None,
                destructive,
                true,
            ),
            ApprovalLevel::Gated { reason } => (true, Some(reason), true, true),
        }
    } else {
        let ro =
            spec.is_some_and(|s| s.read_only) && !tools.needs_approval(&c.name, &c.arguments, true);
        (Shared::needs_approval(run.approval, ro), None, false, !ro)
    };

    if must_ask
        && !sh
            .request_approval_with_reason(
                &run.session,
                agent_id.as_deref(),
                c,
                reason,
                destructive,
                &run.cancel,
            )
            .await
    {
        return CallOutcome {
            result: ToolResult::err("the user denied this call"),
            artifacts: Vec::new(),
            mode: None,
        };
    }

    // In Team mode (`agent_id.is_some()`), serialize mutating calls to the same Craft app (`T5.2`).
    let lease_guard = if agent_id.is_some() && is_write {
        if let Some(app_id) = target_craft_app_for_lease(c) {
            Some(sh.app_lease(app_id).lock_owned().await)
        } else {
            None
        }
    } else {
        None
    };

    // Bridge Craft Anything `--inbox` prompts (`core_person_ask` / `core_action_approve`) into
    // WorkCraft's approval flow (`T4.5`).
    let inbox_watcher = if c.name.starts_with("craft-anything__") {
        let inbox_cancel = Arc::new(Cancel::default());
        let handle = sh.spawn_craft_anything_inbox_watcher(
            tools.roots_context().inbox_dir(),
            run.session.clone(),
            agent_id.clone(),
            Arc::clone(&inbox_cancel),
        );
        Some((inbox_cancel, handle))
    } else {
        None
    };

    let outcome = tools
        .call_with_id(ctx, &c.id, &c.name, c.arguments.clone())
        .await;

    if let Some((ic, handle)) = inbox_watcher {
        ic.cancel();
        handle.abort();
    }
    drop(lease_guard);

    outcome
}
