# Inspiration sources

What was studied for WorkCraft chat, what was taken (as an idea, written fresh in Rust, no code copied), and why. Update it when a new
source shapes a change. `SOURCES.md` lists the crates and repositories the code depends on; this file lists what only inspired it.

"Studied" means read and used. "Skimmed" means opened and judged, with little taken. Nothing below was run except where it says so.

## Studied and used

| Source (local path under `artcraft/`) | Taken | Where in WorkCraft | Why useful |
| :--- | :--- | :--- | :--- |
| `apps/craft-anything-0.3.0/crates/ai` (`http.rs`, `price.rs`, `context.rs`, `anthropic.rs`) | Status errors with a next step and never an echoed key; per-model price table with cache read and write rates; old tool results shortened while every call keeps its result; `cache_control` placement | `llm/src/http.rs`, `llm/src/price.rs`, `agents/src/context.rs`, Anthropic body | Same Rust and egui ecosystem; its failure modes are documented in `docs/pitfalls.md`, several of which (a wait message that reassures, an error that points at nothing, a retry that repeats work) shaped the retry and notice rules |
| `apps/craft-anything-0.3.0/crates/ui-egui/src/chat.rs` | Chat items folded from events, suggestions per target, composer height measured and remembered | Confirmed the existing structure in `view_chat.rs` rather than changing it | Showed the current renderer is already close to a proven pattern |
| `workcraft/workcraft/src/chat` and `docs/AI-TOKENS.md` (the TypeScript workspace AI) | Tool router with `find-tools`, history compaction, smart review, straight-to-specialist routing, per-provider cache table | `craft/router.rs`, `context.rs`, Team `smart_review` and `direct_route` | The baseline to beat; kept the ideas, then made routing cache-stable and history-derived |
| `artifical-intelligence/documentation/documentations 05.11.2026` (DeepSeek thinking mode and pricing, Anthropic pricing, OpenAI references) | Reasoning replay, thinking switch, cache counters, prices and off-peak hours | `llm/src/openai.rs`, `llm/src/price.rs` | Provider rules from the provider's own pages; the replay requirement could not be reproduced live (see `docs/AI.md`) |
| `artifical-intelligence/other-documentations/suggestion.md` (multi-model routing guide) | Production fallback and graceful degradation; cheap model for the bulk, strong model for judgment | `fallback_model`: one automatic switch per turn when the model is unreachable, overloaded, out of credit or silent | The one routing idea that is both simple and safe without benchmark data; tiered model routing by task difficulty was left out (needs measurements this repository does not have) |
| `artifical-intelligence/other-documentations/context-enrichment.md` | Dynamic tool-schema pruning, KV-cache prefix alignment, rolling compaction, "context poisoning" as the problem | Already the shape of the router and `context::fit`; reinforced keeping the prefix stable | Confirmed the direction; AST graphs and hybrid RAG are for code agents and were not needed here |
| `artifical-intelligence/other-documentations/opencode-agentic-architecture.md` and `artifical-intelligence/ide_documentations/opencode` | Three-level permission model (allow, ask, deny), read-only "plan" persona, daemon decoupled from clients | Matches the existing approval policy (free, ask, gated); the control channel and `workcraft-cli serve` are the decoupled daemon | Validated the gated-even-in-Auto rule |
| `artifical-intelligence/ide_documentations` (Claude Code and Desktop, Cursor, Windsurf, Antigravity, OpenCode, Trae, ZCode) | Where each app keeps MCP settings and in what shape | `workcraft-cli mcp setup`, `clients`, `snippet` (`cli/src/ide.rs`) | Pairing needs exact files; Trae and ZCode document no file, so they get a snippet |
| `apps/solidcraft-0.30.1/docs/mcp.md` | "Edit in place, back up once, leave edited entries alone" for IDE pairing | `cli/src/ide.rs` | A tested behaviour for the same problem |
| `other/open/Handy-0.10.0` (already used for voice in the workspace) | Local speech engine reached through a command line | Not used in this chat yet | Noted for a later voice round |

## Skimmed, with ideas noted but not built

| Source | Idea | Why not built now |
| :--- | :--- | :--- |
| `other/open/LibreChat-main` | Conversation search and presets, message branching arrows, artifacts side panel | Search and branching already exist in WorkCraft; the artifacts side panel is now built (Artifacts tab with a full-size view, alpha.6) |
| `other/open/open-webui-0.12.4` | Suggested prompts on the empty state, a hover action row on messages (Regenerate and Copy, alpha.6), `/` prompt commands, per-message token and speed statistics | Empty-state suggestions and `/` commands exist; this round added the animated cards and the installed-apps strip; per-message statistics are now in the toolbar (last reply) rather than on each message |
| `other/open/bb-desktop-v0.48.1` | Threads you can follow live, steer at any point or hand off; every surface (desktop, CLI, HTTP) drives the same engine | Already how WorkCraft works (command bus, control channel, `mcp`); steering a running Team plan is built (Skip and Redo, alpha.6) |
| `other/open/ollama-0.40.12` | Context length and `truncate` behaviour | Confirmed by test on the installed Ollama, see `docs/AI.md` |
| `other/open/openplayground-main`, `document-0.0.9`, `DocumentServer-master`, `DesktopEditors-master`, `web-apps-master`, `craft-ipad-apps-1.0.0` | Model comparison playground; office-suite code | Not relevant to the chat |
| `artifical-intelligence/other-documentations/odysseus-ai-workspace.md` | Hardware-aware model choice, MCP hub with approval gates, deep-research agent | The approval gates match; hardware-aware choice would need GPU probing (the workspace does it, this chat uses a fixed 16k window) |
| `artifical-intelligence/other-documentations/the-agentic-workflow.md` | ReAct, plan-and-solve, re-planning on failure | Team mode already plans a task graph, retries with the reviewer's note and adds tasks; nothing new to add |
| `artifical-intelligence/other-documentations` hermes-agent, openclaw, runvil | Agent platforms | Studied earlier for team orchestration (see `SOURCES.md`); not re-read this round |
| `other` (egui, winit, craft-ai-integrations, craft-i18n, craft-launcher, craft-libs) | egui animation API (`animate_bool_with_time`), repaint rules | The hover animation uses egui's own animation helpers; the rest is unchanged |

## How to use this file

When a change is inspired by something, add a row with the path, the one idea taken, where it lives in the code and why it was worth it.
Do not paste code from these projects; their licences differ and several are not compatible with this repository's.
