# WorkCraft Full ArtCraft MCP Integration — Handoff Status (`STATUS.md`)

**Generated:** 2026-11-06  
**Target Workspace:** `C:\xampp\htdocs\projects\artcraft\workcraft\workcraft-ai` (Pure Rust workspace)  
**Plan Reference:** `PLAN.md` & `AGENTS.md`

> **UPDATE 2026-11-06 (alpha.2):** every item in section 3 is done. apps_contract re-run green, apps_headless (22 tests) and apps_live (3 tests) written and passing against the real apps, version bumped to 0.1.0-alpha.2, CHANGELOG/README/AGENTS/ARCHITECTURE/packaging/SOURCES/docs/CRAFT-APPS.md/PLAN.md section 11 updated, fmt + clippy -D warnings + cargo test --workspace green. The text below is the original handoff, kept for history.

---

## 1. Executive Summary

WorkCraft's core architecture, MCP transport, vision/image pipeline, sandboxed headless/live orchestration, approval policy engine, cross-app uniform facade (`craft_*` + `craft_handoff`), Team specialist routing + per-app writer leases, bundled skills (17), app icons (17), CLI output, and egui desktop UI have been **implemented and verified compiling cleanly across the entire Rust workspace** (`cargo clippy --workspace --all-targets -- -D warnings` = 0 warnings, `cargo test --workspace` + live 17-app MCP contract/schema suite passing).

Work was paused cleanly at the user's request (`weekly limit at 5%`) right before writing the remaining opt-in real-app workflow suites (`apps_headless.rs`, `apps_live.rs`) and performing the release version bump + documentation pass.

---

## 2. What Was Completed

### Phase 0 — Baseline Hygiene & Live Probing (`T0.1–T0.3`)
- **Zero-warning clippy baseline (`T0.1`):** Fixed all 13 Rust 1.99 clippy lints across `workcraft-mcp`, `workcraft-agents`, and `workcraft-app`.
- **Live 17-App MCP Probe (`T0.2–T0.3`):** Created `crates/agents/examples/probe_apps.rs`, probed all **17 installed `<app>-cli mcp` servers** (`17/17 connected, 0 failed`), and committed golden tool-surface snapshots under `crates/agents/tests/snapshots/tools-<app>.json`:
  1. `wordcraft` (`WordCraft`)
  2. `gridcraft` (`GridCraft`)
  3. `deckcraft` (`DeckCraft`)
  4. `pdfcraft` (`PdfCraft`)
  5. `photocraft` (`PhotoCraft`)
  6. `vectorcraft` (`VectorCraft`)
  7. `lightcraft` (`LightCraft`)
  8. `designcraft` (`DesignCraft`)
  9. `filmcraft` (`FilmCraft`)
  10. `effectcraft` (`EffectCraft`)
  11. `soundcraft` (`SoundCraft`)
  12. `cadcraft` (`CADCraft`)
  13. `figcraft` (`FigCraft`)
  14. `texturecraft` (`TextureCraft`) — **newly added**
  15. `materialcraft` (`MaterialCraft`) — **newly added**
  16. `solidcraft` (`SolidCraft`) — **newly added (installed bonus app)**
  17. `craft-anything` (`Craft Anything`) — **newly added**

### Phase 1 — Unified 17-App Registry, Modes, Live Auth & Sandbox Roots (`T1.1–T1.8`)
- **Single Source of Truth (`T1.1`):** Deleted duplicate `McpConfig::craft_defaults()` from `crates/mcp/src/config.rs` and replaced legacy `crates/agents/src/craft.rs` with modular `crates/agents/src/craft/`:
  - `spec.rs`: Static `SPECS: [AppSpec; 17]` covering install paths, CLI/GUI names, env overrides, aliases, `RootsPolicy`, unique non-7878 control ports (`7981..7992`), `LiveBridgeKind`, `LiveAuth`, `lean_tools`, `live_only_tools`, per-tool timeouts, interchange formats, icons, skills, specialists, and `VerbMap`.
  - `discover.rs`: Cross-platform binary discovery (`<ENV>_CLI`, config `craft_apps` overrides, `PATH`, `Program Files`, `LOCALAPPDATA\Programs`, `ArtCraft/<Name>`) + cached `--version` probing.
  - `live.rs`: Explicit `CraftMode` (`Headless | Live | Off`), safe `headless_mcp_args` (`--automation-read-root`/`--automation-write-root`, `--read-root`/`--write-root`, `--allow-path`, `--clean-room --inbox`), live port probing + auth token wiring (`control-key.local-<port>`, PhotoCraft `--control-token-file`, FilmCraft 64-hex `FILMCRAFT_CONTROL_TOKEN`, TextureCraft/MaterialCraft `%TEMP%\<id>-control-<port>.token`, DesignCraft `~/.designcraft/control-<port>.token`, CADCraft `%LOCALAPPDATA%\cadcraft-control-<port>.token`), `ensure_headless`, `ensure_live_with_roots`, and own-PID tracking (`spawned_gui_pid` / `take_spawned_gui_pid`).
- **Typed Craft Errors (`T1.8`):** Added `AppNotInstalled`, `AppNotRunning`, `DeniedByRoot`, and `ToolFailed` to `workcraft_core::Error` (`crates/core/src/error.rs`) with machine-readable `.code()` and actionable `.hint()`.
- **Settings & Engine Commands (`T1.2`, `T1.4`):** Added `craft_modes`, `extra_roots`, `full_tool_catalog`, `craft_anything_deep`, and `craft_anything_allow_run` to `Config` (`crates/agents/src/config.rs`), plus engine commands `craft.list`, `craft.mode`, `craft.test`, and `craft.open` (`crates/agents/src/engine.rs`).

### Phase 2 — Vision Pipeline, Artifact Saving & MCP Transport Hardening (`T2.1–T2.7`)
- **MCP Image Retention (`T2.1`):** Updated `crates/core/src/tool.rs` (`ImagePart`, `ToolResult.images`, `ToolSpec.{read_only_hint, destructive}`) and `crates/mcp/src/client.rs` to retain base64 MCP `type: "image"` blocks (4 MiB/image cap, max 8 images/call) instead of discarding them.
- **Artifact Persistence (`T2.2`):** `Tools::call` (`crates/agents/src/tools.rs`) saves returned render/screenshot images to `<workspace>/.workcraft/artifacts/<app>-<call_id>-<idx>.<ext>` and emits `Event::ToolFinished { artifacts, mode, .. }`.
- **LLM Multimodal Vision Forwarding (`T2.3`):** Added `ChatRequest.vision` + `ProviderConfig.vision` (`supports_vision`, `is_local_model`) and wired tool-result images across `anthropic.rs` (inline `tool_result` image content blocks), `openai.rs`, and `ollama.rs` (synthetic user follow-up image frame when `vision == true`; text-only path hint when `vision == false`).
- **Quirks Normalization (`T2.5`):** Created `crates/agents/src/craft/quirks.rs` handling DesignCraft data-URI decoding, CADCraft JSON `{ok, png_base64}` extraction, TextureCraft CPU render fallback (`cpu: true` retry on headless Vulkan failure), and relative-root path rewriting for TextureCraft & MaterialCraft (nested objects, arrays, `out_dir`, `dir`, `paths`, `files`).
- **MCP Lifecycle Hardening (`T2.6`, `T2.7`):** Implemented `tools/list` caching + `notifications/tools/list_changed` invalidation, child-death detection + automatic 1x reconnect, per-call timeout + `notifications/cancelled`, `McpHub::disconnect_idle()`, `McpHub::clear()`, and `McpClient::from_handler` (`Transport::InMemory` for fast hermetic tests).

### Phase 3 — Lean Tool Profiles, Token Budgets, Approval Gates & Uniform Verbs (`T3.1–T3.5`)
- **Lean Profiles & Budgets (`T3.1`, `T3.2`):** `filter_app_tools` strips `live_only_tools` in `Headless` mode and enforces `<= 16` curated tools and `<= 5,000` schema tokens per app when `full_tool_catalog == false`. `apply_tool_budget` caps total tools at `60` for local models (`Ollama` / `LM Studio`) and `128` for cloud models, prioritizing `@app` mentions in the user prompt.
- **Computed Approval Policy (`T3.3`):** Implemented `crates/agents/src/craft/policy.rs` (`classify_craft_call` -> `Read | Mutate | Gated`). Paid cloud generation (`assets_image_generate`, `assets_model_generate`), script/eval/Lisp/JS execution (`script`, `eval`, `lisp`, `command_line`, `script_ui`, and `script.*`/`*.eval`/`js.*` inside generic `command_run`), permanent deletion, and print/sign **always ask for approval even in `ApprovalMode::Auto`** with an explicit human-readable `reason` and `destructive` flag.
- **Uniform `craft_*` Facade (`T3.5`):** Exposed `craft_status`, `craft_activate`, `craft_open`, `craft_new`, `craft_save`, `craft_inspect`, `craft_render`, `craft_export`, `craft_commands`, `craft_run`, `craft_batch`, and `craft_handoff` in `crates/agents/src/tools.rs`.

### Phase 4 & 5 — New Connectors, Skills, Icons, Team Leases & UI (`T4.1–T5.6`)
- **17 Bundled Skills (`T4.3`):** Added `crates/agents/skills/{texturecraft,materialcraft,solidcraft,craft-anything}/SKILL.md`, updated `soundcraft/SKILL.md` (removed `--demo` MCP guidance) and `filmcraft/SKILL.md` (documented `FILMCRAFT_CONTROL_TOKEN`), and registered all 17 skills in `crates/agents/src/skills.rs`.
- **17 App Icons (`T4.2`):** Added 64×64 PNG icons `crates/app/assets/icons/{texturecraft,materialcraft,solidcraft,craft-anything}.png` and updated `crates/app/src/assets.rs`.
- **Craft Anything Clean-Room & Inbox Bridge (`T4.5`):** Enforced `--clean-room --inbox <DIR>` by default (`Off` until enabled) and implemented `Shared::process_craft_anything_inbox_once` + `spawn_craft_anything_inbox_watcher` (`crates/agents/src/shared.rs`) routing `<id>.request.json` prompts through WorkCraft's approval system and writing `<id>.answer.json` atomically.
- **Cross-App Handoff & Team Specialists (`T5.1–T5.4`):** Implemented `crates/agents/src/craft/handoff.rs` (`craft_handoff` format compatibility matrix + staging into `<workspace>/handoffs/` without silent re-encoding), expanded Team specialists (`three-d`, `explorer`, plus updated `image`, `design`, `video`, `document`) in `crates/agents/src/team.rs`, and added per-app async writer leases (`Shared::app_leases`) serializing concurrent mutations to the same app while keeping reads and distinct-app writes parallel.
- **CLI & Desktop UI (`T2.4`, `T5.5`, `T5.6`):**
  - `crates/cli/src/main.rs`: Displays `[headless]`/`[live]` mode tags, saved `artifact:` paths, `[DESTRUCTIVE]` tags, and approval `reason`.
  - `crates/app/src/view_chat.rs` & `view_team.rs`: Added `live`/`headless` badges, cached clickable inline render thumbnails (`Action::OpenPath`), `Always gated` destructive styling, and approval `reason` banners; `@app` autocomplete and voice vocabulary (`voice.rs`) now cover all 17 apps dynamically via `craft::specs()`.
  - `crates/app/src/view_settings.rs`: Upgraded **Settings > Tools & MCP** with all 17 apps, version/port/CLI tooltip, `Headless` / `Live` / `Off` mode chips, `Test` button, `Open window` button, `full_tool_catalog` toggle, `extra_roots` editor, and Craft Anything `--clean-room` notice.

### Completed Tests So Far
- **Workspace Unit Tests (`L1`):** All unit tests in `workcraft-core`, `workcraft-mcp`, `workcraft-llm`, `workcraft-store`, `workcraft-control`, `workcraft-agents` (`policy`, `quirks`, `handoff`, `spec`, `tools`, `skills`), and `workcraft-app` pass.
- **Engine & Agent Integration Suite (`L5`, `crates/agents/tests/engine.rs`):** **19/19 passing**, including the 4 new tests:
  - `gated_tools_ask_even_in_auto_mode_with_explicit_reason`
  - `mention_prioritizes_app_under_local_budget_and_persists_vision_artifacts`
  - `team_mode_writer_lease_serializes_same_app_mutations`
  - `craft_anything_inbox_request_routes_through_approval_and_writes_answer`
- **Contract Snapshot & Live 17-App Drift/Token Cap Suite (`L2`, `crates/agents/tests/apps_contract.rs`):** Both offline snapshot validation and live 17-app MCP server handshake + lean schema token cap (`<= 5,000` tokens across all 17 installed apps) implemented (`vectorcraft` and `figcraft` lean lists just trimmed to 13 and 11 tools so all 17 sit strictly under 5,000 real tokens).

---

## 3. What Is Still Missing (Exact Checklist to Resume Later)

When your weekly quota resets, tell the agent:  
> *"Read `C:\xampp\htdocs\projects\artcraft\workcraft\workcraft-ai\STATUS.md` and `PLAN.md`, then finish items 1–5 in Section 3."*

### 1. Re-run `apps_contract` After the Final `vectorcraft`/`figcraft` Trim (~5 seconds)
Run:
```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$env:WORKCRAFT_LIVE_APPS = "1"
Set-Location C:\xampp\htdocs\projects\artcraft\workcraft\workcraft-ai
cargo test -p workcraft-agents --test apps_contract -- --include-ignored --nocapture
```
Expected: `2 passed; 0 failed` (confirming `vectorcraft` and `figcraft` lean token counts are now `< 5,000`).

### 2. Create & Run `crates/agents/tests/apps_headless.rs` (`L3` + `L7` from `PLAN.md` §7.1)
Create `crates/agents/tests/apps_headless.rs` (`#[ignore]` tests invoked via `--include-ignored` so default offline `cargo test` stays `< 2s`):
- **Isolated scratch setup per test:** `<temp>/wc-headless-<uuid>` with `TEXTURECRAFT_NO_PREFS=1`, `MATERIALCRAFT_NO_PREFS=1`, `SOUNDCRAFT_NO_PREFS=1`, `CRAFT_ANYTHING_CONFIG_DIR=<scratch>/ca-cfg`, `hub.clear().await`, and `remove_dir_all` on teardown.
- **17 Real-App Headless Workflows (`L3`):**
  1. `wordcraft`: `new_document` -> `type_text` -> `get_text` -> `render_page` -> `save_document` (`.docx`).
  2. `gridcraft`: `new_workbook` -> `set_cell` (`A1=10`, `A2=32`, `A3==SUM(A1:A2)`) -> `get_cell` (`42`) -> `save_workbook` (`.xlsx`).
  3. `deckcraft`: `new_presentation` -> `add_text_box` -> `inspect_document` -> `render_slide` -> `save_presentation` (`.pptx`).
  4. `pdfcraft`: `craft_new` (`from: "blank", pages: 2`) -> `tool_call` `page_add_text` -> `doc_save` (`%PDF-`) -> `doc_open` -> `text_extract` -> `page_render`.
  5. `photocraft`: 240×180 PNG -> `doc_open` -> `command_run` `filter.blur.gaussianBlur` -> `doc_render_preview` (verify artifact PNG!) -> `doc_export`.
  6. `vectorcraft`: `draw_shape` -> `add_text` -> `inspect_document` -> `render_preview` -> `export` `.svg` & `.png`.
  7. `lightcraft`: 320×240 JPEG -> `import` -> `set_develop` -> `render_photo` -> `export` `.jpg`.
  8. `designcraft`: `new_document` -> `execute` `frame.create` -> `place_image` -> `render_page` -> `export_png` + `execute` `file.exportPdf` (`%PDF-`).
  9. `filmcraft`: copy `test_clip.mp4` -> `media_import` -> `command_run` `file.newSequenceFromClip` -> `command_run` `timeline.razor` -> `render_frame` -> `command_run` `file.exportFcpxml`.
  10. `effectcraft`: `open_project` (`new: true`) -> `command_run` `comp.new` + `layer.newSolid` + `layer.newText` -> `render_frame` -> `save_project` (`.ecproj`).
  11. `soundcraft`: generate PCM `.wav` -> `new_session` -> `import_audio` -> `inspect_session` -> `bounce_mix` (`RIFF` WAV) -> `save_session` (`.scraft`).
  12. `cadcraft`: `new_drawing` -> `script` (`LINE` + `CIRCLE`) -> `inspect_drawing` -> `render` -> `save` (`.dxf`).
  13. `figcraft`: `create_node` (`FRAME` + `TEXT`) -> `inspect_document` -> `screenshot` -> `export` (`.svg`) -> `save_file` (`.figcraft`).
  14. `texturecraft`: `project_new` -> `paint_stroke` -> `viewport_render` (`cpu: true`) -> `textures_export` (PNG maps in relative dir) -> `project_save` (`.tcraft`).
  15. `materialcraft`: `graph_new` (`template: "pbr"`) -> `library_list` -> `view_render` -> `outputs_export` (PNG maps) -> `package_save` (`.sbs`; `.mcraft` was removed in MaterialCraft 0.2.0).
  16. `solidcraft`: `new_design` -> `execute` `sketch.create` (`plane: "XY"`) + `sketch.line` (`points: [[0,0],[40,0],[40,30],[0,30]], closed: true`) + `sketch.finish` + `solid.extrude` (`distance: 12`) -> `inspect_design` -> `screenshot` -> `export` `.step` & `.glb`.
  17. `craft-anything`: enable `CraftMode::Headless` -> `core_session_show` -> `core_brief_write` -> `core_investigation_show` -> `core_report_write` (never invoke paid `assets_*_generate` tools).
- **Cross-App Handoff & Facade Test (`T3.5`, `T5.1`):** Verify `craft_handoff` (`vectorcraft` PNG/SVG -> `deckcraft__insert_picture`; `solidcraft` GLB -> `texturecraft__mesh_import`).
- **Failure-Injection Suite (`L7` / `T1.8`):** Missing binary (`AppNotInstalled`), closed live port on read (`app_not_running` without launching GUI), outside-root path (`denied_by_root`), `craft-anything` refused while `Off`, and killed stdio MCP child (`hub.child_pid`) -> automatic 1x reconnect.

### 3. Create & Run `crates/agents/tests/apps_live.rs` (`L4` from `PLAN.md` §7.1)
Create opt-in `--ignored` live GUI bridge tests (`--test-threads=1`) for representative live-capable apps (including token-authenticated `photocraft` on port `7984`, `filmcraft` on port `7986`, and `vectorcraft` on port `7985`):
- Launch via `ensure_live_with_roots`, mutate + inspect + capture live screenshot, and clean up **strictly by own spawned PID** (`take_spawned_gui_pid(id)` — never kill processes by name).

### 4. Version Bump & Mandatory Documentation (`T6.2–T6.3` + `AGENTS.md` Rules)
Per `AGENTS.md` standing rules:
- **Bump version from `0.1.0-alpha.1` -> `0.1.0-alpha.2`** in **both**:
  - `Cargo.toml` (`[workspace.package] version = "0.1.0-alpha.2"`)
  - `crates/app/build.rs` (Windows resource version metadata)
- **Prepend `## [0.1.0-alpha.2] - 2026-11-06` to `CHANGELOG.md`** (never edit older entries).
- **Create `docs/CRAFT-APPS.md`:** Complete reference of all 17 connectors, CLI flags, `Headless` vs `Live` modes, ports & auth mechanisms, sandbox roots, lean tool profiles, `craft_*` facade verbs, `craft_handoff` matrix, approval gates, and measured test timings.
- **Update mandatory existing docs in the same change:**
  - `README.md` (17 Craft apps, headless/live modes, vision support, `craft_handoff`, Settings > Tools & MCP)
  - `AGENTS.md` (updated crate layout `crates/agents/src/craft/*`, 17 skills, test commands)
  - `docs/ARCHITECTURE.md` (craft module diagram, vision/artifact flow, writer leases, inbox bridge)
  - `packaging/README.md` (17 icons & bundled skills notes)
  - `SOURCES.md` (add `texturecraft`, `materialcraft`, `solidcraft`, `craft-anything` source repositories & icons)
  - `PLAN.md` §11 (fill in the `Result` column of the Verification Matrix).

### 5. Final Gate (`T6.4`)
Run and confirm exit code `0` for all four:
```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
Set-Location C:\xampp\htdocs\projects\artcraft\workcraft\workcraft-ai
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p workcraft-cli -- run craft.list
```

---

## 4. Known Upstream App Constraints (Already Accounted For in Design)

1. **PdfCraft & Craft Anything have no live GUI `--control` socket upstream:** WorkCraft intentionally keeps their MCP connection in `Headless` mode (`supports_live() == false`) while `craft.open` can still open the desktop viewer/GUI for file inspection.
2. **TextureCraft & MaterialCraft disable headless file/bake/render tools when bridged live (`--bridge`):** Upstream intentionally splits batch GPU/CPU rendering & texture export into headless MCP and interactive viewport/UI screenshots (`viewport_screenshot`, `view_screenshot`) into live mode. WorkCraft's mode selector (`Headless` default vs `Live`) and `live_only_tools` filter reflect this cleanly.
3. **TypeScript `projects/artcraft/workcraft/workcraft` repo:** Intentionally out of scope per `PLAN.md` (`Q1`); all changes live strictly inside `workcraft-ai`.

