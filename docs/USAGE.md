# WorkCraft AI: usage and reference

This page is the long reference that used to be the README (the repository was called WorkCraft chat before). The short introduction is the [README](../README.md).

The chat and agent control surface of the Craft suite, in Rust. Talk to a model, let it use the Craft
apps through MCP or their control channels, or give a goal to a team of agents. One engine, three ways in:
a desktop app (egui), a headless CLI, and an MCP server plus control channel so other agents can drive it.

Version `0.1.0-alpha.9`. This is **desktop only**: it is not a web app. It is an alpha.

## The three WorkCraft projects

| Project | What it is | Status |
| :--- | :--- | :--- |
| **WorkCraft workspace** (`visoire/workcraft`, TypeScript, in `../workcraft`) | The main project: the open-source workspace with the AI assistant, Team mode, MCP and the four apps | Open source, alpha |
| **WorkCraft home** (`visoire/workcraft-home`, in `../workcraft-home`) | The main homepage | Private, not open source |
| **WorkCraft chat** (this repository, `visoire/workcraft-ai`) | The AI chat for the desktop only, in Rust | Not released. It becomes the next project once the workspace has a stable release |

It does not change the workspace. The providers, agents, skills and MCP ideas flow from the workspace into this chat,
so the two stay close. Design and a survey of the Craft apps it connects to: [docs/ARCHITECTURE.md](ARCHITECTURE.md).
Every GitHub repository used here is listed in [SOURCES.md](../SOURCES.md).

## Try it now (Windows)

Run `dist\WorkCraft.exe` (the desktop app) or `dist\WorkCraft-CLI.exe chat` (terminal). Data goes to `~/workcraft/chat`. For a hosted model set
`DEEPSEEK_API_KEY` (or another provider's key) before starting, or add the key in Settings, Providers; for a local one start Ollama.
Rebuild them with `cargo build --release -p workcraft-app -p workcraft-cli`. What inspired the design: [USED-REPOS.md](../USED-REPOS.md).

## Build and run

Needs Rust 1.90 or newer (built here with 1.99) and, on Windows, the MSVC Build Tools.

```sh
cargo build --release
cargo run -p workcraft-app --bin workcraft            # the desktop app
cargo run -p workcraft-cli -- chat "Summarise notes.txt"   # one answer in the terminal
cargo run -p workcraft-cli -- chat                    # interactive
```

Local models need [Ollama](https://ollama.com) running. For hosted models set `OPENAI_API_KEY` or
`ANTHROPIC_API_KEY`, or add providers in `config.json`.

Data lives in `~/workcraft/chat` (override with `WORKCRAFT_CHAT_HOME`; chats from `~/Workspace/chat` move there once; Settings, Data folder has an Open folder button):

| Path | What |
|---|---|
| `config.json` | model (`provider:model`), approval mode, `think`, `num_ctx`, `max_steps`, extra providers |
| `mcp.json` | MCP servers, Claude Desktop format ([example](../examples/mcp.json)) |
| `control-targets.json` | control channels of running Craft apps ([example](../examples/control-targets.json)) |
| `agents/*.json`, `teams/*.json` | agent and team definitions ([example](../examples/agents/colorist.json)) |
| `skills/` | extra skills; `~/.agents/skills` and `$WORKCRAFT_SKILLS` are read too |
| `sessions/*.jsonl` | one file per chat, append only, branches by parent id |
| `workspace/` | the folder the built-in file tools may touch |

## What it does

- **Chat.** Streaming, Markdown and code blocks, reasoning shown separately, image and text attachments
  (drop files on the window), many sessions, branching (edit an earlier message, switch branches with `<` `>`),
  switch model or provider in the middle of a chat.
- **Single and Team mode.** Team mode: a leader plans a task graph, specialists run in parallel with scoped
  tools, the leader reviews each result (retry with a note, add tasks), specialists can hand off to a
  different specialist, all share a small memory board, and in manual mode the plan itself needs approval.
  The built-in `craft` team has image, design, video, three-d and document specialists (and an optional explorer),
  scoped to the matching apps, with one writer per app at a time.
- **Tools.** Built-in file tools confined to the workspace, every MCP tool as `<server>__<tool>`, and
  `craft_commands` / `craft_run` for a running Craft app's control channel. Reads never ask; in manual
  mode every write waits for Allow or Deny (UI, CLI prompt, or `approval.respond` from anywhere).
- **Reliable and cheap to run.** Retries with notices, a stall guard and plain error messages in every provider adapter; DeepSeek, OpenAI and
  Anthropic rules (reasoning, caching, token parameters); tokens, cache hits and cost after every reply; a tool router that sends only the
  tools the chat needs (36,480 to 6,578 prompt tokens for a greeting); history fitted to the context window; Ollama always told its window.
  `workcraft-cli mcp setup` adds WorkCraft to Claude Code, Cursor, Windsurf, OpenCode and other IDEs. See [docs/AI.md](AI.md).
- **17 Craft apps.** PhotoCraft, VectorCraft, FilmCraft, LightCraft, PdfCraft, EffectCraft, DesignCraft, FigCraft,
  TextureCraft, MaterialCraft, WordCraft, SoundCraft, DeckCraft, CADCraft, GridCraft, Craft Anything and SolidCraft are
  found on the computer (also under `Program Files\ArtCraft\`) and connected as MCP servers on demand, headless by
  default and live in the app window on request. Pictures they return (renders, previews, screenshots) are saved,
  shown on the tool card and sent to vision models. Paid, code-running and deleting tools always ask. Uniform
  `craft_*` verbs and `craft_handoff` move work between apps. Details, ports and tests:
  [docs/CRAFT-APPS.md](CRAFT-APPS.md).
- **Skills.** agentskills.io format. The 17 bundled Craft skills (the 13 `artcraft-skills` plus TextureCraft,
  MaterialCraft, SolidCraft and Craft Anything); the prompt carries a one-line
  index and the model reads a skill with `read_skill` when it needs it.
- **Desktop window.** Light, dark or system theme (Inter, neutral grays, the WorkCraft blue for primary
  actions). Chats grouped by date with search; a model picker grouped by provider (Local / API) with
  search; a Single / Team switch; tool calls as cards with the Craft app icon, status, input and output;
  code blocks with Copy; folded thoughts. In Team mode the activity panel shows each agent (colour, icon,
  live status: thinking, using a tool, needs you, finished), the plan as steps that run in parallel with a
  progress bar, and every tool call. Approvals show above the composer with Allow / Deny. Settings
  (Ctrl+,) cover theme, default model, reasoning, context window, tool steps, instructions, providers,
  approval mode, the team, MCP servers and skills. Window preferences live in `ui.json`.
- **Providers.** Settings > Providers lists Ollama and LM Studio (local), Claude, ChatGPT, Gemini,
  DeepSeek, Grok, GLM, Mistral, Groq, OpenRouter and Together (cloud), each with its logo and status
  (ready with N models, key not accepted, not running), plus any OpenAI-compatible endpoint you add.
  Commands: `provider.info`, `provider.save`, `provider.remove`. Gemini goes through Google's
  OpenAI-compatible endpoint; every cloud provider except Claude uses the OpenAI adapter.
- **Attachments.** The + button opens the system file dialog; files can also be dropped on the window
  (an overlay shows while dragging). Images up to 8 MB, text files up to 200 KB.
- **Every action is a command id** (`chat.send`, `team.run`, `model.set`, `approval.respond`, ...):
  `workcraft-cli commands` lists them. The UI, the CLI, the control channel and the MCP server all call the
  same bus.

## Driving it from outside

```sh
workcraft --control            # app with the control channel; address and token go to control.json
workcraft-cli serve            # the same, headless
workcraft-cli mcp              # MCP server on stdio
claude mcp add workcraft -- workcraft-cli mcp
```

The control channel is the Craft protocol: loopback TCP only, one JSON line per request, first line
`{"id":"auth","method":"auth","params":{"token":"<64 hex>"}}`, then `engine.execute`, `engine.commands`,
`events.subscribe`. Every WorkCraft command is also an MCP tool (`chat.send` is `chat_send`), plus
`command_list` and `command_run`. A chat started from outside shows up live in the window.

## What has been verified, and what has not

Verified on Windows 11 (`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo test --workspace` pass):

- Unit and integration tests for every crate, including the scripted-model engine tests (tools, approval,
  branching, cancel, loop guard, team order, retry, hand-off, plan approval, gated tools, writer leases, vision) and an
  MCP round trip of WorkCraft's own client against the real `workcraft-cli mcp` binary over stdio.
- **All 17 installed Craft apps, headless:** every `<app>-cli mcp` server starts and matches its recorded tool snapshot,
  and one workflow per app passes against the real app (`apps_headless`). **Four live windows** (PhotoCraft,
  VectorCraft, FilmCraft, SolidCraft) were started with `--control`, bridged, read, written and screenshotted (`apps_live`).
  Craft Anything was only used read-only in `--clean-room`; no paid tool was called.
- **Real models:** five Craft prompts through `workcraft-cli chat`. DeepSeek (`deepseek-chat`) completed 5 of 5
  against the real apps; `qwen2.5:7b` and `qwen3.5:4b` completed none (see [docs/CRAFT-APPS.md](CRAFT-APPS.md)).
- Live, against a local Ollama: single mode with a tool call (`qwen3.5:4b`), and a team run with parallel
  specialists, review and a written file (`qwen2.5:7b`, 49 s).
- The desktop window, started with `--control` and driven through the channel while it rendered the chat.
- The 17 bundled Craft skills parse.

Not verified:

- The OpenAI-compatible and Anthropic adapters were only tested against the documented wire format, never
  against a live API (no keys were used).
- The Streamable HTTP MCP client was never run against a server.
- Live windows of the other 13 Craft apps (their bridges follow the apps' documented flags), a local model that
  finishes a Craft task, and whether produced pictures look right: the workflows check results and files, not aesthetics.
- The redesigned window was checked from window captures in both themes, with a single-mode answer
  (thoughts, list, code block) and a team run (plan approval clicked in the window, two specialists,
  review). Not clicked through by hand: the Settings window, branch switching, attachments, the model
  picker popup.
- macOS and Linux were not built. Nothing is packaged or signed ([packaging](../packaging/README.md)).
- Small local models are weak at planning and tool use. A reasoning model that ends without an answer is
  retried once with thinking off; the 4B model was still slow (about 90 s for one tool call on a 6 GB GPU).

Not built yet: a native Gemini adapter (the OpenAI-compatible endpoint is used), the MCP server over
HTTP, Markdown tables and links, wasm.

UI test hook: `WORKCRAFT_UI_OPEN` opens one screen at start so the window can be checked from a capture:
`settings:<general|models|providers|agents|tools|skills>[@scroll]`, `provider:<id>`, `custom`, `picker`,
`chat:<session>`, `attach:<path>|<path>`.

The app bundles Inter (SIL Open Font License 1.1, from Open WebUI's static fonts) and 64 px copies of the
17 Craft app icons in `crates/app/assets/`.

Live Craft tests (opt in, they start real apps; see [docs/CRAFT-APPS.md](CRAFT-APPS.md)):
`cargo test -p workcraft-agents --test apps_headless -- --include-ignored` and `--test apps_live`.

## Notes

- Uses `eframe` 0.37 from crates.io, the same version family as the Craft apps, not their pinned fork.
  Combo boxes are egui's own, which the Craft apps replace to work around egui#5225.
- A provider key is either saved from Settings into `config.json` in the data folder (plain text on this
  computer, as the WorkCraft workspace does) or read from the environment variable named in `key_env`.
  Keys are never written to sessions, and `config.get`, `config.set` and `provider.info` (so also the
  control channel and MCP) only ever return `has_key` and a hint such as `sk-…abcd`.
- `control.json` holds the control token in plain text in the data folder while the app runs, like the
  Craft apps' `--control-token-file`. It is removed on exit.

## Versioning and rules for AI coding agents

WorkCraft chat is in an **alpha phase** (`0.1.0-alpha.N`) until the workspace is stable and this project starts for
real; no stable release is created until the maintainer says so explicitly. **Every meaningful change is a new alpha
version with a changelog entry.** If you are an AI agent, do all of this in the same change, without being asked:

1. **Raise the alpha number** (`0.1.0-alpha.N` to `N+1`) in `Cargo.toml` (`[workspace.package] version`) and in the
   version strings of `crates/app/build.rs`; `Cargo.lock` follows when you build.
2. **Update `CHANGELOG.md`:** a new entry at the top under `## [0.1.0-alpha.N] - YYYY-MM-DD`. Never rewrite older entries.
3. **Update the docs:** this README, `AGENTS.md`, `docs/ARCHITECTURE.md` and `packaging/README.md`, wherever the change
   is described.
4. **Update `SOURCES.md`** when a crate, protocol, provider or idea from another repository is added.
5. **Run the checks:** `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`.

Licence: MIT OR Apache-2.0.
