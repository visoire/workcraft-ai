//! Pairing with IDEs and agent apps: write a `workcraft` entry into their MCP settings.
//!
//! `workcraft-cli mcp setup` finds the apps that are installed (their settings folder exists), adds a
//! `workcraft` server that starts `workcraft-cli mcp`, and tells what it did. It edits a file in place: only the
//! `workcraft` entry changes, the rest of the file keeps its order, the file is copied to `<name>.bak` once,
//! a file that does not parse is left alone, and an entry the person changed is never overwritten.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// The name of the server in other apps' settings.
pub const SERVER_NAME: &str = "workcraft";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Mac,
    Linux,
}

/// Where to look. A value instead of globals, so tests can use a temp folder.
#[derive(Debug, Clone)]
pub struct Env {
    pub home: PathBuf,
    pub appdata: Option<PathBuf>,
    pub os: Os,
}

impl Env {
    #[must_use]
    pub fn detect() -> Self {
        Self {
            home: workcraft_core::paths::home_dir(),
            appdata: std::env::var_os("APPDATA").map(PathBuf::from),
            os: if cfg!(windows) {
                Os::Windows
            } else if cfg!(target_os = "macos") {
                Os::Mac
            } else {
                Os::Linux
            },
        }
    }
}

/// How an app writes its MCP servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// `{"mcpServers": {"name": {"command": "...", "args": [...]}}}` (Claude, Cursor, Windsurf, Gemini, Antigravity).
    McpServers,
    /// `{"mcp": {"name": {"type": "local", "command": ["...", "mcp"], "enabled": true}}}`.
    OpenCode,
    /// `{"servers": {"name": {"type": "stdio", "command": "...", "args": [...]}}}`.
    VsCode,
    /// No known settings file: print the entry to paste.
    Manual,
}

#[derive(Debug, Clone)]
pub struct Client {
    pub id: &'static str,
    pub label: &'static str,
    pub shape: Shape,
    /// The settings file to edit (or that would be created).
    pub file: Option<PathBuf>,
    /// The app is installed: the folder its settings live in exists.
    pub installed: bool,
    /// Why the file cannot be edited by this tool, when it cannot.
    pub note: Option<String>,
}

/// Every app this tool knows, with its settings file on this computer.
#[must_use]
pub fn clients(env: &Env) -> Vec<Client> {
    let h = &env.home;
    let dir_exists = |p: &Path| p.is_dir();
    let mk = |id, label, shape, file: PathBuf, folder: PathBuf| Client {
        id,
        label,
        shape,
        installed: dir_exists(&folder),
        file: Some(file),
        note: None,
    };
    let claude_desktop_dir = match env.os {
        Os::Windows => env
            .appdata
            .clone()
            .unwrap_or_else(|| h.join("AppData").join("Roaming"))
            .join("Claude"),
        Os::Mac => h.join("Library").join("Application Support").join("Claude"),
        Os::Linux => h.join(".config").join("Claude"),
    };
    let vscode_user = match env.os {
        Os::Windows => env
            .appdata
            .clone()
            .unwrap_or_else(|| h.join("AppData").join("Roaming"))
            .join("Code")
            .join("User"),
        Os::Mac => h
            .join("Library")
            .join("Application Support")
            .join("Code")
            .join("User"),
        Os::Linux => h.join(".config").join("Code").join("User"),
    };
    let opencode_dir = h.join(".config").join("opencode");
    let mut opencode = mk(
        "opencode",
        "OpenCode",
        Shape::OpenCode,
        opencode_dir.join("opencode.json"),
        opencode_dir.clone(),
    );
    if opencode_dir.join("opencode.jsonc").is_file()
        && !opencode_dir.join("opencode.json").is_file()
    {
        // JSON with comments: rewriting it would lose them.
        opencode.shape = Shape::Manual;
        opencode.note =
            Some("settings are in opencode.jsonc (with comments); add the entry by hand".into());
    }
    vec![
        Client {
            id: "claude-code",
            label: "Claude Code",
            shape: Shape::McpServers,
            installed: h.join(".claude.json").is_file() || h.join(".claude").is_dir(),
            file: Some(h.join(".claude.json")),
            note: None,
        },
        mk(
            "claude-desktop",
            "Claude Desktop",
            Shape::McpServers,
            claude_desktop_dir.join("claude_desktop_config.json"),
            claude_desktop_dir.clone(),
        ),
        mk(
            "cursor",
            "Cursor",
            Shape::McpServers,
            h.join(".cursor").join("mcp.json"),
            h.join(".cursor"),
        ),
        mk(
            "windsurf",
            "Windsurf",
            Shape::McpServers,
            h.join(".codeium").join("windsurf").join("mcp_config.json"),
            h.join(".codeium").join("windsurf"),
        ),
        mk(
            "antigravity",
            "Antigravity",
            Shape::McpServers,
            h.join(".gemini").join("config").join("mcp_config.json"),
            h.join(".gemini").join("config"),
        ),
        Client {
            id: "gemini",
            label: "Gemini CLI",
            shape: Shape::McpServers,
            installed: h.join(".gemini").join("settings.json").is_file(),
            file: Some(h.join(".gemini").join("settings.json")),
            note: None,
        },
        opencode,
        mk(
            "vscode",
            "VS Code",
            Shape::VsCode,
            vscode_user.join("mcp.json"),
            vscode_user.clone(),
        ),
        Client {
            id: "trae",
            label: "Trae",
            shape: Shape::Manual,
            installed: false,
            file: None,
            note: Some(
                "add it in Trae under Settings, MCP, Add manually, and paste the entry below"
                    .into(),
            ),
        },
    ]
}

/// The entry for `exe` in the shape `shape`, as a JSON value.
#[must_use]
pub fn entry(shape: Shape, exe: &Path) -> Value {
    let exe = exe.to_string_lossy();
    match shape {
        Shape::McpServers | Shape::Manual => json!({ "command": exe, "args": ["mcp"] }),
        Shape::OpenCode => json!({ "type": "local", "command": [exe, "mcp"], "enabled": true }),
        Shape::VsCode => json!({ "type": "stdio", "command": exe, "args": ["mcp"] }),
    }
}

fn container_key(shape: Shape) -> &'static str {
    match shape {
        Shape::McpServers | Shape::Manual => "mcpServers",
        Shape::OpenCode => "mcp",
        Shape::VsCode => "servers",
    }
}

/// A complete snippet for an app, to paste by hand.
#[must_use]
pub fn snippet(shape: Shape, exe: &Path) -> String {
    let mut inner = Map::new();
    inner.insert(SERVER_NAME.to_string(), entry(shape, exe));
    let mut root = Map::new();
    root.insert(container_key(shape).to_string(), Value::Object(inner));
    serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added,
    /// Our own entry, pointing at an older location of the program.
    Refreshed,
    Unchanged,
    /// An entry named `workcraft` that is not ours (other program or arguments): left as it is.
    LeftAlone,
}

/// Whether `existing` is an entry this tool wrote (it starts `workcraft-cli mcp`).
fn is_ours(existing: &Value) -> bool {
    let (cmd, args): (Option<String>, Vec<String>) = match existing.get("command") {
        Some(Value::String(c)) => (
            Some(c.clone()),
            existing
                .get("args")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        ),
        Some(Value::Array(a)) => {
            let all: Vec<String> = a
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect();
            (all.first().cloned(), all.into_iter().skip(1).collect())
        }
        _ => (None, Vec::new()),
    };
    let Some(cmd) = cmd else { return false };
    let base = Path::new(&cmd)
        .file_name()
        .map(|f| f.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    (base == "workcraft-cli" || base == "workcraft-cli.exe") && args == ["mcp"]
}

/// Add or refresh the `workcraft` entry in the settings text `text` (`None`: no file yet).
/// Returns the new text and what changed; an unreadable file is an error and is not touched.
pub fn merge(text: Option<&str>, shape: Shape, exe: &Path) -> Result<(String, Change), String> {
    let mut root: Value = match text.map(str::trim) {
        None | Some("") => Value::Object(Map::new()),
        Some(t) => serde_json::from_str(t)
            .map_err(|e| format!("the file is not valid JSON ({e}); not touching it"))?,
    };
    let Some(obj) = root.as_object_mut() else {
        return Err("the file does not hold a JSON object; not touching it".into());
    };
    let key = container_key(shape);
    let servers = obj
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(servers) = servers.as_object_mut() else {
        return Err(format!("\"{key}\" is not an object; not touching it"));
    };
    let new = entry(shape, exe);
    let change = match servers.get(SERVER_NAME) {
        None => Change::Added,
        Some(old) if *old == new => Change::Unchanged,
        Some(old) if is_ours(old) => Change::Refreshed,
        Some(_) => Change::LeftAlone,
    };
    if matches!(change, Change::Added | Change::Refreshed) {
        servers.insert(SERVER_NAME.to_string(), new);
    }
    let out = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    Ok((out + "\n", change))
}

/// What `setup` did for one app.
#[derive(Debug)]
pub struct Outcome {
    pub client: &'static str,
    pub file: Option<PathBuf>,
    pub result: Result<Change, String>,
    /// Text to paste, for apps this tool cannot edit.
    pub snippet: Option<String>,
}

/// Pair with `selected` apps (`None`: every installed one). With `dry_run` nothing is written.
#[must_use]
pub fn setup(env: &Env, exe: &Path, selected: Option<&[String]>, dry_run: bool) -> Vec<Outcome> {
    let mut out = Vec::new();
    for c in clients(env) {
        let wanted = match selected {
            Some(ids) => ids.iter().any(|i| i == c.id),
            None => c.installed && c.shape != Shape::Manual,
        };
        if !wanted {
            continue;
        }
        if c.shape == Shape::Manual {
            out.push(Outcome {
                client: c.id,
                file: c.file.clone(),
                result: Err(c
                    .note
                    .clone()
                    .unwrap_or_else(|| "paste the entry below".into())),
                snippet: Some(snippet(Shape::McpServers, exe)),
            });
            continue;
        }
        let Some(file) = c.file.clone() else { continue };
        let existing = std::fs::read_to_string(&file).ok();
        let result = match merge(existing.as_deref(), c.shape, exe) {
            Err(e) => Err(e),
            Ok((text, change)) => {
                if matches!(change, Change::Added | Change::Refreshed) && !dry_run {
                    write_with_backup(&file, existing.is_some(), &text).map(|()| change)
                } else {
                    Ok(change)
                }
            }
        };
        out.push(Outcome {
            client: c.id,
            file: Some(file),
            result,
            snippet: None,
        });
    }
    out
}

fn write_with_backup(file: &Path, existed: bool, text: &str) -> Result<(), String> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    if existed {
        let bak = PathBuf::from(format!("{}.bak", file.display()));
        if !bak.exists() {
            std::fs::copy(file, &bak).map_err(|e| format!("cannot back up the file: {e}"))?;
        }
    }
    let tmp = PathBuf::from(format!("{}.workcraft-tmp", file.display()));
    std::fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, file).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot replace {}: {e}", file.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe() -> PathBuf {
        PathBuf::from(if cfg!(windows) {
            r"C:\Tools\WorkCraft-CLI.exe"
        } else {
            "/opt/workcraft/workcraft-cli"
        })
    }

    fn temp_env() -> (Env, PathBuf) {
        let dir = std::env::temp_dir().join(format!("wc-ide-{}", workcraft_core::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        (
            Env {
                home: dir.clone(),
                appdata: Some(dir.join("AppData")),
                os: Os::Linux,
            },
            dir,
        )
    }

    #[test]
    fn a_new_file_gets_the_entry_and_other_servers_keep_their_order() {
        let existing = r#"{"theme":"dark","mcpServers":{"zeta":{"command":"z"},"alpha":{"command":"a"}},"last":1}"#;
        let (text, change) = merge(Some(existing), Shape::McpServers, &exe()).unwrap();
        assert_eq!(change, Change::Added);
        let v: Value = serde_json::from_str(&text).unwrap();
        let keys: Vec<&String> = v["mcpServers"].as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["zeta", "alpha", "workcraft"],
            "insertion order is kept"
        );
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["last"], 1);
        assert_eq!(v["mcpServers"]["workcraft"]["args"], json!(["mcp"]));
        let (empty, c2) = merge(None, Shape::VsCode, &exe()).unwrap();
        assert_eq!(c2, Change::Added);
        assert_eq!(
            serde_json::from_str::<Value>(&empty).unwrap()["servers"]["workcraft"]["type"],
            "stdio"
        );
    }

    #[test]
    fn running_again_changes_nothing_a_moved_program_is_refreshed_and_edits_are_respected() {
        let (first, _) = merge(None, Shape::McpServers, &exe()).unwrap();
        let (_, again) = merge(Some(&first), Shape::McpServers, &exe()).unwrap();
        assert_eq!(again, Change::Unchanged);
        let moved = PathBuf::from(if cfg!(windows) {
            r"D:\Apps\WorkCraft-CLI.exe"
        } else {
            "/usr/local/bin/workcraft-cli"
        });
        let (text, c) = merge(Some(&first), Shape::McpServers, &moved).unwrap();
        assert_eq!(c, Change::Refreshed);
        assert!(text.contains(moved.to_string_lossy().replace('\\', "\\\\").as_str()));
        // The person added an argument: that entry is theirs now.
        let edited =
            r#"{"mcpServers":{"workcraft":{"command":"workcraft-cli","args":["mcp","--no-mcp"]}}}"#;
        let (kept, c) = merge(Some(edited), Shape::McpServers, &exe()).unwrap();
        assert_eq!(c, Change::LeftAlone);
        assert!(kept.contains("--no-mcp"));
        // Another program under the same name is not ours either.
        let other = r#"{"mcpServers":{"workcraft":{"command":"node","args":["server.js"]}}}"#;
        assert_eq!(
            merge(Some(other), Shape::McpServers, &exe()).unwrap().1,
            Change::LeftAlone
        );
    }

    #[test]
    fn opencode_uses_its_own_shape_and_is_recognised_as_ours() {
        let (text, _) = merge(None, Shape::OpenCode, &exe()).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["mcp"]["workcraft"]["type"], "local");
        assert_eq!(v["mcp"]["workcraft"]["command"][1], "mcp");
        let (_, c) = merge(Some(&text), Shape::OpenCode, &exe()).unwrap();
        assert_eq!(c, Change::Unchanged);
    }

    #[test]
    fn broken_files_are_never_touched() {
        for bad in ["{not json", "[1,2]", r#"{"mcpServers": 5}"#] {
            assert!(
                merge(Some(bad), Shape::McpServers, &exe()).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn setup_finds_installed_apps_backs_up_once_and_dry_run_writes_nothing() {
        let (env, dir) = temp_env();
        std::fs::create_dir_all(dir.join(".cursor")).unwrap();
        std::fs::write(
            dir.join(".cursor").join("mcp.json"),
            r#"{"mcpServers":{"other":{"command":"x"}}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.join(".codeium").join("windsurf")).unwrap();

        let dry = setup(&env, &exe(), None, true);
        let ids: Vec<&str> = dry.iter().map(|o| o.client).collect();
        assert_eq!(
            ids,
            ["cursor", "windsurf"],
            "only apps whose settings folder exists"
        );
        assert!(
            !dir.join(".codeium")
                .join("windsurf")
                .join("mcp_config.json")
                .exists(),
            "dry run writes nothing"
        );

        let done = setup(&env, &exe(), None, false);
        assert!(
            done.iter().all(|o| o.result == Ok(Change::Added)),
            "{done:?}"
        );
        let cursor = std::fs::read_to_string(dir.join(".cursor").join("mcp.json")).unwrap();
        assert!(cursor.contains("workcraft") && cursor.contains("other"));
        assert!(
            dir.join(".cursor").join("mcp.json.bak").exists(),
            "the old file was kept"
        );
        assert!(
            dir.join(".codeium")
                .join("windsurf")
                .join("mcp_config.json")
                .exists()
        );
        // Second run: nothing to do, the backup is not overwritten with the new file.
        let again = setup(&env, &exe(), None, false);
        assert!(again.iter().all(|o| o.result == Ok(Change::Unchanged)));
        let bak = std::fs::read_to_string(dir.join(".cursor").join("mcp.json.bak")).unwrap();
        assert!(!bak.contains("workcraft"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn named_clients_are_set_up_even_when_not_detected_and_manual_ones_print_a_snippet() {
        let (env, dir) = temp_env();
        let out = setup(
            &env,
            &exe(),
            Some(&["claude-code".to_string(), "trae".to_string()]),
            false,
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].result, Ok(Change::Added));
        assert!(dir.join(".claude.json").exists());
        assert!(
            out[1].result.is_err() && out[1].snippet.as_deref().unwrap().contains("mcpServers")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn opencode_with_comments_is_left_for_the_person() {
        let (env, dir) = temp_env();
        let d = dir.join(".config").join("opencode");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("opencode.jsonc"), "{ // my settings\n}").unwrap();
        let c = clients(&env)
            .into_iter()
            .find(|c| c.id == "opencode")
            .unwrap();
        assert_eq!(c.shape, Shape::Manual);
        assert!(c.installed && c.note.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }
}
