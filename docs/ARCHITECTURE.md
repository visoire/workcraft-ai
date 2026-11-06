# WorkCraft architecture

WorkCraft is the chat and agent control surface of the Craft suite: a native Rust app (egui/eframe)
plus a headless CLI, both on one engine. Humans talk to models, models call tools, and the tools
reach every Craft app over MCP or the app's own control channel.

This repository is the Rust implementation of **WorkCraft chat**, the desktop-only AI chat. It is the third WorkCraft
project: the open-source workspace in `../workcraft` is the main project and stays as it is (it is the reference for the
Team mode design, the task DAG, leader review and the tool vocabulary), `../workcraft-home` is the private homepage, and
this chat becomes the next project once the workspace has a stable release. Every GitHub source is in
[SOURCES.md](../SOURCES.md).

## What already exists in the tree (survey)

| Source | What we take from it |
|---|---|
| `artcraft/mcp/artcraft-mcp-v1.1.0/rust/artcraft_mcp_cli` | Hand-written JSON-RPC over stdio, protocol `2025-03-26`/`2024-11-05`, tool results as `content[] + isError`. We follow the same minimal approach. |
| `<app>-cli mcp` (PhotoCraft, FilmCraft, ...) | Each Craft app ships an MCP server over stdio: `command_list`, `command_run`, `command_batch`, `doc_*`, `jobs_*`. Flags such as `--compact`, `--headless`, `--bridge 127.0.0.1:<port>`. |
| Control channel (`--control <port>`) | Authenticated loopback TCP, one JSON line per request, first line `auth` with a 64-hex token, then `engine.execute`, `engine.commands`, `ui.inspect`, `app.open`, `jobs.*`. Every user action is an engine command id (about 700 in PhotoCraft). |
| `artcraft-skills/*/SKILL.md` | 13 agentskills.io skills (WorkCraft adds 4 more for TextureCraft, MaterialCraft, SolidCraft, Craft Anything) (YAML front matter `name`, `description`, then Markdown). Loaded as agent knowledge. |
| `craft-ai-integrations` | Provider contract, consent, secrets. Used as a design reference; WorkCraft keeps its own thin provider trait so it builds without the egui fork. |
| `craft-libs` and `craftrules` | Never-crash lints, `[lints] workspace = true`, `MIT OR Apache-2.0`, edition 2024. Copied exactly. |
| `../workcraft/src/chat/team.ts` | Roles, task DAG, concurrency limit, leader review with bounded retries. Re-implemented in Rust. |

## Crates

```
workcraft-core      data model: Message, Role, ToolSpec, ToolCall, Event, AgentDef, TeamConfig, Error, paths. No network, no async runtime.
workcraft-llm       Provider trait + adapters: Ollama (native /api/chat), OpenAI-compatible, Anthropic.
workcraft-store     sessions as JSONL (one file per session, append-only, branch = parent pointer).
workcraft-mcp       JSON-RPC 2.0, MCP client (stdio child process, Streamable HTTP) and MCP server.
workcraft-control   command-id registry + loopback control channel server and client.
workcraft-agents    tool registry, single agent loop, team orchestration, skills loader.
workcraft-cli       headless binary: chat, serve (control channel), mcp (stdio server), commands.
workcraft-app       egui/eframe desktop app on the same engine.
```

Dependency direction: `core` <- `llm`, `store`, `mcp`, `control` <- `agents` <- `cli`, `app`.
`core` has no async runtime. A wasm build is a stated goal for later and has not been tried yet (`uuid` and `chrono` need their web features there).

## Data flow

```
UI (egui) --------\
CLI --------------+--> Command bus (command ids) --> Engine --> Agent loop --> Provider (Ollama/OpenAI/Anthropic)
Control channel --/                                      |            |
MCP server  ------/                                      |            +--> Tool registry
                                                         |                  |- builtin (files, memory, ask_user)
                                                         |                  |- MCP client tools  <app>.<tool>
                                                         |                  |- control-channel tools (engine.execute)
                                                         |                  `- skills (read on demand)
                                                         `--> Store (JSONL) + Event stream (UI, control, MCP notifications)
```

Every important action is a command with a stable id (`chat.send`, `chat.new`, `chat.branch`,
`session.list`, `model.set`, `mode.set`, `team.run`, `approval.respond`, `mcp.connect`, ...). The UI
buttons, the CLI sub-commands, the control channel and the WorkCraft MCP server all dispatch the
same ids through one `CommandBus`, so nothing is UI-only.

## Providers

`Provider` is an async trait: `chat_stream(request) -> Stream<ChatDelta>`. A delta is text, reasoning,
a tool call, usage, or done. Switching local to cloud mid-conversation is a change of the
`ModelRef { provider, model }` on the next turn; history is provider neutral (`Message`).

| Adapter | Wire format | Notes |
|---|---|---|
| Ollama | NDJSON on `/api/chat` | tools, `think`, `num_ctx`, `keep_alive`, images; default `http://127.0.0.1:11434` |
| OpenAI-compatible | SSE on `/chat/completions` | any base URL: OpenAI, DeepSeek, LM Studio, vLLM, OpenRouter |
| Anthropic | SSE on `/v1/messages` | tool_use blocks, system prompt, images |
| Google | planned | same trait |

API keys come from the environment or a key file in the data dir; they never enter session files.

## MCP surface

Client: connects to servers declared in `mcp.json` (same shape as Claude Desktop: `command`, `args`,
`env`, or `url`). Each Craft app is one entry, for example `photocraft-cli mcp`. Tool names are exposed
to the model as `<server>__<tool>`; schemas are passed through unchanged. Results are truncated by a
byte budget before they enter the context.

### AI core (alpha.4)

Retries, a stall guard and readable errors wrap every adapter; provider rules live in `openai::Quirks` (OpenAI reasoning models,
DeepSeek reasoning replay and thinking switch) and the Anthropic body (prompt caching); usage carries cache and reasoning tokens
and is priced per model call; the agent copies the history through `context::fit` before each call and routes Craft app tools
through `craft::router`. Details, measurements and what was not verified: [AI.md](AI.md).

### Craft apps (alpha.2)

The vendors' own `<app>-cli mcp` servers are reused, not reimplemented. `crates/agents/src/craft/` wraps them:

```
SPECS (spec.rs, 17 AppSpec) --> discover.rs  --> Found { cli, gui, version }
                          \--> live.rs      --> ServerEntry (headless: roots, no window | live: --control port + token)
                           \-> policy.rs    --> lean filter, tool budget, Read | Mutate | Gated
                            \> quirks.rs    --> path rewriting, image/data-URI normalization
Tools::call --> classify --> approval (Gated asks even in Auto) --> McpHub.call --> images saved to
               .workcraft/artifacts/ --> Event::ToolFinished { artifacts, mode } --> card thumbnail / vision block
```

- **Modes:** `headless` (default, stdio, no window), `live` (window started with `--control <port>`, server bridged),
  `off`. Ports are unique per app and checked at start (`verify_unique_ports`). Auth per app is in
  [CRAFT-APPS.md](CRAFT-APPS.md).
- **Budget:** per-app lean profiles (at most 16 tools, 5,000 tokens). Since alpha.4 a request carries only the tools of the apps
  the chat is about (`craft/router.rs`); the 60 (local) / 120 (hosted) tool cap still applies after that, `@mentioned` apps first.
- **Vision:** `ToolResult.images` (4 MiB each, 8 per call) goes to Anthropic as `tool_result` image blocks and to
  OpenAI-compatible and Ollama providers as a follow-up image message; non-vision models get the file path.
- **Team:** a per-app async writer lease in `Shared::app_leases` serializes writes to one app; reads and writes to
  different apps stay parallel.
- **Craft Anything:** started `--clean-room --inbox <dir>`; `<id>.request.json` files become WorkCraft approval
  cards and the answer is written atomically to `<id>.answer.json`.
- **MCP client:** `tools/list` cached and invalidated by `notifications/tools/list_changed`; a dead stdio child is
  detected and restarted once; calls have per-tool timeouts and send `notifications/cancelled`.

Server: `workcraft-cli mcp` (stdio) exposes the command bus:

| Tool | Does |
|---|---|
| `chat_send` | send a message in a session, wait for the answer, return text and tool trace |
| `chat_new`, `chat_list`, `chat_get`, `chat_branch` | session management |
| `team_run` | run a team (or a named preset) on a goal |
| `agent_define`, `agent_list` | create and list agent definitions |
| `model_list`, `model_set`, `mode_set` | model and mode control |
| `approval_list`, `approval_respond` | human-in-the-loop from another agent |
| `command_list`, `command_run` | generic access by command id, same names as the Craft apps |

## Modes

Single mode: one agent, one session, tools on. Team mode: a leader plans a task graph, workers run
with scoped tools, the leader reviews.

Orchestration patterns, all built from the same `TaskGraph` scheduler:

- Sequential pipeline: each task depends on the one before.
- Parallel: independent tasks run up to `concurrency`.
- Swarm: the leader may append tasks while the graph runs (bounded by `max_tasks`).
- Hand-off: a worker returns `handoff { to, note }`, the scheduler adds a dependent task for that role.
- Shared memory: a key/value board all agents of a run can read and write (`memory_get/set`).
- Supervision: the leader reviews each result; rejection retries up to `max_retries` with a targeted note.
- Human in the loop: tool calls and task results marked `needs_approval` pause the run until
  `approval.respond` arrives from the UI, CLI, control channel or MCP.

## Never crash

Every crate root starts with the Craft lint attribute (deny `unwrap_used`, `expect_used`, `panic`,
`todo`, `unimplemented`, `unreachable`), `unsafe` is forbidden by the workspace, and clippy runs with
`-D warnings`. Errors are values: a tool failure goes back to the model as a tool result, a provider
failure becomes an `Error` event, a damaged session line is skipped and counted, and the UI thread
only receives events over channels. Guards that exist and are tested: a loop guard (three identical
failing calls end the turn), a step limit per message, a hand-off cap and depth limit in teams, a key
limit on the shared memory board, byte limits on tool output, path confinement for the file tools, and
a retry with thinking off when a reasoning model ends without an answer.

## Persistence

Sessions are JSONL in `<data>/sessions/<id>.jsonl`: a header line, then one line per message with
`id` and `parent`. A branch is a message whose parent is not the latest; the session index is rebuilt
from the files, so a damaged line is skipped and reported, never fatal. Data dir: `~/workcraft/chat`
(shared with the TypeScript workspace data root), override with `WORKCRAFT_CHAT_HOME`.

## Packaging

Release matrix follows `craft-common-actions`: macOS universal, Windows x64 and ARM64, Linux AppImage,
deb and rpm. This repository carries the workspace and CI skeleton; signing and publishing reuse the
shared actions. A wasm build of `workcraft-core` is kept compiling to leave room for a web version.

## Build order and status

Built as vertical slices. See the README for what has been verified and what has not.

1. core, llm (Ollama, OpenAI-compatible, Anthropic), store, agents single mode, CLI chat. Done.
2. MCP client (stdio and Streamable HTTP), control channel client and server, skills loader. Done.
3. Team orchestration (plan, validate, parallel scheduler, review, retry, hand-off, shared memory,
   plan approval). Done.
4. egui app on the same engine. Done, first version.
5. WorkCraft MCP server over stdio. Done. Streamable HTTP server, Google adapter, packaging and signing: not done.
