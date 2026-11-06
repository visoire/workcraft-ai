# Sources

Every GitHub repository WorkCraft chat builds on, connects to or learned from. Keep this file current: when you add a
crate, a protocol, a provider or an idea taken from another repository, add it here in the same change (see
[README.md](README.md) and [AGENTS.md](AGENTS.md)). The workspace's list is
[SOURCES.md of visoire/workcraft](https://github.com/visoire/workcraft/blob/main/SOURCES.md); the AI providers and
protocols below are the same ones.

Links were checked on 2026-11-05. A repository marked *not published yet* returns 404 for visitors.

## The WorkCraft projects

| Project | Visibility | Role |
| :--- | :--- | :--- |
| [visoire/workcraft-chat](https://github.com/visoire/workcraft-chat) | not published yet | This project: the desktop-only AI chat in Rust (egui app, CLI, MCP server) |
| [visoire/workcraft](https://github.com/visoire/workcraft) | public, MIT OR Apache-2.0 | The workspace: the main project this chat follows (providers, agents, skills and MCP ideas come from it) |
| [visoire/workcraft-home](https://github.com/visoire/workcraft-home) | private, not open source | The main homepage |

## Craft apps and skills

| Repository | Used for |
| :--- | :--- |
| [storytold/artcraft](https://github.com/storytold/artcraft) | The Craft apps suite; control-channel and MCP conventions the engine follows |
| [storytold/wordcraft](https://github.com/storytold/wordcraft), [gridcraft](https://github.com/storytold/gridcraft), [deckcraft](https://github.com/storytold/deckcraft), [pdfcraft](https://github.com/storytold/pdfcraft), [photocraft](https://github.com/storytold/photocraft), [vectorcraft](https://github.com/storytold/vectorcraft), [lightcraft](https://github.com/storytold/lightcraft), [designcraft](https://github.com/storytold/designcraft), [filmcraft](https://github.com/storytold/filmcraft), [effectcraft](https://github.com/storytold/effectcraft), [soundcraft](https://github.com/storytold/soundcraft), [cadcraft](https://github.com/storytold/cadcraft), [figcraft](https://github.com/storytold/figcraft) | The 13 apps found on the computer and connected as MCP servers (`<app>-cli mcp`, or live over `--control`) |
| TextureCraft 0.3.2, MaterialCraft 0.1.2, Craft Anything 0.3.0 (installed apps; sources under `artcraft/apps/`, no public repository checked), SolidCraft 0.30.1 (installed app; source in `artcraft/apps/solidcraft-0.30.1`, its `docs/mcp.md` and `skills/` were read for alpha.3) | Four more connectors added in 0.1.0-alpha.2 (`texturecraft-cli mcp`, `materialcraft-cli mcp`, `craft-anything-cli mcp`, `solidcraft-cli mcp`). Their 64 px icons are scaled from each app's own `assets/app-icon/` master; SolidCraft's comes from `solidcraft-cli icon` |
| creative-apps Claude plugin (local copy at `artcraft/mcp/creative-apps-claude-plugin-main`, no repository link verified) | Launcher install-folder convention (`Program Files\ArtCraft\<App>`, `<APP>_CLI`) and the `test_card.png/jpg`, `test_clip.mp4` inputs the headless tests reuse |
| [midasmatterhorn/artcraft-skills](https://github.com/midasmatterhorn/artcraft-skills) | The 13 skills bundled in `crates/agents/skills/` |
| [agentskills/agentskills](https://github.com/agentskills/agentskills) | The agent skills format |
| [emilk/egui](https://github.com/emilk/egui) | `eframe`/`egui`: the desktop app (the Craft apps use the same toolkit) |
| [rust-windowing/winit](https://github.com/rust-windowing/winit) | Windowing under eframe |

## Protocols, provider references and agent platforms

| Repository | Used for |
| :--- | :--- |
| [modelcontextprotocol/modelcontextprotocol](https://github.com/modelcontextprotocol/modelcontextprotocol) | The MCP specification (the client and server in `crates/mcp` are written by hand against it) |
| [modelcontextprotocol/typescript-sdk](https://github.com/modelcontextprotocol/typescript-sdk), [python-sdk](https://github.com/modelcontextprotocol/python-sdk) | Reference |
| [ollama/ollama](https://github.com/ollama/ollama) | Native `/api/chat` NDJSON adapter |
| [anthropics/anthropic-sdk-typescript](https://github.com/anthropics/anthropic-sdk-typescript) | Messages API reference for the Anthropic adapter |
| [openai/openai-node](https://github.com/openai/openai-node) | Chat Completions reference for the OpenAI-compatible adapter (Grok, GLM, Mistral, Groq, OpenRouter, Together AI, LM Studio) |
| [googleapis/js-genai](https://github.com/googleapis/js-genai), [google-gemini/gemini-cli](https://github.com/google-gemini/gemini-cli) | Gemini reference |
| [deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness) | Agent loop reference |
| [anomalyco/opencode](https://github.com/anomalyco/opencode) | Agent platform reference |
| [openclaw/openclaw](https://github.com/openclaw/openclaw), [NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent) | Agent platforms studied for team orchestration and tool handling |
| [samanhappy/mcphub](https://github.com/samanhappy/mcphub), [mcp-use/mcp-use](https://github.com/mcp-use/mcp-use), [HenryLinyy/local-llm-mcp](https://github.com/HenryLinyy/local-llm-mcp), [sethuram2003/mcp-ollama_server](https://github.com/sethuram2003/mcp-ollama_server), [mudler/MCPs](https://github.com/mudler/MCPs) | MCP hub and local-LLM MCP projects studied in the local-LLM snapshot |

## Rust crates (workspace dependencies)

| Repository | Crate |
| :--- | :--- |
| [serde-rs/serde](https://github.com/serde-rs/serde), [serde-rs/json](https://github.com/serde-rs/json) | `serde`, `serde_json` |
| [dtolnay/thiserror](https://github.com/dtolnay/thiserror), [dtolnay/async-trait](https://github.com/dtolnay/async-trait) | `thiserror`, `async-trait` |
| [chronotope/chrono](https://github.com/chronotope/chrono), [uuid-rs/uuid](https://github.com/uuid-rs/uuid) | `chrono`, `uuid` |
| [tokio-rs/tokio](https://github.com/tokio-rs/tokio), [tokio-rs/tracing](https://github.com/tokio-rs/tracing), [rust-lang/futures-rs](https://github.com/rust-lang/futures-rs) | `tokio`, `tracing`, `futures-util` |
| [seanmonstar/reqwest](https://github.com/seanmonstar/reqwest) | `reqwest` (HTTP, streaming, rustls) |
| [zip-rs/zip2](https://github.com/zip-rs/zip2), [tafia/quick-xml](https://github.com/tafia/quick-xml), [tafia/calamine](https://github.com/tafia/calamine), [jrmuizel/pdf-extract](https://github.com/jrmuizel/pdf-extract), [jmcnamara/rust_xlsxwriter](https://github.com/jmcnamara/rust_xlsxwriter) | `zip`, `quick-xml`, `calamine`, `pdf-extract`, `rust_xlsxwriter`: reading and writing office files |
| [image-rs/image](https://github.com/image-rs/image), [PolyMeilex/rfd](https://github.com/PolyMeilex/rfd), [marshallpierce/rust-base64](https://github.com/marshallpierce/rust-base64) | `image`, `rfd`, `base64` |

## Reference material used for alpha.4 (local snapshots, not on GitHub)

| Source | Used for |
| :--- | :--- |
| `artifical-intelligence/documentation/documentations 05.11.2026` (DeepSeek thinking-mode guide and pricing, Anthropic pricing, OpenAI and Gemini API references) | Provider rules in `llm/src/openai.rs`, built-in prices in `llm/src/price.rs` |
| `artifical-intelligence/ide_documentations` (Claude Code and Desktop, Cursor, Windsurf, Antigravity, OpenCode, Trae, ZCode) | Settings files and shapes for `workcraft-cli mcp setup` |
| `artcraft/apps/craft-anything-0.3.0/crates/ai` (`http.rs`, `price.rs`, `context.rs`) | Patterns reused in our own code: plain error messages with a next step, per-model price with cache rates, shortening old tool results while keeping every call next to its result |
| `workcraft/workcraft/src/chat` and `docs/AI-TOKENS.md` (TypeScript workspace) | The tool router, history compaction, smart review and direct routing ideas |
| `artcraft/apps/solidcraft-0.30.1/docs/mcp.md` | The `mcp setup` behaviour (edit in place, backup, leave edited entries alone) |

## Other sources (not on GitHub)

Provider documentation: [modelcontextprotocol.io](https://modelcontextprotocol.io), [docs.anthropic.com](https://docs.anthropic.com),
[platform.openai.com/docs](https://platform.openai.com/docs), [ai.google.dev](https://ai.google.dev/gemini-api/docs),
[api-docs.deepseek.com](https://api-docs.deepseek.com), [docs.x.ai](https://docs.x.ai), [docs.z.ai](https://docs.z.ai),
[ollama.com](https://ollama.com). The Craft apps: [getartcraft.com](https://getartcraft.com).
