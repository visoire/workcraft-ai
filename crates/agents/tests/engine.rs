use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::mpsc::Sender;
use workcraft_agents::mcp_server::WorkCraftMcp;
use workcraft_agents::{Engine, EngineOptions};
use workcraft_control::CommandBus;
use workcraft_core::team::ApprovalMode;
use workcraft_core::{Delta, Event, Result, StopReason, ToolCall};
use workcraft_llm::{ChatRequest, Provider, Registry};
use workcraft_mcp::ToolHandler;

type Script = dyn Fn(&ChatRequest, usize) -> Vec<Delta> + Send + Sync;

struct Mock {
    calls: AtomicUsize,
    script: Box<Script>,
}

#[async_trait]
impl Provider for Mock {
    fn id(&self) -> &str {
        "ollama"
    }

    async fn chat(&self, req: ChatRequest, tx: Sender<Delta>) -> Result<()> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        for d in (self.script)(&req, n) {
            if tx.send(d).await.is_err() {
                return Ok(());
            }
        }
        Ok(())
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        Ok(vec!["mock".into()])
    }
}

fn text(t: &str) -> Vec<Delta> {
    vec![Delta::Text(t.into()), Delta::Done(StopReason::Stop)]
}

fn call(name: &str, args: Value) -> Vec<Delta> {
    vec![
        Delta::ToolCall(ToolCall {
            id: format!("c{name}"),
            name: name.into(),
            arguments: args,
        }),
        Delta::Done(StopReason::ToolUse),
    ]
}

fn last_is_tool(req: &ChatRequest) -> bool {
    req.messages
        .last()
        .is_some_and(|m| m.role == workcraft_core::Role::Tool)
}

async fn engine(
    script: impl Fn(&ChatRequest, usize) -> Vec<Delta> + Send + Sync + 'static,
    approval: ApprovalMode,
) -> (Engine, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("wc-engine-{}", uuid::Uuid::new_v4().simple()));
    let mut reg = Registry::new();
    reg.add(Arc::new(Mock {
        calls: AtomicUsize::new(0),
        script: Box::new(script),
    }));
    let e = Engine::open(
        dir.clone(),
        EngineOptions {
            providers: Some(reg),
            connect_mcp: false,
            approval: Some(approval),
        },
    )
    .await
    .unwrap();
    (e, dir)
}

async fn run(e: &Engine, id: &str, p: Value) -> Result<Value> {
    e.execute(id, p).await
}

#[tokio::test]
async fn providers_saved_in_settings_are_built_and_keys_never_leave_the_engine() {
    let dir = std::env::temp_dir().join(format!("wc-prov-{}", uuid::Uuid::new_v4().simple()));
    // Real providers (no mock), so saving one rebuilds the registry.
    let e = Engine::open(dir.clone(), EngineOptions::default())
        .await
        .unwrap();
    let secret = "sk-test-0123456789abcdef";
    let info = run(
        &e,
        "provider.save",
        json!({ "id": "deepseek", "kind": "openai", "name": "DeepSeek",
                "base_url": "https://api.deepseek.com/v1", "api_key": secret }),
    )
    .await
    .unwrap();
    let ds = info
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "deepseek")
        .cloned()
        .unwrap();
    assert_eq!(ds["has_key"], true);
    assert_eq!(ds["key_from"], "saved");
    assert!(
        run(&e, "provider.list", json!({}))
            .await
            .unwrap()
            .to_string()
            .contains("deepseek")
    );

    // Nothing the engine returns carries the key.
    for (cmd, p) in [
        ("config.get", json!({})),
        ("provider.info", json!({})),
        ("config.set", json!({ "max_steps": 10 })),
    ] {
        let v = run(&e, cmd, p).await.unwrap().to_string();
        assert!(!v.contains(secret), "{cmd} leaked the key");
    }
    // The key is on disk, and leaving api_key out of a save keeps it.
    run(
        &e,
        "provider.save",
        json!({ "id": "deepseek", "kind": "openai", "name": "DS" }),
    )
    .await
    .unwrap();
    let file = std::fs::read_to_string(dir.join("config.json")).unwrap();
    assert!(file.contains(secret));
    // null removes the key, remove drops the provider.
    run(
        &e,
        "provider.save",
        json!({ "id": "deepseek", "kind": "openai", "api_key": null }),
    )
    .await
    .unwrap();
    assert!(
        !std::fs::read_to_string(dir.join("config.json"))
            .unwrap()
            .contains(secret)
    );
    run(&e, "provider.remove", json!({ "id": "deepseek" }))
        .await
        .unwrap();
    assert!(
        !run(&e, "provider.list", json!({}))
            .await
            .unwrap()
            .to_string()
            .contains("deepseek")
    );
    // Bad input is refused.
    assert!(
        run(&e, "provider.save", json!({ "id": "Bad Id" }))
            .await
            .is_err()
    );
    assert!(
        run(
            &e,
            "provider.save",
            json!({ "id": "x", "kind": "gemini-native" })
        )
        .await
        .is_err()
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn single_mode_uses_a_tool_and_persists_the_chat() {
    let (e, dir) = engine(
        |req, _| {
            if last_is_tool(req) {
                text("Saved it.")
            } else {
                call("write_file", json!({"path":"a.txt","content":"hi"}))
            }
        },
        ApprovalMode::Auto,
    )
    .await;
    let s = run(&e, "chat.new", json!({})).await.unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    let r = run(
        &e,
        "chat.send",
        json!({"session": s, "text": "write a file"}),
    )
    .await
    .unwrap();
    assert_eq!(r["text"], "Saved it.");
    assert_eq!(
        std::fs::read_to_string(dir.join("workspace").join("a.txt")).unwrap(),
        "hi"
    );

    let got = run(&e, "chat.get", json!({"session": s})).await.unwrap();
    let roles: Vec<&str> = got["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, vec!["user", "assistant", "tool", "assistant"]);
    assert_eq!(
        got["header"]["title"], "write a file",
        "the first message names the chat"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn manual_mode_waits_for_approval_then_runs_or_refuses() {
    for allow in [true, false] {
        let (e, dir) = engine(
            |req, _| {
                if last_is_tool(req) {
                    text("done")
                } else {
                    call("write_file", json!({"path":"b.txt","content":"x"}))
                }
            },
            ApprovalMode::Manual,
        )
        .await;
        let mut events = e.subscribe_events();
        let s = run(&e, "chat.new", json!({})).await.unwrap()["session"]
            .as_str()
            .unwrap()
            .to_string();
        let e2 = e.clone();
        let s2 = s.clone();
        let turn = tokio::spawn(async move {
            run(&e2, "chat.send", json!({"session": s2, "text": "go"})).await
        });

        let id = loop {
            if let Event::ApprovalRequested { id, call, .. } =
                tokio::time::timeout(Duration::from_secs(5), events.recv())
                    .await
                    .unwrap()
                    .unwrap()
            {
                assert_eq!(call.name, "write_file");
                break id;
            }
        };
        assert!(
            !dir.join("workspace").join("b.txt").exists(),
            "nothing runs before approval"
        );
        assert_eq!(
            run(&e, "approval.list", json!({}))
                .await
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        run(&e, "approval.respond", json!({"id": id, "allow": allow}))
            .await
            .unwrap();
        turn.await.unwrap().unwrap();
        assert_eq!(dir.join("workspace").join("b.txt").exists(), allow);
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[tokio::test]
async fn reads_never_ask() {
    let (e, dir) = engine(
        |req, _| {
            if last_is_tool(req) {
                text("listed")
            } else {
                call("list_dir", json!({}))
            }
        },
        ApprovalMode::Manual,
    )
    .await;
    let s = run(&e, "chat.new", json!({})).await.unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    let r = tokio::time::timeout(
        Duration::from_secs(5),
        run(&e, "chat.send", json!({"session": s, "text": "ls"})),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(r["text"], "listed");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn branching_adds_a_second_child_and_history_follows_the_branch() {
    let (e, dir) = engine(|_, n| text(&format!("answer {n}")), ApprovalMode::Auto).await;
    let s = run(&e, "chat.new", json!({})).await.unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    run(&e, "chat.send", json!({"session": s, "text": "first"}))
        .await
        .unwrap();
    let first = run(&e, "chat.get", json!({"session": s})).await.unwrap();
    let user_id = first["messages"][0]["id"].as_str().unwrap().to_string();
    let a1 = first["messages"][1]["id"].as_str().unwrap().to_string();
    run(&e, "chat.send", json!({"session": s, "text": "second"}))
        .await
        .unwrap();
    // Branch: ask a different second question from the first answer.
    let b = run(
        &e,
        "chat.send",
        json!({"session": s, "text": "other second", "leaf": a1}),
    )
    .await
    .unwrap();
    let kids = run(&e, "chat.children", json!({"session": s, "message": a1}))
        .await
        .unwrap();
    assert_eq!(
        kids.as_array().unwrap().len(),
        2,
        "two replies below the first answer"
    );
    let on_branch = run(
        &e,
        "chat.get",
        json!({"session": s, "leaf": b["message_id"]}),
    )
    .await
    .unwrap();
    let contents: Vec<&str> = on_branch["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["content"].as_str().unwrap())
        .collect();
    assert_eq!(contents.len(), 4);
    assert_eq!(contents[0], "first");
    assert_eq!(contents[2], "other second");
    assert!(!contents.contains(&"second"));
    assert!(
        run(
            &e,
            "chat.send",
            json!({"session": s, "text": "x", "leaf": "nope"})
        )
        .await
        .is_err()
    );
    let _ = user_id;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn model_can_change_mid_conversation() {
    let (e, dir) = engine(
        |req, _| text(&format!("model={}", req.model)),
        ApprovalMode::Auto,
    )
    .await;
    let s = run(&e, "chat.new", json!({"model": "ollama:small"}))
        .await
        .unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        run(&e, "chat.send", json!({"session": s, "text": "a"}))
            .await
            .unwrap()["text"],
        "model=small"
    );
    run(
        &e,
        "model.set",
        json!({"session": s, "model": "ollama:big"}),
    )
    .await
    .unwrap();
    assert_eq!(
        run(&e, "chat.send", json!({"session": s, "text": "b"}))
            .await
            .unwrap()["text"],
        "model=big"
    );
    assert_eq!(
        run(
            &e,
            "chat.send",
            json!({"session": s, "text": "c", "model": "ollama:once"})
        )
        .await
        .unwrap()["text"],
        "model=once"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_model_that_only_thinks_is_retried_with_thinking_off() {
    let (e, dir) = engine(
        |req, _| {
            if req.options.think == Some(false) {
                text("herons wade in shallow water")
            } else {
                vec![
                    Delta::Reasoning("hmm, five words...".into()),
                    Delta::Done(StopReason::Stop),
                ]
            }
        },
        ApprovalMode::Auto,
    )
    .await;
    let s = run(&e, "chat.new", json!({})).await.unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    let r = run(&e, "chat.send", json!({"session": s, "text": "five words"}))
        .await
        .unwrap();
    assert_eq!(r["text"], "herons wade in shallow water");
    assert!(r["stopped"].is_null());
    let got = run(&e, "chat.get", json!({"session": s})).await.unwrap();
    assert_eq!(
        got["messages"].as_array().unwrap().len(),
        2,
        "the empty attempt is not stored"
    );

    // A model that never answers, even with thinking off, ends with an explicit reason instead of silence.
    let (e2, dir2) = engine(
        |_, _| vec![Delta::Done(StopReason::Stop)],
        ApprovalMode::Auto,
    )
    .await;
    let s2 = run(&e2, "chat.new", json!({})).await.unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    let r2 = run(&e2, "chat.send", json!({"session": s2, "text": "hi"}))
        .await
        .unwrap();
    assert_eq!(r2["stopped"], "the model returned no answer");
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
}

#[tokio::test]
async fn the_loop_guard_stops_repeated_failing_calls() {
    let (e, dir) = engine(
        |_, _| call("read_file", json!({"path":"missing.txt"})),
        ApprovalMode::Auto,
    )
    .await;
    let s = run(&e, "chat.new", json!({})).await.unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    let r = run(&e, "chat.send", json!({"session": s, "text": "read it"}))
        .await
        .unwrap();
    assert!(
        r["stopped"].as_str().unwrap().contains("three times"),
        "{r}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn the_loop_guard_also_stops_a_successful_call_repeated_forever() {
    let (e, dir) = engine(
        |_, _| call("list_dir", json!({"path": "."})),
        ApprovalMode::Auto,
    )
    .await;
    let s = run(&e, "chat.new", json!({})).await.unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    let r = run(&e, "chat.send", json!({"session": s, "text": "list it"}))
        .await
        .unwrap();
    assert!(r["stopped"].as_str().unwrap().contains("six times"), "{r}");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_model_that_cannot_be_reached_hands_over_to_the_fallback_once() {
    struct Down;
    #[async_trait]
    impl Provider for Down {
        fn id(&self) -> &str {
            "deepseek"
        }
        async fn chat(&self, _: ChatRequest, _: Sender<Delta>) -> Result<()> {
            Err(workcraft_core::Error::Http(
                "cannot reach the service".into(),
            ))
        }
        async fn list_models(&self) -> Result<Vec<String>> {
            Ok(vec![])
        }
    }
    let dir = std::env::temp_dir().join(format!("wc-engine-{}", uuid::Uuid::new_v4().simple()));
    let mut reg = Registry::new();
    reg.add(Arc::new(Down));
    reg.add(Arc::new(Mock {
        calls: AtomicUsize::new(0),
        script: Box::new(|_, _| text("answered by the fallback")),
    }));
    let e = Engine::open(
        dir.clone(),
        EngineOptions {
            providers: Some(reg),
            connect_mcp: false,
            approval: Some(ApprovalMode::Auto),
        },
    )
    .await
    .unwrap();
    run(&e, "config.set", json!({"fallback_model": "ollama:mock"}))
        .await
        .unwrap();
    let s = run(&e, "chat.new", json!({"model": "deepseek:x"}))
        .await
        .unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    let mut events = e.subscribe_events();
    let r = run(
        &e,
        "chat.send",
        json!({"session": s, "text": "hi", "model": "deepseek:x"}),
    )
    .await
    .unwrap();
    assert_eq!(r["text"], "answered by the fallback");
    let mut saw_notice = false;
    while let Ok(ev) = events.try_recv() {
        saw_notice |= matches!(ev, workcraft_core::Event::Notice { message, .. } if message.contains("Continuing with mock"));
    }
    assert!(saw_notice, "the switch is announced");
    // Without a fallback the failure is reported as before.
    run(&e, "config.set", json!({"fallback_model": ""}))
        .await
        .unwrap();
    let s2 = run(&e, "chat.new", json!({"model": "deepseek:x"}))
        .await
        .unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        run(
            &e,
            "chat.send",
            json!({"session": s2, "text": "hi", "model": "deepseek:x"})
        )
        .await
        .is_err()
    );
    let _ = std::fs::remove_dir_all(dir);
}
#[tokio::test]
async fn a_running_task_can_be_skipped_and_redone_by_the_person() {
    struct Plan;
    #[async_trait]
    impl Provider for Plan {
        fn id(&self) -> &str {
            "ollama"
        }
        async fn chat(&self, req: ChatRequest, tx: Sender<Delta>) -> Result<()> {
            let user = req
                .messages
                .last()
                .map(|m| m.content.clone())
                .unwrap_or_default();
            let reply = if is_leader(&req) {
                if user.contains("Plan the work") {
                    r#"{"tasks":[{"id":"a","title":"A","agent":"image","instructions":"slow"},{"id":"b","title":"B","agent":"document","instructions":"after a","depends_on":["a"]}]}"#.to_string()
                } else if user.contains("Review the result") {
                    r#"{"verdict":"accept"}"#.to_string()
                } else {
                    "final".to_string()
                }
            } else if user.contains("Task a") && !user.contains("again please") {
                // The first attempt hangs until the person steps in.
                tokio::time::sleep(Duration::from_secs(30)).await;
                "never".to_string()
            } else if user.contains("again please") {
                "redone with the note".to_string()
            } else {
                "b done".to_string()
            };
            let _ = tx.send(Delta::Text(reply)).await;
            let _ = tx.send(Delta::Done(StopReason::Stop)).await;
            Ok(())
        }
        async fn list_models(&self) -> Result<Vec<String>> {
            Ok(vec![])
        }
    }
    let dir = std::env::temp_dir().join(format!("wc-engine-{}", uuid::Uuid::new_v4().simple()));
    let mut reg = Registry::new();
    reg.add(Arc::new(Plan));
    let e = Engine::open(
        dir.clone(),
        EngineOptions {
            providers: Some(reg),
            connect_mcp: false,
            approval: Some(ApprovalMode::Auto),
        },
    )
    .await
    .unwrap();
    let s = run(&e, "chat.new", json!({})).await.unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    let mut events = e.subscribe_events();
    let e2 = e.clone();
    let s2 = s.clone();
    let turn = tokio::spawn(async move {
        e2.execute(
            "chat.send",
            json!({"session": s2, "text": "go", "team": "craft"}),
        )
        .await
    });
    // Wait until task a runs, then run it again with a note.
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let Event::TaskUpdate { task, status, .. } = ev
            && task == "a"
            && status == workcraft_core::TaskStatus::Running
        {
            break;
        }
    }
    run(
        &e,
        "team.steer",
        json!({"session": s, "task": "a", "action": "redo", "note": "again please"}),
    )
    .await
    .unwrap();
    let r = tokio::time::timeout(Duration::from_secs(10), turn)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let tasks = r["tasks"].as_array().unwrap();
    assert_eq!(tasks[0]["result"], "redone with the note");
    assert_eq!(
        tasks[1]["result"], "b done",
        "the dependent task ran after the redo"
    );
    // Nobody is listening once the plan is over.
    assert!(
        run(
            &e,
            "team.steer",
            json!({"session": s, "task": "a", "action": "skip"})
        )
        .await
        .is_err()
    );
    let _ = std::fs::remove_dir_all(dir);
}
#[tokio::test]
async fn cancel_stops_a_running_turn() {
    struct Slow;
    #[async_trait]
    impl Provider for Slow {
        fn id(&self) -> &str {
            "ollama"
        }
        async fn chat(&self, _: ChatRequest, tx: Sender<Delta>) -> Result<()> {
            let _ = tx.send(Delta::Text("thinking".into())).await;
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(())
        }
        async fn list_models(&self) -> Result<Vec<String>> {
            Ok(vec![])
        }
    }
    let dir = std::env::temp_dir().join(format!("wc-engine-{}", uuid::Uuid::new_v4().simple()));
    let mut reg = Registry::new();
    reg.add(Arc::new(Slow));
    let e = Engine::open(
        dir.clone(),
        EngineOptions {
            providers: Some(reg),
            connect_mcp: false,
            approval: Some(ApprovalMode::Auto),
        },
    )
    .await
    .unwrap();
    let s = run(&e, "chat.new", json!({})).await.unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    let (e2, s2) = (e.clone(), s.clone());
    let turn =
        tokio::spawn(
            async move { run(&e2, "chat.send", json!({"session": s2, "text": "go"})).await },
        );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        run(&e, "chat.cancel", json!({"session": s})).await.unwrap()["cancelled"],
        true
    );
    let r = tokio::time::timeout(Duration::from_secs(5), turn)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(r["stopped"], "cancelled");
    // The partial answer was kept, and the session accepts the next message.
    let got = run(&e, "chat.get", json!({"session": s})).await.unwrap();
    assert_eq!(got["messages"].as_array().unwrap().len(), 2);
    let _ = std::fs::remove_dir_all(dir);
}

fn is_leader(req: &ChatRequest) -> bool {
    req.system
        .as_deref()
        .is_some_and(|s| s.contains("You are Team leader"))
}

#[tokio::test]
async fn team_run_plans_runs_in_dependency_order_reviews_and_answers() {
    let plan = json!({"tasks":[
        {"id":"t1","title":"Crop","agent":"image","instructions":"crop the photo","depends_on":[]},
        {"id":"t2","title":"Cut","agent":"video","instructions":"cut the clip","depends_on":[]},
        {"id":"t3","title":"Report","agent":"document","instructions":"write it up","depends_on":["t1","t2"]}
    ]});
    let plan_text = plan.to_string();
    let (e, dir) = engine(
        move |req, _| {
            let user = req
                .messages
                .last()
                .map(|m| m.content.clone())
                .unwrap_or_default();
            if is_leader(req) {
                if user.contains("Plan the work") {
                    text(&plan_text)
                } else if user.contains("Review the result") {
                    text(r#"{"verdict":"accept","note":"","add_tasks":[]}"#)
                } else {
                    text("All three jobs are done.")
                }
            } else if user.contains("Results of the tasks you depend on") {
                text(&format!(
                    "report built from: {}",
                    user.contains("[t1") && user.contains("[t2")
                ))
            } else {
                text("worker result")
            }
        },
        ApprovalMode::Auto,
    )
    .await;
    let mut events = e.subscribe_events();
    let r = run(&e, "team.run", json!({"goal": "make the package"}))
        .await
        .unwrap();
    assert_eq!(r["text"], "All three jobs are done.");
    let tasks = r["tasks"].as_array().unwrap();
    assert!(tasks.iter().all(|t| t["status"] == "done"), "{tasks:?}");
    let t3 = tasks.iter().find(|t| t["id"] == "t3").unwrap();
    assert_eq!(
        t3["result"], "report built from: true",
        "t3 saw the results of t1 and t2"
    );

    // Event order: t3 only starts after both t1 and t2 are done.
    let mut done = Vec::new();
    let mut t3_started_after = None;
    while let Ok(ev) = events.try_recv() {
        if let Event::TaskUpdate { task, status, .. } = ev {
            if status == workcraft_core::TaskStatus::Done {
                done.push(task.clone());
            }
            if task == "t3" && status == workcraft_core::TaskStatus::Running {
                t3_started_after = Some(done.clone());
            }
        }
    }
    let before = t3_started_after.unwrap();
    assert!(before.contains(&"t1".to_string()) && before.contains(&"t2".to_string()));

    // The session holds the plan results and the final answer.
    let s = r["session"].as_str().unwrap();
    let chat = run(&e, "chat.get", json!({"session": s})).await.unwrap();
    let agents: Vec<&str> = chat["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["agent"].as_str())
        .collect();
    assert_eq!(agents.last(), Some(&"leader"));
    assert!(agents.contains(&"image") && agents.contains(&"video") && agents.contains(&"document"));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn team_retries_a_rejected_task_then_accepts() {
    let reviews = Arc::new(AtomicUsize::new(0));
    let r2 = Arc::clone(&reviews);
    let (e, dir) = engine(
        move |req, _| {
            let user = req.messages.last().map(|m| m.content.clone()).unwrap_or_default();
            if is_leader(req) {
                if user.contains("Plan the work") {
                    text(r#"{"tasks":[{"id":"a","title":"A","agent":"image","instructions":"do a"}]}"#)
                } else if user.contains("Review the result") {
                    if r2.fetch_add(1, Ordering::SeqCst) == 0 {
                        text(r#"{"verdict":"retry","note":"too vague"}"#)
                    } else {
                        text(r#"{"verdict":"accept"}"#)
                    }
                } else {
                    text("final")
                }
            } else if user.contains("too vague") {
                text("better result")
            } else {
                text("first try")
            }
        },
        ApprovalMode::Auto,
    )
    .await;
    let r = run(&e, "team.run", json!({"goal": "g"})).await.unwrap();
    assert_eq!(r["tasks"][0]["result"], "better result");
    assert_eq!(r["tasks"][0]["status"], "done");
    assert_eq!(reviews.load(Ordering::SeqCst), 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn team_handoff_adds_a_follow_up_task() {
    let (e, dir) = engine(
        |req, _| {
            let user = req.messages.last().map(|m| m.content.clone()).unwrap_or_default();
            if is_leader(req) {
                if user.contains("Plan the work") {
                    text(r#"{"tasks":[{"id":"a","title":"A","agent":"image","instructions":"edit"}]}"#)
                } else if user.contains("Review the result") {
                    text(r#"{"verdict":"accept"}"#)
                } else {
                    text("final")
                }
            } else if user.contains("Continue after") || user.contains("lay it out") {
                text("laid out")
            } else {
                text("edited\nHANDOFF {\"to\":\"document\",\"note\":\"lay it out\"}")
            }
        },
        ApprovalMode::Auto,
    )
    .await;
    let r = run(&e, "team.run", json!({"goal": "g"})).await.unwrap();
    let tasks = r["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 2);
    assert_eq!(
        tasks[0]["result"], "edited",
        "the HANDOFF line is not part of the result"
    );
    assert_eq!(tasks[1]["agent"], "document");
    assert_eq!(tasks[1]["result"], "laid out");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn an_invalid_plan_is_repaired_once_then_reported() {
    let (e, dir) = engine(
        |_, _| text(r#"{"tasks":[{"id":"a","title":"A","agent":"nobody","instructions":"x"}]}"#),
        ApprovalMode::Auto,
    )
    .await;
    let err = run(&e, "team.run", json!({"goal": "g"}))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("could not produce a valid plan") && err.contains("nobody"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn manual_team_run_needs_plan_approval() {
    let (e, dir) = engine(
        |req, _| {
            let user = req
                .messages
                .last()
                .map(|m| m.content.clone())
                .unwrap_or_default();
            if user.contains("Plan the work") {
                text(r#"{"tasks":[{"id":"a","title":"A","agent":"image","instructions":"x"}]}"#)
            } else {
                text("ok")
            }
        },
        ApprovalMode::Manual,
    )
    .await;
    let mut events = e.subscribe_events();
    let (e2,) = (e.clone(),);
    let turn = tokio::spawn(async move { e2.execute("team.run", json!({"goal": "g"})).await });
    let id = loop {
        if let Event::ApprovalRequested { id, call, .. } =
            tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap()
        {
            assert_eq!(call.name, "team_plan");
            break id;
        }
    };
    e.execute("approval.respond", json!({"id": id, "allow": false}))
        .await
        .unwrap();
    let r = turn.await.unwrap().unwrap();
    assert_eq!(r["stopped"], "plan not approved");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn the_workcraft_mcp_server_exposes_every_command() {
    let (e, dir) = engine(|_, _| text("hello from the model"), ApprovalMode::Auto).await;
    let bus: Arc<dyn CommandBus> = Arc::new(e.clone());
    let mcp = WorkCraftMcp::new(bus);
    let tools = mcp.list_tools().await;
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    for n in [
        "chat_send",
        "chat_new",
        "team_run",
        "agent_define",
        "approval_respond",
        "model_set",
        "mode_set",
        "command_run",
        "command_list",
    ] {
        assert!(names.contains(&n), "{n} missing");
    }
    let new = mcp.call_tool("chat_new", json!({"title": "via mcp"})).await;
    assert!(new.ok, "{}", new.text);
    let session = serde_json::from_str::<Value>(&new.text).unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    let sent = mcp
        .call_tool("chat_send", json!({"session": session, "text": "hi"}))
        .await;
    assert!(sent.text.contains("hello from the model"), "{}", sent.text);
    let generic = mcp
        .call_tool("command_run", json!({"command": "chat.list"}))
        .await;
    assert!(generic.ok && generic.text.contains("via mcp"));
    assert!(
        !mcp.call_tool("command_run", json!({"command": "nope"}))
            .await
            .ok
    );
    assert!(!mcp.call_tool("nope", json!({})).await.ok);

    // agent.define round trip and persistence
    let def = mcp
        .call_tool(
            "agent_define",
            json!({"agent": {"id": "colorist", "name": "Colorist", "tools": ["lightcraft__*"]}}),
        )
        .await;
    assert!(def.ok, "{}", def.text);
    assert!(dir.join("agents").join("colorist.json").exists());
    assert!(
        mcp.call_tool("agent_list", json!({}))
            .await
            .text
            .contains("colorist")
    );
    assert!(
        !mcp.call_tool(
            "agent_define",
            json!({"agent": {"id": "../evil", "name": "x"}})
        )
        .await
        .ok
    );
    let _ = std::fs::remove_dir_all(dir);
}

struct FakeCraftServer {
    app: String,
    tools: Vec<workcraft_core::ToolSpec>,
    active_writes: Arc<AtomicUsize>,
    max_concurrent_writes: Arc<AtomicUsize>,
    tiny_png_b64: Option<String>,
}

#[async_trait]
impl ToolHandler for FakeCraftServer {
    fn info(&self) -> (String, String) {
        (self.app.clone(), "1.0.0-test".into())
    }

    async fn list_tools(&self) -> Vec<workcraft_core::ToolSpec> {
        self.tools.clone()
    }

    async fn call_tool(&self, name: &str, _args: Value) -> workcraft_core::ToolResult {
        if !workcraft_core::looks_read_only(name) {
            let cur = self.active_writes.fetch_add(1, Ordering::SeqCst) + 1;
            let _ = self.max_concurrent_writes.fetch_max(cur, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(60)).await;
            self.active_writes.fetch_sub(1, Ordering::SeqCst);
        }
        if let Some(b64) = &self.tiny_png_b64
            && name.contains("render")
        {
            return workcraft_core::ToolResult {
                text: json!({ "ok": true, "tool": name }).to_string(),
                ok: true,
                images: vec![workcraft_core::ImagePart {
                    mime_type: "image/png".into(),
                    data: b64.clone(),
                }],
            };
        }
        workcraft_core::ToolResult::ok(json!({ "ok": true, "tool": name }).to_string())
    }
}

fn make_tiny_png_b64() -> String {
    use base64::Engine as _;
    let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([40, 120, 240, 255]));
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, image::ImageFormat::Png)
        .unwrap();
    base64::engine::general_purpose::STANDARD.encode(buf.into_inner())
}

#[tokio::test]
async fn gated_tools_ask_even_in_auto_mode_with_explicit_reason() {
    let (e, dir) = engine(
        |req, _| {
            if last_is_tool(req) {
                text("script finished")
            } else {
                call(
                    "photocraft__command_run",
                    json!({ "id": "script.eval", "params": { "code": "1 + 1" } }),
                )
            }
        },
        ApprovalMode::Auto,
    )
    .await;

    let handler = Arc::new(FakeCraftServer {
        app: "photocraft".into(),
        tools: vec![workcraft_core::ToolSpec {
            name: "command_run".into(),
            description: "Run command".into(),
            input_schema: json!({"type": "object", "properties": {"id": {"type": "string"}}}),
            read_only: false,
            read_only_hint: Some(false),
            destructive: false,
        }],
        active_writes: Arc::new(AtomicUsize::new(0)),
        max_concurrent_writes: Arc::new(AtomicUsize::new(0)),
        tiny_png_b64: None,
    });
    let client = workcraft_mcp::McpClient::from_handler("photocraft", vec![], handler)
        .await
        .unwrap();
    e.shared.tools().hub.add(client).await;
    run(&e, "config.set", json!({})).await.unwrap();

    let mut events = e.subscribe_events();
    let s = run(&e, "chat.new", json!({})).await.unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    let (e2, s2) = (e.clone(), s.clone());
    let turn =
        tokio::spawn(
            async move { run(&e2, "chat.send", json!({"session": s2, "text": "eval"})).await },
        );

    let (id, reason, destructive) = loop {
        if let Event::ApprovalRequested {
            id,
            call,
            reason,
            destructive,
            ..
        } = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap()
        {
            assert_eq!(call.name, "photocraft__command_run");
            break (id, reason, destructive);
        }
    };
    assert!(destructive, "gated script tool must be marked destructive");
    assert!(
        reason.as_deref().is_some_and(|r| !r.is_empty()),
        "gated approval must carry an explicit reason: {reason:?}"
    );
    run(&e, "approval.respond", json!({"id": id, "allow": true}))
        .await
        .unwrap();
    let r = turn.await.unwrap().unwrap();
    assert_eq!(r["text"], "script finished");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn mention_prioritizes_app_under_local_budget_and_persists_vision_artifacts() {
    let seen_budgets = Arc::new(std::sync::Mutex::new(Vec::<(usize, bool, usize)>::new()));
    let sb = Arc::clone(&seen_budgets);
    let (e, dir) = engine(
        move |req, _| {
            let has_photo = req
                .tools
                .iter()
                .any(|t| t.name == "photocraft__doc_render_preview");
            let tool_imgs = req
                .messages
                .iter()
                .filter(|m| m.role == workcraft_core::Role::Tool)
                .map(|m| m.attachments.len())
                .sum::<usize>();
            sb.lock()
                .unwrap()
                .push((req.tools.len(), has_photo, tool_imgs));
            if last_is_tool(req) {
                text("rendered ok")
            } else {
                call("photocraft__doc_render_preview", json!({"max_side": 256}))
            }
        },
        ApprovalMode::Auto,
    )
    .await;

    // Register 5 noisy MCP servers (5 * 14 = 70 tools) + photocraft so total > LOCAL_TOOL_BUDGET (60).
    for srv_idx in 0..5 {
        let srv_name = format!("extra{srv_idx}");
        let tools = (0..14)
            .map(|i| workcraft_core::ToolSpec {
                name: format!("tool_{i}"),
                description: "extra tool".into(),
                input_schema: json!({"type": "object", "properties": {}}),
                read_only: true,
                read_only_hint: Some(true),
                destructive: false,
            })
            .collect();
        let h = Arc::new(FakeCraftServer {
            app: srv_name.clone(),
            tools,
            active_writes: Arc::new(AtomicUsize::new(0)),
            max_concurrent_writes: Arc::new(AtomicUsize::new(0)),
            tiny_png_b64: None,
        });
        e.shared
            .tools()
            .hub
            .add(
                workcraft_mcp::McpClient::from_handler(&srv_name, vec![], h)
                    .await
                    .unwrap(),
            )
            .await;
    }
    let photo_handler = Arc::new(FakeCraftServer {
        app: "photocraft".into(),
        tools: vec![workcraft_core::ToolSpec {
            name: "doc_render_preview".into(),
            description: "Render preview".into(),
            input_schema: json!({"type": "object", "properties": {"max_side": {"type": "integer"}}}),
            read_only: true,
            read_only_hint: Some(true),
            destructive: false,
        }],
        active_writes: Arc::new(AtomicUsize::new(0)),
        max_concurrent_writes: Arc::new(AtomicUsize::new(0)),
        tiny_png_b64: Some(make_tiny_png_b64()),
    });
    e.shared
        .tools()
        .hub
        .add(
            workcraft_mcp::McpClient::from_handler("photocraft", vec![], photo_handler)
                .await
                .unwrap(),
        )
        .await;
    run(&e, "config.set", json!({})).await.unwrap();

    let mut events = e.subscribe_events();
    let s = run(&e, "chat.new", json!({"model": "ollama:llava:7b"}))
        .await
        .unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    let r = run(
        &e,
        "chat.send",
        json!({"session": s, "text": "@photocraft render preview"}),
    )
    .await
    .unwrap();
    assert_eq!(r["text"], "rendered ok");

    let recorded = seen_budgets.lock().unwrap().clone();
    assert_eq!(recorded.len(), 2);
    assert!(
        recorded[0].0 <= workcraft_agents::craft::LOCAL_TOOL_BUDGET,
        "local model budget exceeded: {}",
        recorded[0].0
    );
    assert!(
        recorded[0].1,
        "@photocraft tool must be prioritized inside the local budget"
    );
    assert_eq!(
        recorded[1].2, 1,
        "vision model must receive the rendered PNG attachment on the follow-up turn"
    );

    let mut artifact_found = false;
    while let Ok(ev) = events.try_recv() {
        if let Event::ToolFinished { artifacts, .. } = ev
            && let Some(first) = artifacts.first()
        {
            assert!(std::path::Path::new(first).is_file(), "missing {first}");
            artifact_found = true;
        }
    }
    assert!(
        artifact_found,
        "expected saved PNG artifact in Event::ToolFinished"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn team_mode_writer_lease_serializes_same_app_mutations() {
    let active = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));
    let plan = json!({"tasks":[
        {"id":"w1","title":"Write intro","agent":"document","instructions":"intro","depends_on":[]},
        {"id":"w2","title":"Write outro","agent":"document","instructions":"outro","depends_on":[]}
    ]});
    let plan_str = plan.to_string();
    let (e, dir) = engine(
        move |req, _| {
            let user = req
                .messages
                .last()
                .map(|m| m.content.clone())
                .unwrap_or_default();
            if is_leader(req) {
                if user.contains("Plan the work") {
                    text(&plan_str)
                } else if user.contains("Review the result") {
                    text(r#"{"verdict":"accept"}"#)
                } else {
                    text("both written safely")
                }
            } else if last_is_tool(req) {
                text("typed")
            } else {
                call("wordcraft__type_text", json!({"text": "hello"}))
            }
        },
        ApprovalMode::Auto,
    )
    .await;

    let wc = Arc::new(FakeCraftServer {
        app: "wordcraft".into(),
        tools: vec![workcraft_core::ToolSpec {
            name: "type_text".into(),
            description: "Type text".into(),
            input_schema: json!({"type": "object", "properties": {"text": {"type": "string"}}}),
            read_only: false,
            read_only_hint: Some(false),
            destructive: false,
        }],
        active_writes: Arc::clone(&active),
        max_concurrent_writes: Arc::clone(&max_seen),
        tiny_png_b64: None,
    });
    e.shared
        .tools()
        .hub
        .add(
            workcraft_mcp::McpClient::from_handler("wordcraft", vec![], wc)
                .await
                .unwrap(),
        )
        .await;
    run(&e, "config.set", json!({})).await.unwrap();

    let r = run(&e, "team.run", json!({"goal": "co-author doc"}))
        .await
        .unwrap();
    assert_eq!(r["text"], "both written safely");
    assert_eq!(
        max_seen.load(Ordering::SeqCst),
        1,
        "per-app writer lease must serialize concurrent mutations to wordcraft"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn craft_anything_inbox_request_routes_through_approval_and_writes_answer() {
    let (e, dir) = engine(|_, _| text("ok"), ApprovalMode::Auto).await;
    let mut events = e.subscribe_events();
    let inbox_session = dir.join("craft-anything-inbox").join("session-ca-1");
    std::fs::create_dir_all(&inbox_session).unwrap();
    let req_path = inbox_session.join("req-42.request.json");
    let ans_path = inbox_session.join("req-42.answer.json");
    std::fs::write(
        &req_path,
        serde_json::to_string_pretty(&json!({
            "id": "req-42",
            "kind": "approval",
            "title": "Approve target_probe",
            "detail": "Clean-room confirmation required",
            "choices": ["allow", "deny"]
        }))
        .unwrap(),
    )
    .unwrap();

    let sh = Arc::clone(&e.shared);
    let inbox_clone = inbox_session.clone();
    let seen = Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new()));
    let seen_clone = Arc::clone(&seen);
    let watcher = tokio::spawn(async move {
        let mut guard = seen_clone.lock().await;
        let cancel = workcraft_agents::shared::Cancel::default();
        sh.process_craft_anything_inbox_once(
            &inbox_clone,
            &mut guard,
            "session-ca-1",
            None,
            &cancel,
        )
        .await;
    });

    let approval_id = loop {
        if let Event::ApprovalRequested {
            id, call, reason, ..
        } = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap()
        {
            assert_eq!(call.name, "craft-anything__core_action_approve");
            assert!(
                call.arguments.to_string().contains("target_probe"),
                "{}",
                call.arguments
            );
            assert!(
                reason
                    .as_deref()
                    .is_some_and(|r| r.contains("Clean-room confirmation"))
            );
            break id;
        }
    };

    run(
        &e,
        "approval.respond",
        json!({ "id": approval_id, "allow": true }),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), watcher)
        .await
        .unwrap()
        .unwrap();

    assert!(
        ans_path.is_file(),
        "expected answer file at {}",
        ans_path.display()
    );
    let ans: Value = serde_json::from_str(&std::fs::read_to_string(&ans_path).unwrap()).unwrap();
    assert_eq!(ans["allow"], true);
    assert_eq!(ans["choice"], "allow");

    // A second scan with the same `seen` set must be a no-op (no duplicate approval emitted).
    {
        let mut guard = seen.lock().await;
        let cancel = workcraft_agents::shared::Cancel::default();
        e.shared
            .process_craft_anything_inbox_once(
                &inbox_session,
                &mut guard,
                "session-ca-1",
                None,
                &cancel,
            )
            .await;
    }
    let dup = tokio::time::timeout(Duration::from_millis(120), async {
        loop {
            if let Ok(Event::ApprovalRequested { .. }) = events.recv().await {
                return true;
            }
        }
    })
    .await;
    assert!(
        dup.is_err(),
        "seen request should not emit a second ApprovalRequested"
    );
    let _ = std::fs::remove_dir_all(dir);
}
