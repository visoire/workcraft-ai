# PLAN: Full ArtCraft MCP coverage for WorkCraft chat

> **Status:** analysis and plan only. Nothing in the code was changed to produce this file.
> **Written:** 2026-11-05, against `workcraft-ai` at `0.1.0-alpha.1`.
> **For:** the implementing AI agent (and the maintainer who reviews it).
> **Read first:** [AGENTS.md](AGENTS.md) (versioning, changelog and lint rules apply to every change below) and
> [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

Everything in section 2 was measured on this machine on 2026-11-05 with the installed apps, not copied from
documentation. Where a statement comes from documentation only, it says so.

---

## 0. TL;DR

1. **Eight of the sixteen apps are not "missing MCP support". WorkCraft cannot find them.** `craft.rs` looks in
   `C:\Program Files\<App>\` and on `PATH`. The ArtCraft Launcher installs eight apps in
   `C:\Program Files\ArtCraft\<App>\`. Running the real `WorkCraft-CLI.exe run craft.list` reports VectorCraft,
   LightCraft, DesignCraft, FilmCraft, EffectCraft, SoundCraft, CADCraft and FigCraft as `installed=false`, although
   they are installed. Their connector code already exists. **One path fix switches on 8 apps.**
2. **All 16 apps (and a 17th, SolidCraft) already ship a working `<app>-cli mcp` server.** I started every one of
   them over stdio and ran `initialize` + `tools/list`: 17 of 17 answered, 921 tools in total. The right strategy
   is therefore **not to write 16 new MCP servers**. It is to build a thin, data-driven *connector layer* in
   WorkCraft around the vendors' servers (discovery, modes, policy, results, budget) and to add what no single app
   can offer: cross-app handoff, a uniform verb facade and team scoping.
3. **The hard problem is context size, not coverage.** The full `tools/list` of all apps is about 700 KB
   (~178k tokens). Even with `--compact` for LightCraft and PdfCraft it is ~320 KB (~82k tokens). WorkCraft's default model is a
   4B local model (`qwen3.5:4b` is what Ollama has here). Connecting everything the way `craft.rs` does today would
   make the chat unusable. A tool budget and on-demand activation are phase-1 work, not polish.
4. **The model cannot see anything the apps render.** `parse_tool_result` turns every MCP image into the text
   `[image: image/png]` (verified: CADCraft `render` and MaterialCraft `view_render` return real PNGs, 4 KB and
   712 KB). For creative apps that is the biggest functional gap after discovery.
5. **Four apps are not known to WorkCraft at all**: TextureCraft, MaterialCraft, Craft Anything (in the user's
   list) and SolidCraft (installed, not in the list). They need an app spec, icon, skill, port and policy.
6. **Craft Anything has paid and code-running tools** (`assets_image_generate` ~\$0.15 per call via fal.ai,
   `--allow-run`, `--deep`). It needs hard gates that no other app needs.

Proposed order: **Phase 0** baseline → **Phase 1** registry and discovery (8 apps appear) → **Phase 2** results,
images and errors → **Phase 3** tool budget and approval policy → **Phase 4** wave-2 apps → **Phase 5** handoff,
team and UI → **Phase 6** tests, docs, changelog. Detailed tasks are in section 6.

---

## 1. Scope, naming and assumptions

| Item | Decision | Why |
| :--- | :--- | :--- |
| "WorkCraft Chat" | `projects/artcraft/workcraft/workcraft-ai` (Rust: egui app, `workcraft-cli`, MCP server, control channel) | Its README calls itself "WorkCraft chat". `AGENTS.md` and the `craft.rs` module are here. |
| TypeScript workspace | `projects/artcraft/workcraft/workcraft` is **out of scope**, listed as a follow-up in section 9 | It has its own `src/server/craft.ts` for 13 apps; same path bug likely. Do not mix the two in one change. |
| The "16 apps" | Exactly the user's list, plus **SolidCraft** as a bonus app (installed, `solidcraft-cli mcp` works) | SolidCraft 0.30.1 is in `C:\Program Files\SolidCraft`. When this plan was written there was no source for it; `artcraft/apps/solidcraft-0.30.1` was added on 2026-11-06 and is the reference for alpha.3. It is a separate parametric 3D B-rep CAD app next to CADCraft (2D drafting), not its successor (Q2 answered). |
| Changelog | The file is `CHANGELOG.md` (the request says `changelog.md`) | Windows is case-insensitive; keep the existing name. |
| Versioning | Each phase that ships is one alpha: `0.1.0-alpha.2`, `.3`, ... | Rule from `AGENTS.md`: bump `Cargo.toml` + `crates/app/build.rs`, add a `CHANGELOG.md` entry, update docs, `SOURCES.md`, run fmt, clippy, test. |
| Tests | Real apps only for the live tests; scripted models for engine logic | The user requires testing against the installed apps. |

---

## 2. Findings

### 2.1 What is installed (measured)

Install roots, versions and server facts. "Tools" is `tools/list` of the headless server, started as shown in the
"Headless args" column. Tokens are bytes/4 of the JSON, an estimate. "Ann." is how many tools carry
`annotations.readOnlyHint` (WorkCraft uses it for read/write classification, otherwise it guesses from the name).

| # | App | Ver. | Install dir (under `C:\Program Files\`) | Tools (full → compact) | ~Tokens | Ann. | Headless args used for the probe | Live bridge flag (`<app> --control PORT` running) |
| -: | :--- | :--- | :--- | :--- | -: | :--- | :--- | :--- |
| 1 | PhotoCraft | 0.20.2 | `PhotoCraft` | 26 | 8k | 26/26 | `mcp` (+ `--automation-read-root/--automation-write-root`) | `--bridge 127.0.0.1:PORT` + `--control-token[-file]` |
| 2 | VectorCraft | 0.21.1 | `ArtCraft\VectorCraft` | 33 | 10k | 33/33 | `mcp --headless` | `--connect ADDR` (default tries 127.0.0.1:7979, then falls back to headless) |
| 3 | FilmCraft | 0.17.1 | `ArtCraft\FilmCraft` | 17 | 3k | 17/17 | `mcp` | `--bridge 127.0.0.1:PORT` |
| 4 | LightCraft | 0.18.1 | `ArtCraft\LightCraft` | **400 → 17** | **54k → 5k** | 400/400 | `mcp --demo --compact` | `--connect [ADDR]` (default 127.0.0.1:7980) |
| 5 | PdfCraft | 0.18.2 | `PdfCraft` | **189 → 17** | **50k → 4k** | 189/189 | `mcp --compact` | none over TCP: `pdfcraft ui --control FILE` (socket file) |
| 6 | EffectCraft | 0.21.1 | `ArtCraft\EffectCraft` | 27 | 6k | 27/27 | `mcp` | `--bridge PORT` |
| 7 | DesignCraft | 0.17.2 | `ArtCraft\DesignCraft` | 26 | 5k | 26/26 | `mcp --sample` (opens a sample doc; per its help, plain `mcp` works without a window) | `--connect PORT` |
| 8 | FigCraft | 0.15.1 | `ArtCraft\FigCraft` | 19 | 8k | 19/19 | `mcp` | `--attach PORT` (refuses automation roots) |
| 9 | TextureCraft | 0.3.2 | `TextureCraft` | 22 | 3k | **0/22** | `mcp` (read/write roots required for files) | `--bridge PORT --token T` |
| 10 | MaterialCraft | 0.1.2 | `MaterialCraft` | 19 | 3k | **0/19** | `mcp` (read/write roots required for files) | `--bridge PORT --token T` |
| 11 | WordCraft | 0.15.2 | `WordCraft` | 17 | 2k | 17/17 | `mcp` | `--connect HOST:PORT` (needs `control-key.local-<port>`) |
| 12 | SoundCraft | 0.17.1 | `ArtCraft\SoundCraft` | 17 | 1k | **0/17** | `mcp` works and starts an **empty** session (WorkCraft passes `--demo`, see G12) | `--connect PORT` |
| 13 | DeckCraft | 0.15.1 | `DeckCraft` | 25 | 4k | 25/25 | `mcp --sample` (plain `mcp` works) | `--connect PORT` |
| 14 | CADCraft | 0.17.1 | `ArtCraft\CADCraft` | 12 | 1k | **0/12** | `mcp` | `--connect HOST:PORT` (+ `--lazy`, `--launch`) |
| 15 | GridCraft | 0.16.1 | `GridCraft` | 22 | 5k | 22/22 | `mcp` | `--connect PORT` |
| 16 | Craft Anything | 0.3.0 | `Craft Anything` | 29 (dynamic toolsets) | 7k | 29/29 | `mcp [--project X.cany] [--inbox DIR]` | none (app answers `core_person_ask` / `core_action_approve` through `--inbox`) |
| +  | SolidCraft | 0.30.1 | `SolidCraft` | 21 | 7k | 21/21 | `mcp --headless` | `--connect 127.0.0.1:PORT [--token T]` |

Total: **921 tools, ~178k tokens (~700 KB)** in full mode; **366 tools, ~82k tokens (~320 KB)** with `--compact` on
LightCraft and PdfCraft.
Every server answered `initialize` with protocol `2025-03-26` (PdfCraft's stderr banner says `2025-06-18`).
Longest `<app>__<tool>` name: 45 characters (`lightcraft__cmd_library_removeFolder`), under the 64-character limit of
the model APIs, but there is no test that guards it.

Other things that are installed and matter:

- The ArtCraft Launcher (0.14.1, running; `AppData\Local\Programs\ArtCraft Launcher`, data in
  `AppData\Local\ArtCraft Launcher`) is what puts eight apps under `Program Files\ArtCraft\`; the other apps came
  from standalone installers (`Program Files\<App>\`). **Both layouts must be supported.** Linux and macOS layouts
  come from the community plugin's docs only and are untested here. `AppData\Local\ArtCraft\artcraft.exe` (0.41.0) is
  the separate ArtCraft AI Studio app, not one of the 16.
- `solidcraft-cli.exe` is the only CLI on `PATH`. No other CLI is, so `PATH` discovery cannot be relied on.
- No app window is running now; the only listening ports in the 5170-10000 range are unrelated (7680).
- Ollama runs locally with `qwen3.5:4b`.

### 2.2 The 13 apps the code knows today

`crates/agents/src/craft.rs` `APPS` (13 entries) lists: WordCraft, GridCraft, DeckCraft, PdfCraft, PhotoCraft,
VectorCraft, LightCraft, DesignCraft, FilmCraft, EffectCraft, SoundCraft, CADCraft, FigCraft.
Missing from it: **TextureCraft, MaterialCraft, Craft Anything, SolidCraft**.

Where the same app list is duplicated (each must be updated today for one new app, which is the root of several bugs):

| File | What it hard-codes |
| :--- | :--- |
| `crates/agents/src/craft.rs` | `APPS`, `ON_BY_DEFAULT` (4 apps), `default_control_port`, `live_mcp_args`, `server()` special cases for PhotoCraft/FigCraft |
| `crates/mcp/src/config.rs` | `McpConfig::craft_defaults()` (8 apps, **a different list again**) |
| `crates/app/src/assets.rs` | 13 icon `include_bytes!` entries |
| `crates/agents/src/config.rs` | `craft_team()`: 3 specialists with fixed app groups |
| `crates/agents/src/prompt.rs` | BASE prompt names PhotoCraft/VectorCraft/FilmCraft/WordCraft/GridCraft and DeckCraft command names |
| `crates/agents/src/tools.rs` | `open_craft_app`, `wordcraft__*` and `deckcraft__*` special cases inside `call_mcp_tool`, `PATH_ARGS` |
| `crates/agents/skills/` | 13 bundled skills (no texturecraft, materialcraft, solidcraft, craft-anything) |

### 2.3 How the integration works today (so the plan fits it)

- **Hub.** `workcraft_mcp::McpHub` keeps one `McpClient` per server and namespaces tools `<server>__<tool>`
  (`sanitize` keeps `[A-Za-z0-9-]`, so `craft-anything__...` is legal). `connect_all` connects enabled servers in
  parallel; one failing server never blocks the others.
- **Discovery to servers.** `craft::discover()` finds CLIs, `craft::merge()` adds every *enabled* app that `mcp.json`
  does not already name. If the desktop app is running (`live_port_for`), the server is started in live mode
  (`live_mcp_args`), otherwise headless.
- **Live window.** `craft::ensure_live()` launches the GUI with `--control <port>` and reconnects the server in live
  mode. `tools.rs::call_mcp_tool` calls it **before every tool call of a known Craft app**.
- **Approvals.** `ToolSpec.read_only` comes from `annotations.readOnlyHint`, else the name heuristic
  `looks_read_only`. Manual mode asks for every non-read-only call; reads never ask.
- **Results.** `parse_tool_result`: text parts joined, images become `[image: <mime>]`, `structuredContent` used
  when there is no content, truncated at 64 KB. Request timeout 60 s, call timeout 300 s.
- **Team.** `craft_team()`: leader + 3 specialists (`image`: Photo/Vector/Light; `video`: Film/Effect/Sound;
  `document`: Word/Grid/Deck/Pdf/Design). Tool scope per agent is a glob list (`photocraft__*`). Concurrency 3. No
  per-app or per-file lock in the Rust scheduler (the TypeScript version has `src/mcp/locks.ts`).
- **Control channel.** `craft_commands` / `craft_run` talk the Craft control protocol directly for apps listed in
  `control-targets.json`.
- **Verified state:** `README.md` says "No Craft app is installed here ... not talked to". That is now false and must
  be corrected when the first live test passes.

### 2.4 Gaps and bugs (each has a task in section 6)

| ID | Gap | Evidence | Impact |
| :--- | :--- | :--- | :--- |
| **G1** | Discovery misses `Program Files\ArtCraft\<App>\` | `craft.list` output: 8 apps `installed=false` | 8 of 16 apps unusable |
| **G2** | 4 apps unknown (TextureCraft, MaterialCraft, Craft Anything, SolidCraft) | `APPS` has 13 entries | no discovery, icon, skill, policy |
| **G3** | Enabled-by-default is only the document suite; PhotoCraft is marked "no live bridge"; control ports collide | `ON_BY_DEFAULT`; PhotoCraft **and** TextureCraft both default to 7878, GridCraft/FigCraft/VectorCraft all to 7979 in the app docs; `ensure_live` hard-codes 7878 for PhotoCraft | wrong app answers on a shared port; new apps would collide |
| **G4** | MCP images are dropped | `parse_tool_result` → `[image: image/png]`; real servers return PNG | the model cannot look at renders, previews or screenshots |
| **G5** | No tool budget | 82k tokens (compact) / 178k (full) if all connect; default model is 4B | local models fail, API cost explodes |
| **G6** | Read/write classification is weak for apps without annotations (CADCraft, MaterialCraft, SoundCraft, TextureCraft = 70 tools) | `looks_read_only` has no `render`, `screenshot`, `measure`, `compare`, `check`, `wait`, `pending`, `running`; `render`, `screenshot`, `ui_screenshot`, `view_render` ask for approval although they only read | approval fatigue in manual mode |
| **G7** | The generic tools cannot be classified by name | `command_run`, `execute`, `batch`, `command_line`, `script`, `js_run`, `call_app`, `tool_call`, `run_script` can read or write or execute code | writes run unasked, or every call asks |
| **G8** | `ensure_live` opens a GUI window before *any* call, even a read, even if the user wants headless | `tools.rs` line ~801 | surprising windows; contradicts "good error handling when an app is not running" |
| **G9** | App hacks are inline in `call_mcp_tool` and lists are duplicated in 7 places | section 2.2 | adding an app is a 7-file change; behavior drifts |
| **G10** | Path-argument resolution handles 9 keys; the apps use others | probe of all schemas: `dir`, `paths`, `files`, `folder`, `out_dir`, `source`, `input`, `file` (PdfCraft, SolidCraft) are not resolved | relative paths from the model fail or resolve against the wrong folder |
| **G11** | File sandbox policy is uneven | PhotoCraft and FigCraft get the **whole home folder** as read and write root; TextureCraft and MaterialCraft document "file access only below the roots" and take paths *relative to roots* (not yet tested without roots); DeckCraft started with no roots saved to `%TEMP%` unconfined (verified) | inconsistent blast radius; TextureCraft/MaterialCraft file tools probably fail without roots (verify in T4.4) |
| **G12** | SoundCraft is started with `--demo` | `craft.rs` `APPS` extra args; plain `soundcraft-cli mcp` starts an empty session (verified) | the agent edits a demo song instead of a blank session |
| **G13** | No tool-list refresh | Craft Anything changes its tool list on `enable_toolset`; `McpClient` ignores `notifications/tools/list_changed`; `hub.tools()` calls `tools/list` on every spec build | stale or slow tool lists |
| **G14** | Overlapping tool names | LightCraft, VectorCraft, EffectCraft expose both `command_*` and legacy `list_commands`/`run_command`/`batch`/`execute_command` | wasted budget, model confusion |
| **G15** | Long-running work has no progress model | PhotoCraft `jobs_list`/`jobs_cancel`, PdfCraft `ocr_*`, SolidCraft `running`/`pending`, FilmCraft/EffectCraft renders, TextureCraft `bake` | one 300 s timeout, no polling, no cancel |
| **G16** | No per-app lock in Team mode | `team.rs` runs 3 specialists in parallel; no mutex per app or document | two agents can drive one app window or file at once |
| **G17** | Team knows 3 specialists | `craft_team()` | no 3D/material/CAD or software-exploration role |
| **G18** | 4 icons missing (the 4 new apps) | `crates/app/assets/icons/` has 13 PNGs | tool cards show a generic icon |
| **G19** | Errors are not typed | `Error::Mcp(String)`; "app not installed" / "not running" / "denied by root" / "timeout" look alike to the model | the model cannot choose a fix |
| **G20** | Child process death is not detected | `McpClient` keeps a dead stdio child | every later call times out after 60 s |
| **G21** | Baseline is not green | `README.md`: `cargo clippy ... -D warnings` reports 4 `collapsible_if` lints | the lint gate of `AGENTS.md` fails before any change |
| **G22** | MCP *resources* are ignored | CADCraft `cadcraft://drawing`, `cadcraft://commands` | minor; useful cheap context |

---

## 3. Design decisions

**D1. Reuse the vendors' servers. Do not reimplement them.**
Each app ships a maintained `mcp` server that already passes `tools/list`. Re-implementing 17 of them would copy 900
tools and fall behind with every app release. WorkCraft's job is the layer around them.
*Exception allowed:* if an app's server lacks something WorkCraft needs and the app has a CLI subcommand for it
(for example `texturecraft-cli render`, `materialcraft-cli export`), WorkCraft may add a **WorkCraft-side wrapper
tool** (`<app>__...` is reserved for the vendor; wrappers are `craft_*`).

**D2. One `AppSpec` registry replaces every duplicated list.**
A single table (Rust data, tested) describes each app: ids, names, install roots, CLI name, headless args, live
args builder, port, auth, file-root policy, lean tool profile, risk table, quirks, icon, skill, handoff formats,
timeouts. Adding the 18th app is one entry plus assets (section 5.1).

**D3. Three connection modes per app, chosen per app and per call.**

| Mode | What runs | Use | Default |
| :--- | :--- | :--- | :--- |
| `headless` | `<app>-cli mcp ...` in-process engine, files only | batch work, reads, CI, no windows | **default for every app** |
| `live` | the desktop window with `--control PORT`, server bridged to it | the user wants to *watch* or hand-edit; UI tools (`ui_*`, `pointer`, `screenshot`) | on request: `open_craft_app`, `@app live`, Settings toggle "Show window" |
| `off` | not connected | | user choice |

`ensure_live()` stops running before every call. It runs only when the app's mode is `live` (or the model calls
`open_craft_app`). A read against a not-running app in `live` mode returns a typed `app_not_running` error that says
how to start it (G8, G19).

**D4. Connect on demand, expose a lean tool profile, cap the budget.**
- *Availability* of an app is separate from *connection*. Every discovered app is `available`; it connects when
  (a) the user @mentions it, (b) an agent's scope names it, (c) the model calls `craft_activate`, or (d) it is in
  the user's "always on" set (default: the document suite, as today). Idle servers disconnect after N minutes.
- Each app has a **lean profile**: an allowlist of the high-value tools, target ≤ 16 tools and ≤ 5k tokens.
  Everything else stays reachable through the app's own generic tool (`command_run`, `tool_call`, `execute`) or
  `tool_search` (PdfCraft compact already has both). Native names stay `photocraft__doc_open`, as the request wants.
- A global budget (default 60 tools for local models, 120 for API models, configurable) drops the lowest-priority
  tools first and says so in the system prompt.
- Duplicates (G14) are hidden in the lean profile.

**D5. A uniform `craft_*` verb facade, in addition to the native tools (recommended, optional).**
Apps name the same action differently (`doc_open`, `open_file`, `open_document`, `open_presentation`,
`open_session`, `open`, `package_open`, `project_open`). A small facade gives the model one vocabulary and constant
~3k tokens however many apps are active:

`craft_status`, `craft_activate`, `craft_open`, `craft_new`, `craft_inspect`, `craft_render` (returns an image),
`craft_export`, `craft_save`, `craft_commands`, `craft_run`, `craft_batch`, `craft_handoff`.

Each takes `app` and is routed by the app spec's verb map (section 5.3). Native tools remain; the facade is how
weaker models and Team workers stay inside the budget. If the maintainer prefers only native names (open question
Q8), skip the facade and keep D4.

**D6. Approval policy is computed, not guessed.** Order of evidence:
1. `annotations`: `readOnlyHint` (read), `destructiveHint` (always shown as destructive in the card).
2. The app spec's **risk table** for generic tools: command-id patterns marked read (`inspect.*`, `*.list`,
   `query.*`, `view.zoom*`, `jobs.list`), write, or **gated**.
3. The name heuristic (extended list: `render`, `screenshot`, `measure`, `compare`, `check`, `pending`, `running`,
   `wait`).
4. Unknown means write.

**Gated** tools ask even in Auto mode, with the reason shown in the card: code execution (`js_run`, `js_*`, `script`,
`run_script`, `core_*` with `--allow-run`), paid or networked calls (`assets_image_generate`, `assets_model_generate`,
`lens.downloadDatabase`, model downloads), permanent deletion (`cmd_photo_deletePermanently`,
`cmd_library_emptyRecentlyDeleted`, `redact_apply`, `doc_protect`), `sign_*`, `doc_print`. Gated-in-Auto is a policy
change: see open question Q5.

**D7. Images are first-class results.** `ToolResult` gets `images: Vec<ImagePart>` (mime, bytes). The engine saves each
image to `<workspace>/.workcraft/artifacts/<session>/<n>.png`, puts the path in the text result, shows a thumbnail in
the tool card, and forwards the image to vision-capable models (Anthropic `tool_result` image block, OpenAI-compatible
user image message, Ollama `images`). A per-model `vision` flag in the registry decides; non-vision models get the path
and a size/dimension description only.

**D8. Port registry, no collisions.** Keep WorkCraft's existing assignments (they are already de-conflicted from the
apps' documented defaults) and fill the gaps. New: PhotoCraft **7984** (free slot; today hard-coded 7878),
TextureCraft **7990**, MaterialCraft **7991**, SolidCraft **7992**. A startup check refuses to launch an app on a port
another app owns.

**D9. Safe-by-default file roots.** One policy for every headless server that accepts roots: read root = workspace +
folders the user added; write root = workspace + an output folder. **Not** the home folder (today's PhotoCraft and
FigCraft behavior). Apps whose paths are *relative to roots* (TextureCraft, MaterialCraft) get the same roots so
absolute paths from the model inside them work. Decision needed: open question Q4.

---

## 4. Prioritization

Ranking criteria: usefulness to an agent, whether the app already has code in WorkCraft (cost), how rich and safe
its server is, how well it complements the existing document suite.

| Tier | Apps | Why | Done when |
| :--- | :--- | :--- | :--- |
| **A (first, verified deepest)** | PhotoCraft, VectorCraft, DesignCraft, LightCraft, FilmCraft | Highest creative value; every one has a rich, annotated server; 4 of 5 are already coded and only blocked by G1; images and layout feed the document suite (charts, logos, covers) | every golden workflow in section 7 passes headless **and** live, with image feedback |
| **B** | SoundCraft, EffectCraft, FigCraft, CADCraft, MaterialCraft, TextureCraft | Already coded (first four) or small and clean (last two); each is a distinct domain | golden workflows pass headless; live where the app has a bridge |
| **C (gated)** | SolidCraft, Craft Anything | SolidCraft is outside the user's list (its source is now in `artcraft/apps/solidcraft-0.30.1`, read in alpha.3); Craft Anything has paid and code-running tools | works behind the gates of D6; docs state the risk |
| **Existing** | WordCraft, GridCraft, DeckCraft, PdfCraft | Already good | regression-tested, moved onto the registry, quirks moved out of `call_mcp_tool` |

Wave mapping: Phase 1 delivers Tier A + the four coded Tier-B apps through discovery alone; Phase 4 delivers the rest.

There are only those apps (16):
PhotoCraft

VectorCraft

FilmCraft

LightCraft

PdfCraft

EffectCraft

DesignCraft

FigCraft

TextureCraft

MaterialCraft

WordCraft

SoundCraft

DeckCraft

CADCraft

GridCraft

Craft Anything

---

## 5. Target architecture

### 5.1 `AppSpec` registry

New module `crates/agents/src/craft/` (split the 500-line `craft.rs`):

```
craft/mod.rs        re-exports; keeps the public names the rest of the code uses (APPS, discover, ensure_live, ...)
craft/spec.rs       AppSpec + the table of 17 entries
craft/discover.rs   install roots, env override, PATH, version probe
craft/live.rs       control ports, token handling, ensure_live, mode switching
craft/policy.rs     risk tables, gating, lean profiles, budget
craft/quirks.rs     the per-app hacks now inside tools.rs (WordCraft create_document alias, DeckCraft selection, ...)
craft/handoff.rs    format matrix and craft_handoff
```

`AppSpec` fields (all `&'static`, no allocation at startup):

| Field | Example (PhotoCraft) |
| :--- | :--- |
| `id`, `name`, `kind` | `photocraft`, `PhotoCraft`, `Photos` |
| `install_dirs` | `["PhotoCraft", "ArtCraft/PhotoCraft"]` under `ProgramFiles`, `ProgramW6432`, `LOCALAPPDATA`, `LOCALAPPDATA/Programs` |
| `cli`, `gui` | `photocraft-cli`, `photocraft` |
| `env_override` | `PHOTOCRAFT_CLI` (same convention as the community plugin, so users can point at portable installs) |
| `headless_args(roots)` | `["mcp","--automation-read-root",R,"--automation-write-root",W]` |
| `live_args(port, token)` | `["mcp","--bridge","127.0.0.1:{port}", "--control-token-file", F]` |
| `port`, `auth` | `7984`, `TokenFile(temp/photocraft-control-{port}.token)` |
| `roots` | `Roots::Args` / `Roots::None` / `Roots::ReadWriteRelative` |
| `lean` | the allowlist of native tools |
| `risk` | table of `(tool or command pattern, Read/Write/Gated, reason)` |
| `tool_timeout` | `{ default: 300s, per_tool: [("bake", 900s)] }` |
| `handoff` | `reads: [png,jpg,tif,psd,...]`, `writes: [...]` |
| `icon`, `skill` | `icons/photocraft.png`, `photocraft` |
| `quirks` | function pointers for pre/post call rewriting |

Every other module reads this table. The seven duplicated lists of section 2.2 are deleted. A unit test asserts that
every `AppSpec` has an icon, a skill, a unique id and port, a lean profile that names real tools (checked against the
recorded snapshot of section 5.5), and `<id>__<longest tool>` ≤ 64 characters.

### 5.2 Discovery order

1. `<APP>_CLI` environment variable (error if set but missing: no silent fallback).
2. Every `install_dirs` entry under `ProgramFiles`, `ProgramW6432`, `LOCALAPPDATA`, `LOCALAPPDATA\Programs`
   (**includes `ArtCraft\<App>`**), plus the launcher's own manifest if readable.
3. Absolute `PATH` entries.
4. macOS: `/Applications/<App>.app/Contents/MacOS`, `/usr/local/bin`, `/opt/homebrew/bin`. Linux: `/usr/bin`, `~/.local/bin`
   (documented by the apps, untested here, mark as such).

`craft.list` gains: `version` (from `--version` or file version, cached), `cli`, `gui`, `mode`, `port`,
`tools` (count after lean filter), `health` (`ok` | `not_installed` | `not_running` | `error: ...`), `last_error`.

### 5.3 Verb map for the facade (D5)

Native tool each verb routes to, from the real tool lists. `cmd:` means "through the app's generic command tool, id to be
confirmed with `command_list` in task T3.4".

| App | open | new | save | inspect | render (image) | export |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| PhotoCraft | `doc_open` | `doc_new` | `doc_save` | `doc_inspect` | `doc_render_preview` | `doc_export` |
| VectorCraft | `open_file` | cmd | `save_file` | `inspect_document` | `render_preview` | `export` |
| FilmCraft | cmd | cmd | cmd | `project_inspect` / `sequence_inspect` | `render_frame` | cmd (also `media_import` for input) |
| LightCraft | `import` | `--library` at start | auto | `doc_inspect` | `render_photo` | `export` |
| PdfCraft | `doc_open` | `doc_create` | `doc_save` | `doc_info` / `doc_inspect` | `page_render` | `doc_export_images` / `doc_export_office` |
| EffectCraft | `open_project` | cmd | `save_project` | `get_project` | `render_frame` | cmd |
| DesignCraft | `open_document` | `new_document` | `save_document` | `inspect_document` | `render_page` | `export_png` |
| FigCraft | `open_file` | cmd | `save_file` | `inspect_document` | `screenshot` | `export` |
| TextureCraft | `project_open` | `project_new` | `project_save` | `session_inspect` | `viewport_render` | `textures_export` / `mesh_export` |
| MaterialCraft | `package_open` | `graph_new` | `package_save` | `session_inspect` | `view_render` | `outputs_export` |
| WordCraft | `open_document` | `new_document` | `save_document` | `inspect_document` | `render_page` | cmd |
| SoundCraft | `open_session` | `new_session` | `save_session` | `inspect_session` | `screenshot` (check headless in T3.5) | `bounce_mix` |
| DeckCraft | `open_presentation` | `new_presentation` | `save_presentation` | `inspect_document` | `render_slide` | `export` |
| CADCraft | `open` | `new_drawing` | `save` (`.svg/.png/.pdf` exports) | `inspect_drawing` | `render` | `save` with extension |
| GridCraft | `open_workbook` | `new_workbook` | `save_workbook` | `inspect_workbook` | `screenshot` (check headless in T3.5) | cmd |
| SolidCraft | `open` | `new_design` | `save` | `inspect_design` | `screenshot` | `export` |
| Craft Anything | not a document app: use native `core_*` tools only | | | | | |

### 5.4 Cross-app handoff (`craft_handoff`, section 7 has the flows to test)

`craft_handoff { from_app, to_app, file, [as_format] }` checks the `handoff` formats of both specs, converts only when
the *source app* can write the target format (never silently re-encodes), places the file in the shared workspace
folder `handoffs/<from>-<to>-<n>.<ext>`, and returns the path plus the call the receiving app should make next.
Known useful flows (all file-based, so they work headless):

| From | To | File | Receiving tool |
| :--- | :--- | :--- | :--- |
| PhotoCraft / LightCraft / VectorCraft (PNG, JPG, SVG) | DeckCraft, WordCraft, DesignCraft, FigCraft, PdfCraft | image | `deckcraft__insert_picture`, `designcraft__place_image`, `wordcraft` insert command, `pdfcraft__page_add_image` |
| LightCraft `export` | PhotoCraft | JPG/TIFF/DNG | `photocraft__doc_open` |
| VectorCraft `export` | FigCraft, DesignCraft | SVG/PDF | `designcraft__place_image` |
| MaterialCraft `outputs_export` | TextureCraft | PNG maps / `.tcmat` | `texturecraft__material_load`, `resource_import` |
| SolidCraft `export` | TextureCraft | STL/OBJ/3MF (GLB if supported: verify) | `texturecraft__mesh_import` |
| CADCraft `save` `.svg/.pdf` | DesignCraft, PdfCraft | SVG/PDF | `designcraft__place_image`, `pdfcraft__doc_open` |
| SoundCraft `bounce_mix` | FilmCraft | WAV | `filmcraft__media_import` |
| EffectCraft render | FilmCraft | MP4/MOV/image sequence | `filmcraft__media_import` |
| DesignCraft `export` / WordCraft export | PdfCraft | PDF | `pdfcraft__doc_open` |
| PdfCraft `page_render` / `doc_export_images` | PhotoCraft | PNG | `photocraft__doc_open` |
| Craft Anything `assets_image_generate` / `assets_model_generate` | any | PNG / GLB | gated (paid, D6) |

### 5.5 Contract snapshot

Check in `crates/agents/tests/snapshots/tools-<app>.json` (name, required args, annotations; no descriptions): the
output of the probe in section 10. A test (live, ignored by default) re-probes the installed apps and **fails on
drift** (removed tool, renamed tool, new required argument). That is how the lean profiles and verb maps stay honest
when the apps update. Version numbers go into the snapshot header.

---

## 6. Work breakdown

Sizes: S ≤ 0.5 day, M ≈ 1 day, L ≈ 2-3 days for one agent. Each task lists files and an acceptance check. Do the
phases in order; phases 4 and 5 can overlap.

### Phase 0: Baseline (alpha.1, no version bump)

| ID | Task | Size | Acceptance |
| :--- | :--- | :--- | :--- |
| T0.1 | Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` from a clean checkout using `%USERPROFILE%\.cargo\bin`. Record results in the PR. | S | numbers written down |
| T0.2 | Fix the 4 `collapsible_if` lints in `docs.rs` and `tools.rs` (G21). | S | clippy `-D warnings` is green |
| T0.3 | Port the probe (`scratch/probe.mjs`, section 10) to a Rust example `crates/agents/examples/probe_apps.rs` using the existing `McpClient`. It prints the table of section 2.1 and writes the snapshots of 5.5. | M | output matches section 2.1 on this machine |
| T0.4 | Save the *current* `craft.list` and a current-state transcript as the "before" evidence for the changelog. | S | files in `docs/evidence/` (optional) |

### Phase 1: Registry, discovery, modes → **0.1.0-alpha.2**

| ID | Task | Size | Files | Acceptance |
| :--- | :--- | :--- | :--- | :--- |
| T1.1 | Create `craft/` module and `AppSpec` (5.1). Move the 13 current apps in with **identical behavior** first; delete `APPS`/`ON_BY_DEFAULT`/`default_control_port`/`live_mcp_args` and the duplicated lists in `config.rs`, `assets.rs`, `prompt.rs`. | L | `crates/agents/src/craft/*`, `crates/mcp/src/config.rs`, `crates/agents/src/prompt.rs` | all existing tests pass unchanged |
| T1.2 | Discovery per 5.2, including `ArtCraft\<App>` (G1) and `<APP>_CLI`. | M | `craft/discover.rs` | `craft.list` shows all 16 as installed on this machine; test with a fake `ProgramFiles` tree; test `<APP>_CLI` error |
| T1.3 | Move quirks out of `call_mcp_tool` into `craft/quirks.rs` (WordCraft `create_document`, `open_document {}`, DeckCraft selection, `tool_search` retry). Dispatch by spec. | M | `tools.rs`, `craft/quirks.rs` | `tests/engine.rs` still green; no `wordcraft__`/`deckcraft__` string literal left in `tools.rs` |
| T1.4 | Connection modes (D3): `headless` default, `live` on request. `ensure_live` only for `live` apps. Add command `craft.mode {app, mode}`. Remove G12 (`--demo` for SoundCraft). | M | `craft/live.rs`, `engine.rs`, `commands` | reading a not-live app never opens a window; `craft.mode photocraft live` opens it and bridges |
| T1.5 | Port registry (D8) and the startup collision check; replace the hard-coded 7878. | S | `craft/live.rs` | unit test: all ports unique, none equal 7878 |
| T1.6 | Auth matrix: verify, per app, how the CLI authenticates to a window started with `--control PORT` (token file in `%TEMP%`, `control-key.local-<port>`, env token). Record in a table in `docs/ARCHITECTURE.md`. | M | live, on this machine | every Tier-A app bridges once; the table is committed |
| T1.7 | Availability vs connection (D4): `craft_apps` stays `Map<String,bool>` for compatibility (`true` = always on, `false` = off, absent = default policy), add on-demand connect and idle disconnect. | M | `config.rs`, `engine.rs` | an `available` app connects on first mention; `craft.list` shows `connected` |
| T1.8 | Child-death detection and one automatic reconnect (G20); typed errors (G19): `AppNotInstalled`, `AppNotRunning`, `Timeout`, `DeniedByRoot`, `ToolFailed`, each with a `hint` the model can act on. | M | `crates/mcp/src/client.rs`, `crates/core/src/error.rs` | killing the child of a test server yields a reconnect or a typed error in under 5 s |

### Phase 2: Results, images, paths → **0.1.0-alpha.3** (with phase 3)

| ID | Task | Size | Files | Acceptance |
| :--- | :--- | :--- | :--- | :--- |
| T2.1 | `ToolResult.images` (D7); `parse_tool_result` keeps image bytes, caps at 4 MB each / 8 per call. | M | `crates/core/src/tool.rs`, `crates/mcp/src/client.rs` | unit test with a 1×1 PNG |
| T2.2 | Save artifacts; text result carries the path and pixel size. | S | `agents/src/tools.rs` | CADCraft `render` produces a file under `.workcraft/artifacts/` |
| T2.3 | Vision plumbing in the three adapters + a `vision` flag per model; non-vision gets text only. | L | `crates/llm/*`, `registry.rs` | scripted-model test sees the image block for an Anthropic request and an OpenAI-compatible request |
| T2.4 | Tool card thumbnail, click to open full size. | M | `crates/app/src/view_chat.rs`, `widgets.rs` | window capture in both themes |
| T2.5 | Path resolution: extend to `dir`, `paths`, `files`, `folder`, `out_dir`, `source`, `input`, arrays of paths, and nested `params`/`steps` (G10). Data-driven per app spec. | M | `tools.rs` | table-driven test over every key in the probe |
| T2.6 | Long-running jobs (G15): per-tool timeouts from the spec; recognise `still running:` (SolidCraft), `jobs_list` (PhotoCraft), ask the model to poll; wire cancel. | M | `client.rs`, `craft/spec.rs` | a deliberately slow command returns "still running" and `craft_status` shows it |
| T2.7 | Handle `notifications/tools/list_changed`, cache `tools/list` per connection (G13). | M | `client.rs`, `hub.rs` | Craft Anything `enable_toolset` makes new tools appear next turn; `tools/list` is not called per spec build |

### Phase 3: Tool budget, approvals → **0.1.0-alpha.3**

| ID | Task | Size | Files | Acceptance |
| :--- | :--- | :--- | :--- | :--- |
| T3.1 | Lean profiles (D4), generated from the snapshot and committed as data. Hide duplicates (G14). Targets: ≤ 16 tools, ≤ 5k tokens per app (proposals in section 8). | M | `craft/policy.rs` | `craft.list` reports tool counts; a test prints tokens per app and fails above target |
| T3.2 | Budget (D4): global cap by model class, priority order, a system-prompt line stating what was dropped. | M | `agents/src/agent.rs`, `prompt.rs` | with all 17 apps connected a 4B model sees ≤ 60 tools |
| T3.3 | Approval classifier (D6): annotations, risk tables, extended name heuristic, `destructiveHint` shown on the card, gated list and reason string. | L | `core/src/tool.rs`, `craft/policy.rs`, `agent.rs` | table test: every tool of every snapshot is classified; `render`/`screenshot` never ask; `core_*` paid tools always ask |
| T3.4 | Generic tools: classify by *command id* for `command_run`, `execute`, `batch`, `command_batch`, `run_command`, `command_line`, `script`. Fetch each app's `command_list` once and build the table from ids (`menu`, `enabled`, label verbs) plus a hand-written override list. | L | `craft/policy.rs` | `photocraft__command_run {id:"filter.blur.gaussianBlur"}` is a write, `{id:"view.zoomIn"}` is a read |
| T3.5 | Facade (D5) if Q8 says yes: `craft_*` verbs, routed by spec. | L | `agents/src/tools.rs`, `craft/spec.rs` | every verb of 5.3 works for every Tier-A app in a live test |

### Phase 4: Wave-2 apps → **0.1.0-alpha.4**

| ID | Task | Size | Acceptance |
| :--- | :--- | :--- | :--- |
| T4.1 | `AppSpec` entries for **TextureCraft, MaterialCraft, SolidCraft, Craft Anything** (ports 7990-7992, roots policy, lean profile, risk table). | M | `craft.list` shows 17 apps; snapshot test passes |
| T4.2 | Icons: downscale to 64 px the 1024 px masters `artcraft/apps/{texturecraft-0.3.2,materialcraft-0.1.2,craft-anything-0.3.0}/assets/app-icon/<app>-1024.png` (same size and method as the existing 13 in `crates/app/assets/icons/`). SolidCraft has no source folder: generate it with `solidcraft-cli icon --out icon.png --size 256`. Add the four to `assets.rs`. Keep the existing 13. Use only official artwork (`artcraft/assets/icons/icons.md`). | S | tool cards show the right icon for all 17 |
| T4.3 | Skills: write `texturecraft`, `materialcraft`, `solidcraft`, `craft-anything` SKILL.md in the agentskills.io format into `crates/agents/skills/`, from the app docs (`docs/control-protocol.md`, `mcp` tool descriptions). SolidCraft: also surface its own `skill_reference` tool (it serves four bundled skills: solidcraft-modeling, -mechanisms, -editing, -checking) in the lean profile; Craft Anything: base the skill on `list_toolsets`, `craft-anything-cli capabilities` and `doctor`. Also check the 13 existing skills against the installed versions. | L | `read_skill` lists 17 skills; `build.rs` embeds them |
| T4.4 | TextureCraft/MaterialCraft file semantics: paths relative to roots, `command_run` refuses host-path commands (stated in the tool description). Pass roots from the shared policy (D9), teach the path resolver to make them relative when inside the root. | M | `project_new` → `textures_export` works with absolute paths from the model |
| T4.5 | Craft Anything gates: start only with `--clean-room`, never `--allow-run`, never `--deep` unless the user turns it on in Settings; `assets_*` always ask with the cost; document the legal/safety note in `docs/`. `--inbox DIR` points at a WorkCraft folder so `core_person_ask` and `core_action_approve` surface as WorkCraft approval cards (reuse `approval.respond`). | L | a `core_person_ask` call shows an Allow/Deny/choices card in the window |
| T4.6 | SolidCraft: `--headless`, `--allow-path` for the roots instead of `--allow-any-path`, `pending`/`running` for long runs. | S | headless `new_design` → `execute` → `measure` → `export` works |

### Phase 5: Cross-app, Team, UI → **0.1.0-alpha.5**

| ID | Task | Size | Acceptance |
| :--- | :--- | :--- | :--- |
| T5.1 | `craft_handoff` and the format matrix (5.4). | M | every flow of 5.4 that can run here has a live test |
| T5.2 | Per-app lease in the Team scheduler: one writer per app session at a time, reads parallel (G16). Port the idea from `src/mcp/locks.ts`. | M | two parallel tasks on one app serialize in the log; on two apps they run in parallel |
| T5.3 | New specialists: `three-d` (TextureCraft, MaterialCraft, SolidCraft, CADCraft), `design` (DesignCraft, FigCraft, VectorCraft), keep `image`, `video`, `document` but regroup so each owns ≤ 5 apps and a budget. Optional `explorer` (Craft Anything) off by default. Leader prompt gets a generated capability table from `AppSpec`. | M | `team.run` plans a task for each specialist on a mixed goal; scopes enforced |
| T5.4 | System prompt: replace the hard-coded app list with a generated line from the specs; keep the DeckCraft guidance, move it into the DeckCraft quirk text. | S | prompt snapshot test |
| T5.5 | Settings > Craft apps: version, path, mode switch, port, health, "Test connection" button, tool count, "Show window" toggle, always-on toggle. Tool card badge `live` / `headless`. | L | window captures of the Settings screen and a card |
| T5.6 | Single-mode `@app` mentions activate the app (D4). Blip reacts to the new apps (existing `on_tool_start`). | S | `@photocraft` connects it and its tools appear |

### Phase 6: Tests, docs, changelog (in every alpha, finalized at 0.1.0-alpha.5)

| ID | Task | Acceptance |
| :--- | :--- | :--- |
| T6.1 | Test plan of section 7 implemented. | CI runs unit + contract tests; the live suite runs locally with `WORKCRAFT_LIVE_APPS=1` |
| T6.2 | `CHANGELOG.md`: one entry per alpha, written in the same change (`AGENTS.md`). | entries for alpha.2 to alpha.5 |
| T6.3 | Docs: `README.md` (coverage table, remove "No Craft app is installed here", move items from "Not verified" to "Verified" only for what T6.1 proved), `AGENTS.md` (new module, rules), `docs/ARCHITECTURE.md` (registry, modes, budget, auth matrix, ports), new `docs/CRAFT-APPS.md` (per-app tool guide, modes, flags, approval notes, handoffs), `packaging/README.md`, `SOURCES.md` (apps' repos, the community plugin's launcher convention). | links resolve |
| T6.4 | Final summary: coverage status table (section 11) filled with measured results. | table has no "unknown" |

---

## 7. Test plan (against the real apps)

### 7.1 Levels

| Level | What | Where | Runs |
| :--- | :--- | :--- | :--- |
| L1 unit | `AppSpec` invariants, discovery with fake trees, classifier tables, path resolver, image parsing, budget | `cargo test` | always |
| L2 contract | re-probe installed apps and diff against the snapshot (5.5); every lean-profile tool exists; every tool name ≤ 64 chars | `crates/agents/tests/apps_contract.rs`, `#[ignore]` unless `WORKCRAFT_LIVE_APPS=1` | locally |
| L3 headless golden workflows | one scripted session per app (table 7.2) through `McpHub` | `crates/agents/tests/apps_headless.rs` | locally |
| L4 live window | start the GUI with `--control <port>` from the registry, bridge, run a read and a write, take a screenshot, close **only the process the test started** | `crates/agents/tests/apps_live.rs` | locally, Tier A first |
| L5 engine end-to-end | scripted model calls the facade and native tools; approval flow; Team run over two apps | `crates/agents/tests/engine.rs` (extend) | always (no apps needed: uses a fake server binary) + locally with real apps |
| L6 real model | `workcraft-cli chat --model ollama:qwen3.5:4b` on 5 prompts (one per tier-A app) and a Team run; record transcripts | manual, results into `docs/CRAFT-APPS.md` | before each alpha |
| L7 failure injection | app not installed (bad `<APP>_CLI`), app not running in `live` mode, child killed mid-call, tool returns `isError`, root denial, timeout, port in use, token missing | L1/L5 with the fake server, then 3 real ones (PhotoCraft, MaterialCraft, SolidCraft) | always + locally |

### 7.2 Headless golden workflows (calls verified to exist in the real tool lists)

Reuse the existing assets: `artcraft/mcp/creative-apps-claude-plugin-main/tests/inputs/test_card.png|jpg|test_clip.mp4`
and `tests/steps/{photocraft,vectorcraft,filmcraft,lightcraft,pdfcraft,effectcraft,designcraft}.json`, which already
passed on the vendor versions listed in that plugin's README. Add steps for the other apps. Every workflow ends with a
**render and a look at the picture** (image result, saved artifact) and a re-read of the saved file. "A passing call is
not proof the picture is right" (the plugin's own warning).

| App | Workflow (tool order) | Check |
| :--- | :--- | :--- |
| PhotoCraft | `doc_open test_card.png` → `command_run filter.blur.gaussianBlur {radius:12}` → `doc_inspect` → `doc_render_preview` → `doc_export out.png` → `doc_new 800x600` → `doc_export out.jpg` | exported PNG differs from input; image result present |
| VectorCraft | `open_file` or new → `draw_shape` → `set_paint` → `add_text` → `pathfinder` → `inspect_document` → `render_preview` → `export svg` | SVG parses, contains the shape and text |
| FilmCraft | `media_import test_clip.mp4` → `project_inspect` → `sequence_inspect` → `command_run` (trim, via `command_list` id) → `render_frame` → export | frame image present; export file plays (size > 0) |
| LightCraft | `import test_card.jpg` → `query_photos` → `get_develop` → `set_develop exposure` → `apply_preset` → `render_photo` → `export` | export differs from original |
| PdfCraft | `doc_open` → `doc_info` → `text_extract` → `page_render` → `doc_combine`/`doc_split` → `doc_save` | page count after split correct |
| EffectCraft | `open_project`/empty → `add_effect` → `add_keyframe` → `set_property` → `render_frame` → `save_project` | frame image present |
| DesignCraft | `new_document` → `set_story_text` → `place_image` → `render_page` → `export_png` | PNG present |
| FigCraft | `create_node` → `set_properties` → `inspect_document` → `export` → `screenshot` | exported file present |
| TextureCraft | `project_new` (builtin sphere) → `paint_stroke` → `bake` → `viewport_render` → `textures_export` → `project_save` | exported maps present, saved project reopens |
| MaterialCraft | `graph_new template:bricks` → `node_param` → `view_render` → `outputs_export` → `package_save` | 712 KB-class PNG returned (verified today); exported maps present |
| WordCraft | `new_document` → `type_text` → `inspect_document` → `render_page` → `save_document` | file present, text round-trips |
| SoundCraft | `new_session` → `import_audio` → `execute` (gain) → `inspect_session` → `bounce_mix` | WAV present, length > 0 |
| DeckCraft | `new_presentation` → `add_slide` → `add_text_box` → `insert_picture` → `render_slide` → `save_presentation` | pptx/dcraft present |
| CADCraft | `new_drawing` → `command_line rectang 0,0 100,60` → `inspect_drawing` → `render` → `save .svg` | verified today up to `render`; `counts.Polyline == 1` |
| GridCraft | `new_workbook` → `write_range` → `evaluate_formula` → `insert_chart` → `save_workbook` | formula result correct |
| SolidCraft | `new_design` → `execute sketch.rectangle.two_point` → `execute` extrude (id from `list_commands`) → `measure` → `screenshot` → `export STEP/STL` | volume matches the expected value |
| Craft Anything | `list_toolsets` → `core_session_show` → `core_targets_list` → `core_journal_read` (read-only, `--clean-room`, no network) | gates verified: `assets_image_generate` asks and is refused in a test without approval |

Facts already verified in this analysis (no GUI started, no user file touched; outputs went to `%TEMP%` and were removed):
CADCraft `new_drawing`/`command_line`/`inspect_drawing`/`render` (PNG), MaterialCraft `session_inspect`/`library_list`/
`view_render` (PNG 712 KB), SoundCraft `inspect_session` empty and demo, DeckCraft `save_presentation` without roots.

### 7.3 Live workflows (Tier A first)

For each app: launch through the registry (`AppSpec.port`, token), wait for the port, bridge, then
1. a read (`*_inspect`), 2. a write visible in the window, 3. `ui_screenshot`/`screenshot` and look at it,
4. `craft.mode headless` again (window stays, server switches), 5. stop the process the test started.

Rules (from `AGENTS.md`, repeat them in the test helper): use `WORKCRAFT_CHAT_HOME=<scratch>`; never kill by command-line
pattern, only the PID the test spawned; no writes outside a scratch folder; never touch the installed `Artcraft` folder in the home directory or
`~/workcraft`; Craft Anything only with `--clean-room`, no network, no paid tool.

### 7.4 Pass criteria per tier

- **Tier A:** L1, L2, L3, L4, L7 all green; L6 passes for 4 of 5 prompts on `qwen3.5:4b`, 5 of 5 on a stronger model.
- **Tier B:** L1, L2, L3, L7 green; L4 where the app has a bridge.
- **Tier C:** L1, L2, L3 (read-only for Craft Anything), L7 gates.
- **Regression:** the existing suite plus WordCraft/GridCraft/DeckCraft/PdfCraft L3.

---

## 8. Lean tool profiles (proposal, to confirm in T3.1)

Target ≤ 16 tools, ≤ 5k tokens per app. "Reach" is how the model gets the rest.

| App | Lean set (native names) | Reach | Notes |
| :--- | :--- | :--- | :--- |
| PhotoCraft | `doc_open doc_new doc_inspect doc_render_preview doc_export doc_save doc_close doc_place command_list command_run command_batch jobs_list session_list` | `command_run` (~700 engine ids) | `ui_*`, `control_call` live only |
| VectorCraft | `open_file save_file inspect_document render_preview draw_shape draw_path set_paint add_text transform pathfinder apply_effect export undo command_list command_run command_batch` | `command_run` | hide `list_commands run_command inspect_ui` aliases; `pointer_gesture press_key type_text answer_dialog invoke_menu open_panel screenshot` live only |
| FilmCraft | `media_import project_inspect sequence_inspect doc_inspect render_frame render_preview command_list command_run command_batch` | `command_run` | `ui_*` live only |
| LightCraft | `--compact` set minus duplicates: `command_list command_run command_batch doc_inspect render_preview import query_photos select_photos list_controls get_develop set_develop apply_preset crop render_photo export` | `command_run` (the other ~380 ids) | do not expose the 383 `cmd_*` tools (54k tokens) |
| DesignCraft | `inspect_document get_story set_story_text new_document open_document save_document place_image render_page export_png execute batch list_commands screenshot` | `execute` | pointer/key/ime/click/drag/menu/dialog tools live only |
| FigCraft | `create_node set_properties inspect_document get_node select export open_file save_file undo redo run_command batch list_commands screenshot` | `run_command` | `pointer key type_text inspect_ui wait_for_changes` live only |
| SoundCraft | `inspect_session new_session open_session save_session import_audio bounce_mix execute batch list_commands` | `execute` | UI tools live only; no annotations: classify by table |
| EffectCraft | `get_project get_comp get_layer set_property add_keyframe add_effect list_effects open_project save_project render_frame render_preview undo redo command_list command_run command_batch` | `command_run` | hide `execute_command` and `batch` aliases |
| CADCraft | all 12 | n/a | 1k tokens; no annotations: classify by table |
| TextureCraft | all except `ui_*`: `project_new project_open project_save mesh_import mesh_export material_load material_save resource_import paint_stroke bake bake_mesh_add textures_export viewport_render session_inspect command_list command_run command_batch` | `command_run` | no annotations |
| MaterialCraft | `package_open package_save graph_new node_add node_connect node_param bitmap_import library_list outputs_export view_render session_inspect command_list command_run command_batch` | `command_run` | no annotations |
| SolidCraft | all except `crash_reports compare_picture`: `new_design open save inspect_design measure body_topology set_parameter check_parameters export screenshot execute batch list_commands describe_command undo redo skill_reference running pending` | `execute` | `compare_picture` only on request |
| Craft Anything | Essentials toolset (the server default; no `--all-tools`) | `list_toolsets` / `enable_toolset` | dynamic list (G13); assets and run tools gated |
| WordCraft / GridCraft / DeckCraft / PdfCraft | today's behavior; PdfCraft keeps `--compact` | `tool_search` / `tool_call` | keep quirks |

---

## 9. Risks, open questions and follow-ups

### Risks

| Risk | Mitigation |
| :--- | :--- |
| Apps update and tool lists change | contract test with snapshot (5.5); lean profiles reference names, test fails loudly |
| A 4B model cannot use even 60 tools | facade (D5), activation by mention, Team scopes; L6 measures it |
| Windows opened by tests disturb the user | headless first; live tests are opt-in; own-PID cleanup only |
| Vision plumbing differs per provider | adapter unit tests with recorded wire formats; flag per model |
| Paid or code-running tools (Craft Anything) | gates in D6, `--clean-room`, no `--allow-run` default |
| Whole-home roots today | D9 and Q4; do not widen anywhere |
| ~~SolidCraft has no source folder~~ | resolved 2026-11-06: source added in `artcraft/apps/solidcraft-0.30.1`; `docs/mcp.md` read and the tools probed (alpha.3) |
| Linux/macOS untested | keep the layout code, label "not verified" in the README |

### Open questions (the implementing agent should ask, or assume the **bold** default and say so)

| ID | Question | Default |
| :--- | :--- | :--- |
| Q1 | Is "WorkCraft Chat" the Rust `workcraft-ai`, and is the TypeScript `workcraft` workspace out of scope? | **yes, yes** |
| Q2 | Is SolidCraft in scope, and is it the successor of CADCraft or a separate app? | **in scope as Tier C; separate app** |
| Q3 | Default policy: connect on demand (all apps available, document suite always on) or only the document suite? | **on demand** |
| Q4 | File roots: workspace-only (D9) or keep the home folder for PhotoCraft/FigCraft? | **workspace + user-added folders** |
| Q5 | Should gated tools ask even in Auto mode? | **yes** |
| Q6 | Is Craft Anything wanted in the chat at all (it studies and rebuilds third-party software)? | **yes, off by default, clean room** |
| Q7 | Icons: the official 1024 px assets from the press kit, or the masters shipped in each app's source (`assets/app-icon/<app>-1024.png`)? | **the app masters for the 3 new ones, `solidcraft-cli icon` for SolidCraft** |
| Q8 | Add the `craft_*` verb facade, or native names only? | **add it, native names stay** |
| Q9 | Should WorkCraft write `mcp.json` entries for other agents (Claude Code) as well? | **no, out of scope** |

### Follow-ups (not in this plan)

- Same path fix and registry idea for `workcraft/src/server/craft.ts` (TypeScript workspace).
- The ArtCraft **AI Studio** MCP (`artcraft/mcp/artcraft-mcp-v1.1.0`: image, video, 3D, voice generation tools, cost estimator)
  is a different server: not one of the 16 apps. A good later connector, with the same paid-tool gates.
- A WorkCraft MCP resource view for `*://drawing`-style resources (G22).

---

## 10. Reproduce the analysis

Probe scripts and raw results from this analysis are in
a local scratch folder of the analysis session (not part of this repository):

| File | What |
| :--- | :--- |
| `probe.mjs` | `initialize` + `tools/list` for all 17 CLIs (`node probe.mjs <outdir> [app,app]`) |
| `probe2.mjs` | same with `--compact` for LightCraft and PdfCraft |
| `call.mjs` | calls a tool sequence: `node call.mjs <exe> <args.json> <calls.json>` |
| `tools-<app>.json`, `compact-<app>.json` | the recorded tool lists |
| `help-<app>.txt`, `mcphelp-<app>.txt` | `--help` and `mcp --help` of each CLI |

Discovery check (shows G1): from the repository root

```powershell
$env:WORKCRAFT_CHAT_HOME = "$env:TEMP\wc-scratch"
.\WorkCraft-CLI.exe run craft.list
```

Expected before the fix: VectorCraft, LightCraft, DesignCraft, FilmCraft, EffectCraft, SoundCraft, CADCraft, FigCraft
show `installed=false`. Expected after phase 1: all 16 installed (17 with SolidCraft).

Environment: Windows 11, cargo/rustc 1.99.1, Node 24, Ollama with `qwen3.5:4b`, ArtCraft Launcher 0.14.1.

---

## 11. Coverage status

"Before" was measured on 2026-11-05 (alpha.1), "Result" on 2026-11-06 (alpha.2, T6.4). H = headless workflow passed against the real
app (`apps_headless`), L = live window driven and stopped by PID (`apps_live`), C = contract snapshot and live drift check
passed (`apps_contract`), tools = lean tools shown to the model.

| # | App | Found before | Tools before | Icon | Skill | Tier | Result (alpha.2) |
| -: | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| 1 | PhotoCraft | yes | 26 if enabled | yes | yes | A | found, C, H, **L**, 13 tools |
| 2 | VectorCraft | no (G1) | 0 | yes | yes | A | found, C, H, **L**, 12 tools |
| 3 | FilmCraft | no (G1) | 0 | yes | yes | A | found, C, H, **L**, 9 tools |
| 4 | LightCraft | no (G1) | 0 | yes | yes | A | found, C, H, 15 tools |
| 5 | PdfCraft | yes | compact 17 | yes | yes | existing | found, C, H, 16 tools; no live bridge upstream |
| 6 | EffectCraft | no (G1) | 0 | yes | yes | B | found, C, H, 16 tools |
| 7 | DesignCraft | no (G1) | 0 | yes | yes | A | found, C, H, 13 tools |
| 8 | FigCraft | no (G1) | 0 | yes | yes | B | found, C, H, 10 tools |
| 9 | TextureCraft | no (G2) | 0 | added | added | B | found, C, H (CPU render), 16 tools |
| 10 | MaterialCraft | no (G2) | 0 | added | added | B | found, C, H, 14 tools |
| 11 | WordCraft | yes | 17 | yes | yes | existing | found, C, H, 12 tools |
| 12 | SoundCraft | no (G1) | 0 | yes | yes | B | found, C, H (no `--demo`), 9 tools |
| 13 | DeckCraft | yes | 25 | yes | yes | existing | found, C, H (text verified inside the `.pptx`), 16 tools |
| 14 | CADCraft | no (G1) | 0 | yes | yes | B | found, C, H, 12 tools |
| 15 | GridCraft | yes | 22 | yes | yes | existing | found, C, H, 16 tools |
| 16 | Craft Anything | no (G2) | 0 | added | added | C | found, C, H read-only in `--clean-room`; no paid tool called; no live bridge upstream |
| + | SolidCraft | no (G2) | 0 | added | added | C | found, C, H (box with `expect`, volume 14400, STEP/STL/3MF/OBJ/DXF, reopen), **L**, 14 tools (alpha.3, checked against its source) |

Result: **17 of 17 found** by `craft.list`; 17 of 17 servers handshake and match their snapshots; 17 of 17 headless workflows
pass (22 tests including a handoff and 4 failure cases); 4 of 17 live windows verified (PhotoCraft, VectorCraft, FilmCraft, SolidCraft); L6: DeepSeek 5 of 5 prompts, `qwen2.5:7b` and `qwen3.5:4b` 0 of 5;
all lean profiles under 5,000 tokens (largest: VectorCraft 4932, FigCraft 4685, SolidCraft 4666).

Not reached: a local model that completes a task, and live windows for 13 apps. Open question defaults Q1 to Q8 were
taken as proposed; Q9 (writing `mcp.json` for other agents) stays out of scope.
