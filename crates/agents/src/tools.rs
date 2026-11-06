//! The tools an agent can call: files and documents anywhere on this computer, memory, skills, every
//! MCP tool as `<server>__<tool>` (the Craft apps among them), the uniform `craft_*` verb facade, and
//! the control channels of running Craft apps.
//!
//! Paths: relative paths are inside the workspace folder; absolute paths and `~/...` reach the rest of
//! the computer. Reading outside the workspace and every write go through approval in manual mode, and
//! every write is recorded first so the answer can be undone (see [`crate::checkpoints`]).

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};
use workcraft_control::client::ControlClient;
use workcraft_core::{Error, ImagePart, Result, ToolResult, ToolSpec, truncate_bytes};
use workcraft_mcp::McpHub;

use crate::checkpoints::Checkpoints;
use crate::config::{Config, ControlTarget};
use crate::craft::{
    AppSpec, ApprovalLevel, CraftMode, RootsContext, RootsPolicy, VerbTarget, classify_tool_call,
    ensure_headless, ensure_live_with_roots, execute_handoff, filter_app_tools, find_spec,
    live_port_for, mode_for, quirks,
};
use crate::docs;
use crate::skills::Skills;

const MAX_TEXT_BYTES: u64 = 8 * 1024 * 1024;
/// Characters `read_file` returns per call; `offset` reads on.
const READ_WINDOW: usize = 40_000;
const MAX_LIST: usize = 500;
/// Keys on the shared memory board. It is for facts other agents need, not a log.
const MAX_MEMORY_KEYS: usize = 12;

/// Tools that read a path the agent names.
const READ_TOOLS: [&str; 4] = ["read_file", "list_dir", "search_files", "file_info"];

/// Single-path argument names across the 17 Craft MCP servers.
const SINGLE_PATH_ARGS: [&str; 17] = [
    "path",
    "file",
    "file_path",
    "output",
    "out",
    "target",
    "dest",
    "destination",
    "save_path",
    "in",
    "input",
    "source",
    "src",
    "dir",
    "folder",
    "out_dir",
    "output_dir",
];

/// Array-of-paths argument names across the 17 Craft MCP servers (`filmcraft__media_import`, `lightcraft__cmd_library_import`, `pdfcraft` merge).
const MULTI_PATH_ARGS: [&str; 3] = ["paths", "files", "inputs"];

/// Argument keys that represent directories rather than files (allowing `.` in relative-root servers).
const DIR_ARGS: [&str; 4] = ["dir", "folder", "out_dir", "output_dir"];

/// A key/value board shared by the agents of one run.
#[derive(Debug, Default)]
pub struct Memory(Mutex<BTreeMap<String, String>>);

impl Memory {
    fn with<T>(&self, f: impl FnOnce(&mut BTreeMap<String, String>) -> T) -> T {
        let mut g = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut g)
    }
}

/// Who is calling.
#[derive(Clone)]
pub struct ToolCtx {
    pub session: String,
    pub agent: Option<String>,
    pub memory: Arc<Memory>,
    /// The user message being answered: changes are grouped under it for undo.
    pub turn: String,
}

pub struct Tools {
    pub workspace: PathBuf,
    pub extra_roots: Vec<PathBuf>,
    pub skills: Arc<Skills>,
    pub hub: McpHub,
    pub controls: BTreeMap<String, ControlTarget>,
    /// The engine's data folder: its key file is never readable by a tool.
    pub data_dir: PathBuf,
    pub checkpoints: Arc<Checkpoints>,
    pub craft_apps: BTreeMap<String, bool>,
    pub craft_modes: BTreeMap<String, CraftMode>,
    pub full_tool_catalog: bool,
    pub craft_anything_deep: bool,
    pub craft_anything_allow_run: bool,
    pub allow_craft_on_demand: bool,
}

/// Outcome of executing a tool call with rich metadata for events and session attachments.
pub struct CallOutcome {
    pub result: ToolResult,
    pub artifacts: Vec<String>,
    pub mode: Option<String>,
}

fn obj(props: Value, req: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": req })
}

fn sp(d: &str) -> Value {
    json!({ "type": "string", "description": d })
}

fn human_size(n: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

fn modified(meta: &std::fs::Metadata) -> String {
    meta.modified()
        .ok()
        .map(|t| {
            chrono::DateTime::<chrono::Local>::from(t)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// Folders a tool must never write into, whatever the approval mode.
fn system_folder(p: &Path) -> bool {
    let s = p.to_string_lossy().to_lowercase().replace('/', "\\");
    let win = [
        "c:\\windows",
        "c:\\program files",
        "c:\\programdata\\microsoft",
    ];
    let unix = [
        "/etc", "/usr", "/bin", "/sbin", "/system", "/library", "/boot", "/dev", "/proc",
    ];
    win.iter().any(|w| s.starts_with(w))
        || unix.iter().any(|u| p.starts_with(u) && cfg!(not(windows)))
}

fn ext_for_mime(mime: &str) -> &'static str {
    let m = mime.to_ascii_lowercase();
    if m.contains("jpeg") || m.contains("jpg") {
        "jpg"
    } else if m.contains("webp") {
        "webp"
    } else if m.contains("gif") {
        "gif"
    } else if m.contains("bmp") {
        "bmp"
    } else {
        "png"
    }
}

fn sanitize_file_stem(raw: &str) -> String {
    let s: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    if s.is_empty() { "call".to_string() } else { s }
}

impl Tools {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn from_config(
        workspace: PathBuf,
        skills: Arc<Skills>,
        hub: McpHub,
        controls: BTreeMap<String, ControlTarget>,
        data_dir: PathBuf,
        checkpoints: Arc<Checkpoints>,
        cfg: &Config,
        allow_craft_on_demand: bool,
    ) -> Self {
        Self {
            workspace,
            extra_roots: cfg.extra_roots.clone(),
            skills,
            hub,
            controls,
            data_dir,
            checkpoints,
            craft_apps: cfg.craft_apps.clone(),
            craft_modes: cfg.craft_modes.clone(),
            full_tool_catalog: cfg.full_tool_catalog,
            craft_anything_deep: cfg.craft_anything_deep,
            craft_anything_allow_run: cfg.craft_anything_allow_run,
            allow_craft_on_demand,
        }
    }

    #[must_use]
    pub fn roots_context(&self) -> RootsContext {
        RootsContext {
            workspace: Some(self.workspace.clone()),
            extra_roots: self.extra_roots.clone(),
            data_dir: Some(self.data_dir.clone()),
            craft_anything_deep: self.craft_anything_deep,
            craft_anything_allow_run: self.craft_anything_allow_run,
        }
    }

    fn builtin_specs(&self) -> Vec<ToolSpec> {
        let path_d = "File path. Relative paths are in the workspace; absolute paths (C:\\Users\\..., /home/...) and ~/ reach any folder on this computer.";
        let app_d = "Craft app id or name (photocraft, vectorcraft, filmcraft, lightcraft, pdfcraft, effectcraft, designcraft, figcraft, texturecraft, materialcraft, wordcraft, soundcraft, deckcraft, cadcraft, gridcraft, solidcraft, craft-anything).";
        let mut v = vec![
            ToolSpec::new(
                "read_file",
                "Read a file anywhere on this computer. Text files come back as they are; PDF, Word (.docx), PowerPoint (.pptx), Excel (.xlsx, .xls, .ods) and OpenDocument files come back as their text. Long files come in windows: pass offset to read on.",
                obj(json!({ "path": sp(path_d), "offset": { "type": "integer", "description": "Character to start at (default 0)" } }), &["path"]),
            ).read_only(),
            ToolSpec::new(
                "list_dir",
                "List a folder with sizes and dates. Default: the workspace. Works for any folder on this computer.",
                obj(json!({ "path": sp("Folder"), "depth": { "type": "integer", "description": "Levels to go down (1 to 4, default 1)" } }), &[]),
            ).read_only(),
            ToolSpec::new(
                "search_files",
                "Find files under a folder by name (part of the name or a pattern like *.docx) and, optionally, by text they contain.",
                obj(json!({ "path": sp("Folder to search, default the workspace"), "name": sp("Part of the file name, or a pattern with *"), "text": sp("Only files containing this text (text files and documents)") }), &[]),
            ).read_only(),
            ToolSpec::new(
                "file_info",
                "Size, dates and type of a file or folder.",
                obj(json!({ "path": sp(path_d) }), &["path"]),
            ).read_only(),
            ToolSpec::new(
                "write_file",
                "Create or overwrite a text file (txt, md, csv, json, html, code). For Word, Excel or PowerPoint files use create_document, create_spreadsheet or create_presentation.",
                obj(json!({ "path": sp(path_d), "content": sp("Full file content") }), &["path", "content"]),
            ),
            ToolSpec::new(
                "edit_file",
                "Replace text in an existing file: exact text in text files, and the text of .docx, .pptx and .xlsx files (formatting is kept). Read the file first so find matches exactly.",
                obj(json!({ "path": sp(path_d), "find": sp("Exact text to find"), "replace": sp("Replacement text"), "all": { "type": "boolean", "description": "Replace every match (default: the text must occur once)" } }), &["path", "find", "replace"]),
            ),
            ToolSpec::new(
                "copy_file",
                "Copy a file, for example to work on a copy and keep the original.",
                obj(json!({ "from": sp(path_d), "to": sp("Destination path") }), &["from", "to"]),
            ),
            ToolSpec::new(
                "create_document",
                "Create a Word document (.docx) from Markdown: # headings, paragraphs, - lists, 1. lists, > quotes, ``` code, | tables |, **bold**, *italic*. A .md or .txt path writes the text as it is. Pass open=true to also open it in WordCraft.",
                obj(json!({ "path": sp("Where to save, ending in .docx (or .md, .txt)"), "title": sp("Optional title at the top"), "content": sp("The document in Markdown"), "open": { "type": "boolean", "description": "Open the created document in the WordCraft desktop app" } }), &["path", "content"]),
            ),
            ToolSpec::new(
                "create_spreadsheet",
                "Create an Excel workbook (.xlsx) or a .csv file. Numbers stay numbers; text starting with = is a formula (=SUM(B2:B9)). Pass open=true to also open it in GridCraft.",
                obj(json!({
                    "path": sp("Where to save, ending in .xlsx or .csv"),
                    "sheets": { "type": "array", "description": "Sheets in order", "items": { "type": "object", "properties": {
                        "name": sp("Sheet name"),
                        "rows": { "type": "array", "description": "Rows of cells; the first row is the header", "items": { "type": "array", "items": {} } },
                        "header": { "type": "boolean", "description": "Bold and freeze the first row (default true)" }
                    }, "required": ["rows"] } },
                    "open": { "type": "boolean", "description": "Open the created spreadsheet in the GridCraft desktop app" }
                }), &["path", "sheets"]),
            ),
            ToolSpec::new(
                "create_presentation",
                "Create a PowerPoint deck (.pptx, 16:9): each slide has a title and bullet points. The first slide is the title slide. Pass open=true to also open it in DeckCraft.",
                obj(json!({
                    "path": sp("Where to save, ending in .pptx"),
                    "slides": { "type": "array", "items": { "type": "object", "properties": {
                        "title": sp("Slide title"),
                        "bullets": { "type": "array", "items": { "type": "string" } }
                    }, "required": ["title"] } },
                    "open": { "type": "boolean", "description": "Open the created presentation in the DeckCraft desktop app" }
                }), &["path", "slides"]),
            ),
            ToolSpec::new(
                "open_craft_app",
                "Open a Craft desktop app window on screen and connect to it live so edits happen visibly. Optionally pass path to open a file in that app.",
                obj(json!({
                    "app": sp(app_d),
                    "path": sp("Optional file path to open in the app window")
                }), &["app"]),
            ),
            // Uniform `craft_*` facade across all 17 ArtCraft apps (D5, T3.5, T5.1):
            ToolSpec::new(
                "craft_status",
                "Report installed, connected, and live status (version, mode, port, tool count, health) for one Craft app or all 17 ArtCraft apps.",
                obj(json!({ "app": sp("Optional Craft app id; omit for all 17 apps") }), &[]),
            ).read_only(),
            ToolSpec::new(
                "craft_activate",
                "Connect an installed Craft app on demand in headless (default, silent) or live (desktop window) mode so its `<app>__*` tools become active.",
                obj(json!({
                    "app": sp(app_d),
                    "mode": { "type": "string", "enum": ["headless", "live"], "description": "Connection mode (default: headless)" }
                }), &["app"]),
            ).read_only(),
            ToolSpec::new(
                "craft_open",
                "Open a file or project in any Craft app using its native open tool (headless by default).",
                obj(json!({ "app": sp(app_d), "path": sp(path_d) }), &["app", "path"]),
            ),
            ToolSpec::new(
                "craft_new",
                "Create a new document, canvas, composition, session, or 3D/CAD part in any Craft app.",
                obj(json!({
                    "app": sp(app_d),
                    "name": sp("Optional document/project title"),
                    "preset": sp("Optional page/canvas preset (e.g. A4, 1080p)"),
                    "width": { "type": "number", "description": "Optional width" },
                    "height": { "type": "number", "description": "Optional height" }
                }), &["app"]),
            ),
            ToolSpec::new(
                "craft_inspect",
                "Inspect the active document, layers, slides, sheets, timeline, tracks, graph, or 3D topology in any Craft app.",
                obj(json!({ "app": sp(app_d) }), &["app"]),
            ).read_only(),
            ToolSpec::new(
                "craft_render",
                "Render a visual PNG/JPEG preview of the active canvas, page, slide, frame, or 3D viewport in a Craft app. Omit path for an in-memory preview.",
                obj(json!({
                    "app": sp(app_d),
                    "path": sp("Optional output image path inside the workspace"),
                    "page": { "type": "integer", "description": "Optional 0-based page or slide index" },
                    "width": { "type": "integer", "description": "Optional preview width in pixels" },
                    "height": { "type": "integer", "description": "Optional preview height in pixels" }
                }), &["app"]),
            ).read_only(),
            ToolSpec::new(
                "craft_export",
                "Export the active document, artwork, sequence, mix, or 3D model from a Craft app to an output file.",
                obj(json!({
                    "app": sp(app_d),
                    "path": sp("Destination file path (or output directory for TextureCraft/MaterialCraft)"),
                    "format": sp("Optional format override (e.g. png, svg, pdf, glb, step, wav, mp4)")
                }), &["app", "path"]),
            ),
            ToolSpec::new(
                "craft_save",
                "Save the active native project/document in a Craft app.",
                obj(json!({ "app": sp(app_d), "path": sp("Optional save-as file path") }), &["app"]),
            ),
            ToolSpec::new(
                "craft_batch",
                "Execute a batch of engine commands in a Craft app in one round-trip.",
                obj(json!({
                    "app": sp(app_d),
                    "steps": { "type": "array", "description": "List of {command/id, params} objects", "items": { "type": "object" } }
                }), &["app", "steps"]),
            ),
            ToolSpec::new(
                "craft_handoff",
                "Validate interchange format compatibility between two Craft apps and stage an exported file into <workspace>/handoffs/ with the recommended receiving tool call.",
                obj(json!({
                    "from_app": sp("Source Craft app id"),
                    "to_app": sp("Target Craft app id"),
                    "file": sp("Path to the exported file to hand off"),
                    "as_format": sp("Optional expected format extension (e.g. png, svg, glb, pdf)")
                }), &["from_app", "to_app", "file"]),
            ),
            // The memory tools only touch the in-process board of the current run, so they never ask.
            ToolSpec::new("memory_get", "Read a value from the shared memory of this run.", obj(json!({ "key": sp("Key") }), &["key"])).read_only(),
            ToolSpec::new("memory_set", "Store a value in the shared memory of this run, visible to every agent.", obj(json!({ "key": sp("Key"), "value": sp("Value") }), &["key", "value"])).read_only(),
            ToolSpec::new("memory_list", "List the keys in the shared memory of this run.", obj(json!({}), &[])).read_only(),
            ToolSpec::new("list_skills", "List the available skills with their descriptions.", obj(json!({}), &[])).read_only(),
            ToolSpec::new("find_tools", "List tools of Craft apps that are not loaded in this request. Give a few words: an app, a file type or an action (for example \"svg logo\" or \"set a cell\"). The tools of the apps you find are available from the next step.", obj(json!({ "query": sp("A few words about what you need") }), &["query"])).read_only(),
            ToolSpec::new("read_skill", "Read a skill (SKILL.md), or one of its reference files. A long skill comes as its first part with the list of its sections; read one with `section`. Names are not case sensitive.", obj(json!({ "name": sp("Skill name, for example wordcraft"), "file": sp("Optional file under the skill, for example references/commands.md"), "section": sp("Optional: the title (or part of it) of one section to read, from the list a long skill ends with"), "full": { "type": "boolean", "description": "Read a long skill in full instead of its first part" } }), &["name"])).read_only(),
            ToolSpec::new("fetch_url", "Fetch a web page or URL (https://...). Text/HTML pages return readable text; images and binary files are automatically saved to the workspace and return their local file path.", obj(json!({ "url": sp("Full http:// or https:// URL") }), &["url"])).read_only(),
            ToolSpec::new(
                "download_file",
                "Download an image (.jpg, .png, .webp, .svg) or file from an http:// or https:// URL (or a Wikipedia/Wikimedia Commons file name) and save it locally so it can be inserted into DeckCraft, WordCraft, or PdfCraft. Returns the saved local file path.",
                obj(json!({
                    "url": sp("Full http:// or https:// URL, or a Wikimedia/Wikipedia File:... name"),
                    "path": sp("Optional destination file path (defaults to the workspace folder)")
                }), &["url"]),
            ),
            ToolSpec::new(
                "search_wikimedia_images",
                "Search Wikipedia / Wikimedia Commons for free images and historical photos by keyword or topic (for example 'Sarajevo 1914', 'French Revolution', 'United Nations flag') and download the best matching image to the workspace so its local path can be passed directly to insert.picture.",
                obj(json!({
                    "query": sp("Search keywords or topic"),
                    "limit": { "type": "integer", "description": "Maximum number of results (1 to 10, default 5)" },
                    "download": { "type": "boolean", "description": "Download the top matching image to the workspace and return saved_path (default true)" },
                    "path": sp("Optional destination path for the downloaded image")
                }), &["query"]),
            ).read_only(),
        ];
        if !self.controls.is_empty() || !self.hub.server_names().is_empty() {
            v.push(
                ToolSpec::new(
                    "craft_commands",
                    "List or search the command ids of a Craft app.",
                    obj(
                        json!({ "app": sp(app_d), "filter": sp("Only ids containing this text") }),
                        &["app"],
                    ),
                )
                .read_only(),
            );
            v.push(ToolSpec::new(
                "craft_run",
                "Run one engine command id in a Craft app.",
                obj(json!({ "app": sp(app_d), "command": sp("Command id, for example filter.blur.gaussianBlur"), "params": { "type": "object", "description": "Command parameters" } }), &["app", "command"]),
            ));
        }
        v
    }

    /// Built-in tools plus filtered MCP tools (lean profiles by default, `T3.1`).
    /// Connect installed Craft apps that a question needs but that are not connected yet (headless, never a
    /// window). Apps that are off, not installed or already connected are skipped.
    pub async fn connect_on_demand(&self, ids: &std::collections::BTreeSet<&'static str>) {
        if !self.allow_craft_on_demand {
            return;
        }
        let roots = self.roots_context();
        for &id in ids {
            if self.hub.is_connected(id).await
                || !crate::craft::available_on_demand(id, &self.craft_apps, &self.craft_modes)
            {
                continue;
            }
            if crate::craft::mode_for(id, &self.craft_apps, &self.craft_modes)
                == crate::craft::CraftMode::Headless
            {
                let _ = crate::craft::ensure_headless(id, &roots, &self.hub).await;
            }
        }
    }

    pub async fn specs(&self) -> Vec<ToolSpec> {
        let mut v = self.builtin_specs();
        let raw_mcp = self.hub.tools().await;
        let mut by_server: BTreeMap<String, Vec<ToolSpec>> = BTreeMap::new();
        let mut other_mcp = Vec::new();

        for mut t in raw_mcp {
            let level = classify_tool_call(&t.name, &json!({}), Some(&t));
            t.read_only = matches!(level, ApprovalLevel::Free);
            if let ApprovalLevel::AskInManual { destructive } = level {
                t.destructive |= destructive;
            }
            if let Some((srv, _)) = workcraft_mcp::hub::split_name(&t.name) {
                by_server.entry(srv.to_string()).or_default().push(t);
            } else {
                other_mcp.push(t);
            }
        }

        for (srv, list) in by_server {
            if let Some(spec) = find_spec(&srv) {
                let actual_mode = self.effective_connected_mode(spec).await;
                v.extend(filter_app_tools(
                    spec,
                    actual_mode,
                    self.full_tool_catalog,
                    list,
                ));
            } else {
                v.extend(list);
            }
        }
        v.extend(other_mcp);
        v
    }

    /// Determine whether `spec` is currently bridged live or running headless in `hub`.
    pub async fn effective_connected_mode(&self, spec: &AppSpec) -> CraftMode {
        if let Some(args) = self.hub.client_args(spec.id).await
            && args
                .iter()
                .any(|a| matches!(a.as_str(), "--connect" | "--bridge" | "--attach"))
        {
            return CraftMode::Live;
        }
        mode_for(spec.id, &self.craft_apps, &self.craft_modes)
    }

    /// Resolve a path the agent wrote. Returns the path and whether it is inside the workspace.
    pub fn resolve(&self, raw: &str) -> Result<(PathBuf, bool)> {
        let raw = raw.trim().trim_matches('"');
        if raw.is_empty() {
            return Err(Error::Invalid("empty path".into()));
        }
        let p = if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
            workcraft_core::paths::home_dir().join(rest)
        } else if raw == "~" {
            workcraft_core::paths::home_dir()
        } else {
            let p = Path::new(raw);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                self.workspace.join(p)
            }
        };
        let full = normalize(&p);
        let inside = self.is_inside_authorized_roots(&full);
        // The engine's own key file stays private whatever the approval mode.
        for secret in ["config.json", "control.json"] {
            let f = self.data_dir.join(secret);
            if canonical_or_self(&full) == canonical_or_self(&f) {
                return Err(Error::Denied(format!(
                    "{} holds WorkCraft's keys and cannot be opened by a tool",
                    f.display()
                )));
            }
        }
        Ok((full, inside))
    }

    fn is_inside_workspace(&self, full: &Path) -> bool {
        let ws = self
            .workspace
            .canonicalize()
            .unwrap_or_else(|_| self.workspace.clone());
        canonical_or_self(full).starts_with(&ws)
    }

    fn is_inside_authorized_roots(&self, full: &Path) -> bool {
        if self.is_inside_workspace(full) {
            return true;
        }
        let canon = canonical_or_self(full);
        self.extra_roots.iter().any(|r| {
            let cr = r.canonicalize().unwrap_or_else(|_| r.clone());
            canon.starts_with(&cr)
        })
    }

    /// In manual mode: does this call need the user's OK? Writes always; reads outside the workspace.
    #[must_use]
    pub fn needs_approval(&self, name: &str, args: &Value, read_only: bool) -> bool {
        if !read_only {
            return true;
        }
        if READ_TOOLS.contains(&name) {
            let raw = args.get("path").and_then(Value::as_str).unwrap_or(".");
            return match self.resolve(raw) {
                Ok((_, inside)) => !inside,
                Err(_) => false,
            };
        }
        false
    }

    fn for_write(&self, ctx: &ToolCtx, tool: &str, raw: &str) -> Result<PathBuf> {
        let (p, _) = self.resolve(raw)?;
        if system_folder(&p) {
            return Err(Error::Denied(format!(
                "{} is a system folder; WorkCraft does not write there",
                p.display()
            )));
        }
        if p.is_dir() {
            return Err(Error::Invalid(format!("{} is a folder", p.display())));
        }
        self.checkpoints.save(&ctx.session, &ctx.turn, tool, &p)?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(p)
    }

    /// Save returned MCP `ImagePart`s into `<workspace>/.workcraft/artifacts/<session>/<call_id>-<idx>.<ext>`,
    /// appending `[saved image: <path> (<W>x<H>)]` to `res.text` (`T2.2`).
    fn persist_returned_images(
        &self,
        session: &str,
        call_id: &str,
        res: &mut ToolResult,
    ) -> Vec<String> {
        if res.images.is_empty() {
            return Vec::new();
        }
        let sess_dir = self
            .workspace
            .join(".workcraft")
            .join("artifacts")
            .join(sanitize_file_stem(session));
        if std::fs::create_dir_all(&sess_dir).is_err() {
            return Vec::new();
        }
        let stem = sanitize_file_stem(call_id);
        let mut saved = Vec::with_capacity(res.images.len());
        for (idx, img) in res.images.iter().enumerate() {
            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(img.data.trim())
            else {
                continue;
            };
            let ext = ext_for_mime(&img.mime_type);
            let file_path = sess_dir.join(format!("{stem}-{idx}.{ext}"));
            if std::fs::write(&file_path, &bytes).is_err() {
                continue;
            }
            let dims = image::load_from_memory(&bytes)
                .ok()
                .map(|im| (im.width(), im.height()));
            let path_str = file_path.display().to_string();
            let tag = match dims {
                Some((w, h)) => format!("[saved image: {path_str} ({w}x{h})]"),
                None => format!("[saved image: {path_str}]"),
            };
            if res.text.is_empty() {
                res.text = tag;
            } else {
                res.text.push('\n');
                res.text.push_str(&tag);
            }
            saved.push(path_str);
        }
        saved
    }

    pub async fn call(&self, ctx: &ToolCtx, name: &str, args: Value) -> ToolResult {
        self.call_with_id(ctx, "call", name, args).await.result
    }

    pub async fn call_with_id(
        &self,
        ctx: &ToolCtx,
        call_id: &str,
        name: &str,
        args: Value,
    ) -> CallOutcome {
        match self.call_inner(ctx, call_id, name, args).await {
            Ok(o) => o,
            Err(e) => CallOutcome {
                result: ToolResult::err(e.to_string()),
                artifacts: Vec::new(),
                mode: None,
            },
        }
    }

    async fn open_in_live_app(&self, app_id: &str, file_path: &Path) -> Option<String> {
        let roots = self.roots_context();
        let live = ensure_live_with_roots(app_id, Some(file_path), &roots, &self.hub)
            .await
            .ok()?;
        if let Some(spec) = find_spec(app_id)
            && let VerbTarget::Tool(open_sub) = spec.verbs.open
        {
            let full = format!("{}__{open_sub}", spec.id);
            let _ = self
                .hub
                .call(&full, json!({ "path": file_path.to_string_lossy() }))
                .await;
        }
        Some(live)
    }

    async fn call_inner(
        &self,
        ctx: &ToolCtx,
        call_id: &str,
        name: &str,
        args: Value,
    ) -> Result<CallOutcome> {
        // The adapter keeps text that is not valid JSON as `{"_raw": ...}`. That is almost always an answer cut off
        // by the length limit in the middle of a tool call: say so, and ask for smaller calls.
        if let Some(raw) = args
            .as_object()
            .filter(|o| o.len() == 1)
            .and_then(|o| o.get("_raw"))
            .and_then(Value::as_str)
        {
            return Err(Error::Invalid(format!(
                "the arguments of `{name}` were not valid JSON ({} characters arrived{}). Send one complete JSON object, and split a large request into several smaller calls",
                raw.chars().count(),
                if raw.trim_end().ends_with(['}', ']']) {
                    ""
                } else {
                    ", the text ends in the middle: it was probably cut off by the length limit"
                }
            )));
        }
        let plain = |r: ToolResult| CallOutcome {
            result: r,
            artifacts: Vec::new(),
            mode: None,
        };
        let s = |k: &str| args.get(k).and_then(Value::as_str);
        let need = |k: &str| s(k).ok_or_else(|| Error::Invalid(format!("missing argument '{k}'")));
        match name {
            "read_file" => self
                .read_file(
                    need("path")?,
                    args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize,
                )
                .map(plain),
            "list_dir" => {
                let (p, _) = self.resolve(s("path").unwrap_or("."))?;
                let depth = args
                    .get("depth")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .clamp(1, 4) as usize;
                let mut lines = Vec::new();
                list(&p, &p, depth, &mut lines)?;
                let more = lines.len().saturating_sub(MAX_LIST);
                lines.truncate(MAX_LIST);
                let mut text = if lines.is_empty() {
                    format!("{} is empty", p.display())
                } else {
                    format!("{}\n{}", p.display(), lines.join("\n"))
                };
                if more > 0 {
                    text.push_str(&format!("\n[{more} more]"));
                }
                Ok(plain(ToolResult::ok(text)))
            }
            "search_files" => {
                let (root, _) = self.resolve(s("path").unwrap_or("."))?;
                let pat = s("name").unwrap_or("").to_lowercase();
                let text = s("text").map(str::to_lowercase);
                let mut hits = Vec::new();
                search(&root, &pat, text.as_deref(), 0, &mut hits);
                Ok(plain(ToolResult::ok(if hits.is_empty() {
                    format!("nothing found under {}", root.display())
                } else {
                    hits.join("\n")
                })))
            }
            "file_info" => {
                let (p, _) = self.resolve(need("path")?)?;
                let m = std::fs::metadata(&p)
                    .map_err(|e| Error::Io(format!("{}: {e}", p.display())))?;
                Ok(plain(ToolResult::ok(format!(
                    "{}\n{}, {}, modified {}",
                    p.display(),
                    if m.is_dir() { "folder" } else { docs::kind(&p) },
                    human_size(m.len()),
                    modified(&m)
                ))))
            }
            "write_file" => {
                let raw = need("path")?;
                let content = need("content")?;
                if docs::is_document(Path::new(raw))
                    && !matches!(docs::ext(Path::new(raw)).as_str(), "csv" | "tsv")
                {
                    return Err(Error::Invalid(
                        "write_file writes plain text; use create_document, create_spreadsheet or create_presentation for Office files".into(),
                    ));
                }
                let p = self.for_write(ctx, "write_file", raw)?;
                std::fs::write(&p, content)?;
                Ok(plain(ToolResult::ok(format!(
                    "wrote {} to {}",
                    human_size(content.len() as u64),
                    p.display()
                ))))
            }
            "edit_file" => {
                let (find, replace) = (need("find")?, need("replace")?);
                let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
                let (probe, _) = self.resolve(need("path")?)?;
                if !probe.is_file() {
                    return Err(Error::NotFound(format!(
                        "{} does not exist",
                        probe.display()
                    )));
                }
                let p = self.for_write(ctx, "edit_file", need("path")?)?;
                let e = docs::ext(&p);
                if matches!(
                    e.as_str(),
                    "docx" | "docm" | "dotx" | "pptx" | "pptm" | "xlsx" | "xlsm"
                ) {
                    let n = docs::replace_in_office(&p, find, replace)?;
                    if n == 0 {
                        return Err(Error::NotFound(format!(
                            "'{find}' was not found in the text of {} (text split by formatting is not matched; for rich edits use the Craft app)",
                            p.display()
                        )));
                    }
                    return Ok(plain(ToolResult::ok(format!(
                        "replaced {n} place(s) in {}",
                        p.display()
                    ))));
                }
                let text = std::fs::read_to_string(&p)
                    .map_err(|e| Error::Io(format!("{}: {e}", p.display())))?;
                let n = text.matches(find).count();
                if n == 0 {
                    return Err(Error::NotFound(format!(
                        "the text to find is not in {}; read the file and copy it exactly",
                        p.display()
                    )));
                }
                if n > 1 && !all {
                    return Err(Error::Invalid(format!(
                        "the text occurs {n} times; add more context or pass all=true"
                    )));
                }
                std::fs::write(&p, text.replace(find, replace))?;
                Ok(plain(ToolResult::ok(format!(
                    "replaced {n} place(s) in {}",
                    p.display()
                ))))
            }
            "copy_file" => {
                let (from, _) = self.resolve(need("from")?)?;
                if !from.is_file() {
                    return Err(Error::NotFound(format!(
                        "{} does not exist",
                        from.display()
                    )));
                }
                let to = self.for_write(ctx, "copy_file", need("to")?)?;
                std::fs::copy(&from, &to)?;
                Ok(plain(ToolResult::ok(format!("copied to {}", to.display()))))
            }
            "create_document" => {
                let raw = need("path")?;
                let content = need("content")?;
                let p = self.for_write(ctx, "create_document", raw)?;
                match docs::ext(&p).as_str() {
                    "docx" => docs::write_docx(&p, s("title"), content)?,
                    "md" | "txt" | "markdown" => {
                        let t = s("title")
                            .map(|t| format!("# {t}\n\n{content}"))
                            .unwrap_or_else(|| content.to_string());
                        std::fs::write(&p, t)?;
                    }
                    other => {
                        return Err(Error::Invalid(format!(
                            "create_document makes .docx, .md or .txt, not .{other}"
                        )));
                    }
                }
                let mut msg = format!("created {}", p.display());
                if args.get("open").and_then(Value::as_bool) == Some(true)
                    && let Some(live) = self.open_in_live_app("wordcraft", &p).await
                {
                    msg.push_str(&format!(" ({live})"));
                }
                Ok(plain(ToolResult::ok(msg)))
            }
            "create_spreadsheet" => {
                let raw = need("path")?;
                let sheets: Vec<docs::Sheet> = args
                    .get("sheets")
                    .and_then(Value::as_array)
                    .ok_or_else(|| Error::Invalid("missing argument 'sheets'".into()))?
                    .iter()
                    .map(|s| docs::Sheet {
                        name: s["name"].as_str().unwrap_or("").to_string(),
                        rows: s["rows"]
                            .as_array()
                            .map(|r| {
                                r.iter()
                                    .map(|row| {
                                        row.as_array().cloned().unwrap_or_else(|| vec![row.clone()])
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        header: s["header"].as_bool().unwrap_or(true),
                    })
                    .collect();
                let p = self.for_write(ctx, "create_spreadsheet", raw)?;
                match docs::ext(&p).as_str() {
                    "xlsx" => docs::write_xlsx(&p, &sheets)?,
                    "csv" => {
                        let rows = sheets.first().map(|s| s.rows.clone()).unwrap_or_default();
                        std::fs::write(&p, csv(&rows))?;
                    }
                    other => {
                        return Err(Error::Invalid(format!(
                            "create_spreadsheet makes .xlsx or .csv, not .{other}"
                        )));
                    }
                }
                let mut msg = format!("created {} ({} sheet(s))", p.display(), sheets.len().max(1));
                if args.get("open").and_then(Value::as_bool) == Some(true)
                    && let Some(live) = self.open_in_live_app("gridcraft", &p).await
                {
                    msg.push_str(&format!(" ({live})"));
                }
                Ok(plain(ToolResult::ok(msg)))
            }
            "create_presentation" => {
                let raw = need("path")?;
                let slides: Vec<docs::Slide> = args
                    .get("slides")
                    .and_then(Value::as_array)
                    .ok_or_else(|| Error::Invalid("missing argument 'slides'".into()))?
                    .iter()
                    .map(|s| docs::Slide {
                        title: s["title"].as_str().unwrap_or("").to_string(),
                        bullets: s["bullets"]
                            .as_array()
                            .map(|b| {
                                b.iter()
                                    .filter_map(|x| x.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        notes: s["notes"].as_str().map(str::to_string),
                    })
                    .collect();
                let p = self.for_write(ctx, "create_presentation", raw)?;
                if docs::ext(&p) != "pptx" {
                    return Err(Error::Invalid(
                        "create_presentation makes .pptx files".into(),
                    ));
                }
                docs::write_pptx(&p, &slides)?;
                let mut msg = format!("created {} ({} slides)", p.display(), slides.len());
                if args.get("open").and_then(Value::as_bool) == Some(true)
                    && let Some(live) = self.open_in_live_app("deckcraft", &p).await
                {
                    msg.push_str(&format!(" ({live})"));
                }
                Ok(plain(ToolResult::ok(msg)))
            }
            "open_craft_app" => {
                let app_raw = need("app")?;
                let open_path = match s("path").filter(|p| !p.trim().is_empty()) {
                    Some(raw) => Some(self.resolve(raw)?.0),
                    None => None,
                };
                let roots = self.roots_context();
                let status =
                    ensure_live_with_roots(app_raw, open_path.as_deref(), &roots, &self.hub)
                        .await
                        .map_err(Error::Mcp)?;
                if let Some(spec) = find_spec(app_raw)
                    && let Some(ref p) = open_path
                    && p.is_file()
                    && let VerbTarget::Tool(open_sub) = spec.verbs.open
                {
                    let full = format!("{}__{open_sub}", spec.id);
                    let _ = self
                        .hub
                        .call(&full, json!({ "path": p.to_string_lossy() }))
                        .await;
                }
                Ok(CallOutcome {
                    result: ToolResult::ok(status),
                    artifacts: Vec::new(),
                    mode: Some("live".into()),
                })
            }
            "craft_status" => self.call_craft_status(s("app")).await.map(plain),
            "craft_activate" => self.call_craft_activate(need("app")?, s("mode")).await,
            "craft_handoff" => {
                let (src, _) = self.resolve(need("file")?)?;
                let val = execute_handoff(
                    need("from_app")?,
                    need("to_app")?,
                    &src,
                    s("as_format"),
                    &self.workspace,
                )?;
                let mut artifacts = Vec::new();
                if let Some(staged) = val.get("staged_path").and_then(Value::as_str) {
                    let ext = Path::new(staged)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if matches!(
                        ext.as_str(),
                        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
                    ) {
                        artifacts.push(staged.to_string());
                    }
                }
                Ok(CallOutcome {
                    result: ToolResult::ok(serde_json::to_string_pretty(&val)?),
                    artifacts,
                    mode: None,
                })
            }
            "craft_open" | "craft_new" | "craft_inspect" | "craft_render" | "craft_export"
            | "craft_save" | "craft_batch" => self.call_craft_verb(ctx, call_id, name, args).await,
            "memory_get" => {
                let k = need("key")?;
                Ok(plain(ctx.memory.with(|m| m.get(k).cloned()).map_or_else(
                    || ToolResult::err(format!("no value for '{k}'")),
                    ToolResult::ok,
                )))
            }
            "memory_set" => {
                let (k, v) = (need("key")?.to_string(), need("value")?.to_string());
                let stored = ctx.memory.with(|m| {
                    if !m.contains_key(&k) && m.len() >= MAX_MEMORY_KEYS {
                        return false;
                    }
                    m.insert(k.clone(), truncate_bytes(&v, 32 * 1024));
                    true
                });
                if !stored {
                    return Err(Error::Invalid(format!(
                        "the shared memory holds {MAX_MEMORY_KEYS} keys already; it is for facts other agents need, so reuse a key"
                    )));
                }
                Ok(plain(ToolResult::ok(format!("stored '{k}'"))))
            }
            "memory_list" => {
                let keys = ctx.memory.with(|m| m.keys().cloned().collect::<Vec<_>>());
                Ok(plain(ToolResult::ok(if keys.is_empty() {
                    "the shared memory is empty".to_string()
                } else {
                    keys.join("\n")
                })))
            }
            "list_skills" => {
                let i = self.skills.index(&[]);
                Ok(plain(ToolResult::ok(if i.is_empty() {
                    "no skills are installed".to_string()
                } else {
                    i
                })))
            }
            "find_tools" => {
                let query = need("query")?;
                self.connect_on_demand(&crate::craft::apps_named_in(query))
                    .await;
                let all = self.hub.tools().await;
                let hits = crate::craft::router::search(&all, query, 14);
                let apps = crate::craft::apps_named_in(query);
                if hits.is_empty() {
                    Ok(plain(ToolResult::ok(format!(
                        "No Craft tool matches \"{query}\". Installed apps: {}. Use `craft_status` to see which are available.",
                        crate::craft::specs()
                            .iter()
                            .map(|s| s.id)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))))
                } else {
                    let mut out = String::from("Available from the next step:\n");
                    for (name, desc) in &hits {
                        out.push_str(&format!("- {name}: {desc}\n"));
                    }
                    if !apps.is_empty() {
                        out.push_str(&format!(
                            "Apps loaded: {}. Read the app's skill with `read_skill` before a larger task.\n",
                            apps.iter().copied().collect::<Vec<_>>().join(", ")
                        ));
                    }
                    Ok(plain(ToolResult::ok(out)))
                }
            }
            "read_skill" => {
                // Small models send `skill` for `name` and `file: "skill"` for the main file.
                let skill = ["name", "skill", "skill_name", "skillName", "skill_id", "id", "app", "app_id", "title", "topic"]
                    .iter()
                    .find_map(|k| s(k))
                    .or_else(|| {
                        // One stray text argument is the name, whatever the model called it.
                        let strs: Vec<&str> = args
                            .as_object()
                            .into_iter()
                            .flatten()
                            .filter(|(k, _)| !matches!(k.as_str(), "file" | "section" | "full"))
                            .filter_map(|(_, v)| v.as_str())
                            .filter(|v| !v.trim().is_empty())
                            .collect();
                        (strs.len() == 1).then(|| strs[0])
                    })
                    .ok_or_else(|| {
                        let names: Vec<&str> = self.skills.list().iter().map(|k| k.name.as_str()).collect();
                        Error::Invalid(format!(
                            "read_skill needs the skill name, for example {{\"name\": \"wordcraft\"}}. Available skills: {}.",
                            names.join(", ")
                        ))
                    })?;
                let file = s("file").filter(|f| {
                    let f = f.trim().to_ascii_lowercase();
                    !(f.is_empty() || f == "skill" || f == "skill.md")
                });
                let full = args.get("full").and_then(Value::as_bool).unwrap_or(false);
                Ok(plain(ToolResult::ok(self.skills.read_view(
                    skill,
                    file,
                    s("section"),
                    full,
                )?)))
            }
            "fetch_url" => self.fetch_url(ctx, need("url")?).await.map(plain),
            "download_file" => self
                .download_file_tool(ctx, need("url")?, s("path"))
                .await
                .map(plain),
            "search_wikimedia_images" => {
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(5)
                    .clamp(1, 10) as usize;
                let download = args
                    .get("download")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                self.search_wikimedia_images_tool(ctx, need("query")?, limit, download, s("path"))
                    .await
                    .map(plain)
            }
            "craft_commands" => {
                let app_raw = need("app")?;
                if self.controls.contains_key(app_raw) {
                    let mut c = self.control(app_raw).await?;
                    let v = c.commands().await?;
                    let filter = s("filter").unwrap_or_default().to_lowercase();
                    let text = match v.as_array() {
                        Some(items) => items
                            .iter()
                            .map(|i| {
                                i.get("id")
                                    .and_then(Value::as_str)
                                    .map_or_else(|| i.to_string(), str::to_string)
                            })
                            .filter(|id| filter.is_empty() || id.to_lowercase().contains(&filter))
                            .take(MAX_LIST)
                            .collect::<Vec<_>>()
                            .join("\n"),
                        None => v.to_string(),
                    };
                    return Ok(plain(ToolResult::ok(truncate_bytes(&text, 64 * 1024))));
                }
                self.call_craft_verb(ctx, call_id, "craft_commands", args)
                    .await
            }
            "craft_run" => {
                let app_raw = need("app")?;
                if self.controls.contains_key(app_raw) {
                    let mut c = self.control(app_raw).await?;
                    let v = c
                        .execute(
                            need("command")?,
                            args.get("params").cloned().unwrap_or(Value::Null),
                        )
                        .await?;
                    return Ok(plain(ToolResult::ok(truncate_bytes(
                        &v.to_string(),
                        64 * 1024,
                    ))));
                }
                self.call_craft_verb(ctx, call_id, "craft_run", args).await
            }
            other if other.contains(workcraft_mcp::hub::SEPARATOR) => {
                self.call_mcp_tool(ctx, call_id, other, args).await
            }
            other => Err(Error::NotFound(format!("tool '{other}'"))),
        }
    }

    async fn call_craft_status(&self, app_filter: Option<&str>) -> Result<ToolResult> {
        let (found, errs) = crate::craft::discover_with_errors();
        let connected = self.hub.servers().await;
        let mut counts = BTreeMap::new();
        let mut last_errors = BTreeMap::new();
        for sp in crate::craft::specs() {
            if let Some(tools) = self.hub.server_tools(sp.id).await {
                counts.insert(sp.id.to_string(), tools.len());
            }
            if let Some(e) = self.hub.last_error(sp.id).await {
                last_errors.insert(sp.id.to_string(), e);
            }
        }
        let full = crate::craft::report_full(
            &found,
            &errs,
            &self.craft_apps,
            &self.craft_modes,
            &connected,
            &counts,
            &last_errors,
        );
        if let Some(raw) = app_filter.filter(|a| !a.trim().is_empty()) {
            let spec = find_spec(raw)
                .ok_or_else(|| Error::NotFound(format!("unknown Craft app '{raw}'")))?;
            let item = full
                .as_array()
                .and_then(|arr| arr.iter().find(|v| v["id"].as_str() == Some(spec.id)))
                .cloned()
                .unwrap_or_else(|| json!({ "id": spec.id }));
            return Ok(ToolResult::ok(serde_json::to_string_pretty(&item)?));
        }
        Ok(ToolResult::ok(serde_json::to_string_pretty(&full)?))
    }

    async fn call_craft_activate(
        &self,
        app_raw: &str,
        mode_raw: Option<&str>,
    ) -> Result<CallOutcome> {
        let spec = find_spec(app_raw)
            .ok_or_else(|| Error::NotFound(format!("unknown Craft app '{app_raw}'")))?;
        if spec.off_by_default
            && self.craft_apps.get(spec.id) != Some(&true)
            && !self.craft_modes.contains_key(spec.id)
        {
            return Err(Error::Denied(format!(
                "{} is disabled by default for safety. Enable '{}' in Settings > Tools & MCP (or via `craft.set` / `craft.mode`) first.",
                spec.name, spec.id
            )));
        }
        let target_mode = match mode_raw.and_then(CraftMode::parse) {
            Some(CraftMode::Live) => CraftMode::Live,
            _ => CraftMode::Headless,
        };
        let roots = self.roots_context();
        let status = match target_mode {
            CraftMode::Live => ensure_live_with_roots(spec.id, None, &roots, &self.hub)
                .await
                .map_err(|e| map_connect_err(spec, e))?,
            _ => ensure_headless(spec.id, &roots, &self.hub)
                .await
                .map_err(|e| map_connect_err(spec, e))?,
        };
        let filtered = if let Some(raw_tools) = self.hub.server_tools(spec.id).await {
            filter_app_tools(spec, target_mode, self.full_tool_catalog, raw_tools)
                .into_iter()
                .map(|t| format!("{}__{}", spec.id, t.name))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let payload = json!({
            "ok": true,
            "app": spec.id,
            "name": spec.name,
            "mode": target_mode.as_str(),
            "status": status,
            "tools": filtered,
        });
        Ok(CallOutcome {
            result: ToolResult::ok(serde_json::to_string_pretty(&payload)?),
            artifacts: Vec::new(),
            mode: Some(target_mode.as_str().to_string()),
        })
    }

    /// Translate a uniform `craft_*` verb into the target app's native `<app>__<tool>` MCP call (`T3.5`).
    async fn call_craft_verb(
        &self,
        ctx: &ToolCtx,
        call_id: &str,
        verb: &str,
        args: Value,
    ) -> Result<CallOutcome> {
        let app_raw = args
            .get("app")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Invalid("missing argument 'app'".into()))?;
        let spec = find_spec(app_raw)
            .ok_or_else(|| Error::NotFound(format!("unknown Craft app '{app_raw}'")))?;

        let str_to_verb = |s: &'static str| {
            if s.is_empty() {
                VerbTarget::None
            } else {
                VerbTarget::Tool(s)
            }
        };
        let target = match verb {
            "craft_open" => spec.verbs.open,
            "craft_new" => spec.verbs.new,
            "craft_inspect" => spec.verbs.inspect,
            "craft_render" => spec.verbs.render,
            "craft_export" => spec.verbs.export,
            "craft_save" => spec.verbs.save,
            "craft_commands" => str_to_verb(spec.verbs.commands),
            "craft_run" => str_to_verb(spec.verbs.run),
            "craft_batch" => {
                if !spec.verbs.batch.is_empty() {
                    VerbTarget::Tool(spec.verbs.batch)
                } else {
                    str_to_verb(spec.verbs.run)
                }
            }
            _ => VerbTarget::None,
        };

        if matches!(target, VerbTarget::None) {
            return Err(Error::Invalid(format!(
                "{} does not support `{verb}`; check `{}__{}` or read the '{}' skill.",
                spec.name,
                spec.id,
                spec.lean_tools.first().copied().unwrap_or("help"),
                spec.skill
            )));
        }

        let (subtool, mapped_args) = build_verb_invocation(spec, verb, target, &args)?;
        let full_name = format!("{}__{subtool}", spec.id);
        self.call_mcp_tool(ctx, call_id, &full_name, mapped_args)
            .await
    }

    fn read_file(&self, raw: &str, offset: usize) -> Result<ToolResult> {
        let (p, _) = self.resolve(raw)?;
        let meta = std::fs::metadata(&p).map_err(|e| Error::Io(format!("{}: {e}", p.display())))?;
        if meta.is_dir() {
            return Err(Error::Invalid(format!(
                "{} is a folder; use list_dir",
                p.display()
            )));
        }
        if !docs::is_document(&p) && meta.len() > MAX_TEXT_BYTES {
            return Err(Error::Invalid(format!(
                "{} is {}; the limit for text files is {}",
                p.display(),
                human_size(meta.len()),
                human_size(MAX_TEXT_BYTES)
            )));
        }
        let text = docs::extract_text(&p)?;
        let total = text.chars().count();
        if offset >= total && total > 0 {
            return Err(Error::Invalid(format!(
                "offset {offset} is past the end ({total} characters)"
            )));
        }
        let window: String = text.chars().skip(offset).take(READ_WINDOW).collect();
        let end = offset + window.chars().count();
        let mut out = window;
        if offset > 0 || end < total {
            out.push_str(&format!(
                "\n\n[{}: characters {offset} to {end} of {total}{}]",
                p.display(),
                if end < total {
                    format!(". Read on with offset={end}")
                } else {
                    String::new()
                }
            ));
        }
        Ok(ToolResult::ok(out))
    }

    async fn control(&self, app: &str) -> Result<ControlClient> {
        let t = self
            .controls
            .get(app)
            .ok_or_else(|| Error::NotFound(format!("no control target for '{app}'")))?;
        ControlClient::connect(&t.addr, &t.resolve_token()?).await
    }

    async fn call_mcp_tool(
        &self,
        ctx: &ToolCtx,
        call_id: &str,
        full_name: &str,
        mut args: Value,
    ) -> Result<CallOutcome> {
        let Some((srv, subtool)) = workcraft_mcp::hub::split_name(full_name) else {
            return Err(Error::NotFound(format!("tool '{full_name}'")));
        };
        strip_null_args(&mut args);

        let craft_spec = find_spec(srv);
        let is_mock_server = self.hub.is_connected(srv).await
            && self
                .hub
                .client_args(srv)
                .await
                .is_some_and(|a| !a.iter().any(|x| x == "mcp"));

        let mut active_mode = CraftMode::Headless;

        if let Some(spec) = craft_spec
            && !is_mock_server
        {
            let roots = self.roots_context();
            // Explicit window-opening aliases always launch/bridge live.
            if matches!(subtool, "start_app" | "open_app" | "launch" | "launch_app") {
                let open_path = args
                    .get("path")
                    .or_else(|| args.get("file"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .and_then(|raw| self.resolve(raw).ok().map(|(p, _)| p));
                let status =
                    ensure_live_with_roots(spec.id, open_path.as_deref(), &roots, &self.hub)
                        .await
                        .map_err(|e| map_connect_err(spec, e))?;
                return Ok(CallOutcome {
                    result: ToolResult::ok(status),
                    artifacts: Vec::new(),
                    mode: Some("live".into()),
                });
            }

            let cfg_mode = mode_for(spec.id, &self.craft_apps, &self.craft_modes);
            if cfg_mode == CraftMode::Off {
                return Err(Error::Denied(format!(
                    "{} is turned off (mode: off). Enable '{}' in Settings > Tools & MCP or via `craft.mode`.",
                    spec.name, spec.id
                )));
            }

            if cfg_mode == CraftMode::Live {
                active_mode = CraftMode::Live;
                let running_port = live_port_for(spec.id, spec.name);
                let is_free_read = matches!(
                    classify_tool_call(full_name, &args, None),
                    ApprovalLevel::Free
                );
                if running_port.is_none() && is_free_read {
                    return Err(Error::AppNotRunning {
                        app: spec.name.to_string(),
                        hint: format!(
                            "Start {} (or call `open_craft_app` with app='{}'), or switch '{}' to headless mode.",
                            spec.name, spec.id, spec.id
                        ),
                    });
                }
                ensure_live_with_roots(spec.id, None, &roots, &self.hub)
                    .await
                    .map_err(|e| map_connect_err(spec, e))?;
            } else {
                // Headless mode (default): never open a surprise GUI window (T1.2, D3).
                if !self.hub.is_connected(spec.id).await && self.allow_craft_on_demand {
                    ensure_headless(spec.id, &roots, &self.hub)
                        .await
                        .map_err(|e| map_connect_err(spec, e))?;
                }
                active_mode = self.effective_connected_mode(spec).await;
            }
        }

        // Resolve relative file paths, auto-download remote http(s) URLs, enforce sandbox roots,
        // rewrite relative paths for TextureCraft/MaterialCraft (`RootsPolicy::RelativeReadWrite`),
        // and snapshot overwritten files for undo (`T2.5`, `T4.4`).
        self.resolve_or_download_mcp_paths(ctx, full_name, craft_spec, active_mode, &mut args)
            .await?;

        if craft_spec.is_some()
            && !is_mock_server
            && let Some(intercepted) =
                quirks::pre_call(srv, subtool, &mut args, active_mode, &self.hub).await?
        {
            return Ok(CallOutcome {
                result: intercepted,
                artifacts: Vec::new(),
                mode: Some(active_mode.as_str().to_string()),
            });
        }

        let timeout = craft_spec.map_or(Duration::from_secs(60), |s| s.timeout_for(subtool));
        let mut res = match self
            .hub
            .call_with_timeout(full_name, args.clone(), timeout)
            .await
        {
            Ok(r) => r,
            Err(e) => return Err(self.explain_unknown_craft_tool(srv, subtool, e).await),
        };

        if craft_spec.is_some() {
            res = quirks::post_call(srv, subtool, full_name, &args, active_mode, res, &self.hub)
                .await;
        }

        let artifacts = self.persist_returned_images(&ctx.session, call_id, &mut res);
        let mode_str = craft_spec.map(|_| active_mode.as_str().to_string());
        Ok(CallOutcome {
            result: res,
            artifacts,
            mode: mode_str,
        })
    }

    /// An app answered "unknown tool": tell the model which tools exist, so a small model that guessed a
    /// name (`vectorcraft__new`) can correct itself instead of giving up.
    async fn explain_unknown_craft_tool(&self, srv: &str, subtool: &str, e: Error) -> Error {
        let Some(spec) = find_spec(srv) else {
            return e;
        };
        let msg = e.to_string().to_ascii_lowercase();
        if !(msg.contains("unknown tool") || msg.contains("tool not found")) {
            return e;
        }
        let mode = self.effective_connected_mode(spec).await;
        let tools = self.hub.server_tools(srv).await.unwrap_or_default();
        let names: Vec<String> = filter_app_tools(spec, mode, self.full_tool_catalog, tools)
            .into_iter()
            .map(|t| t.name)
            .collect();
        Error::ToolFailed {
            tool: format!("{srv}__{subtool}"),
            message: format!("{} has no tool called `{subtool}`", spec.name),
            hint: format!(
                "Use one of: {}. To create or open a document use `craft_new` / `craft_open` with app='{}'.",
                names.join(", "),
                spec.id
            ),
        }
    }

    async fn resolve_or_download_mcp_paths(
        &self,
        ctx: &ToolCtx,
        full_name: &str,
        spec: Option<&AppSpec>,
        active_mode: CraftMode,
        args: &mut Value,
    ) -> Result<()> {
        let relative_rw = spec.is_some_and(|s| matches!(s.roots, RootsPolicy::RelativeReadWrite))
            && active_mode == CraftMode::Headless;
        let enforce_sandbox = spec.is_some_and(|s| {
            matches!(
                s.roots,
                RootsPolicy::SingleAutomation
                    | RootsPolicy::RepeatableAutomation
                    | RootsPolicy::RelativeReadWrite
                    | RootsPolicy::AllowPath
            )
        }) && active_mode == CraftMode::Headless;

        self.rewrite_paths_in_value(ctx, full_name, spec, relative_rw, enforce_sandbox, args, 0)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn rewrite_paths_in_value(
        &self,
        ctx: &ToolCtx,
        full_name: &str,
        spec: Option<&AppSpec>,
        relative_rw: bool,
        enforce_sandbox: bool,
        val: &mut Value,
        depth: usize,
    ) -> Result<()> {
        if depth > 4 {
            return Ok(());
        }
        let Some(obj) = val.as_object_mut() else {
            return Ok(());
        };

        for k in SINGLE_PATH_ARGS {
            if let Some(raw) = obj.get(k).and_then(Value::as_str).map(str::to_string) {
                let allow_dot = DIR_ARGS.contains(&k);
                if let Some(rewritten) = self
                    .resolve_mcp_path_arg(
                        ctx,
                        full_name,
                        spec,
                        &raw,
                        relative_rw,
                        enforce_sandbox,
                        allow_dot,
                    )
                    .await?
                {
                    obj.insert(k.to_string(), Value::String(rewritten));
                }
            }
        }

        for k in MULTI_PATH_ARGS {
            if let Some(arr) = obj.get_mut(k).and_then(Value::as_array_mut) {
                for item in arr.iter_mut() {
                    if let Some(raw) = item.as_str().map(str::to_string)
                        && let Some(rewritten) = self
                            .resolve_mcp_path_arg(
                                ctx,
                                full_name,
                                spec,
                                &raw,
                                relative_rw,
                                enforce_sandbox,
                                false,
                            )
                            .await?
                    {
                        *item = Value::String(rewritten);
                    }
                }
            }
        }

        for nested_key in ["params", "arguments"] {
            if let Some(nested) = obj.get_mut(nested_key) {
                Box::pin(self.rewrite_paths_in_value(
                    ctx,
                    full_name,
                    spec,
                    relative_rw,
                    enforce_sandbox,
                    nested,
                    depth + 1,
                ))
                .await?;
            }
        }

        for arr_key in ["commands", "steps"] {
            if let Some(arr) = obj.get_mut(arr_key).and_then(Value::as_array_mut) {
                for elem in arr.iter_mut() {
                    Box::pin(self.rewrite_paths_in_value(
                        ctx,
                        full_name,
                        spec,
                        relative_rw,
                        enforce_sandbox,
                        elem,
                        depth + 1,
                    ))
                    .await?;
                }
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn resolve_mcp_path_arg(
        &self,
        ctx: &ToolCtx,
        tool_name: &str,
        spec: Option<&AppSpec>,
        raw: &str,
        relative_rw: bool,
        enforce_sandbox: bool,
        allow_dot: bool,
    ) -> Result<Option<String>> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        let resolved_path = if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
            let Ok((saved, _, _)) = self.download_to_workspace(ctx, trimmed, None).await else {
                return Ok(None);
            };
            saved
        } else {
            let Ok((p, inside_roots)) = self.resolve(trimmed) else {
                return Ok(None);
            };
            if enforce_sandbox && !inside_roots {
                let app_name = spec.map_or("Craft app", |s| s.name);
                return Err(Error::DeniedByRoot {
                    app: app_name.to_string(),
                    path: p.display().to_string(),
                    hint: format!(
                        "{app_name} only accesses files inside the workspace ({}) or configured extra_roots. Copy the file into the workspace or add its folder to extra_roots.",
                        self.workspace.display()
                    ),
                });
            }
            if relative_rw && !self.is_inside_workspace(&p) {
                let app_name = spec.map_or("Craft app", |s| s.name);
                return Err(Error::DeniedByRoot {
                    app: app_name.to_string(),
                    path: p.display().to_string(),
                    hint: format!(
                        "{app_name} requires paths inside the workspace root ({}).",
                        self.workspace.display()
                    ),
                });
            }
            p
        };

        if resolved_path.is_file() {
            let _ = self
                .checkpoints
                .save(&ctx.session, &ctx.turn, tool_name, &resolved_path);
        }

        if relative_rw {
            let ws_canon = self
                .workspace
                .canonicalize()
                .unwrap_or_else(|_| self.workspace.clone());
            let target_canon = canonical_or_self(&resolved_path);
            if let Ok(rel) = target_canon.strip_prefix(&ws_canon) {
                let rel_slash = rel.to_string_lossy().replace('\\', "/");
                if rel_slash.is_empty() {
                    return Ok(Some(if allow_dot {
                        ".".to_string()
                    } else {
                        String::new()
                    }));
                }
                return Ok(Some(rel_slash));
            }
        }

        Ok(Some(resolved_path.to_string_lossy().into_owned()))
    }

    async fn fetch_url(&self, ctx: &ToolCtx, raw: &str) -> Result<ToolResult> {
        let url = raw.trim();
        check_public_url(url)?;
        let client = http_client()?;
        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| Error::Io(e.to_string()))?;
        let status = resp.status();
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let bytes = resp.bytes().await.map_err(|e| Error::Io(e.to_string()))?;
        if !status.is_success() {
            if (url.contains("wikimedia.org") || url.contains("wikipedia.org"))
                && let Ok((saved, len, mime)) = self.download_to_workspace(ctx, url, None).await
            {
                return Ok(ToolResult::ok(
                    json!({
                        "ok": true,
                        "saved_path": saved.to_string_lossy(),
                        "bytes": len,
                        "size": human_size(len as u64),
                        "content_type": mime,
                        "note": "Downloaded image to workspace; pass saved_path to insert.picture."
                    })
                    .to_string(),
                ));
            }
            return Err(Error::Io(format!("HTTP {status} from {url}")));
        }

        if is_binary_content(&ct, &bytes) {
            let filename = infer_download_filename(url, &ct, &bytes);
            let target = self.for_write(ctx, "fetch_url", &filename)?;
            std::fs::write(&target, &bytes)?;
            return Ok(ToolResult::ok(
                json!({
                    "ok": true,
                    "saved_path": target.to_string_lossy(),
                    "bytes": bytes.len(),
                    "size": human_size(bytes.len() as u64),
                    "content_type": ct,
                    "note": "Saved binary/image file to workspace. Use saved_path in insert.picture or read_file."
                })
                .to_string(),
            ));
        }

        let raw_text = String::from_utf8_lossy(&bytes);
        let body = if ct.contains("html") || raw_text.trim_start().starts_with('<') {
            docs::strip_tags(&raw_text)
        } else {
            raw_text.into_owned()
        };
        Ok(ToolResult::ok(truncate_bytes(body.trim(), 48 * 1024)))
    }

    async fn download_file_tool(
        &self,
        ctx: &ToolCtx,
        url_or_name: &str,
        dest_path: Option<&str>,
    ) -> Result<ToolResult> {
        let (saved, len, ct) = self
            .download_to_workspace(ctx, url_or_name, dest_path)
            .await?;
        Ok(ToolResult::ok(
            json!({
                "ok": true,
                "path": saved.to_string_lossy(),
                "saved_path": saved.to_string_lossy(),
                "bytes": len,
                "size": human_size(len as u64),
                "content_type": ct
            })
            .to_string(),
        ))
    }

    async fn search_wikimedia_images_tool(
        &self,
        ctx: &ToolCtx,
        query: &str,
        limit: usize,
        download_first: bool,
        dest_path: Option<&str>,
    ) -> Result<ToolResult> {
        let hits = search_wikimedia(query, limit).await?;
        if hits.is_empty() {
            return Ok(ToolResult::ok(
                json!({
                    "ok": true,
                    "query": query,
                    "count": 0,
                    "results": [],
                    "note": "No matching Wikimedia Commons images found; try broader keywords."
                })
                .to_string(),
            ));
        }
        let mut saved_path: Option<String> = None;
        if download_first {
            for item in &hits {
                if let Some(dl_url) = item.get("download_url").and_then(Value::as_str)
                    && let Ok((p, _, _)) = self.download_to_workspace(ctx, dl_url, dest_path).await
                {
                    saved_path = Some(p.to_string_lossy().into_owned());
                    break;
                }
            }
        }
        Ok(ToolResult::ok(
            json!({
                "ok": true,
                "query": query,
                "count": hits.len(),
                "saved_path": saved_path,
                "results": hits
            })
            .to_string(),
        ))
    }

    async fn download_to_workspace(
        &self,
        ctx: &ToolCtx,
        raw_url_or_query: &str,
        dest_path: Option<&str>,
    ) -> Result<(PathBuf, usize, String)> {
        let trimmed = raw_url_or_query.trim();
        if trimmed.is_empty() {
            return Err(Error::Invalid("empty URL".into()));
        }

        let client = http_client()?;
        let mut candidate_urls: Vec<String> = Vec::new();

        if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
            check_public_url(trimmed)?;
            if let Some(file_part) = trimmed.split("/wiki/File:").nth(1) {
                let clean = file_part.split(['?', '#']).next().unwrap_or(file_part);
                candidate_urls.push(format!(
                    "https://commons.wikimedia.org/wiki/Special:FilePath/{clean}?width=1280"
                ));
            }
            candidate_urls.push(trimmed.to_string());
            if trimmed.contains("upload.wikimedia.org/")
                && let Some(file_name) = extract_wikimedia_filename(trimmed)
            {
                candidate_urls.push(format!(
                    "https://commons.wikimedia.org/wiki/Special:FilePath/{file_name}?width=1280"
                ));
            }
        } else {
            let file_name = trimmed.strip_prefix("File:").unwrap_or(trimmed).trim();
            candidate_urls.push(format!(
                "https://commons.wikimedia.org/wiki/Special:FilePath/{}?width=1280",
                url_encode(file_name)
            ));
        }

        let mut last_err = String::from("download failed");
        for u in &candidate_urls {
            if check_public_url(u).is_err() {
                continue;
            }
            match client.get(u).send().await {
                Ok(resp) if resp.status().is_success() => {
                    let ct = resp
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("application/octet-stream")
                        .to_ascii_lowercase();
                    if let Ok(bytes) = resp.bytes().await
                        && !bytes.is_empty()
                        && !ct.contains("text/html")
                    {
                        let raw_dest = dest_path
                            .filter(|s| !s.trim().is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| infer_download_filename(u, &ct, &bytes));
                        let target = self.for_write(ctx, "download_file", &raw_dest)?;
                        std::fs::write(&target, &bytes)?;
                        return Ok((target, bytes.len(), ct));
                    }
                }
                Ok(resp) => {
                    last_err = format!("HTTP {} from {u}", resp.status());
                }
                Err(e) => {
                    last_err = format!("{e}");
                }
            }
        }

        let search_topic = extract_search_topic(trimmed);
        if !search_topic.is_empty()
            && let Ok(hits) = search_wikimedia(&search_topic, 3).await
        {
            for item in hits {
                if let Some(dl_url) = item.get("download_url").and_then(Value::as_str)
                    && check_public_url(dl_url).is_ok()
                    && let Ok(resp) = client.get(dl_url).send().await
                    && resp.status().is_success()
                {
                    let ct = resp
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("image/jpeg")
                        .to_ascii_lowercase();
                    if let Ok(bytes) = resp.bytes().await
                        && !bytes.is_empty()
                    {
                        let raw_dest = dest_path
                            .filter(|s| !s.trim().is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| infer_download_filename(dl_url, &ct, &bytes));
                        let target = self.for_write(ctx, "download_file", &raw_dest)?;
                        std::fs::write(&target, &bytes)?;
                        return Ok((target, bytes.len(), ct));
                    }
                }
            }
        }

        Err(Error::Io(last_err))
    }
}

fn map_connect_err(spec: &AppSpec, msg: String) -> Error {
    if msg.contains("not installed") || msg.contains("missing file") {
        Error::AppNotInstalled {
            app: spec.name.to_string(),
            hint: msg,
        }
    } else {
        Error::Mcp(msg)
    }
}

/// Build `(subtool, json_args)` for a `craft_*` uniform facade call (`T3.5`).
fn build_verb_invocation(
    spec: &AppSpec,
    verb: &str,
    target: VerbTarget,
    args: &Value,
) -> Result<(&'static str, Value)> {
    match target {
        VerbTarget::None => Err(Error::Invalid(format!(
            "{} does not support `{verb}`",
            spec.name
        ))),
        VerbTarget::Command {
            runner,
            id_field,
            cmd_id,
            params_field,
        } => {
            let mut params = serde_json::Map::new();
            if let Some(obj) = args.as_object() {
                for (k, v) in obj {
                    if k != "app" {
                        params.insert(k.clone(), v.clone());
                    }
                }
            }
            Ok((
                runner,
                json!({
                    id_field: cmd_id,
                    params_field: Value::Object(params),
                }),
            ))
        }
        VerbTarget::Tool(subtool) => {
            let mut out = serde_json::Map::new();
            if let Some(obj) = args.as_object() {
                for (k, v) in obj {
                    if k != "app" {
                        out.insert(k.clone(), v.clone());
                    }
                }
            }
            match verb {
                "craft_open" => {
                    if let Some(p) = out.get("path").cloned() {
                        if subtool == "cmd_library_import" || subtool == "media_import" {
                            out.remove("path");
                            out.insert("paths".into(), json!([p]));
                        } else if subtool == "core_evidence_import" {
                            out.remove("path");
                            out.insert("file".into(), p);
                        }
                    }
                }
                "craft_export" => {
                    if let Some(p) = out.get("path").cloned() {
                        if matches!(subtool, "textures_export" | "outputs_export") {
                            out.remove("path");
                            out.insert("dir".into(), p);
                        } else if subtool == "cmd_export_run" {
                            out.remove("path");
                            out.insert("dest".into(), p);
                        }
                    }
                }
                "craft_render" => {
                    if spec.id == "deckcraft" && !out.contains_key("slide") {
                        let slide = out.remove("page").unwrap_or_else(|| json!(0));
                        out.insert("slide".into(), slide);
                    }
                    if spec.id == "pdfcraft"
                        && subtool == "page_render"
                        && !out.contains_key("page")
                    {
                        out.insert("page".into(), json!(0));
                    }
                }
                "craft_commands" => {
                    if let Some(f) = out.remove("filter") {
                        out.insert("query".into(), f.clone());
                        out.insert("filter".into(), f);
                    }
                }
                "craft_run" => {
                    let cmd = args
                        .get("command")
                        .and_then(Value::as_str)
                        .ok_or_else(|| Error::Invalid("missing argument 'command'".into()))?;
                    let params = args.get("params").cloned().unwrap_or_else(|| json!({}));
                    out.clear();
                    match subtool {
                        "command_run" => {
                            out.insert("id".into(), Value::String(cmd.into()));
                            out.insert("params".into(), params);
                        }
                        "tool_call" => {
                            out.insert("name".into(), Value::String(cmd.into()));
                            out.insert("arguments".into(), params);
                        }
                        _ => {
                            out.insert("command".into(), Value::String(cmd.into()));
                            out.insert("params".into(), params);
                        }
                    }
                }
                "craft_batch" => {
                    let steps = args
                        .get("steps")
                        .or_else(|| args.get("commands"))
                        .cloned()
                        .unwrap_or_else(|| json!([]));
                    out.clear();
                    if subtool == "command_batch" {
                        out.insert("steps".into(), steps);
                    } else {
                        out.insert("commands".into(), steps);
                    }
                }
                _ => {}
            }
            Ok((subtool, Value::Object(out)))
        }
    }
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent("WorkCraft/1.0 (https://artcraft.local; support@artcraft.local) Mozilla/5.0")
        .build()
        .map_err(|e| Error::Io(e.to_string()))
}

fn check_public_url(raw: &str) -> Result<()> {
    let url = raw.trim();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| Error::Invalid("URL must start with https:// or http://".into()))?;
    let host = rest
        .split(['/', ':', '?', '#'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host.ends_with(".local")
        || host.starts_with("127.")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("169.254.")
        || host == "0.0.0.0"
        || host.contains(':')
    {
        return Err(Error::Denied("URL must be a public web address".into()));
    }
    Ok(())
}

fn is_binary_content(ct: &str, bytes: &[u8]) -> bool {
    if ct.starts_with("image/")
        || ct.starts_with("audio/")
        || ct.starts_with("video/")
        || ct.contains("application/pdf")
        || ct.contains("application/octet-stream")
        || ct.contains("application/zip")
    {
        return true;
    }
    if bytes.starts_with(&[0x89, b'P', b'N', b'G'])
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"GIF8")
        || (bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP")
        || bytes.starts_with(b"%PDF-")
        || bytes.starts_with(&[b'P', b'K', 0x03, 0x04])
    {
        return true;
    }
    bytes.iter().take(512).any(|&b| b == 0)
}

fn infer_download_filename(url: &str, ct: &str, bytes: &[u8]) -> String {
    let ext = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) || ct.contains("png") {
        "png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) || ct.contains("jpeg") || ct.contains("jpg") {
        "jpg"
    } else if bytes.starts_with(b"GIF8") || ct.contains("gif") {
        "gif"
    } else if (bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP")
        || ct.contains("webp")
    {
        "webp"
    } else if ct.contains("svg") {
        "svg"
    } else if bytes.starts_with(b"%PDF-") || ct.contains("pdf") {
        "pdf"
    } else {
        "bin"
    };

    let path_part = url.split(['?', '#']).next().unwrap_or(url);
    let raw_last = path_part.rsplit('/').next().unwrap_or("downloaded");
    let stem: String = raw_last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = stem.trim_matches('_');
    let base = if stem.is_empty() { "image" } else { stem };
    let lower = base.to_ascii_lowercase();
    if lower.ends_with(&format!(".{ext}")) {
        base.to_string()
    } else {
        let stripped = base
            .rsplit_once('.')
            .map_or(base, |(s, _)| if s.is_empty() { base } else { s });
        let truncated: String = stripped.chars().take(64).collect();
        format!("{truncated}.{ext}")
    }
}

fn extract_wikimedia_filename(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if let Some(thumb_idx) = segments.iter().position(|&s| s == "thumb")
        && segments.len() > thumb_idx + 3
    {
        return Some(segments[thumb_idx + 3].to_string());
    }
    segments.last().map(|s| (*s).to_string())
}

fn extract_search_topic(url_or_query: &str) -> String {
    let base = if url_or_query.starts_with("http://") || url_or_query.starts_with("https://") {
        extract_wikimedia_filename(url_or_query).unwrap_or_default()
    } else {
        url_or_query.to_string()
    };
    let without_ext = base
        .rsplit_once('.')
        .map_or(base.as_str(), |(stem, _)| stem);
    without_ext
        .replace(['_', '-', '%'], " ")
        .split_whitespace()
        .filter(|w| !w.chars().all(|c| c.is_ascii_hexdigit()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

async fn search_wikimedia(query: &str, limit: usize) -> Result<Vec<Value>> {
    let client = http_client()?;
    let encoded = url_encode(&format!("filetype:bitmap {query}"));
    let api_url = format!(
        "https://commons.wikimedia.org/w/api.php?action=query&generator=search&gsrsearch={encoded}&gsrnamespace=6&gsrlimit={limit}&prop=imageinfo&iiprop=url|dimensions|mime&iiurlwidth=1280&format=json"
    );
    let resp = client
        .get(&api_url)
        .send()
        .await
        .map_err(|e| Error::Io(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(Error::Io(format!(
            "Wikimedia Commons returned HTTP {}",
            resp.status()
        )));
    }
    let body: Value = resp.json().await.map_err(|e| Error::Io(e.to_string()))?;
    let Some(pages) = body
        .get("query")
        .and_then(|q| q.get("pages"))
        .and_then(Value::as_object)
    else {
        return Ok(Vec::new());
    };
    let mut results = Vec::new();
    for page in pages.values() {
        let title = page.get("title").and_then(Value::as_str).unwrap_or("");
        let Some(info) = page
            .get("imageinfo")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
        else {
            continue;
        };
        let thumb_url = info.get("thumburl").and_then(Value::as_str);
        let full_url = info.get("url").and_then(Value::as_str);
        let Some(download_url) = thumb_url.or(full_url) else {
            continue;
        };
        results.push(json!({
            "title": title,
            "download_url": download_url,
            "original_url": full_url,
            "width": info.get("thumbwidth").or_else(|| info.get("width")),
            "height": info.get("thumbheight").or_else(|| info.get("height")),
            "mime": info.get("mime"),
        }));
    }
    Ok(results)
}

fn csv(rows: &[Vec<Value>]) -> String {
    let cell = |v: &Value| {
        let t = match v {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            o => o.to_string(),
        };
        if t.contains([',', '"', '\n']) {
            format!("\"{}\"", t.replace('"', "\"\""))
        } else {
            t
        }
    };
    rows.iter()
        .map(|r| r.iter().map(cell).collect::<Vec<_>>().join(","))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn list(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) -> Result<()> {
    let rd = std::fs::read_dir(dir).map_err(|e| Error::Io(format!("{}: {e}", dir.display())))?;
    let mut entries: Vec<std::fs::DirEntry> = rd.filter_map(std::result::Result::ok).collect();
    entries.sort_by_key(|e| {
        (
            !e.path().is_dir(),
            e.file_name().to_string_lossy().to_lowercase(),
        )
    });
    for e in entries {
        if out.len() > MAX_LIST {
            return Ok(());
        }
        let p = e.path();
        let rel = p.strip_prefix(root).unwrap_or(&p).display().to_string();
        let Ok(m) = e.metadata() else { continue };
        if m.is_dir() {
            out.push(format!("{rel}/"));
            if depth > 1 {
                let _ = list(root, &p, depth - 1, out);
            }
        } else {
            out.push(format!("{rel}  {}  {}", human_size(m.len()), modified(&m)));
        }
    }
    Ok(())
}

fn name_matches(name: &str, pat: &str) -> bool {
    if pat.is_empty() {
        return true;
    }
    let name = name.to_lowercase();
    if !pat.contains('*') {
        return name.contains(pat);
    }
    let parts: Vec<&str> = pat.split('*').collect();
    let mut pos = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        match name[pos..].find(part) {
            Some(at) if i > 0 || at == 0 => pos += at + part.len(),
            _ => return false,
        }
    }
    parts
        .last()
        .is_none_or(|l| l.is_empty() || name.ends_with(l))
}

fn search(dir: &Path, pat: &str, text: Option<&str>, depth: usize, hits: &mut Vec<String>) {
    if depth > 8 || hits.len() >= 200 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<std::fs::DirEntry> = rd.filter_map(std::result::Result::ok).collect();
    entries.sort_by_key(|e| {
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        let priority = match name.as_str() {
            "pictures" | "photos" | "images" | "documents" | "desktop" | "downloads" => 0,
            _ => 1,
        };
        (priority, name)
    });
    for e in entries {
        let p = e.path();
        let n = e.file_name().to_string_lossy().into_owned();
        let lower = n.to_ascii_lowercase();
        if n.starts_with('.')
            || matches!(
                lower.as_str(),
                "node_modules"
                    | "target"
                    | "$recycle.bin"
                    | "system volume information"
                    | "appdata"
                    | "application data"
                    | "local settings"
                    | "programdata"
                    | "windows"
                    | "program files"
                    | "program files (x86)"
                    | "library"
                    | "caches"
            )
        {
            continue;
        }
        if p.is_dir() {
            search(&p, pat, text, depth + 1, hits);
        } else if name_matches(&n, pat) {
            let ok = match text {
                None => true,
                Some(t) => {
                    std::fs::metadata(&p).map_or(0, |m| m.len()) < 50 * 1024 * 1024
                        && docs::extract_text(&p).is_ok_and(|c| c.to_lowercase().contains(t))
                }
            };
            if ok {
                let size = std::fs::metadata(&p).map_or(0, |m| m.len());
                hits.push(format!("{}  {}", p.display(), human_size(size)));
            }
        }
        if hits.len() >= 200 {
            return;
        }
    }
}

/// `a/b/../c` -> `a/c` without touching the disk.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The canonical form of the nearest existing ancestor, with the rest appended (follows symlinks).
fn canonical_or_self(p: &Path) -> PathBuf {
    let mut anc = p.to_path_buf();
    let mut tail = Vec::new();
    while !anc.exists() {
        match anc.file_name() {
            Some(n) => {
                tail.push(n.to_owned());
                anc.pop();
            }
            None => return p.to_path_buf(),
        }
    }
    let mut full = anc.canonicalize().unwrap_or(anc);
    for t in tail.iter().rev() {
        full.push(t);
    }
    full
}

// Keep `ImagePart` imported and tested directly via artifact saving.
#[allow(dead_code)]
fn _assert_image_part(_p: &ImagePart) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (PathBuf, Tools, ToolCtx) {
        let root = std::env::temp_dir().join(format!("wc-tools-{}", uuid::Uuid::new_v4().simple()));
        let ws = root.join("workspace");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(root.join("config.json"), "{\"secret\":1}").unwrap();
        let tools = Tools::from_config(
            ws,
            Arc::new(Skills::default()),
            McpHub::new(),
            BTreeMap::new(),
            root.clone(),
            Arc::new(Checkpoints::new(root.join("checkpoints"))),
            &Config::default(),
            false,
        );
        let ctx = ToolCtx {
            session: "s".into(),
            agent: None,
            memory: Arc::new(Memory::default()),
            turn: "t1".into(),
        };
        (root, tools, ctx)
    }

    #[tokio::test]
    async fn file_tools_work_inside_and_outside_the_workspace() {
        let (root, tools, ctx) = setup();
        let w = tools
            .call(
                &ctx,
                "write_file",
                json!({"path":"notes/a.txt","content":"hello world"}),
            )
            .await;
        assert!(w.ok, "{}", w.text);
        assert_eq!(
            tools
                .call(&ctx, "read_file", json!({"path":"notes/a.txt"}))
                .await
                .text,
            "hello world"
        );
        assert!(
            tools
                .call(&ctx, "list_dir", json!({"depth": 2}))
                .await
                .text
                .contains("a.txt")
        );
        let outside = root.join("outside.txt");
        std::fs::write(&outside, "from the desktop").unwrap();
        let path = outside.display().to_string();
        assert_eq!(
            tools
                .call(&ctx, "read_file", json!({"path": path}))
                .await
                .text,
            "from the desktop"
        );
        assert!(tools.needs_approval("read_file", &json!({"path": path}), true));
        assert!(!tools.needs_approval("read_file", &json!({"path": "notes/a.txt"}), true));
        assert!(tools.needs_approval("write_file", &json!({"path": "notes/a.txt"}), false));
        let key = root.join("config.json").display().to_string();
        assert!(!tools.call(&ctx, "read_file", json!({"path": key})).await.ok);
        if cfg!(windows) {
            let r = tools
                .call(
                    &ctx,
                    "write_file",
                    json!({"path":"C:\\Windows\\wc-test.txt","content":"x"}),
                )
                .await;
            assert!(!r.ok && r.text.contains("system folder"), "{}", r.text);
        }
        assert!(
            tools
                .call(
                    &ctx,
                    "edit_file",
                    json!({"path":"notes/a.txt","find":"world","replace":"there"})
                )
                .await
                .ok
        );
        assert_eq!(
            std::fs::read_to_string(tools.workspace.join("notes/a.txt")).unwrap(),
            "hello there"
        );
        assert!(
            !tools
                .call(
                    &ctx,
                    "edit_file",
                    json!({"path":"notes/a.txt","find":"nope","replace":"x"})
                )
                .await
                .ok
        );
        assert!(!tools.call(&ctx, "read_file", json!({})).await.ok);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn office_files_are_created_read_searched_and_undone() {
        let (root, tools, ctx) = setup();
        let r = tools.call(&ctx, "create_document", json!({"path":"report.docx","title":"Report","content":"# Intro\n\nHello **team**."})).await;
        assert!(r.ok, "{}", r.text);
        assert!(
            tools
                .call(&ctx, "read_file", json!({"path":"report.docx"}))
                .await
                .text
                .contains("Hello team.")
        );
        let r = tools.call(&ctx, "create_spreadsheet", json!({"path":"sales.xlsx","sheets":[{"name":"Q3","rows":[["Region","Sales"],["North",10]]}]})).await;
        assert!(r.ok, "{}", r.text);
        assert!(
            tools
                .call(&ctx, "read_file", json!({"path":"sales.xlsx"}))
                .await
                .text
                .contains("North\t10")
        );
        let r = tools.call(&ctx, "create_presentation", json!({"path":"deck.pptx","slides":[{"title":"Hi"},{"title":"Plan","bullets":["One","Two"]}]})).await;
        assert!(r.ok, "{}", r.text);
        assert!(
            tools
                .call(
                    &ctx,
                    "edit_file",
                    json!({"path":"report.docx","find":"team","replace":"crew"})
                )
                .await
                .ok
        );
        let found = tools
            .call(&ctx, "search_files", json!({"name":"*.docx","text":"crew"}))
            .await
            .text;
        assert!(found.contains("report.docx"), "{found}");
        assert!(
            !tools
                .call(&ctx, "write_file", json!({"path":"x.docx","content":"no"}))
                .await
                .ok
        );
        let restored = tools.checkpoints.restore("s", "t1").unwrap();
        assert!(restored["failed"].as_array().unwrap().is_empty());
        assert!(!tools.workspace.join("report.docx").exists());
        assert!(!tools.workspace.join("deck.pptx").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn globs_and_paths() {
        assert!(name_matches("Report.DOCX", "*.docx"));
        assert!(name_matches("q3-report.docx", "report"));
        assert!(!name_matches("notes.txt", "*.docx"));
        assert!(name_matches("a-b-c.txt", "a*c*"));
        assert_eq!(normalize(Path::new("a/b/../c")), PathBuf::from("a/c"));
    }

    #[tokio::test]
    async fn memory_is_shared_through_the_context() {
        let (root, tools, ctx) = setup();
        tools
            .call(&ctx, "memory_set", json!({"key":"palette","value":"teal"}))
            .await;
        let other = ToolCtx {
            agent: Some("b".into()),
            ..ctx.clone()
        };
        assert_eq!(
            tools
                .call(&other, "memory_get", json!({"key":"palette"}))
                .await
                .text,
            "teal"
        );
        assert!(
            tools
                .call(&other, "memory_list", json!({}))
                .await
                .text
                .contains("palette")
        );
        assert!(
            !tools
                .call(&other, "memory_get", json!({"key":"nope"}))
                .await
                .ok
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn the_memory_board_has_a_key_limit_but_keys_can_be_updated() {
        let (root, tools, ctx) = setup();
        for i in 0..MAX_MEMORY_KEYS {
            assert!(
                tools
                    .call(
                        &ctx,
                        "memory_set",
                        json!({"key": format!("k{i}"), "value": "v"})
                    )
                    .await
                    .ok
            );
        }
        assert!(
            !tools
                .call(
                    &ctx,
                    "memory_set",
                    json!({"key": "one-too-many", "value": "v"})
                )
                .await
                .ok
        );
        assert!(
            tools
                .call(&ctx, "memory_set", json!({"key": "k0", "value": "new"}))
                .await
                .ok
        );
        assert_eq!(
            tools
                .call(&ctx, "memory_get", json!({"key": "k0"}))
                .await
                .text,
            "new"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn unknown_tools_fail_cleanly_and_specs_are_listed() {
        let (root, tools, ctx) = setup();
        assert!(!tools.call(&ctx, "nope", json!({})).await.ok);
        assert!(!tools.call(&ctx, "ghost__x", json!({})).await.ok);
        let names: Vec<String> = tools.specs().await.into_iter().map(|t| t.name).collect();
        for n in [
            "read_file",
            "create_document",
            "create_spreadsheet",
            "create_presentation",
            "edit_file",
            "download_file",
            "search_wikimedia_images",
            "craft_status",
            "craft_activate",
            "craft_handoff",
        ] {
            assert!(names.contains(&n.to_string()), "{n}");
        }
        assert!(
            !names.contains(&"craft_run".to_string()),
            "no control targets, no control tools"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn relative_read_write_roots_rewrite_paths_and_save_returned_images() {
        let (root, tools, ctx) = setup();
        std::fs::create_dir_all(tools.workspace.join("textures")).unwrap();
        let mat = find_spec("materialcraft").unwrap();
        let mut args = json!({
            "dir": tools.workspace.display().to_string(),
            "path": "textures/albedo.png"
        });
        tools
            .resolve_or_download_mcp_paths(
                &ctx,
                "materialcraft__outputs_export",
                Some(mat),
                CraftMode::Headless,
                &mut args,
            )
            .await
            .unwrap();
        assert_eq!(args["dir"], ".");
        assert_eq!(args["path"], "textures/albedo.png");

        // Outside workspace is denied by root with typed error.
        let outside = root.join("outside.png");
        let mut bad_args = json!({ "path": outside.display().to_string() });
        let err = tools
            .resolve_or_download_mcp_paths(
                &ctx,
                "photocraft__doc_open",
                find_spec("photocraft"),
                CraftMode::Headless,
                &mut bad_args,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code(), "denied_by_root");

        // Returned ImagePart is saved to `.workcraft/artifacts/<session>/` with dimensions.
        let mut img = image::RgbImage::new(4, 3);
        img.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        let mut png_bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut png_bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
        let mut res = ToolResult::ok("rendered".to_string());
        res.images.push(ImagePart {
            mime_type: "image/png".into(),
            data: b64,
        });
        let saved = tools.persist_returned_images("sess1", "c42", &mut res);
        assert_eq!(saved.len(), 1);
        assert!(Path::new(&saved[0]).is_file());
        assert!(res.text.contains("(4x3)"), "{}", res.text);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn search_files_skips_appdata_and_detects_binary_magic() {
        let (root, tools, ctx) = setup();
        let appdata = tools.workspace.join("AppData").join("Local");
        let pictures = tools.workspace.join("Pictures");
        std::fs::create_dir_all(&appdata).unwrap();
        std::fs::create_dir_all(&pictures).unwrap();
        std::fs::write(appdata.join("cache_icon.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        std::fs::write(pictures.join("photo.png"), b"\x89PNG\r\n\x1a\n").unwrap();

        let res = tools
            .call(&ctx, "search_files", json!({ "name": "*.png" }))
            .await;
        assert!(res.ok);
        assert!(res.text.contains("photo.png"), "{}", res.text);
        assert!(!res.text.contains("cache_icon.png"), "{}", res.text);

        assert!(is_binary_content("image/jpeg", b"\xFF\xD8\xFF\xE0"));
        assert!(is_binary_content("text/plain", b"\x89PNG\r\n\x1a\n"));
        assert!(!is_binary_content(
            "text/html",
            b"<html><body>Hello</body></html>"
        ));
        assert_eq!(
            extract_wikimedia_filename("https://upload.wikimedia.org/wikipedia/commons/thumb/2/2f/Flag_of_the_United_Nations.svg/320px-Flag_of_the_United_Nations.svg.png").as_deref(),
            Some("Flag_of_the_United_Nations.svg")
        );
        let _ = std::fs::remove_dir_all(root);
    }
}

/// Small models fill every optional parameter with `null`; the Craft servers reject that
/// ("invalid type: null, expected a boolean"). A null optional means "not given", so drop those keys.
fn strip_null_args(args: &mut Value) {
    if let Some(obj) = args.as_object_mut() {
        obj.retain(|_, v| !v.is_null());
    }
}

#[cfg(test)]
mod null_arg_tests {
    use super::*;

    #[test]
    fn null_optionals_are_dropped_but_values_stay() {
        let mut a =
            json!({"format": "png", "index": null, "path": "a.png", "quality": 0, "flag": false});
        strip_null_args(&mut a);
        assert_eq!(
            a,
            json!({"format": "png", "path": "a.png", "quality": 0, "flag": false})
        );
    }
}
