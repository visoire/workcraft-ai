//! Team mode: a leader plans a task graph, workers run it in parallel with scoped tools, the leader
//! reviews each result, and then writes the final answer.
//!
//! One scheduler covers every pattern:
//! - sequential pipeline: tasks that depend on each other;
//! - parallel: independent tasks, up to `concurrency` at once;
//! - swarm: the leader may add tasks while the graph runs (`add_tasks` in a review), up to `max_tasks`;
//! - hand-off: a worker ends with `HANDOFF {"to": "<agent>", "note": "..."}` and a follow-up task is added;
//! - shared memory: every agent of the run reads and writes the same `memory_*` board;
//! - supervision: a rejected result is retried with the leader's note, up to `max_retries`;
//! - human in the loop: in manual mode the plan itself waits for approval, and so does every write.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::task::JoinSet;
use workcraft_core::{
    AgentDef, ChatOptions, Delta, Error, Event, Message, ModelRef, Result, StopReason, TaskSpec,
    TaskStatus, TeamConfig, ToolCall, Usage, new_id, truncate_bytes,
};
use workcraft_llm::ChatRequest;

use crate::agent::{AgentOutcome, AgentRun, run_agent};
use crate::prompt::system_prompt;
use crate::shared::{Cancel, Shared};
use crate::tools::Memory;

#[derive(Debug, Clone)]
pub struct TaskState {
    pub spec: TaskSpec,
    pub status: TaskStatus,
    pub result: String,
    pub retries: u32,
    pub note: Option<String>,
}

#[derive(Debug, Default)]
pub struct TeamOutcome {
    pub run: String,
    pub text: String,
    pub last_id: Option<String>,
    pub usage: Usage,
    pub tasks: Vec<TaskState>,
    pub stopped: Option<String>,
}

// ------------------------------------------------------------------ pure helpers (tested)

/// Pull a JSON object out of model text: plain, in a code fence, or surrounded by prose.
#[must_use]
pub fn extract_json(text: &str) -> Option<Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Some(v);
    }
    if let Some(start) = t.find("```") {
        let after = &t[start + 3..];
        let after = after.strip_prefix("json").unwrap_or(after);
        if let Some(end) = after.find("```")
            && let Ok(v) = serde_json::from_str::<Value>(after[..end].trim())
        {
            return Some(v);
        }
    }
    let first = t.find('{')?;
    let last = t.rfind('}')?;
    if last > first {
        serde_json::from_str(&t[first..=last]).ok()
    } else {
        None
    }
}

/// Check a list of tasks against the workers, the ids already in the graph and the size limit.
pub fn validate_tasks(
    tasks: &[TaskSpec],
    workers: &[AgentDef],
    existing: &[String],
    max_total: usize,
) -> Result<()> {
    if tasks.is_empty() {
        return Err(Error::Invalid("the plan has no tasks".into()));
    }
    if existing.len() + tasks.len() > max_total {
        return Err(Error::Invalid(format!(
            "too many tasks: the limit is {max_total}"
        )));
    }
    let mut ids: HashSet<&str> = existing.iter().map(String::as_str).collect();
    for t in tasks {
        if t.id.trim().is_empty() || !ids.insert(t.id.as_str()) {
            return Err(Error::Invalid(format!(
                "task id '{}' is empty or used twice",
                t.id
            )));
        }
        if !workers.iter().any(|w| w.id == t.agent) {
            let known: Vec<&str> = workers.iter().map(|w| w.id.as_str()).collect();
            return Err(Error::Invalid(format!(
                "task '{}' uses unknown agent '{}'; known: {}",
                t.id,
                t.agent,
                known.join(", ")
            )));
        }
    }
    for t in tasks {
        for d in &t.depends_on {
            if !ids.contains(d.as_str()) {
                return Err(Error::Invalid(format!(
                    "task '{}' depends on unknown task '{d}'",
                    t.id
                )));
            }
        }
    }
    // Cycle check (Kahn) over the new tasks; dependencies on existing tasks are already resolved.
    let mut indeg: HashMap<&str, usize> = tasks
        .iter()
        .map(|t| {
            (
                t.id.as_str(),
                t.depends_on
                    .iter()
                    .filter(|d| tasks.iter().any(|x| x.id == **d))
                    .count(),
            )
        })
        .collect();
    let mut queue: Vec<&str> = indeg
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(k, _)| *k)
        .collect();
    let mut seen = 0;
    while let Some(id) = queue.pop() {
        seen += 1;
        for t in tasks
            .iter()
            .filter(|t| t.depends_on.iter().any(|d| d == id))
        {
            if let Some(n) = indeg.get_mut(t.id.as_str()) {
                *n -= 1;
                if *n == 0 {
                    queue.push(t.id.as_str());
                }
            }
        }
    }
    if seen != tasks.len() {
        return Err(Error::Invalid("the plan has a dependency cycle".into()));
    }
    Ok(())
}

/// Parse `{"tasks": [...]}` into tasks. Missing optional fields get defaults.
pub fn parse_tasks(v: &Value) -> Result<Vec<TaskSpec>> {
    let arr = v
        .get("tasks")
        .or_else(|| v.get("add_tasks"))
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Invalid("expected a JSON object with a 'tasks' array".into()))?;
    arr.iter()
        .map(|t| {
            serde_json::from_value::<TaskSpec>(t.clone())
                .map_err(|e| Error::Invalid(format!("bad task {t}: {e}")))
        })
        .collect()
}

/// Tasks that can start: pending, with every dependency done.
#[must_use]
pub fn ready(states: &[TaskState]) -> Vec<String> {
    states
        .iter()
        .filter(|s| s.status == TaskStatus::Pending)
        .filter(|s| {
            s.spec.depends_on.iter().all(|d| {
                states
                    .iter()
                    .any(|x| x.spec.id == *d && x.status == TaskStatus::Done)
            })
        })
        .map(|s| s.spec.id.clone())
        .collect()
}

/// Pending tasks that can never run because a dependency failed or was skipped.
#[must_use]
pub fn blocked(states: &[TaskState]) -> Vec<String> {
    states
        .iter()
        .filter(|s| s.status == TaskStatus::Pending)
        .filter(|s| {
            s.spec.depends_on.iter().any(|d| {
                states.iter().any(|x| {
                    x.spec.id == *d && matches!(x.status, TaskStatus::Failed | TaskStatus::Skipped)
                })
            })
        })
        .map(|s| s.spec.id.clone())
        .collect()
}

/// A trailing `HANDOFF {"to":"agent","note":"..."}` line, if the worker wrote one.
#[must_use]
pub fn parse_handoff(text: &str) -> Option<(String, String)> {
    let line = text
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with("HANDOFF "))?;
    let v: Value =
        serde_json::from_str(line.trim_start().trim_start_matches("HANDOFF ").trim()).ok()?;
    let to = v.get("to")?.as_str()?.to_string();
    let note = v
        .get("note")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some((to, note))
}

/// Hand-offs per run, and how many can follow one another. A model that hands work back and forth
/// would otherwise keep a team busy until `max_tasks` runs out.
/// Largest result text passed on to another agent or shown to the leader. A longer result is cut with a note.
pub const RESULT_CAP: usize = 4000;

/// Phrases that make a request about more than one thing, so it needs a plan.
const JOINERS: [&str; 8] = [
    " and then ",
    " then ",
    " also ",
    " compare ",
    " after that",
    " afterwards",
    " as well as ",
    " plus ",
];

/// The one specialist a request is about, when it is short, names apps of a single specialist and does not
/// join several jobs. Such a request needs no plan, no review and no summary: the specialist just does it.
#[must_use]
pub fn direct_specialist(goal: &str, workers: &[AgentDef]) -> Option<String> {
    let lower = goal.to_lowercase();
    if goal.chars().count() > 400 || JOINERS.iter().any(|j| lower.contains(j)) {
        return None;
    }
    let specialists: std::collections::BTreeSet<&str> = crate::craft::apps_named_by_words(goal)
        .into_iter()
        .filter_map(crate::craft::find_spec)
        .map(|sp| sp.specialist)
        .collect();
    if specialists.len() != 1 {
        return None;
    }
    let want = specialists.into_iter().next()?;
    workers.iter().find(|w| w.id == want).map(|w| w.id.clone())
}

pub const MAX_HANDOFFS: usize = 4;
pub const MAX_HANDOFF_DEPTH: u32 = 2;

/// May a worker hand its work on? Not to itself, not in a long chain, not too often.
#[must_use]
pub fn handoff_allowed(from_agent: &str, to_agent: &str, depth: u32, used: usize) -> bool {
    from_agent != to_agent && depth < MAX_HANDOFF_DEPTH && used < MAX_HANDOFFS
}

/// The result text without the HANDOFF line.
#[must_use]
pub fn strip_handoff(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim_start().starts_with("HANDOFF "))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

// ------------------------------------------------------------------ model helper

/// One model call without tools. `emit_as` streams the text as events.
async fn complete(
    sh: &Arc<Shared>,
    model: &ModelRef,
    system: &str,
    user: &str,
    cancel: &Cancel,
    emit_as: Option<(&str, &str)>,
    usage: &mut Usage,
) -> Result<String> {
    let provider = sh.provider(&model.provider)?;
    let req = ChatRequest {
        model: model.model.clone(),
        system: Some(system.to_string()),
        messages: vec![Message::user(user)],
        tools: Vec::new(),
        options: ChatOptions {
            num_ctx: sh.config().num_ctx,
            think: Some(false),
            ..ChatOptions::default()
        },
        vision: false,
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Delta>(128);
    let call = tokio::spawn(async move { provider.chat(req, tx).await });
    let mut text = String::new();
    loop {
        tokio::select! {
            d = rx.recv() => match d {
                Some(Delta::Text(t)) => {
                    if let Some((session, agent)) = emit_as {
                        sh.emit(Event::Text { session: session.to_string(), agent: Some(agent.to_string()), delta: t.clone() });
                    }
                    text.push_str(&t);
                }
                Some(Delta::Usage(u)) => {
                    usage.add(u);
                    if let Some((session, _)) = emit_as {
                        sh.record_usage(session, model, u);
                    }
                }
                Some(Delta::Notice(message)) => {
                    if let Some((session, agent)) = emit_as {
                        sh.emit(Event::Notice { session: session.to_string(), agent: Some(agent.to_string()), message });
                    }
                }
                Some(Delta::Done(StopReason::Length)) => {}
                Some(_) => {}
                None => break,
            },
            () = cancel.cancelled() => {
                call.abort();
                return Err(Error::Cancelled);
            }
        }
    }
    match call.await {
        Ok(r) => r?,
        Err(e) => return Err(Error::Provider(format!("model task failed: {e}"))),
    }
    Ok(text)
}

fn worker_list(team: &TeamConfig) -> String {
    team.workers
        .iter()
        .map(|w| format!("- {}: {} ({})", w.id, w.name, w.scope))
        .collect::<Vec<_>>()
        .join("\n")
}

fn plan_prompt(team: &TeamConfig, goal: &str) -> String {
    format!(
        "Goal:\n{goal}\n\nSpecialists you can assign (use their id):\n{}\n\n\
Plan the work as a JSON object and reply with the JSON only:\n\
{{\"tasks\":[{{\"id\":\"t1\",\"title\":\"short title\",\"agent\":\"<specialist id>\",\"instructions\":\"what to do and what to report back\",\"depends_on\":[]}}]}}\n\n\
Rules: at most {} tasks; ids are short and unique; independent tasks run in parallel, so use depends_on only where a task needs another task's result; \
instructions must be complete, the specialist sees only its own task and the results of its dependencies.",
        worker_list(team),
        team.max_tasks
    )
}

fn task_prompt(
    spec: &TaskSpec,
    states: &[TaskState],
    team: &TeamConfig,
    note: Option<&str>,
) -> String {
    let mut s = format!(
        "Task {} - {}\n\n{}\n",
        spec.id, spec.title, spec.instructions
    );
    let deps: Vec<&TaskState> = states
        .iter()
        .filter(|x| spec.depends_on.contains(&x.spec.id))
        .collect();
    if !deps.is_empty() {
        s.push_str("\nResults of the tasks you depend on:\n");
        for d in deps {
            s.push_str(&format!(
                "\n[{} - {}]\n{}\n",
                d.spec.id,
                d.spec.title,
                truncate_bytes(&d.result, RESULT_CAP)
            ));
        }
    }
    if let Some(n) = note {
        s.push_str(&format!(
            "\nYour previous attempt was rejected. Reviewer note: {n}\n"
        ));
    }
    s.push_str(
        "\nUse the shared memory (memory_get, memory_set) for facts other specialists need. \
When you are done, reply with a short result. Only if a different specialist must continue with work outside your own scope, end your reply with one line: \
HANDOFF {\"to\":\"<specialist id>\",\"note\":\"what they should do\"}\n",
    );
    let others: Vec<String> = team.workers.iter().map(|w| w.id.clone()).collect();
    s.push_str(&format!("Specialist ids: {}\n", others.join(", ")));
    s
}

fn review_prompt(goal: &str, st: &TaskState) -> String {
    format!(
        "Goal of the whole job:\n{goal}\n\nTask {} - {}\nInstructions:\n{}\n\nResult from {}:\n{}\n\n\
Review the result. Reply with JSON only: {{\"verdict\":\"accept\" or \"retry\",\"note\":\"what to fix, if retry\",\"add_tasks\":[]}}. \
add_tasks is optional and takes tasks in the same format as the plan, for work the result shows is still missing.",
        st.spec.id,
        st.spec.title,
        st.spec.instructions,
        st.spec.agent,
        truncate_bytes(&st.result, RESULT_CAP)
    )
}

fn update(sh: &Shared, session: &str, run: &str, st: &TaskState, note: Option<String>) {
    sh.emit(Event::TaskUpdate {
        session: session.to_string(),
        run: run.to_string(),
        task: st.spec.id.clone(),
        status: st.status,
        note,
    });
}

// ------------------------------------------------------------------ the run

pub struct TeamRequest {
    pub session: String,
    pub goal: String,
    pub team: TeamConfig,
    pub model: ModelRef,
    /// Last message of the session, the new messages hang below it.
    pub parent: Option<String>,
    pub cancel: Arc<Cancel>,
}

pub async fn run_team(sh: &Arc<Shared>, req: TeamRequest) -> Result<TeamOutcome> {
    let TeamRequest {
        session,
        goal,
        team,
        model,
        parent,
        cancel,
    } = req;
    let run = new_id();
    let mut out = TeamOutcome {
        run: run.clone(),
        ..TeamOutcome::default()
    };
    let leader_model = team.leader.model.clone().unwrap_or_else(|| model.clone());
    let leader_system = system_prompt(sh, Some(&team.leader));
    let mut tail = parent;

    // --- 1. plan (one repair attempt), or straight to the one specialist the request is about
    let direct = if team.direct_route {
        direct_specialist(&goal, &team.workers)
    } else {
        None
    };
    let mut prompt = plan_prompt(&team, &goal);
    let mut specs: Option<Vec<TaskSpec>> = direct.as_ref().map(|agent| {
        vec![TaskSpec {
            id: "t1".into(),
            title: goal.chars().take(60).collect(),
            agent: agent.clone(),
            instructions: goal.clone(),
            depends_on: Vec::new(),
        }]
    });
    let mut last_err = String::new();
    for _ in 0..(if specs.is_some() { 0 } else { 2 }) {
        let text = complete(
            sh,
            &leader_model,
            &leader_system,
            &prompt,
            &cancel,
            None,
            &mut out.usage,
        )
        .await?;
        match extract_json(&text)
            .ok_or_else(|| Error::Invalid("the reply was not JSON".into()))
            .and_then(|v| parse_tasks(&v))
            .and_then(|t| validate_tasks(&t, &team.workers, &[], team.max_tasks).map(|()| t))
        {
            Ok(t) => {
                specs = Some(t);
                break;
            }
            Err(e) => {
                last_err = e.to_string();
                prompt = format!(
                    "{}\n\nYour previous reply was rejected: {last_err}\nReply again with valid JSON only.",
                    plan_prompt(&team, &goal)
                );
            }
        }
    }
    let Some(specs) = specs else {
        return Err(Error::Invalid(format!(
            "the leader could not produce a valid plan: {last_err}"
        )));
    };
    sh.emit(Event::TeamPlan {
        session: session.clone(),
        run: run.clone(),
        tasks: serde_json::to_value(&specs)?,
    });

    // --- 2. human approval of the plan
    if team.plan_needs_approval() && direct.is_none() {
        let call = ToolCall {
            id: new_id(),
            name: "team_plan".into(),
            arguments: json!({ "tasks": specs }),
        };
        if !sh
            .request_approval(&session, Some("leader"), &call, &cancel)
            .await
        {
            out.stopped = Some("plan not approved".into());
            return Ok(out);
        }
    }

    let mut states: Vec<TaskState> = specs
        .into_iter()
        .map(|spec| TaskState {
            spec,
            status: TaskStatus::Pending,
            result: String::new(),
            retries: 0,
            note: None,
        })
        .collect();
    let memory = Arc::new(Memory::default());
    let mut join: JoinSet<(String, Result<AgentOutcome>)> = JoinSet::new();
    let mut running: HashSet<String> = HashSet::new();
    // How deep each hand-off chain is, and how many hand-offs this run has made.
    let mut depth: HashMap<String, u32> = HashMap::new();
    let mut handoffs = 0usize;
    let mut aborts: HashMap<String, tokio::task::AbortHandle> = HashMap::new();

    // --- 3. schedule
    loop {
        // Orders that arrived while nothing was waiting for them (between two workers finishing).
        apply_steers(sh, &session, &run, &mut states, &mut running, &mut aborts);
        if cancel.is_cancelled() {
            join.abort_all();
            out.stopped = Some("cancelled".into());
            break;
        }
        for id in blocked(&states) {
            if let Some(s) = states.iter_mut().find(|s| s.spec.id == id) {
                s.status = TaskStatus::Skipped;
                update(
                    sh,
                    &session,
                    &run,
                    s,
                    Some("a dependency did not finish".into()),
                );
            }
        }
        for id in ready(&states) {
            if running.len() >= team.concurrency.max(1) {
                break;
            }
            let Some(idx) = states.iter().position(|s| s.spec.id == id) else {
                continue;
            };
            states[idx].status = TaskStatus::Running;
            update(sh, &session, &run, &states[idx], None);
            running.insert(id.clone());

            let spec = states[idx].spec.clone();
            let worker = team.workers.iter().find(|w| w.id == spec.agent).cloned();
            let Some(worker) = worker else { continue };
            let user = Message::user(task_prompt(
                &spec,
                &states,
                &team,
                states[idx].note.as_deref(),
            ));
            let shc = Arc::clone(sh);
            let worker_skills = worker.skills.clone();
            let run_approval = worker.approval.unwrap_or(team.approval);
            let agent_run = AgentRun {
                session: session.clone(),
                model: worker.model.clone().unwrap_or_else(|| model.clone()),
                system: system_prompt(sh, Some(&worker)),
                agent: Some(worker),
                history: vec![user],
                persist: false,
                memory: Arc::clone(&memory),
                cancel: Arc::clone(&cancel),
                approval: run_approval,
                max_steps: sh.config().max_steps,
                options: ChatOptions {
                    num_ctx: sh.config().num_ctx,
                    think: sh.config().think,
                    ..ChatOptions::default()
                },
            };
            let abort = join.spawn(async move {
                let tools = shc.tools();
                if tools.allow_craft_on_demand {
                    let cfg = shc.config();
                    let roots = tools.roots_context();
                    for app_id in &worker_skills {
                        if crate::craft::available_on_demand(
                            app_id,
                            &cfg.craft_apps,
                            &cfg.craft_modes,
                        ) && !tools.hub.is_connected(app_id).await
                        {
                            let _ = crate::craft::ensure_headless(app_id, &roots, &tools.hub).await;
                        }
                    }
                }
                (id, run_agent(&shc, agent_run).await)
            });
            aborts.insert(spec.id.clone(), abort);
        }

        if running.is_empty() {
            break;
        }
        // Wait for a worker to finish, or for an order from the person.
        let joined = tokio::select! {
            j = join.join_next() => match j {
                Some(j) => j,
                None => break,
            },
            () = sh.steer_wake.notified() => {
                apply_steers(sh, &session, &run, &mut states, &mut running, &mut aborts);
                continue;
            }
            () = cancel.cancelled() => continue,
        };
        let Ok((id, result)) = joined else { continue };
        // A worker the person stopped may still report; only a task that is still running counts.
        if !running.remove(&id) {
            continue;
        }
        let Some(idx) = states.iter().position(|s| s.spec.id == id) else {
            continue;
        };

        match result {
            Ok(o) => {
                out.usage.add(o.usage);
                let text = strip_handoff(&o.text);
                let handoff = parse_handoff(&o.text);
                states[idx].result = text.clone();
                let empty_or_stopped = text.is_empty() || o.stopped.is_some();
                states[idx].status = TaskStatus::Reviewing;
                update(sh, &session, &run, &states[idx], o.stopped.clone());

                // Review: the leader decides, unless the result is plainly unusable.
                let mut verdict = if empty_or_stopped {
                    Some((
                        "retry".to_string(),
                        o.stopped.clone().unwrap_or_else(|| "empty result".into()),
                        Vec::new(),
                    ))
                } else if team.review
                    && direct.is_none()
                    && !(team.smart_review && o.tools_ok > 0 && o.tools_failed == 0)
                {
                    review(
                        sh,
                        &leader_model,
                        &leader_system,
                        &goal,
                        &states[idx],
                        &cancel,
                        &mut out.usage,
                        &team,
                        &states,
                    )
                    .await
                } else {
                    None
                };
                if cancel.is_cancelled() {
                    continue;
                }
                let (decision, note, extra) = verdict
                    .take()
                    .unwrap_or_else(|| ("accept".into(), String::new(), Vec::new()));
                if decision == "retry" && states[idx].retries < team.max_retries {
                    states[idx].retries += 1;
                    states[idx].note = Some(note.clone());
                    states[idx].status = TaskStatus::Pending;
                    update(
                        sh,
                        &session,
                        &run,
                        &states[idx],
                        Some(format!("retry: {note}")),
                    );
                    continue;
                }
                if decision == "retry" {
                    states[idx].status = TaskStatus::Failed;
                    update(sh, &session, &run, &states[idx], Some(note));
                    continue;
                }
                states[idx].status = TaskStatus::Done;
                update(sh, &session, &run, &states[idx], None);
                tail = persist_result(sh, &session, tail.as_deref(), &states[idx]);

                // Swarm: tasks the leader added in its review.
                if !extra.is_empty() {
                    let existing: Vec<String> = states.iter().map(|s| s.spec.id.clone()).collect();
                    if validate_tasks(&extra, &team.workers, &existing, team.max_tasks).is_ok() {
                        for spec in extra {
                            let st = TaskState {
                                spec,
                                status: TaskStatus::Pending,
                                result: String::new(),
                                retries: 0,
                                note: None,
                            };
                            update(sh, &session, &run, &st, Some("added by the leader".into()));
                            states.push(st);
                        }
                    }
                }
                // Hand-off: a follow-up task for the named specialist, within the limits.
                let from_depth = depth.get(&states[idx].spec.id).copied().unwrap_or(0);
                if let Some((to, note)) = handoff.filter(|(to, _)| {
                    handoff_allowed(&states[idx].spec.agent, to, from_depth, handoffs)
                }) {
                    handoffs += 1;
                    let spec = TaskSpec {
                        id: format!("handoff-{handoffs}"),
                        title: format!("Continue after {}", states[idx].spec.title),
                        agent: to,
                        instructions: note,
                        depends_on: vec![states[idx].spec.id.clone()],
                    };
                    let existing: Vec<String> = states.iter().map(|s| s.spec.id.clone()).collect();
                    if validate_tasks(
                        std::slice::from_ref(&spec),
                        &team.workers,
                        &existing,
                        team.max_tasks,
                    )
                    .is_ok()
                    {
                        let st = TaskState {
                            spec,
                            status: TaskStatus::Pending,
                            result: String::new(),
                            retries: 0,
                            note: None,
                        };
                        depth.insert(st.spec.id.clone(), from_depth + 1);
                        update(sh, &session, &run, &st, Some("hand-off".into()));
                        states.push(st);
                    }
                }
            }
            Err(Error::Cancelled) => {}
            Err(e) => {
                states[idx].status = TaskStatus::Failed;
                states[idx].result = e.to_string();
                update(sh, &session, &run, &states[idx], Some(e.to_string()));
            }
        }
    }

    // --- 4. final answer
    if out.stopped.is_none()
        && direct.is_some()
        && states.len() == 1
        && states[0].status == TaskStatus::Done
    {
        // One specialist did one job: its result is the answer. No summary call.
        out.text = states[0].result.clone();
        out.last_id = tail;
    } else if out.stopped.is_none() {
        let mut summary = format!("Goal:\n{goal}\n\nResults:\n");
        for s in &states {
            summary.push_str(&format!(
                "\n[{} - {} - {:?}]\n{}\n",
                s.spec.id,
                s.spec.title,
                s.status,
                truncate_bytes(&s.result, RESULT_CAP)
            ));
        }
        summary.push_str("\nWrite the final answer for the user: what was done, what failed or was skipped, and where the results are.");
        let text = complete(
            sh,
            &leader_model,
            &leader_system,
            &summary,
            &cancel,
            Some((&session, "leader")),
            &mut out.usage,
        )
        .await?;
        let mut m = Message::assistant(text.clone()).with_parent(tail.as_deref());
        m.agent = Some("leader".into());
        m.model = Some(leader_model.clone());
        if let Err(e) = sh.store.append(&session, &m) {
            sh.emit(Event::Error {
                session: session.clone(),
                message: format!("could not save message: {e}"),
            });
        }
        sh.emit(Event::MessageAdded {
            session: session.clone(),
            message: m.clone(),
        });
        out.last_id = Some(m.id);
        out.text = text;
    } else {
        out.last_id = tail;
    }
    out.tasks = states;
    Ok(out)
}

/// Carry out the orders a person gave a running plan.
fn apply_steers(
    sh: &Shared,
    session: &str,
    run: &str,
    states: &mut [TaskState],
    running: &mut HashSet<String>,
    aborts: &mut HashMap<String, tokio::task::AbortHandle>,
) {
    for order in sh.take_steers(session) {
        let (task, redo_note) = match &order {
            crate::shared::Steer::Skip { task } => (task.clone(), None),
            crate::shared::Steer::Redo { task, note } => (task.clone(), Some(note.clone())),
        };
        let Some(st) = states.iter_mut().find(|s| s.spec.id == task) else {
            continue;
        };
        // A task that is running is stopped first; its result, if it still arrives, is ignored.
        if running.remove(&task)
            && let Some(h) = aborts.remove(&task)
        {
            h.abort();
        }
        match redo_note {
            None => {
                if !matches!(st.status, TaskStatus::Done) {
                    st.status = TaskStatus::Skipped;
                    update(sh, session, run, st, Some("skipped by you".into()));
                }
            }
            Some(note) => {
                st.note = Some(note.clone());
                st.status = TaskStatus::Pending;
                st.result.clear();
                update(
                    sh,
                    session,
                    run,
                    st,
                    Some(format!("again, as you asked: {note}")),
                );
            }
        }
    }
}
#[allow(clippy::too_many_arguments)]
async fn review(
    sh: &Arc<Shared>,
    leader_model: &ModelRef,
    leader_system: &str,
    goal: &str,
    st: &TaskState,
    cancel: &Cancel,
    usage: &mut Usage,
    team: &TeamConfig,
    _states: &[TaskState],
) -> Option<(String, String, Vec<TaskSpec>)> {
    let text = complete(
        sh,
        leader_model,
        leader_system,
        &review_prompt(goal, st),
        cancel,
        None,
        usage,
    )
    .await
    .ok()?;
    let v = extract_json(&text)?;
    let decision = v
        .get("verdict")
        .and_then(Value::as_str)
        .unwrap_or("accept")
        .to_ascii_lowercase();
    let note = v
        .get("note")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let extra = if v
        .get("add_tasks")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
    {
        parse_tasks(&v).unwrap_or_default()
    } else {
        Vec::new()
    };
    let _ = team;
    Some((
        if decision == "retry" {
            "retry".into()
        } else {
            "accept".into()
        },
        note,
        extra,
    ))
}

/// Add a finished task to the session as an assistant message from its worker.
fn persist_result(
    sh: &Shared,
    session: &str,
    tail: Option<&str>,
    st: &TaskState,
) -> Option<String> {
    let mut m = Message::assistant(format!(
        "**{}** ({})\n\n{}",
        st.spec.title, st.spec.agent, st.result
    ))
    .with_parent(tail);
    m.agent = Some(st.spec.agent.clone());
    if let Err(e) = sh.store.append(session, &m) {
        sh.emit(Event::Error {
            session: session.to_string(),
            message: format!("could not save message: {e}"),
        });
        return tail.map(str::to_string);
    }
    sh.emit(Event::MessageAdded {
        session: session.to_string(),
        message: m.clone(),
    });
    Some(m.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker(id: &str) -> AgentDef {
        AgentDef {
            id: id.into(),
            name: id.into(),
            scope: String::new(),
            prompt: String::new(),
            model: None,
            tools: vec![],
            skills: vec![],
            approval: None,
        }
    }

    fn task(id: &str, agent: &str, deps: &[&str]) -> TaskSpec {
        TaskSpec {
            id: id.into(),
            title: id.into(),
            agent: agent.into(),
            instructions: "do".into(),
            depends_on: deps.iter().map(|d| (*d).to_string()).collect(),
        }
    }

    fn st(t: TaskSpec, status: TaskStatus) -> TaskState {
        TaskState {
            spec: t,
            status,
            result: String::new(),
            retries: 0,
            note: None,
        }
    }

    #[test]
    fn extracts_json_from_prose_and_fences() {
        assert!(extract_json(r#"{"a":1}"#).is_some());
        assert_eq!(
            extract_json("Sure!\n```json\n{\"a\":2}\n```\nDone").unwrap()["a"],
            2
        );
        assert_eq!(
            extract_json("Here you go: {\"a\":3} hope that helps").unwrap()["a"],
            3
        );
        assert!(extract_json("no json at all").is_none());
        assert!(extract_json("} backwards {").is_none());
    }

    #[test]
    fn validation_catches_every_plan_mistake() {
        let w = [worker("image"), worker("doc")];
        let ok = [task("a", "image", &[]), task("b", "doc", &["a"])];
        assert!(validate_tasks(&ok, &w, &[], 10).is_ok());
        assert!(validate_tasks(&[], &w, &[], 10).is_err());
        assert!(validate_tasks(&[task("a", "ghost", &[])], &w, &[], 10).is_err());
        assert!(
            validate_tasks(
                &[task("a", "image", &[]), task("a", "doc", &[])],
                &w,
                &[],
                10
            )
            .is_err()
        );
        assert!(validate_tasks(&[task("a", "image", &["zzz"])], &w, &[], 10).is_err());
        assert!(
            validate_tasks(
                &[task("a", "image", &["b"]), task("b", "doc", &["a"])],
                &w,
                &[],
                10
            )
            .is_err(),
            "cycle"
        );
        assert!(
            validate_tasks(&[task("a", "image", &["a"])], &w, &[], 10).is_err(),
            "self dependency"
        );
        assert!(validate_tasks(&ok, &w, &[], 1).is_err(), "too many");
        assert!(
            validate_tasks(&[task("c", "image", &["a"])], &w, &["a".to_string()], 10).is_ok(),
            "depends on an existing task"
        );
    }

    #[test]
    fn parse_tasks_reads_the_plan_shape() {
        let v = json!({"tasks":[{"id":"t1","title":"T","agent":"image","instructions":"x"}]});
        let t = parse_tasks(&v).unwrap();
        assert_eq!(t[0].depends_on.len(), 0);
        assert!(parse_tasks(&json!({"nope":1})).is_err());
        assert!(parse_tasks(&json!({"tasks":[{"id":"t1"}]})).is_err());
    }

    #[test]
    fn scheduler_ready_and_blocked() {
        let states = vec![
            st(task("a", "x", &[]), TaskStatus::Done),
            st(task("b", "x", &["a"]), TaskStatus::Pending),
            st(task("c", "x", &["b"]), TaskStatus::Pending),
            st(task("d", "x", &[]), TaskStatus::Failed),
            st(task("e", "x", &["d"]), TaskStatus::Pending),
        ];
        assert_eq!(ready(&states), vec!["b"]);
        assert_eq!(blocked(&states), vec!["e"]);
    }

    #[test]
    fn handoff_rules_stop_loops() {
        assert!(handoff_allowed("image", "document", 0, 0));
        assert!(
            !handoff_allowed("document", "document", 0, 0),
            "no hand-off to yourself"
        );
        assert!(
            !handoff_allowed("image", "document", MAX_HANDOFF_DEPTH, 0),
            "chains are short"
        );
        assert!(
            !handoff_allowed("image", "document", 0, MAX_HANDOFFS),
            "per-run cap"
        );
    }

    #[test]
    fn handoff_lines() {
        let t = "Cropped the photo.\nHANDOFF {\"to\":\"doc\",\"note\":\"put it in the report\"}";
        assert_eq!(
            parse_handoff(t),
            Some(("doc".into(), "put it in the report".into()))
        );
        assert_eq!(strip_handoff(t), "Cropped the photo.");
        assert_eq!(parse_handoff("HANDOFF not json"), None);
        assert_eq!(parse_handoff("plain"), None);
    }

    #[test]
    fn a_short_request_about_one_specialist_goes_straight_to_it() {
        let workers = vec![worker("image"), worker("document"), worker("video")];
        assert_eq!(
            direct_specialist("make a spreadsheet with a sum formula", &workers).as_deref(),
            Some("document")
        );
        assert_eq!(
            direct_specialist("blur in.png", &workers).as_deref(),
            Some("image")
        );
        // Two specialists, or two jobs, or no app at all need a plan.
        assert_eq!(
            direct_specialist("blur in.png and put it on a slide", &workers),
            None
        );
        assert_eq!(
            direct_specialist("write a letter, then a spreadsheet", &workers),
            None
        );
        assert_eq!(direct_specialist("summarise the meeting", &workers), None);
        // A specialist the team does not have is not invented.
        assert_eq!(
            direct_specialist("trim video.mp4", &[worker("image")]),
            None
        );
        assert_eq!(
            direct_specialist(&format!("blur in.png {}", "x".repeat(500)), &workers),
            None
        );
    }
}
