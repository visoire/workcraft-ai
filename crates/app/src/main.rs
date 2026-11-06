//! WorkCraft desktop app. Same engine as `workcraft-cli`.
//!
//! `workcraft --control [port]` also opens the authenticated loopback control channel, the way the Craft
//! apps do, so other tools and agents can drive this window. The address and token go to `control.json`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod app;
mod assets;
mod attach;
mod blip;
mod bridge;
mod markdown;
mod providers;
mod theme;
mod tree;
mod view_artifacts;
mod view_chat;
mod view_settings;
mod view_side;
mod view_team;
mod voice;
mod widgets;

use std::sync::Arc;

use serde_json::json;
use workcraft_agents::{Engine, EngineOptions};
use workcraft_control::server::ControlServer;
use workcraft_control::{CommandBus, new_token};
use workcraft_core::paths;
use workcraft_core::team::ApprovalMode;

const USAGE: &str = "workcraft: chat and agents for the Craft apps\n\nUSAGE\n  workcraft [--control [PORT]] [--auto]\n\n  --control [PORT]  open the control channel on 127.0.0.1 (default: a free port)\n  --auto            run tools without asking\n  -V, --version\n";

struct Args {
    control: Option<u16>,
    auto: bool,
}

fn parse(argv: &[String]) -> Result<Option<Args>, String> {
    let mut a = Args {
        control: None,
        auto: false,
    };
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("workcraft {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--auto" => a.auto = true,
            "--control" => {
                a.control = Some(match argv.get(i + 1).and_then(|p| p.parse::<u16>().ok()) {
                    Some(p) => {
                        i += 1;
                        p
                    }
                    None => 0,
                });
            }
            other => return Err(format!("unknown option '{other}'")),
        }
        i += 1;
    }
    Ok(Some(a))
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse(&argv) {
        Ok(Some(a)) => a,
        Ok(None) => return,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    if let Err(e) = start(args) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn start(args: Args) -> Result<(), String> {
    let rt = Arc::new(tokio::runtime::Runtime::new().map_err(|e| e.to_string())?);
    let engine = rt
        .block_on(Engine::open(
            paths::data_dir(),
            // MCP servers start after the window is up.
            EngineOptions {
                providers: None,
                connect_mcp: false,
                approval: args.auto.then_some(ApprovalMode::Auto),
            },
        ))
        .map_err(|e| e.to_string())?;

    let _control = match args.control {
        Some(port) => {
            let bus: Arc<dyn CommandBus> = Arc::new(engine.clone());
            let server = rt
                .block_on(ControlServer::start(bus, port, new_token()))
                .map_err(|e| e.to_string())?;
            let info = json!({ "addr": server.addr.to_string(), "token": server.token, "pid": std::process::id() });
            let file = paths::data_dir().join("control.json");
            std::fs::write(
                &file,
                serde_json::to_string_pretty(&info).unwrap_or_default(),
            )
            .map_err(|e| e.to_string())?;
            eprintln!(
                "control channel on {} (address and token in {})",
                server.addr,
                file.display()
            );
            Some(server)
        }
        None => None,
    };

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("WorkCraft")
        .with_inner_size([1280.0, 820.0])
        .with_min_inner_size([760.0, 480.0])
        .with_drag_and_drop(true);
    if let Some(icon) = assets::window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let rt2 = Arc::clone(&rt);
    let result = eframe::run_native(
        "WorkCraft",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, rt2, engine)))),
    );
    let _ = std::fs::remove_file(paths::data_dir().join("control.json"));
    result.map_err(|e| e.to_string())
}
