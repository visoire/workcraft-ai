# AGENTS.md: WorkCraft chat

Context for AI coding agents and people new to this repository. Read it fully before you change anything. When this
file and the code disagree, the code wins; fix this file in the same change.

## What this repository is

WorkCraft chat: the AI chat and agent control surface of the Craft suite for the **desktop only**, in pure Rust. One
engine with three ways in: an egui desktop app (`workcraft`), a headless CLI (`workcraft-cli`) and an MCP server plus
loopback control channel so other agents can drive it. It is the third WorkCraft project:

| Project | Visibility | Role |
| :--- | :--- | :--- |
| `visoire/workcraft` | public, open source | the workspace (TypeScript): the main project; providers, agents, skills and MCP ideas flow from it into this chat |
| `visoire/workcraft-home` | private, not open source | the main homepage |
| `visoire/workcraft-ai` | public, open source (MIT OR Apache-2.0), alpha | this repository: WorkCraft AI, formerly called WorkCraft chat; published with `scripts/publish.ps1` (see `docs/PUBLISHING.md`) |

Every GitHub repository used here is listed in `SOURCES.md`.

## Versioning, changelog and docs: do this with every change

This project is in an **alpha phase** (`0.1.0-alpha.N`). There is no stable release and none is created until the
maintainer says so explicitly. Every meaningful change is a new alpha version with a changelog entry. Do all of this in
the same change, without being asked:

1. Raise the alpha number (`0.1.0-alpha.N` to `N+1`) in `Cargo.toml` (`[workspace.package] version`) and the version
   strings of `crates/app/build.rs` (`FileVersion`, `ProductVersion`).
2. Add an entry at the top of `CHANGELOG.md` under `## [0.1.0-alpha.N] - YYYY-MM-DD`. Never rewrite older entries.
3. Update every Markdown file that describes the change: `README.md`, this file, `docs/ARCHITECTURE.md`,
   `packaging/README.md`.
4. Update `SOURCES.md` when a crate, protocol, provider or idea from another repository is added.
5. Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`.

## Publishing

The repository is prepared for GitHub: `README.md` is the short public page (the long reference is `docs/USAGE.md`), community files and issue forms are in
`.github/`, `.github/repo.json` holds the description and topics, `.github/workflows/release.yml` builds the Windows pre-release, and
`scripts/publish.ps1` / `publish.sh` check, push, label and release. They print a dry run unless told `-Execute`. **An agent never pushes, tags or publishes
unless the maintainer asks for it in the same conversation**, never puts a key or a personal path in a file, and runs `scripts/publish.ps1 check` before it says the
repository is ready. Guide: `docs/PUBLISHING.md`.

## Chats, Blip and the typed commands

Each chat keeps its own state: `App.running` holds the sessions with a turn in flight, events carry their session, and `mine(session)` decides what the open chat shows
(`awaiting_first` adopts the id of a chat that was just started). Never reset `current` when a turn ends. Typed `/` commands run in `run_typed_command` before a message is
sent. Blip (`crates/app/src/blip.rs`) draws on a grid snapped to whole screen pixels: every offset goes through `q` (half or whole grid units), so never add fractional positions
to a rectangle; effects use `fxp` (they do not follow the body). `BlipAnim::Off` must stay free of repaint requests.

## Crates

```
crates/core      data model: messages, tools, events, errors, paths, AgentDef and TeamConfig
crates/llm       providers: Ollama (native /api/chat), OpenAI-compatible (SSE), Anthropic (SSE), registry; http.rs (retries,
                 stall guard, readable errors), price.rs (cost), toolargs.rs
crates/store     session storage: append-only JSONL with branches by parent id
crates/mcp       hand-written JSON-RPC: stdio, Streamable HTTP and SSE client, stdio server, hub (<server>__<tool>, reconnect
                 cooldown, parallel listing)
crates/control   command ids, loopback control channel (server and client), Craft protocol
crates/agents    the engine (command bus), agent loop, context.rs (window fit), tools, skills, team scheduler, craft/ (app registry,
                 router.rs: which app tools go in a request), WorkCraft MCP server
crates/cli       workcraft-cli: chat, serve, mcp (+ setup, clients, snippet: ide.rs), commands, run, models
crates/app       workcraft: egui app (theme, view_*, widgets, assets, providers presets)
```

Every action is a command id; the MCP tool name is the id with `.` replaced by `_`. The 17 Craft skills are bundled from
`crates/agents/skills/`.

### The AI core (docs/AI.md)

Rules that are easy to break: a request is only retried before the first byte is streamed (`llm/http.rs`); provider differences live in
`openai::Quirks` and the Anthropic body builder, never in the agent loop; the stored chat is never shortened, only the copy sent
(`agents/src/context.rs`); the tool router (`craft/router.rs`) computes its set from the history so it is stable between rounds (the
prompt cache depends on it); cost is recorded per model call (`Shared::record_usage`), never from the session model alone; a message for
a person says what to do next and never contains a URL or a key; `workcraft-cli mcp setup` edits only the `workcraft` entry of an IDE's
settings and never a file that does not parse.

### Agents, teams and approvals

`config.rs` builds the `craft` team (five specialists) and the per-app agents (`app_agents()`, `apps_team()`); `engine.rs` registers them (`build_apps_team` picks the
installed apps when a run starts). `AgentDef.approval` and `TeamConfig.plan_approval` are optional and default to the old behaviour; gated Craft calls ask
whatever they say. English only: no word of another language in the UI, the voice commands or the docs. The data folder is `~/workcraft/chat`
(`core/paths.rs`: `former_chat_dir` moves an older `~/Workspace/chat` once).

### The Craft app registry (`crates/agents/src/craft/`)

All per-app facts live in one table, `SPECS` in `spec.rs`; **never add an app list anywhere else.** To add an app: one
`AppSpec` entry (install dirs, CLI/GUI names, `<APP>_CLI`, roots policy, unique control port, auth, lean tools,
live-only tools, timeouts, formats, icon, skill, specialist), a 64 px icon in `crates/app/assets/icons/` +
`assets.rs`, a `SKILL.md` + `skills.rs`, a snapshot in `crates/agents/tests/snapshots/tools-<id>.json`
(`cargo run -p workcraft-agents --example probe_apps`). Modules: `discover.rs` (finding installs), `live.rs`
(headless/live servers, ports, tokens, own-PID tracking), `policy.rs` (read/write/gated classification, lean filter,
budget), `quirks.rs` (per-app argument and result fixes), `handoff.rs` (`craft_handoff`). Rules: headless is the
default and never opens a window; sandbox roots are the workspace plus user-added folders, never the home folder;
gated tools (paid, code execution, permanent delete, sign, print) ask even in Auto mode; a lean profile stays at or
under 16 tools and 5,000 tokens (`apps_contract` fails otherwise). Full reference: [docs/CRAFT-APPS.md](docs/CRAFT-APPS.md).

### Testing against the real apps

```sh
cargo test --workspace                                                        # offline
WORKCRAFT_LIVE_APPS=1 cargo test -p workcraft-agents --test apps_contract -- --include-ignored
cargo test -p workcraft-agents --test apps_headless -- --include-ignored --test-threads=4
cargo test -p workcraft-agents --test apps_live -- --include-ignored --test-threads=1   # opens real windows
```

Live tests must use a scratch folder, skip when the app is already running, and stop a window only by the PID they
started (the `Drop` guard in `apps_live.rs`). Never kill by process name or command-line pattern. Craft Anything only
with `--clean-room`, never a paid tool. On PowerShell set `WORKCRAFT_LIVE_APPS` with `$env:WORKCRAFT_LIVE_APPS = "1"`.

## Commands

```sh
cargo build --release
cargo run -p workcraft-app --bin workcraft
cargo run -p workcraft-cli -- chat
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

On Windows cargo lives in `%USERPROFILE%\.cargo\bin` (not on the shell PATH by default). If the repository moves, run
`cargo clean -p workcraft-agents` once: the skill bundle embeds absolute paths.

## Conventions and traps

- Pure Rust, edition 2024, `unsafe_code = "deny"`, MIT OR Apache-2.0, `[lints] workspace = true`.
- The product name is **WorkCraft** (type names `WorkCraftMcp`, files `WorkCraft.exe`); commands and crates stay lower
  case (`workcraft`, `workcraft-cli`). Say "workspace", never "IDE".
- eframe 0.37 API: `fn ui(&mut self, ui, frame)`, `Id::unique`, `DroppedFile` is a trait; egui only repaints on events, so
  animations need a repaint window.
- Test without touching real chats: `WORKCRAFT_CHAT_HOME=<scratch>` plus `--control`; `WORKCRAFT_UI_OPEN` opens a screen
  for window captures. Never kill processes by command-line pattern; stop only the process you started.
- A global auto approval overrides a team preset's manual approval, otherwise team runs wait for plan approval.
- Keys are saved masked: `config.get`, `config.set` and `provider.info` return only `has_key` and a hint.