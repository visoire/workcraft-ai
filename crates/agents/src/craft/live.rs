//! Control ports, authentication tokens, headless/live server entries, and `ensure_live` (D3, D8, D9).

use std::collections::{BTreeMap, BTreeSet};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use workcraft_control::new_token;
use workcraft_mcp::{McpHub, ServerEntry};

use crate::craft::discover::{
    Found, default_install_bases, discover_with_errors, find_gui_for_spec,
};
use crate::craft::spec::{
    AppSpec, CraftMode, LiveAuth, LiveBridgeKind, RootsPolicy, SPECS, find_spec,
};

/// Sandbox directory context passed when starting headless or bridged Craft MCP servers (D9).
#[derive(Debug, Clone, Default)]
pub struct RootsContext {
    pub workspace: Option<PathBuf>,
    pub extra_roots: Vec<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub craft_anything_deep: bool,
    pub craft_anything_allow_run: bool,
}

impl RootsContext {
    #[must_use]
    pub fn for_workspace(workspace: PathBuf, data_dir: PathBuf, extra_roots: Vec<PathBuf>) -> Self {
        Self {
            workspace: Some(workspace),
            extra_roots,
            data_dir: Some(data_dir),
            craft_anything_deep: false,
            craft_anything_allow_run: false,
        }
    }

    #[must_use]
    pub fn inbox_dir(&self) -> PathBuf {
        self.data_dir
            .as_ref()
            .cloned()
            .unwrap_or_else(|| std::env::temp_dir().join("workcraft"))
            .join("craft-anything-inbox")
    }
}

/// Default `--control <port>` for each Craft desktop app that supports live MCP bridging.
#[must_use]
pub fn default_control_port(id: &str) -> Option<u16> {
    find_spec(id).and_then(|s| s.port)
}

/// Verify at startup/test time that no two apps share a control port and none uses 7878.
pub fn verify_unique_ports() -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for s in &SPECS {
        if let Some(p) = s.port {
            if p == 7878 {
                return Err(format!("{} uses reserved default port 7878", s.id));
            }
            if !seen.insert(p) {
                return Err(format!("duplicate control port {p} on {}", s.id));
            }
        }
    }
    Ok(())
}

#[must_use]
pub fn is_port_listening(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(60)).is_ok()
}

/// Deterministic token file path in `%TEMP%` for PhotoCraft's `--control-token-file`.
#[must_use]
pub fn photocraft_token_file(port: u16) -> PathBuf {
    std::env::temp_dir().join(format!("photocraft-control-{port}.token"))
}

fn ensure_hex64_token_file(path: &Path) -> String {
    if let Ok(existing) = std::fs::read_to_string(path) {
        let trimmed = existing.trim();
        if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return trimmed.to_string();
        }
    }
    let fresh = new_token();
    let _ = std::fs::write(path, &fresh);
    fresh
}

/// Persistent 64-hex token for FilmCraft's mandatory `FILMCRAFT_CONTROL_TOKEN` on `port`.
#[must_use]
pub fn filmcraft_token_for(port: u16) -> String {
    if let Ok(env_tok) = std::env::var("FILMCRAFT_CONTROL_TOKEN") {
        let t = env_tok.trim();
        if t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit()) {
            return t.to_string();
        }
    }
    let path = std::env::temp_dir().join(format!("workcraft-filmcraft-{port}.token"));
    ensure_hex64_token_file(&path)
}

/// The port from a `control-key.local-<port>` file in the app's settings folder, if any.
fn key_file_port(name: &str) -> Option<u16> {
    let dirs: Vec<PathBuf> = if cfg!(windows) {
        ["APPDATA", "LOCALAPPDATA"]
            .iter()
            .filter_map(|v| std::env::var_os(v).map(|b| PathBuf::from(b).join(name)))
            .collect()
    } else if cfg!(target_os = "macos") {
        vec![
            workcraft_core::paths::home_dir()
                .join("Library/Application Support")
                .join(name),
        ]
    } else {
        vec![
            workcraft_core::paths::home_dir()
                .join(".config")
                .join(name.to_lowercase()),
        ]
    };
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.filter_map(Result::ok) {
            let n = e.file_name().to_string_lossy().into_owned();
            if let Some(port) = n
                .strip_prefix("control-key.local-")
                .and_then(|p| p.parse::<u16>().ok())
                && is_port_listening(port)
            {
                return Some(port);
            }
        }
    }
    None
}

/// Check whether any required control token file on disk is ready for `spec` on `port`.
fn auth_ready_on_disk(spec: &AppSpec, port: u16) -> bool {
    match spec.auth {
        LiveAuth::None | LiveAuth::MandatoryEnvHex64 { .. } => true,
        LiveAuth::ControlKeyFile => key_file_port(spec.name) == Some(port),
        LiveAuth::ExplicitTokenFile => photocraft_token_file(port).is_file(),
        LiveAuth::TempTokenFile { env_var, .. } => {
            std::env::var_os(env_var).is_some_and(|v| v.len() >= 16)
                || std::env::temp_dir()
                    .join(format!("{}-control-{port}.token", spec.id))
                    .is_file()
        }
        LiveAuth::HomeDotTokenFile => workcraft_core::paths::home_dir()
            .join(format!(".{}", spec.id))
            .join(format!("control-{port}.token"))
            .is_file(),
        LiveAuth::LocalAppDataTokenFile => {
            if cfg!(windows) {
                std::env::var_os("LOCALAPPDATA").is_some_and(|base| {
                    PathBuf::from(base)
                        .join(format!("{}-control-{port}.token", spec.id))
                        .is_file()
                })
            } else {
                true
            }
        }
    }
}

/// The active control port of a running Craft desktop app.
#[must_use]
pub fn live_port_for(id: &str, name: &str) -> Option<u16> {
    let spec = find_spec(id)?;
    if !spec.supports_live() {
        return None;
    }
    if matches!(spec.auth, LiveAuth::ControlKeyFile) {
        return key_file_port(name);
    }
    let port = spec.port?;
    if is_port_listening(port) && auth_ready_on_disk(spec, port) {
        Some(port)
    } else {
        None
    }
}

/// Live MCP CLI arguments for connecting `<id>-cli` to a running desktop window on `port`.
#[must_use]
pub fn live_mcp_args(id: &str, port: u16) -> Vec<String> {
    let Some(spec) = find_spec(id) else {
        return vec![
            "mcp".into(),
            "--connect".into(),
            format!("127.0.0.1:{port}"),
        ];
    };
    match spec.live_bridge {
        LiveBridgeKind::None => headless_mcp_args(spec, &RootsContext::default()),
        LiveBridgeKind::ConnectBarePort => {
            vec!["mcp".into(), "--connect".into(), port.to_string()]
        }
        LiveBridgeKind::ConnectHostPort => {
            vec![
                "mcp".into(),
                "--connect".into(),
                format!("127.0.0.1:{port}"),
            ]
        }
        LiveBridgeKind::ConnectHostPortCompact => vec![
            "mcp".into(),
            "--connect".into(),
            format!("127.0.0.1:{port}"),
            "--compact".into(),
        ],
        LiveBridgeKind::BridgeBarePort => {
            vec!["mcp".into(), "--bridge".into(), port.to_string()]
        }
        LiveBridgeKind::BridgeHostPort => {
            vec!["mcp".into(), "--bridge".into(), format!("127.0.0.1:{port}")]
        }
        LiveBridgeKind::BridgeHostPortTokenFile => {
            let tf = photocraft_token_file(port);
            let _ = ensure_hex64_token_file(&tf);
            vec![
                "mcp".into(),
                "--bridge".into(),
                format!("127.0.0.1:{port}"),
                "--control-token-file".into(),
                tf.to_string_lossy().into_owned(),
            ]
        }
        LiveBridgeKind::AttachBarePort => {
            vec!["mcp".into(), "--attach".into(), port.to_string()]
        }
    }
}

/// Headless MCP CLI arguments for `spec` with safe-by-default workspace roots (D9).
#[must_use]
pub fn headless_mcp_args(spec: &AppSpec, roots: &RootsContext) -> Vec<String> {
    if matches!(spec.roots, RootsPolicy::CleanRoomInbox) {
        let mut a = Vec::new();
        if roots.craft_anything_deep {
            a.push("--deep".to_string());
        } else {
            a.push("--clean-room".to_string());
        }
        if roots.craft_anything_allow_run {
            a.push("--allow-run".to_string());
        }
        if let Some(ws) = &roots.workspace {
            let proj = ws.join(".workcraft").join("craft-anything.cany");
            a.push("--project".to_string());
            a.push(proj.to_string_lossy().into_owned());
        }
        a.push("mcp".to_string());
        let inbox = roots.inbox_dir();
        let _ = std::fs::create_dir_all(&inbox);
        a.push("--inbox".to_string());
        a.push(inbox.to_string_lossy().into_owned());
        return a;
    }

    let mut a = vec!["mcp".to_string()];
    a.extend(spec.headless_extra_args.iter().map(|s| (*s).to_string()));

    let Some(ws) = &roots.workspace else {
        return a;
    };
    let _ = std::fs::create_dir_all(ws);
    let ws_str = ws.to_string_lossy().into_owned();

    match spec.roots {
        RootsPolicy::None | RootsPolicy::CleanRoomInbox => {}
        RootsPolicy::SingleAutomation => {
            a.push("--automation-read-root".into());
            a.push(ws_str.clone());
            a.push("--automation-write-root".into());
            a.push(ws_str);
        }
        RootsPolicy::RepeatableAutomation => {
            a.push("--automation-read-root".into());
            a.push(ws_str.clone());
            for extra in &roots.extra_roots {
                if extra.is_dir() {
                    a.push("--automation-read-root".into());
                    a.push(extra.to_string_lossy().into_owned());
                }
            }
            a.push("--automation-write-root".into());
            a.push(ws_str);
        }
        RootsPolicy::RelativeReadWrite => {
            a.push("--read-root".into());
            a.push(ws_str.clone());
            a.push("--write-root".into());
            a.push(ws_str);
        }
        RootsPolicy::AllowPath => {
            a.push("--allow-path".into());
            a.push(ws_str);
            for extra in &roots.extra_roots {
                if extra.is_dir() {
                    a.push("--allow-path".into());
                    a.push(extra.to_string_lossy().into_owned());
                }
            }
        }
    }
    a
}

/// Build the `ServerEntry` for `found` according to `mode` and `roots`.
#[must_use]
pub fn server_for(found: &Found, mode: CraftMode, roots: &RootsContext) -> ServerEntry {
    let spec = find_spec(found.id);
    let mut env = BTreeMap::new();

    let args = match (mode, found.live_port, spec) {
        (CraftMode::Live, Some(port), Some(sp)) if sp.supports_live() => {
            if let LiveAuth::MandatoryEnvHex64 { env_var } = sp.auth {
                env.insert(env_var.to_string(), filmcraft_token_for(port));
            }
            live_mcp_args(found.id, port)
        }
        (_, _, Some(sp)) => headless_mcp_args(sp, roots),
        _ => vec!["mcp".to_string()],
    };

    if std::env::var_os("WORKCRAFT_LIVE_APPS").is_some()
        && let Some(sp) = spec
    {
        if let LiveAuth::TempTokenFile { no_prefs_env, .. } = sp.auth {
            env.insert(no_prefs_env.to_string(), "1".to_string());
        }
        if sp.id == "craft-anything"
            && let Some(dd) = &roots.data_dir
        {
            env.insert(
                "CRAFT_ANYTHING_CONFIG_DIR".to_string(),
                dd.join("craft-anything-config")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }

    ServerEntry {
        command: Some(found.cli.to_string_lossy().into_owned()),
        args,
        env,
        ..ServerEntry::default()
    }
}

/// Backwards-compatible `ServerEntry` helper (uses live port if `found.live_port` is set, else headless).
#[must_use]
pub fn server(f: &Found) -> ServerEntry {
    let mode = if f.live_port.is_some() {
        CraftMode::Live
    } else {
        CraftMode::Headless
    };
    server_for(f, mode, &RootsContext::default())
}

/// Track PIDs of desktop windows launched by WorkCraft in this process so live tests can clean up
/// exclusively their own spawned processes.
fn spawned_pids() -> &'static Mutex<BTreeMap<String, u32>> {
    static PIDS: OnceLock<Mutex<BTreeMap<String, u32>>> = OnceLock::new();
    PIDS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

#[must_use]
pub fn spawned_gui_pid(app_id: &str) -> Option<u32> {
    spawned_pids()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(app_id)
        .copied()
}

pub fn take_spawned_gui_pid(app_id: &str) -> Option<u32> {
    spawned_pids()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(app_id)
}

/// Ensure a Craft app is connected in `Headless` mode in `hub` (never opens a desktop window).
pub async fn ensure_headless(
    raw_id: &str,
    roots: &RootsContext,
    hub: &McpHub,
) -> std::result::Result<String, String> {
    if std::env::var_os("WORKCRAFT_NO_CRAFT_DISCOVERY").is_some_and(|v| !v.is_empty()) {
        return Err("Craft app discovery is disabled in test mode".into());
    }
    let spec = find_spec(raw_id).ok_or_else(|| format!("unknown Craft app '{raw_id}'"))?;
    let (found_list, errs) = discover_with_errors();
    if let Some(err) = errs.get(spec.id) {
        return Err(err.clone());
    }
    let found = found_list
        .into_iter()
        .find(|f| f.id == spec.id)
        .ok_or_else(|| {
            format!(
                "{} is not installed (checked Program Files, ArtCraft/{}, and {})",
                spec.name, spec.name, spec.env_override
            )
        })?;

    let headless_found = Found {
        live_port: None,
        ..found
    };
    let entry = server_for(&headless_found, CraftMode::Headless, roots);
    let current = hub.client_args(spec.id).await;
    if !hub.is_connected(spec.id).await || current.as_deref() != Some(entry.args.as_slice()) {
        let count = hub
            .connect_one(spec.id, &entry)
            .await
            .map_err(|e| format!("could not start {} headless MCP: {e}", spec.name))?;
        return Ok(format!(
            "{} connected in headless mode ({count} tools)",
            spec.name
        ));
    }
    Ok(format!("{} is ready in headless mode", spec.name))
}

/// Ensure the Craft desktop app is open on screen and (if it supports live MCP) connected in `hub`
/// in live mode so tool calls visibly drive the open desktop window.
pub async fn ensure_live_with_roots(
    raw_id: &str,
    open_file: Option<&Path>,
    roots: &RootsContext,
    hub: &McpHub,
) -> std::result::Result<String, String> {
    if std::env::var_os("WORKCRAFT_NO_CRAFT_DISCOVERY").is_some_and(|v| !v.is_empty()) {
        return Err("Craft app discovery is disabled in test mode".into());
    }
    verify_unique_ports()?;
    let spec = find_spec(raw_id).ok_or_else(|| format!("unknown Craft app '{raw_id}'"))?;
    let (found_list, errs) = discover_with_errors();
    if let Some(err) = errs.get(spec.id) {
        return Err(err.clone());
    }
    let found_opt = found_list.into_iter().find(|f| f.id == spec.id);
    let cli = found_opt.as_ref().map(|f| f.cli.clone());
    let gui = found_opt
        .and_then(|f| f.gui)
        .or_else(|| find_gui_for_spec(spec, cli.as_deref(), &default_install_bases()));

    if !spec.supports_live() {
        let Some(gui_path) = gui else {
            return Err(format!("{} is not installed on this computer", spec.name));
        };
        let mut cmd = std::process::Command::new(&gui_path);
        if let Some(f) = open_file {
            cmd.arg(f);
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let child = cmd.spawn().map_err(|e| {
            format!(
                "could not launch {} ({}): {e}",
                spec.name,
                gui_path.display()
            )
        })?;
        spawned_pids()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(spec.id.to_string(), child.id());

        if let Some(cli_path) = cli
            && !hub.is_connected(spec.id).await
        {
            let f = Found {
                id: spec.id,
                name: spec.name,
                kind: spec.kind,
                cli: cli_path,
                gui: Some(gui_path),
                version: None,
                live_port: None,
            };
            let _ = hub
                .connect_one(spec.id, &server_for(&f, CraftMode::Headless, roots))
                .await;
        }
        return Ok(match open_file {
            Some(f) => format!("Opened {} with {}", spec.name, f.display()),
            None => format!("Opened {} (MCP stays headless)", spec.name),
        });
    }

    let target_port = spec.port.unwrap_or(7981);
    let mut active_port = live_port_for(spec.id, spec.name);
    let mut launched_window = false;

    if active_port.is_none() {
        let Some(gui_path) = gui.clone() else {
            return Err(format!("{} desktop app is not installed", spec.name));
        };
        let mut cmd = std::process::Command::new(&gui_path);
        cmd.args(["--control", &target_port.to_string()]);

        match spec.auth {
            LiveAuth::ExplicitTokenFile => {
                let tf = photocraft_token_file(target_port);
                let _ = ensure_hex64_token_file(&tf);
                cmd.arg("--control-token-file").arg(&tf);
                if let Some(ws) = &roots.workspace {
                    let _ = std::fs::create_dir_all(ws);
                    cmd.arg("--automation-read-root")
                        .arg(ws)
                        .arg("--automation-write-root")
                        .arg(ws);
                }
            }
            LiveAuth::MandatoryEnvHex64 { env_var } => {
                cmd.env(env_var, filmcraft_token_for(target_port));
            }
            LiveAuth::TempTokenFile { no_prefs_env, .. }
                if std::env::var_os("WORKCRAFT_LIVE_APPS").is_some() =>
            {
                cmd.env(no_prefs_env, "1");
            }
            _ => {}
        }

        if spec.id == "figcraft"
            && let Some(ws) = &roots.workspace
        {
            let _ = std::fs::create_dir_all(ws);
            cmd.arg("--automation-read-root")
                .arg(ws)
                .arg("--automation-write-root")
                .arg(ws);
        }

        if let Some(f) = open_file {
            cmd.arg(f);
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let child = cmd.spawn().map_err(|e| {
            format!(
                "could not start {} ({}): {e}",
                spec.name,
                gui_path.display()
            )
        })?;
        spawned_pids()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(spec.id.to_string(), child.id());
        launched_window = true;

        // Wait up to 8 seconds for the GUI window to initialize its control listener and token file.
        for _ in 0..80 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Some(p) = live_port_for(spec.id, spec.name) {
                active_port = Some(p);
                break;
            }
        }
    }

    let Some(port) = active_port else {
        return Err(format!(
            "started {}, but its control port {target_port} did not become ready in time",
            spec.name
        ));
    };

    let Some(cli_path) = cli else {
        return Ok(format!("Opened {} on port {port}", spec.name));
    };

    let found = Found {
        id: spec.id,
        name: spec.name,
        kind: spec.kind,
        cli: cli_path,
        gui,
        version: None,
        live_port: Some(port),
    };
    let entry = server_for(&found, CraftMode::Live, roots);
    let current_args = hub.client_args(spec.id).await;
    let needs_reconnect = launched_window
        || !hub.is_connected(spec.id).await
        || current_args.as_deref() != Some(entry.args.as_slice());

    if needs_reconnect {
        hub.connect_one(spec.id, &entry)
            .await
            .map_err(|e| format!("could not bridge {} MCP on port {port}: {e}", spec.name))?;
    }

    Ok(match open_file {
        Some(f) => format!(
            "{} is open and live on port {port} ({})",
            spec.name,
            f.display()
        ),
        None => format!("{} is open and live on port {port}", spec.name),
    })
}

/// Backwards-compatible `ensure_live` wrapper.
pub async fn ensure_live(
    raw_id: &str,
    open_file: Option<&Path>,
    hub: &McpHub,
) -> std::result::Result<String, String> {
    ensure_live_with_roots(raw_id, open_file, &RootsContext::default(), hub).await
}
