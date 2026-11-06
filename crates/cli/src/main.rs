//! `workcraft-cli`: the headless face of WorkCraft. Same engine as the desktop app.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod args;
mod ide;

use std::io::{BufRead, Write};
use std::sync::Arc;

use args::{Cmd, McpAction, Opts, USAGE};
use serde_json::{Value, json};
use workcraft_agents::{Engine, EngineOptions, WorkCraftMcp};
use workcraft_control::server::ControlServer;
use workcraft_control::{CommandBus, new_token};
use workcraft_core::team::ApprovalMode;
use workcraft_core::{Event, Result, paths};

#[tokio::main]
async fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, opts) = match args::parse(&argv) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    match cmd {
        Cmd::Help => print!("{USAGE}"),
        Cmd::Version => println!("workcraft-cli {}", env!("CARGO_PKG_VERSION")),
        other => {
            if let Err(e) = run(other, opts).await {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
    }
}

async fn open(opts: &Opts, connect_mcp: bool) -> Result<Engine> {
    let engine = Engine::open(
        paths::data_dir(),
        EngineOptions {
            providers: None,
            connect_mcp: connect_mcp && !opts.no_mcp,
            approval: opts.auto.then_some(ApprovalMode::Auto),
        },
    )
    .await?;
    for w in engine.warnings() {
        eprintln!("warning: {w}");
    }
    Ok(engine)
}

async fn run(cmd: Cmd, opts: Opts) -> Result<()> {
    match cmd {
        Cmd::Chat { prompt } => chat(&opts, prompt).await,
        Cmd::Serve { port } => serve(&opts, port).await,
        Cmd::Mcp {
            action: McpAction::Serve,
        } => {
            // stdout belongs to the protocol, so warnings stay on stderr.
            let engine = open(&opts, true).await?;
            let bus: Arc<dyn CommandBus> = Arc::new(engine);
            workcraft_mcp::server::serve_stdio(Arc::new(WorkCraftMcp::new(bus))).await
        }
        Cmd::Mcp { action } => {
            mcp_pairing(&action, &opts);
            Ok(())
        }
        Cmd::Commands { filter } => {
            let engine = open(&opts, false).await?;
            let f = filter.unwrap_or_default().to_lowercase();
            for c in engine
                .commands()
                .into_iter()
                .filter(|c| f.is_empty() || c.id.to_lowercase().contains(&f))
            {
                println!("{:<18} {}", c.id, c.description);
            }
            Ok(())
        }
        Cmd::Run { command, params } => {
            let engine = open(&opts, true).await?;
            let params: Value = match params {
                Some(p) => serde_json::from_str(&p).map_err(|e| {
                    workcraft_core::Error::Invalid(format!("params are not JSON: {e}"))
                })?,
                None => Value::Null,
            };
            let printer = tokio::spawn(print_events(engine.clone(), false));
            let r = engine.execute(&command, params).await?;
            printer.abort();
            println!("{}", serde_json::to_string_pretty(&r)?);
            Ok(())
        }
        Cmd::Models => {
            let engine = open(&opts, false).await?;
            let r = engine.execute("model.list", Value::Null).await?;
            println!("{}", serde_json::to_string_pretty(&r)?);
            Ok(())
        }
        Cmd::Help | Cmd::Version => Ok(()),
    }
}

/// `mcp setup`, `mcp clients` and `mcp snippet`: pair with IDEs and agent apps.
fn mcp_pairing(action: &McpAction, opts: &Opts) {
    let env = ide::Env::detect();
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("workcraft-cli"));
    match action {
        McpAction::Clients => {
            for c in ide::clients(&env) {
                let found = if c.installed {
                    "found    "
                } else {
                    "not found"
                };
                println!(
                    "{:<15} {:<15} {found}  {}",
                    c.id,
                    c.label,
                    c.file.as_ref().map_or_else(
                        || "(no settings file known)".to_string(),
                        |f| f.display().to_string()
                    )
                );
            }
        }
        McpAction::Snippet { client } => {
            let id = client.to_ascii_lowercase();
            match ide::clients(&env).into_iter().find(|c| c.id == id) {
                Some(c) => println!(
                    "{}",
                    ide::snippet(
                        if c.shape == ide::Shape::Manual {
                            ide::Shape::McpServers
                        } else {
                            c.shape
                        },
                        &exe
                    )
                ),
                None => {
                    eprintln!(
                        "error: unknown app ''{client}''. Known: {}",
                        ide::clients(&env)
                            .iter()
                            .map(|c| c.id)
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    std::process::exit(2);
                }
            }
        }
        McpAction::Setup => {
            let outcomes = ide::setup(&env, &exe, opts.clients.as_deref(), opts.dry_run);
            if outcomes.is_empty() {
                println!(
                    "No supported IDE or agent app was found. Name one with --client (see `workcraft-cli mcp clients`)."
                );
            }
            let mut failed = false;
            for o in &outcomes {
                let where_ = o
                    .file
                    .as_ref()
                    .map(|f| format!(" ({})", f.display()))
                    .unwrap_or_default();
                match &o.result {
                    Ok(change) => {
                        let what = match (change, opts.dry_run) {
                            (ide::Change::Added, false) => "added",
                            (ide::Change::Added, true) => "would add",
                            (ide::Change::Refreshed, false) => "updated (the program moved)",
                            (ide::Change::Refreshed, true) => "would update (the program moved)",
                            (ide::Change::Unchanged, _) => "already set up",
                            (ide::Change::LeftAlone, _) => {
                                "left alone: a ''workcraft'' entry you changed is there"
                            }
                        };
                        println!("{:<15} {what}{where_}", o.client);
                    }
                    Err(e) => {
                        failed = true;
                        println!("{:<15} not done: {e}{where_}", o.client);
                        if let Some(s) = &o.snippet {
                            println!("{s}");
                        }
                    }
                }
            }
            if !opts.dry_run
                && outcomes
                    .iter()
                    .any(|o| matches!(o.result, Ok(ide::Change::Added | ide::Change::Refreshed)))
            {
                println!("Restart the app(s) above to load the server.");
            }
            if failed && opts.clients.is_some() {
                std::process::exit(1);
            }
        }
        McpAction::Serve => {}
    }
}
async fn serve(opts: &Opts, port: u16) -> Result<()> {
    let engine = open(opts, true).await?;
    let bus: Arc<dyn CommandBus> = Arc::new(engine);
    let server = ControlServer::start(bus, port, new_token()).await?;
    let info = json!({ "addr": server.addr.to_string(), "token": server.token, "pid": std::process::id() });
    let file = paths::data_dir().join("control.json");
    std::fs::write(&file, serde_json::to_string_pretty(&info)?)?;
    println!(
        "control channel on {} (address and token in {})",
        server.addr,
        file.display()
    );
    let _ = tokio::signal::ctrl_c().await;
    let _ = std::fs::remove_file(&file);
    Ok(())
}

/// Print the engine's events. In interactive mode, ask on stdin before a waiting call.
async fn print_events(engine: Engine, interactive: bool) {
    let mut rx = engine.subscribe_events();
    let mut agent_shown: Option<String> = None;
    while let Ok(ev) = rx.recv().await {
        match ev {
            Event::Text { agent, delta, .. } => {
                if agent != agent_shown {
                    if let Some(a) = &agent {
                        print!("\n[{a}] ");
                    }
                    agent_shown = agent;
                }
                print!("{delta}");
                let _ = std::io::stdout().flush();
            }
            Event::ToolStarted { agent, call, .. } => {
                let who = agent.map(|a| format!("{a}: ")).unwrap_or_default();
                eprintln!(
                    "\n  > {who}{}({})",
                    call.name,
                    short(&call.arguments.to_string(), 120)
                );
            }
            Event::ToolFinished {
                ok,
                output,
                artifacts,
                mode,
                ..
            } => {
                let mode_tag = mode
                    .as_deref()
                    .filter(|m| !m.is_empty())
                    .map(|m| format!(" [{m}]"))
                    .unwrap_or_default();
                eprintln!(
                    "  {}{mode_tag} {}",
                    if ok { "ok" } else { "failed:" },
                    short(&output, 160)
                );
                for art in artifacts {
                    eprintln!("    artifact: {art}");
                }
            }
            Event::TeamPlan { tasks, .. } => {
                for t in tasks.as_array().into_iter().flatten() {
                    eprintln!(
                        "  plan: {} [{}] {}",
                        t["id"].as_str().unwrap_or("?"),
                        t["agent"].as_str().unwrap_or("?"),
                        t["title"].as_str().unwrap_or("")
                    );
                }
            }
            Event::TaskUpdate {
                task, status, note, ..
            } => {
                eprintln!(
                    "  task {task}: {status:?}{}",
                    note.map(|n| format!(" ({n})")).unwrap_or_default()
                );
            }
            Event::ApprovalRequested {
                id,
                call,
                reason,
                destructive,
                ..
            } if interactive => {
                let engine = engine.clone();
                let tag = if destructive { "[DESTRUCTIVE] " } else { "" };
                let why = reason
                    .as_deref()
                    .map(|r| format!("\n    reason: {r}"))
                    .unwrap_or_default();
                eprintln!(
                    "\n  ? {tag}allow {}({}){why}  [y/N] ",
                    call.name,
                    short(&call.arguments.to_string(), 200)
                );
                let answer = tokio::task::spawn_blocking(|| {
                    let mut s = String::new();
                    let _ = std::io::stdin().lock().read_line(&mut s);
                    s
                })
                .await
                .unwrap_or_default();
                let allow = matches!(answer.trim().to_lowercase().as_str(), "y" | "yes");
                let _ = engine
                    .execute("approval.respond", json!({ "id": id, "allow": allow }))
                    .await;
            }
            Event::ApprovalRequested {
                id,
                call,
                reason,
                destructive,
                ..
            } => {
                let tag = if destructive { "[DESTRUCTIVE] " } else { "" };
                let why = reason
                    .as_deref()
                    .map(|r| format!(" [{r}]"))
                    .unwrap_or_default();
                eprintln!(
                    "\n  ? {tag}waiting for approval {id}: {}{why} (answer with approval.respond)",
                    call.name
                );
            }
            Event::Notice { message, .. } => eprintln!("\n  [{message}]"),
            Event::TurnFinished {
                usage, cost_usd, ..
            } => {
                if usage.prompt_tokens() + usage.output_tokens > 0 {
                    eprintln!("  [{}]", usage.summary(cost_usd));
                }
            }
            Event::Error { message, .. } => eprintln!("\n  error: {message}"),
            _ => {}
        }
    }
}

fn short(s: &str, max: usize) -> String {
    let one_line: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        one_line
    } else {
        format!("{}...", one_line.chars().take(max).collect::<String>())
    }
}

async fn chat(opts: &Opts, prompt: Option<String>) -> Result<()> {
    let engine = open(opts, true).await?;
    let printer = tokio::spawn(print_events(engine.clone(), true));
    let mut session = opts.session.clone();
    // A prompt on the command line is answered once; without one this is a REPL.
    let once = prompt.is_some();
    let mut pending = prompt;
    let cfg = engine.shared.config();
    eprintln!(
        "model {}  |  approval {:?}  |  /new, /model <id>, /quit",
        opts.model.as_deref().unwrap_or(&cfg.model),
        cfg.approval
    );
    loop {
        let text = match pending.take() {
            Some(p) => p,
            None => {
                eprint!("\nyou> ");
                let (n, line) = tokio::task::spawn_blocking(|| {
                    let mut s = String::new();
                    let n = std::io::stdin().lock().read_line(&mut s).unwrap_or(0);
                    (n, s)
                })
                .await
                .unwrap_or((0, String::new()));
                if n == 0 {
                    break;
                }
                line.trim().to_string()
            }
        };
        match text.as_str() {
            "" => continue,
            "/quit" | "/exit" => break,
            "/new" => {
                session = None;
                eprintln!("new chat");
                continue;
            }
            _ => {}
        }
        if let Some(m) = text.strip_prefix("/model ") {
            engine
                .execute("model.set", json!({ "model": m.trim() }))
                .await?;
            eprintln!("model set to {}", m.trim());
            continue;
        }
        let mut p = json!({ "text": text });
        if let Some(s) = &session {
            p["session"] = json!(s);
        }
        if let Some(m) = &opts.model {
            p["model"] = json!(m);
        }
        if let Some(t) = &opts.team {
            p["team"] = json!(t);
        }
        match engine.execute("chat.send", p).await {
            Ok(r) => {
                session = r["session"].as_str().map(str::to_string);
                if let Some(s) = r["stopped"].as_str() {
                    eprintln!("\n  (stopped: {s})");
                }
                println!();
            }
            Err(e) => eprintln!("\nerror: {e}"),
        }
        if once {
            break;
        }
    }
    printer.abort();
    if let Some(s) = session {
        eprintln!("session {s}");
    }
    Ok(())
}
