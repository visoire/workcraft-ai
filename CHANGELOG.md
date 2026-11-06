# Changelog

All notable changes to WorkCraft chat. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

**Alpha phase.** WorkCraft chat is in an alpha phase. There is no stable release and none is created until the
maintainer says so. Every meaningful change raises the alpha number (`0.1.0-alpha.N`) in `Cargo.toml` and adds an entry at
the top of this file under `## [0.1.0-alpha.N] - YYYY-MM-DD`.

## [0.1.0-alpha.9] - 2026-11-06

### Chats that keep working, more @ and / , a calmer and livelier Blip, the newest Craft apps, ready to publish

- **Every chat keeps its own work.** Starting a new chat while the AI is still working no longer pulls its reply into the new chat: each chat
  runs on its own, its tool cards, tasks, approvals and answer stay in that chat, and you can start, switch and send in other chats meanwhile.
  The chat list shows a pulsing dot on chats that are working and a green dot on chats that finished while you were elsewhere (with a quiet
  notice); an approval from another chat is marked "Other chat".
- **More @ mentions.** Besides agents, apps and teams: folders (workspace, desktop, documents, downloads, pictures, videos, music, home),
  `@skill:<name>`, `@all`, `@web`, `@artifacts`, `@team:apps` and `@team:craft`. The popup ranks matches, scrolls to the selected row and
  shows a description column.
- **Slash commands that run.** About 19 more commands: `/stop`, `/retry`, `/team apps|craft`, `/rename`, `/model`, `/think`, `/data`,
  `/rescan`, `/apps`, `/agents`, `/settings`, `/summarize`, `/explain`, `/improve`, `/render`, `/convert`, `/handoff`, `/help`. A command
  typed in full and sent with Enter is executed, not sent to the model.
- **Shift+Enter** inserts a line break at the caret (it did not before, in some situations); **Enter** sends.
- **Middle-click scroll mode** in the chat: click the wheel, move the mouse up or down to scroll (faster the farther you move), click again or press
  Escape to stop.
- **Skills.** `read_skill` accepts the argument names models actually send (`name`, `skill`, `skill_name`, `id`, `path`) and, for an unknown
  skill, lists the ones that exist instead of failing with "missing argument 'name'".
- **Blip, overhauled.** New states Working (a tool runs: narrowed eyes, busy hands, a turning gear), Waiting (a call needs your OK: raised hand and a
  question mark), Error (crossed eyes, a shake, drooping antennae, a sweat drop) and Success (a hop with both arms up, a check mark), next to Thinking
  (eyes look around, a thought bubble grows), Listening (sound ripples) and Idle (breathing, glances, regular and sometimes double blinks).
  Fixed: eyes that changed width while moving, shimmering edges and clipped effects (the pixel scale is now a whole number of screen pixels, every
  movement snaps to half or whole grid units, effects stand still while the body hops). Cheaper: 30 frames a second only while something happens, 10 when
  idle, none when switched off. Settings, Blue Blip mascot: **Movement** (Full, Calm, Off), **Size**, **Speech bubbles**, two new colors (Violet, Rose) and two
  new accessories (Cap, Bow tie).
- **Newest Craft apps.** Checked against the apps installed today: PhotoCraft 0.21.0, VectorCraft 0.22.0, FilmCraft 0.18.0, LightCraft 0.19.0, PdfCraft 0.19.0,
  EffectCraft 0.22.0, DesignCraft 0.18.0, FigCraft 0.16.0, TextureCraft 0.4.0, MaterialCraft 0.2.0, WordCraft 0.16.0, SoundCraft 0.18.0, DeckCraft 0.16.0,
  CADCraft 0.18.0, GridCraft 0.17.0, SolidCraft 0.30.1, Craft Anything 0.5.0. All 17 start headless, the snapshots are new, and the 22 headless
  workflows pass. MaterialCraft 0.2.0 saves native `.sbs` packages (`.mcraft` is gone); the skill and the format lists say so.
- **Craft Anything 0.5.0.** `core_finding_edit` joins the default set, and the skill now carries its working method: resume from the handoff, label claims as
  observed, derived or inferred, cite evidence ids, absence is not proof, summary first then pages, and what to do for each error code.
- **Ready to publish.** README rewritten for GitHub (the old one is `docs/USAGE.md`), CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, issue forms, pull request
  template, labels, repository description and topics (`.github/repo.json`), a release workflow that builds the Windows files and publishes a
  pre-release, a stricter `.gitignore`, and `scripts/publish.ps1` / `scripts/publish.sh` (check for secrets, init, push, meta, labels, changelog, release; dry run
  unless you add `-Execute`). Guide: `docs/PUBLISHING.md`. Nothing is uploaded by this change.
- **Fixes.** The repository URL in `Cargo.toml` is `visoire/workcraft-ai`; local paths are out of `PLAN.md`; the deprecated `Id::new` call is gone.

## [0.1.0-alpha.8] - 2026-11-06

### Every Craft app found, one agent per app, English only, Handy 0.10.0, the right data folder

- **Data folder.** The data folder is `~/workcraft/chat` (it showed `~/Workspace/chat` on a computer where both folders exist). Chats
  that an older version wrote to `~/Workspace/chat` (or `~/WorkCraft/chat`, `~/OpenCraft/chat`) move to `~/workcraft/chat` once, on the
  first start; when the folder cannot be renamed it is copied and the original stays. Settings, Data folder has an **Open folder**
  button (it creates the folder first, then shows it in Explorer, Finder or the file manager) next to Copy path.
- **Craft app connectors.** Settings, Craft apps (Connectors) shows "N of 17 Craft apps found" and how many are connected, a
  **Rescan** button and **Switch all on**, lists the installed apps first and the missing ones under "Not found on this computer", and
  says where it looked. Discovery also looks in `%USERPROFILE%\ArtCraft\<App>`, in folders whose name only resembles the app
  (`ArtCraft Apps\wordcraft`), and finds SolidCraft's `solvecraft-cli` under its old file name. `<APP>_CLI` still points at an install
  anywhere else.
- **One agent for every Craft app.** 16 new agents, `wordcraft-agent` ... `solidcraft-agent` (Craft Anything has the explorer): each sees only
  the tools of its own app plus the shared read and handoff tools, and reads its app's skill. They can be mentioned with `@`, edited
  in Settings, Agents, and they make up the new **apps** team (a leader and one agent per installed Craft app). Settings, Agents has a
  new "Team mode" switch between the Craft team (five specialists) and One agent per app; the choice is remembered.
- **More approvals for the team.** An agent can have its own approval (follow the team, ask first, or auto); a team can ask for the plan
  to be approved always, never, or only when approvals are on Ask first. A global Auto still turns every prompt off, and paid
  generation, code execution, permanent deletion and signing always ask.
- **English only.** The spoken-command words in other languages are gone from voice input (the `@` and `/` words, the send phrases, the
  Windows Speech language, the examples); the app speaks English.
- **Handy 0.10.0.** The labels, the checks and the settings text are for Handy 0.10.0. The installed version is read from the program file
  and Settings says "Update to 0.10.0" for an older one. `HANDY_PATH` points at a Handy build somewhere else.
- **Tests.** The chat folder move, the per-app agents (tool scopes, one agent per app), the agent and plan approvals, the version
  comparison and the English-only voice words.

## [0.1.0-alpha.7] - 2026-11-06

### Voice that hears quiet microphones, and a Blip that listens

- **Voice.** A recording is brought to a healthy level (at most 16 times) and padded with half a second of silence on each side
  before Handy hears it; Handy 0.10 chunks recordings with voice activity detection, which dropped quiet speech and ended as
  "No speech detected". The error now says what was wrong (too quiet, or heard but no words) and Blip says it too instead of
  "Voice input error". The labels say Handy without a version; 0.9.8 and 0.10 both work. WorkCraft does not update Handy.
- **Blip** has a Listening state (wide eyes, headset, sound arcs that take turns) while the microphone is open.
- Matches the workspace 1.0.0-alpha.6 (same Blip colors, same voice handling).
## [0.1.0-alpha.6] - 2026-11-06

### Artifacts panel, message actions, plan steering, a visible Blip

- **Artifacts tab** in the right panel (Activity | Artifacts N): pictures and files that Craft apps produced in the chat, as a
  thumbnail grid plus a file list. Click a picture for a full-size view (Open, Copy path, Close; Esc closes); click a file to open
  it. Derived from the tool results already in the chat, so it survives reopening a chat.
- **Regenerate** on the last reply of a turn: asks the same question again as a new branch.
- **Calmer messages**: consecutive replies from the same agent are grouped under one avatar and header; tool steps without text
  take a thin row.
- **Steer a running Team plan**: Skip or Redo (with a note) on a pending, running or failed task, through the new command
  `team.steer`. A stopped worker can no longer overwrite the new state.
- **Blip dock**: the mascot and its speech bubble sit at the composer while a turn runs, so reactions are seen in real use.
- **Settings, Tools & MCP fixed**: the mode chips and the Test and Open window buttons overlapped the app name and port badge. They now sit on their own row under each app, the header wraps, and "0/0 tools" is replaced by a plain status.
- **Shift+Enter** now adds a new line in the composer (it used to send); Enter still sends.
- **One Undo per answer**: the duplicate Undo changes chip next to the copy button is gone; the "files changed in this turn" bar keeps it.
- **Blip stays**: the mascot at the composer no longer disappears after a reply.
- **The accent follows Blip's colour theme**: choosing Emerald, Coral and so on in Settings recolours buttons, selection and links too, not just the mascot (saved with the Blip theme).
- Tests: a running task can be skipped and redone; the fallback model takes over once.
- Not verified: the Skip/Redo controls against a live Team run in the window, and a click-through of Regenerate. The Artifacts tab
  and grouped messages were checked in a window capture.

## [0.1.0-alpha.5] - 2026-11-06

### Fallback model, a livelier Blip, a calmer empty state, and a runnable build

- **Fallback model.** Set `fallback_model` (for example `ollama:qwen2.5:7b`) and a turn whose model cannot be reached, is overloaded,
  rate limited, out of credit or silent continues once with the fallback, announced as a notice. A wrong key, a bad request or a
  too-long conversation do not switch. Also settable with `config.set` (`fallback_model`, `tool_routing`, `context_window`).
- **Retry notices are visible.** The retry and fallback lines now appear as a toast above the composer, not only as a speech bubble.
- **Blip.** Reacts to a retry (worried, glasses on, says why), to a picture from an app (celebrates, names the app), to a long wait (a
  line after 20 s and every 25 s, with the minutes after a minute), and closes a reply with what it cost ("Done! That reply cost $0.0017").
- **Empty state.** The suggestion cards fade and lift on hover, and a strip shows the Craft app icons found on the computer
  ("17 Craft apps found. Type @ to pick one.") with the name on hover.
- **Build.** `dist/WorkCraft.exe` (the desktop app) and `dist/WorkCraft-CLI.exe`, release builds. The app was started, its window
  captured and closed; the CLI ran a real DeepSeek GridCraft task.
- **Docs.** New `USED-REPOS.md` lists what was studied (Craft Anything, the workspace AI, the provider docs, the IDE docs, Open WebUI,
  LibreChat, BB Desktop and others), what was taken and what was left.
- **Not done:** the message-level hover actions, an artifacts side panel and steering a running Team plan (see `USED-REPOS.md`); the
  fallback has an engine test but was not run against a real failing provider.

## [0.1.0-alpha.4] - 2026-11-06

### A faster, cheaper, more resilient AI core: retries, context fit, tool routing, cost, MCP transports, IDE pairing

Details, measurements and what was not verified are in [docs/AI.md](docs/AI.md).

- **Local models can finally see their tools.** Ollama starts a model with 4,096 tokens and cut the start of a longer prompt (the
  system prompt and the tool list) without a word: measured, a 12,555-token prompt reached the model as 2,050 tokens and the secret
  word at its start was lost. Every Ollama request now states its window (16,384, or `num_ctx`) and sets `truncate: false`. With this and the
  tool router below, `qwen2.5:7b` went from 0 to 3 of 4 file tasks against the real Craft apps and `qwen3.5:4b` from 0 to 2 of 4.
- **Tool router.** A request carries the built-in tools, the `craft_*` verbs and the tools of the apps the chat is about (named in the
  message, already used, or found with the new `find_tools`), not all 92 connected tools. `"hello"` went from 36,480 to 6,578 real prompt tokens
  (DeepSeek count); a whole GridCraft task averages 15,600 tokens per call instead of about 38,000. Apps are connected on demand and the
  set only grows within a chat, so provider prompt caches stay valid. Setting `tool_routing`. New command `tools.stats`.
- **Context fit.** Before each call the history copy is shortened to the model's window (old tool results, then old turns, with a note and
  a notice); the stored chat is not changed. Skills come as their first 7 KB plus a list of sections (`read_skill section=`).
- **Retries and readable failures in every adapter.** 408, 425, 429, 5xx, 529 and connection failures are retried before streaming starts
  (4 attempts, backoff, `Retry-After`), announced as notices; a 120 s (300 s local) stall ends the turn; errors say what to do ("the API key was not
  accepted. Check it in Settings, Providers", "Start the Ollama app", "the conversation is too long for this model's context window") and never show a URL.
- **Provider rules.** DeepSeek: reasoning sent back with tool results, `thinking` switch, cache hit counters. OpenAI reasoning models:
  `max_completion_tokens`, no `temperature`. Anthropic: `cache_control` on the system prompt, the last tool and the end of the history; cache reads
  and writes counted. Cut-off tool calls are reported as such; one stray closing brace is forgiven.
- **Cost.** Usage has cache read and write and reasoning tokens. Cost is priced per model call (DeepSeek with off-peak halving, the Claude
  models, your own `prices`), shown after each reply in the terminal and the desktop toolbar. DeepSeek's prefix cache served 80 to 99 % of the prompt in the runs.
- **Team mode.** A short request about one specialist's apps goes straight to it (no plan, review or summary: three or more model calls
  fewer); smart review skips the leader's review of a worker that called tools without an error; results passed on are capped at 4,000
  characters. Settings `direct_route`, `smart_review`.
- **MCP.** The older HTTP+SSE transport (and automatic fallback from Streamable HTTP); a busy server is retried and a forgotten session is
  renewed; servers start with a 30 s limit and are listed in parallel with a 15 s limit; a server that cannot restart is left alone for 5 s
  up to 5 minutes instead of being restarted on every step.
- **IDE pairing.** `workcraft-cli mcp setup` (and `clients`, `snippet`) adds a `workcraft` server to Claude Code, Claude Desktop, Cursor,
  Windsurf, Antigravity, Gemini CLI, OpenCode and VS Code: only that entry changes, a backup is kept, edited entries and broken files are left alone.
- **Icons.** The PdfCraft icon was out of date (the app now has a red background); PdfCraft, Craft Anything, MaterialCraft and TextureCraft
  were regenerated from the 1024 px masters in the apps' sources, the other 13 already matched.
- **Tests.** About 50 new tests (retry and error rules against a local HTTP server, the three adapters' request bodies and usage, prices, context fit,
  router, skills sections, the hub cooldown, SSE and flaky-HTTP servers, IDE file edits, direct routing, loop guard).
- **Not verified.** Anthropic, OpenAI, Gemini, Grok and GLM were never called (no keys); the DeepSeek 400 for a missing `reasoning_content` could
  not be reproduced; a real `mcp setup` on this computer and an IDE starting the server; a Team run with real models after these changes; the
  desktop toolbar line and the new icons were not looked at in a window; the local models are still unreliable (one run each).

## [0.1.0-alpha.3] - 2026-11-06

### SolidCraft checked against its source, and a first run with real models

- **SolidCraft, from its source.** The source is now in `artcraft/apps/solidcraft-0.30.1`; `docs/mcp.md` and the app's skills
  were read and the server probed. What was wrong in alpha.2: the skill and the format tables said SolidCraft exports
  GLB, SVG, DXF and IGES and saves `.solcraft`. In fact `export` writes only `step`, `3mf`, `3mf-tagged`, `stl`, `stla`,
  `obj` and `urdf`, native files are `.sc`, and a 2D DXF of a view is the command `file.export_view_dxf` (it needs a full
  camera). The handoff to TextureCraft is now STL and OBJ, not GLB. Spec, skill, config prompt and docs are corrected.
- **SolidCraft tool list.** Lean profile is now 14 tools (15 live): added `skill_reference` (the app's own modelling
  guide, which its server instructions point to first), `undo` and, live only, `pending`; dropped `describe_command` and
  `check_parameters` (a free `execute engine.describe` replaces the first). 4,666 tokens.
- **SolidCraft approvals.** `engine.describe`, `engine.commands`, `document.revision`, `model.edges`, `model.faces`,
  `model.threads` and `file.slicers` are free reads; `file.send_to_slicer` and `file.open_in_easyprint` start another
  program and are gated even in Auto mode. A policy test covers both, including inside a `batch`.
- **SolidCraft tests.** The headless workflow now builds a 40 x 30 x 12 box through `batch` with `expect`, checks the
  measured volume (14,400 mm3), proves a wrong `expect` is an error and `undo` repairs it, takes a picture, exports STEP,
  STL, 3MF and OBJ, confirms GLB is refused, saves `.sc`, writes the DXF and reopens the STEP with the same volume. A
  fourth live window test (`apps_live`) builds and measures the same box in the real SolidCraft window.
- **Real models (L6).** Five prompts per model through `workcraft-cli chat`, details in docs/CRAFT-APPS.md. DeepSeek
  (`deepseek-chat`) completed 5 of 5 against the real apps; `qwen2.5:7b` and `qwen3.5:4b` completed none.
- **Fixed because of those runs.** A tool call whose JSON has one closing brace too many is accepted (DeepSeek did this);
  `null` optional arguments are dropped before a Craft call; calling a tool an app does not have returns the app's real
  tool names; `read_skill` accepts `skill` for `name` and ignores `file: "skill"`; the agent loop stops after six
  identical calls in a row, not only after three identical failures. Four new tests.
- **Docs corrected.** The hosted-model tool cap is 120, not 128 (code was right, alpha.2 text was wrong).
- **Not verified.** Live windows of 13 of the 17 apps, any local model completing a task, and whether the SVG and PNG
  files the models produced look right.

## [0.1.0-alpha.2] - 2026-11-06

### Full ArtCraft coverage: 17 Craft apps, found, connected on demand and tested against the real installs

- **All 17 apps are found.** The 16 ArtCraft apps plus SolidCraft. Before, `craft.list` reported eight apps installed by the
  ArtCraft Launcher (`C:\Program Files\ArtCraft\<App>\`) as not installed. Discovery now checks `<APP>_CLI`, the
  `craft_apps` override, `Program Files\<App>`, `Program Files\ArtCraft\<App>`, `%LOCALAPPDATA%\Programs` and `PATH`.
  New connectors: TextureCraft, MaterialCraft, SolidCraft and Craft Anything (with icons and bundled skills).
- **One registry.** `crates/agents/src/craft/` (`spec.rs`, `discover.rs`, `live.rs`, `policy.rs`, `quirks.rs`,
  `handoff.rs`) replaces `craft.rs` and the app lists that were copied into seven files. Adding an app is one entry.
- **Modes.** Headless is the default and never opens a window. Live (the desktop window over `--control`) is chosen with
  `craft.mode`, the Settings chips, `craft.open` or `craft_activate`. Fixed, unique control ports; SoundCraft no longer
  starts with `--demo`. Apps connect on demand (`@app`, an agent scope or `craft_activate`); the document suite stays on.
- **Safe sandbox roots.** Headless servers get the workspace and the folders you add, not the home folder (PhotoCraft and
  FigCraft used to get the whole home folder). TextureCraft and MaterialCraft paths are made relative to their root.
- **The model can see.** MCP images are kept, saved to `.workcraft/artifacts/`, shown as thumbnails on the tool card and
  sent to vision models (Claude, OpenAI-compatible, Ollama). Before, every picture became the text `[image: image/png]`.
- **Lean tool lists.** At most 16 tools and 5,000 tokens per app (enforced by a test), 60 tools in total for local models
  and 120 for hosted models, `@mentioned` apps first. The full lists of the apps were about 178k tokens.
- **Approvals computed, not guessed.** `readOnlyHint` and `destructiveHint`, per-app risk tables for generic tools and a
  longer name heuristic (`render`, `screenshot`, `measure`... are reads). Paid generation (Craft Anything), script and
  code execution, permanent deletion, signing and printing **ask even in Auto mode**, with the reason on the card.
- **Uniform verbs.** `craft_status`, `craft_activate`, `craft_open`, `craft_new`, `craft_save`, `craft_inspect`,
  `craft_render`, `craft_export`, `craft_commands`, `craft_run`, `craft_batch` and `craft_handoff` (file handoff between
  apps, no silent re-encoding). The native names (`photocraft__doc_open`) stay.
- **Robust MCP client.** Typed errors (`app_not_installed`, `app_not_running`, `denied_by_root`, `tool_failed`) with a hint,
  child-death detection with one automatic reconnect, `tools/list` caching with `list_changed` invalidation, per-tool
  timeouts, `notifications/cancelled`, idle disconnect.
- **Team mode.** Specialists `image`, `design`, `video`, `three-d`, `document` and an optional `explorer` (Craft Anything).
  A per-app writer lease keeps two agents from writing to one app at once. Craft Anything runs `--clean-room` and its
  questions and approvals appear as WorkCraft approval cards.
- **Desktop and CLI.** `live`/`headless` badges on tool cards, clickable render thumbnails, gated tools marked, Settings >
  Tools & MCP with all 17 apps (version, port, mode chips, Test, Open window, roots editor, full tool catalog switch),
  `@app` autocomplete and voice vocabulary for all 17. `workcraft-cli` prints mode, artifact paths and approval reasons.
- **Tests, against the real apps.** 17 recorded tool snapshots and a live drift check of all 17 installed servers
  (`tests/apps_contract.rs`); one headless golden workflow per app, a cross-app handoff and four failure cases
  (`tests/apps_headless.rs`, 22 tests); three live windows driven and stopped by PID (`tests/apps_live.rs`:
  PhotoCraft, VectorCraft, FilmCraft); 19 engine tests with scripted models. The live suites are `#[ignore]`d.
- **Fixed on the way.** 13 clippy lints of the newer toolchain; a duplicated `child_pid` method; VectorCraft's lean list
  trimmed to 12 tools to stay under the 5,000-token cap.
- **Docs.** New [docs/CRAFT-APPS.md](docs/CRAFT-APPS.md): per-app coverage, modes, ports, roots, approvals, handoff, how to run the tests.
- **Not verified.** A run with a real local model on the new tools (`qwen3.5:4b`), live windows of the other 14 apps,
  Linux and macOS. The TypeScript workspace was not changed.

## [0.1.0-alpha.1] - 2026-11-05

### First documented alpha

- **Status.** WorkCraft chat is the desktop-only AI chat of the WorkCraft projects, written in Rust. It is not released and
  becomes the next project once the WorkCraft workspace has a stable release. The workspace stays the main project.
- **What is there.** One engine with three ways in: the egui desktop app, the `workcraft-cli` command line and an MCP server
  plus loopback control channel. Single and Team mode (leader, parallel specialists, review, hand-off, shared memory,
  plan approval), providers (Ollama, LM Studio, Claude, ChatGPT, Gemini, DeepSeek, Grok, GLM, Mistral, Groq, OpenRouter,
  Together AI and any OpenAI-compatible endpoint), skills in the agentskills.io format (the 13 artcraft-skills), the 13 Craft
  apps as MCP servers, sessions as append-only JSONL with branches, and approvals.
- **Naming.** The product is called WorkCraft everywhere: `WorkCraftMcp` (before: `WorkcraftMcp`), the Windows file
  description `WorkCraft.exe`, the binaries `WorkCraft.exe` and `WorkCraft-CLI.exe`. The data folder is `~/Workspace/chat`.
- **Alpha versioning.** The version is `0.1.0-alpha.1` (before: 0.1.0), including the Windows file version.
- **Docs and sources.** README, AGENTS.md, CLAUDE.md, ARCHITECTURE.md and the new SOURCES.md describe the three projects
  and the rules for AI agents. `cargo fmt` was run over the whole repository.
