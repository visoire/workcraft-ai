//! Settings and definitions loaded from the data folder. A missing file is the default, a damaged file
//! is an error the caller can show; neither stops the app.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use workcraft_core::team::ApprovalMode;
use workcraft_core::{AgentDef, Error, ModelRef, Result, TeamConfig};
use workcraft_llm::ProviderConfig;

use crate::craft::{CraftMode, RootsContext};

fn is_false(b: &bool) -> bool {
    !*b
}

/// `<data>/config.json`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// Default model, `provider:model`.
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub approval: ApprovalMode,
    /// The folder the file tools may touch. Empty: `<data>/workspace`.
    #[serde(default)]
    pub workspace: Option<PathBuf>,
    /// Additional user-authorized read roots passed to Craft MCP servers (never `$HOME` by default).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_roots: Vec<PathBuf>,
    /// Extra providers. Ollama is always there; OpenAI and Anthropic appear when their key variables are set.
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    /// Prices in US dollars per million tokens, by `model` or `provider:model`. They override the built-in
    /// rates, and give a price to models that have none (cost is never guessed).
    #[serde(default)]
    pub prices: BTreeMap<String, workcraft_llm::price::Price>,
    /// Appended to the base system prompt.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Context window for local models; `None` lets the adapter decide.
    #[serde(default)]
    pub num_ctx: Option<u32>,
    /// Context window to plan with for hosted and OpenAI-compatible models (tokens). `None` picks one from
    /// the model name.
    #[serde(default)]
    pub context_window: Option<u32>,
    /// `provider:model` to continue with, once per turn, when the chosen model cannot be reached or is
    /// overloaded (for example a cloud model with `ollama:qwen2.5:7b` behind it).
    #[serde(default)]
    pub fallback_model: Option<String>,
    /// Send a request only the tools of the Craft apps the chat is about (`find_tools` reaches the rest).
    /// Turn it off to send every connected tool.
    #[serde(default = "yes")]
    pub tool_routing: bool,
    /// Reasoning on or off for models that support it; `None` keeps the model default.
    #[serde(default)]
    pub think: Option<bool>,
    /// Turns of tool use per message before the loop stops.
    #[serde(default = "default_steps")]
    pub max_steps: usize,
    /// Craft apps switched on or off as startup MCP servers (app id -> on). Missing: the default
    /// (the document apps are on, see [`crate::craft::ON_BY_DEFAULT`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub craft_apps: BTreeMap<String, bool>,
    /// Per-app execution mode (`headless`, `live`, or `off`). Missing defaults to `headless`
    /// (`off` for `craft-anything` unless enabled).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub craft_modes: BTreeMap<String, CraftMode>,
    /// Expose every tool reported by Craft MCP servers instead of the curated lean profile.
    #[serde(default, skip_serializing_if = "is_false")]
    pub full_tool_catalog: bool,
    /// Opt-in `--deep` flag for Craft Anything (default `false` enforces `--clean-room`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub craft_anything_deep: bool,
    /// Opt-in `--allow-run` flag for Craft Anything (default `false`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub craft_anything_allow_run: bool,
}

fn default_model() -> String {
    "ollama:qwen3:4b".to_string()
}

fn default_steps() -> usize {
    64
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: default_model(),
            approval: ApprovalMode::Manual,
            workspace: None,
            extra_roots: Vec::new(),
            providers: Vec::new(),
            prices: BTreeMap::new(),
            system_prompt: None,
            num_ctx: None,
            context_window: None,
            fallback_model: None,
            tool_routing: true,
            think: None,
            max_steps: default_steps(),
            craft_apps: BTreeMap::new(),
            craft_modes: BTreeMap::new(),
            full_tool_catalog: false,
            craft_anything_deep: false,
            craft_anything_allow_run: false,
        }
    }
}

impl Config {
    pub fn load(data_dir: &Path) -> Result<Self> {
        let mut cfg: Self =
            read_json(&data_dir.join("config.json")).map(Option::unwrap_or_default)?;
        if cfg.max_steps <= 24 {
            cfg.max_steps = default_steps();
        }
        Ok(cfg)
    }

    pub fn save(&self, data_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(data_dir)?;
        std::fs::write(
            data_dir.join("config.json"),
            serde_json::to_string_pretty(self)?,
        )?;
        Ok(())
    }

    #[must_use]
    pub fn model_ref(&self) -> ModelRef {
        ModelRef::parse(&self.model)
    }

    #[must_use]
    pub fn roots_context(&self, workspace: PathBuf, data_dir: PathBuf) -> RootsContext {
        RootsContext {
            workspace: Some(workspace),
            extra_roots: self.extra_roots.clone(),
            data_dir: Some(data_dir),
            craft_anything_deep: self.craft_anything_deep,
            craft_anything_allow_run: self.craft_anything_allow_run,
        }
    }
}

/// Where a Craft app's control channel listens. `<data>/control-targets.json`:
/// `{"photocraft":{"addr":"127.0.0.1:7878","token_file":"C:/.../token"}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlTarget {
    pub addr: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub token_file: Option<PathBuf>,
}

impl ControlTarget {
    pub fn resolve_token(&self) -> Result<String> {
        if let Some(t) = self.token.as_deref().filter(|t| !t.is_empty()) {
            return Ok(t.to_string());
        }
        if let Some(f) = &self.token_file {
            return Ok(std::fs::read_to_string(f)?.trim().to_string());
        }
        Err(Error::Invalid(
            "control target has no token or token_file".into(),
        ))
    }
}

pub fn load_control_targets(data_dir: &Path) -> Result<BTreeMap<String, ControlTarget>> {
    read_json(&data_dir.join("control-targets.json")).map(Option::unwrap_or_default)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>> {
    match std::fs::read_to_string(path) {
        Ok(t) if t.trim().is_empty() => Ok(None),
        Ok(t) => serde_json::from_str(&t)
            .map(Some)
            .map_err(|e| Error::Invalid(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Every `*.json` in `dir` that parses as `T`. Returns the good ones and the names of the bad ones.
pub fn load_dir<T: for<'de> Deserialize<'de>>(dir: &Path) -> (Vec<T>, Vec<String>) {
    let mut good = Vec::new();
    let mut bad = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (good, bad);
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for p in paths
        .into_iter()
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
    {
        match read_json::<T>(&p) {
            Ok(Some(v)) => good.push(v),
            _ => bad.push(p.display().to_string()),
        }
    }
    (good, bad)
}

fn agent(
    id: &str,
    name: &str,
    scope: &str,
    prompt: &str,
    tools: &[&str],
    skills: &[&str],
) -> AgentDef {
    AgentDef {
        id: id.to_string(),
        name: name.to_string(),
        scope: scope.to_string(),
        prompt: prompt.to_string(),
        model: None,
        tools: tools.iter().map(|s| (*s).to_string()).collect(),
        skills: skills.iter().map(|s| (*s).to_string()).collect(),
        approval: None,
    }
}

const SHARED_SPECIALIST_TOOLS: &[&str] = &[
    "read_file",
    "list_dir",
    "search_files",
    "file_info",
    "copy_file",
    "download_file",
    "search_wikimedia_images",
    "fetch_url",
    "memory_*",
    "read_skill",
    "list_skills",
    "craft_status",
    "craft_activate",
    "craft_open",
    "craft_new",
    "craft_inspect",
    "craft_render",
    "craft_export",
    "craft_save",
    "craft_batch",
    "craft_handoff",
];

fn specialist_tools(apps: &[&str]) -> Vec<String> {
    SHARED_SPECIALIST_TOOLS
        .iter()
        .map(|s| (*s).to_string())
        .chain(apps.iter().map(|a| format!("{a}__*")))
        .collect()
}

/// Optional specialist for Craft Anything clean-room software analysis (added when Craft Anything is enabled).
#[must_use]
pub fn explorer_agent() -> AgentDef {
    let mut def = agent(
        "explorer",
        "Software explorer",
        "Clean-room binary, archive, and evidence analysis: Craft Anything.",
        "You inspect software targets and record clean-room evidence through Craft Anything. Stay within clean-room boundaries and summarize verified findings clearly.",
        &[],
        &["craft-anything"],
    );
    def.tools = specialist_tools(&["craft-anything"]);
    def
}

/// Id of the agent that works only in one Craft app (`wordcraft-agent`, `photocraft-agent`, ...).
#[must_use]
pub fn app_agent_id(app_id: &str) -> String {
    format!("{app_id}-agent")
}

/// Is `id` one of the per-app agents? (Craft Anything has the explorer instead.)
#[must_use]
pub fn is_app_agent(id: &str) -> bool {
    crate::craft::specs()
        .iter()
        .any(|s| s.id != "craft-anything" && app_agent_id(s.id) == id)
}

/// One agent for every Craft app (Craft Anything is covered by the explorer): it sees the tools of its own app and the
/// shared read and handoff tools, and nothing else. They are available as `@wordcraft-agent` and in the `apps` team.
#[must_use]
pub fn app_agents() -> Vec<AgentDef> {
    crate::craft::specs()
        .iter()
        .filter(|s| s.id != "craft-anything")
        .map(|s| {
            let mut def = agent(
                &app_agent_id(s.id),
                &format!("{} agent", s.name),
                &format!("{} ({}): everything that is done inside {}.", s.name, s.kind, s.name),
                &format!(
                    "You are the {name} agent ({kind}). You work only through {name}: read the `{id}` skill first, inspect the file or project before you change it, change only what was asked, check the result with an inspect or render call, and report the exact path of every file you wrote. When the task needs another app, say so and stage the output with craft_handoff instead of guessing.",
                    name = s.name,
                    kind = s.kind,
                    id = s.id
                ),
                &[],
                &[s.id],
            );
            def.tools = specialist_tools(&[s.id]);
            def
        })
        .collect()
}

/// The `apps` team: a leader and one agent per Craft app (`workers`). The leader sends each task to the agent of the app
/// it belongs to and moves files between them with craft_handoff.
#[must_use]
pub fn apps_team(workers: Vec<AgentDef>) -> TeamConfig {
    let map: String = workers
        .iter()
        .map(|w| format!("- {}: {}\n", w.id, w.scope))
        .collect();
    let leader_prompt = format!(
        "You lead a team with one agent for every Craft app. You never call app tools yourself.\nAgents:\n{map}Give each task to the agent of the app it belongs to. Run independent tasks in parallel and add depends_on when one agent needs a file another agent exports (staged with craft_handoff). Keep every task small: one app, one result."
    );
    TeamConfig {
        name: "apps".to_string(),
        leader: agent(
            "leader",
            "Team leader",
            "Plans the work, sends each task to the agent of the right app, reviews the results and writes the final answer.",
            &leader_prompt,
            &[],
            &[],
        ),
        workers,
        concurrency: 4,
        max_retries: 1,
        max_tasks: 32,
        review: true,
        smart_review: true,
        direct_route: true,
        approval: ApprovalMode::Manual,
        plan_approval: None,
    }
}

/// The built-in team: a leader and five domain specialists covering the Craft creative and productivity suite.
/// Specialists get the tools of their apps (MCP servers named after the app) plus shared read and handoff tools.
#[must_use]
pub fn craft_team() -> TeamConfig {
    let mut image = agent(
        "image",
        "Image specialist",
        "Raster photos, layers, retouching, and RAW library development: PhotoCraft, LightCraft.",
        "You edit photos and develop RAW libraries through PhotoCraft and LightCraft. Inspect the image first, apply non-destructive edits, render a preview to verify, and stage shared outputs with craft_handoff when passing work to another specialist.",
        &[],
        &["photocraft", "lightcraft"],
    );
    image.tools = specialist_tools(&["photocraft", "lightcraft"]);

    let mut design = agent(
        "design",
        "Design & layout specialist",
        "Vector illustration, multi-page print/editorial layout, and UI design: VectorCraft, DesignCraft, FigCraft.",
        "You create vector artwork, editorial page layouts, and UI frames through VectorCraft, DesignCraft, and FigCraft. Read the app skill first, inspect geometry/frames before editing, render a preview, and export clean SVG, PDF, or PNG assets.",
        &[],
        &["vectorcraft", "designcraft", "figcraft"],
    );
    design.tools = specialist_tools(&["vectorcraft", "designcraft", "figcraft"]);

    let mut video = agent(
        "video",
        "Video and audio specialist",
        "Timeline editing, motion graphics/VFX compositing, and multitrack audio: FilmCraft, EffectCraft, SoundCraft.",
        "You cut timelines, build motion compositions, and mix audio through FilmCraft, EffectCraft, and SoundCraft. Inspect project state before editing, verify frame renders or loudness metrics, and export final media into the workspace.",
        &[],
        &["filmcraft", "effectcraft", "soundcraft"],
    );
    video.tools = specialist_tools(&["filmcraft", "effectcraft", "soundcraft"]);

    let mut document = agent(
        "document",
        "Document specialist",
        "Word documents, spreadsheets, presentations, and PDFs: WordCraft, GridCraft, DeckCraft, PdfCraft.",
        "You author and edit documents, spreadsheets, slide decks, and PDFs through WordCraft, GridCraft, DeckCraft, and PdfCraft. Inspect the file first, change only what was requested, and verify pages/slides.",
        &[],
        &["wordcraft", "gridcraft", "deckcraft", "pdfcraft"],
    );
    document.tools = specialist_tools(&["wordcraft", "gridcraft", "deckcraft", "pdfcraft"]);
    for extra in [
        "write_file",
        "edit_file",
        "create_document",
        "create_spreadsheet",
        "create_presentation",
    ] {
        document.tools.push(extra.to_string());
    }

    let mut three_d = agent(
        "three-d",
        "3D, CAD & materials specialist",
        "3D PBR texture painting, procedural materials, parametric solid modeling, and 2D/3D CAD drafting: TextureCraft, MaterialCraft, SolidCraft, CADCraft.",
        "You author procedural materials, paint PBR textures, model parametric 3D parts, and draft CAD drawings through TextureCraft, MaterialCraft, SolidCraft, and CADCraft. Check topology/graph state, render previews, and stage exported meshes/maps with craft_handoff.",
        &[],
        &["texturecraft", "materialcraft", "solidcraft", "cadcraft"],
    );
    three_d.tools = specialist_tools(&["texturecraft", "materialcraft", "solidcraft", "cadcraft"]);

    let workers = vec![image, design, video, document, three_d];

    let leader_prompt = "You lead a team of Craft specialists. You never call app tools yourself.\n\
Specialist capability map:\n\
- image: PhotoCraft (raster/layers/exports: png, jpg, webp, tiff), LightCraft (RAW catalog/grades/exports: jpg, tiff)\n\
- design: VectorCraft (SVG/PDF/PNG illustration), DesignCraft (page layout/publishing/PDF), FigCraft (UI frames/components/SVG/PNG)\n\
- video: FilmCraft (timeline editing/MP4/WAV), EffectCraft (motion graphics/VFX/MP4/PNG), SoundCraft (DAW audio/WAV/FLAC/MP3)\n\
- document: WordCraft (DOCX/PDF/MD), GridCraft (XLSX/CSV/charts), DeckCraft (PPTX/PDF/slide PNGs), PdfCraft (PDF merge/forms/OCR/annotations)\n\
- three-d: TextureCraft (3D mesh PBR painting/GLB/PNG maps), MaterialCraft (procedural PBR node graphs/MCMAT/PNG), SolidCraft (parametric 3D B-Rep CAD: .sc, STEP/STL/3MF/OBJ export), CADCraft (DXF/SVG/PDF drafting)\n\
Assign independent tasks in parallel and add depends_on when one specialist consumes another specialist's exported file (staged via craft_handoff).";

    TeamConfig {
        name: "craft".to_string(),
        leader: agent(
            "leader",
            "Team leader",
            "Plans the work across Craft specialists, assigns tasks, reviews every result and writes the final answer.",
            leader_prompt,
            &[],
            &[],
        ),
        workers,
        concurrency: 3,
        max_retries: 1,
        max_tasks: 24,
        review: true,
        smart_review: true,
        direct_route: true,
        approval: ApprovalMode::Manual,
        plan_approval: None,
    }
}

pub fn builtin_agents() -> BTreeMap<String, AgentDef> {
    craft_team()
        .workers
        .into_iter()
        .map(|a| (a.id.clone(), a))
        .collect()
}

fn yes() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        std::env::temp_dir().join(format!("wc-cfg-{}", uuid::Uuid::new_v4().simple()))
    }

    #[test]
    fn missing_config_is_default_and_bad_config_is_an_error() {
        let d = tmp();
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(Config::load(&d).unwrap(), Config::default());
        std::fs::write(d.join("config.json"), "{nope").unwrap();
        assert!(Config::load(&d).is_err());
        Config::default().save(&d).unwrap();
        assert_eq!(Config::load(&d).unwrap().model, "ollama:qwen3:4b");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn partial_config_fills_defaults() {
        let c: Config =
            serde_json::from_str(r#"{"model":"anthropic:claude-sonnet-5-5","approval":"auto"}"#)
                .unwrap();
        assert_eq!(c.model_ref().provider, "anthropic");
        assert_eq!(c.approval, ApprovalMode::Auto);
        assert_eq!(c.max_steps, 64);
        assert!(c.craft_modes.is_empty());
        assert!(!c.full_tool_catalog);
    }

    #[test]
    fn craft_team_scopes_tools_by_app() {
        let t = craft_team();
        assert_eq!(t.workers.len(), 5);
        let image = &t.workers[0];
        assert_eq!(image.id, "image");
        assert!(image.allows("photocraft__command_run"));
        assert!(!image.allows("filmcraft__command_run"));
        assert!(image.allows("read_file"));
        assert!(image.allows("craft_handoff"));
        assert!(!image.allows("write_file"));
        assert!(!t.leader.allows("read_file"));

        // Every Craft app belongs to a specialist (16 in the 5 core workers + craft-anything in explorer).
        let mut all_workers = t.workers.clone();
        all_workers.push(explorer_agent());
        for spec in crate::craft::specs() {
            let probe = format!("{}__ping", spec.id);
            assert!(
                all_workers.iter().any(|w| w.allows(&probe)),
                "no specialist covers {}",
                spec.id
            );
        }
    }

    #[test]
    fn every_craft_app_but_craft_anything_has_its_own_agent() {
        let agents = app_agents();
        let apps: Vec<_> = crate::craft::specs()
            .iter()
            .filter(|s| s.id != "craft-anything")
            .collect();
        assert_eq!(agents.len(), apps.len());
        for (a, s) in agents.iter().zip(apps) {
            assert_eq!(a.id, format!("{}-agent", s.id));
            assert!(is_app_agent(&a.id));
            assert!(
                a.allows(&format!("{}__command_run", s.id)),
                "{} sees its own tools",
                a.id
            );
            assert!(a.allows("craft_handoff") && a.allows("read_file"));
            assert!(
                !a.allows("write_file"),
                "{} cannot write files by itself",
                a.id
            );
            // no other app's tools
            for other in crate::craft::specs().iter().filter(|o| o.id != s.id) {
                assert!(
                    !a.allows(&format!("{}__command_run", other.id)),
                    "{} must not see {}",
                    a.id,
                    other.id
                );
            }
            assert_eq!(a.skills, vec![s.id.to_string()]);
        }
        assert!(!is_app_agent("explorer") && !is_app_agent("image"));
    }

    #[test]
    fn the_apps_team_has_one_worker_per_agent_and_lists_them_for_the_leader() {
        let t = apps_team(app_agents());
        assert_eq!(t.name, "apps");
        assert_eq!(t.workers.len(), 16);
        assert!(
            t.leader.prompt.contains("wordcraft-agent")
                && t.leader.prompt.contains("solidcraft-agent")
        );
        assert!(!t.leader.allows("read_file"));
        assert!(t.plan_needs_approval());
    }

    #[test]
    fn an_agent_can_carry_its_own_approval() {
        let a: AgentDef =
            serde_json::from_str(r#"{"id":"a","name":"A","approval":"auto"}"#).unwrap();
        assert_eq!(a.approval, Some(ApprovalMode::Auto));
        let b: AgentDef = serde_json::from_str(r#"{"id":"b","name":"B"}"#).unwrap();
        assert_eq!(b.approval, None);
        assert!(!serde_json::to_string(&b).unwrap().contains("approval"));
    }

    #[test]
    fn load_dir_separates_good_and_bad() {
        let d = tmp();
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.json"), r#"{"id":"a","name":"A"}"#).unwrap();
        std::fs::write(d.join("b.json"), "oops").unwrap();
        std::fs::write(d.join("c.txt"), "ignored").unwrap();
        let (good, bad) = load_dir::<AgentDef>(&d);
        assert_eq!((good.len(), bad.len()), (1, 1));
        let _ = std::fs::remove_dir_all(d);
    }
}
