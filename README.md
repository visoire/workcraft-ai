<div align="center">

# WorkCraft AI

**A local-first AI chat and multi-agent desktop app that works with your files and your Craft apps. Written in Rust.**

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
[![Status: alpha](https://img.shields.io/badge/status-alpha-orange)](#alpha-warning)
[![Rust 1.90+](https://img.shields.io/badge/rust-1.90%2B-b7410e?logo=rust)](https://www.rust-lang.org/)
[![Windows verified](https://img.shields.io/badge/Windows-verified-0078D6)](#platforms)
[![macOS and Linux: untested](https://img.shields.io/badge/macOS%20%C2%B7%20Linux-untested-lightgrey)](#platforms)
[![CI](https://github.com/visoire/workcraft-ai/actions/workflows/ci.yml/badge.svg)](https://github.com/visoire/workcraft-ai/actions/workflows/ci.yml)

</div>

> **Alpha software.** WorkCraft AI is under active development. Things break, settings and data formats can change
> between alphas, and there is no stable release yet. Back up what matters and read [what has and has not been
> verified](#what-has-been-verified). Please report what you find.

WorkCraft AI is a desktop chat for talking to a model that can actually do things: read and write files in a folder you
choose, call MCP tools, and drive the [Craft apps](https://github.com/storytold) (documents, spreadsheets, photos, video,
CAD and more) installed on your computer. You can run it as a single assistant or give a goal to a **team of agents** that
plans, works in parallel and reviews each other. Models run locally with Ollama or through your own API keys.

One engine, three ways in: a **desktop app** (egui), a **CLI**, and an **MCP server** plus a loopback control channel so
other agents (Claude Code, Cursor and others) can drive it.

It is one of two sibling projects:

| Project | What it is |
| :--- | :--- |
| **[WorkCraft AI](https://github.com/visoire/workcraft-ai)** (this repository) | The native Rust desktop AI chat, CLI and MCP server |
| **[WorkCraft Workspace](https://github.com/visoire/workcraft)** | The TypeScript local-first workspace (documents, spreadsheets, presentations, PDFs, with an AI assistant) for the browser and iPadOS |

## What it is, and what it is not

| It is | It is not |
| :--- | :--- |
| A native desktop app, a CLI and an MCP server in pure Rust | A web app, a hosted service or a mobile app |
| Local-first: chats and settings are plain files in `~/workcraft/chat` | A cloud product: there is no account, no telemetry and no server of ours |
| A client for models you choose (Ollama, LM Studio, Claude, ChatGPT, Gemini, DeepSeek, Grok, GLM, Mistral, Groq, OpenRouter, Together or any OpenAI-compatible endpoint) | A model: it ships none and sells no tokens |
| A way to connect 17 Craft apps and any MCP server to a chat | A replacement for those apps, and not an official ArtCraft product |
| An alpha you can try today | Stable, signed or packaged for any store |

## Features

- **Chat.** Streaming, Markdown and code blocks, reasoning shown separately, image and text attachments, many chats
  that **keep working in the background** while you open another, branching (edit an earlier message and switch
  branches), and a model switch in the middle of a chat.
- **Single and Team mode.** A leader plans a task graph, specialists run in parallel with scoped tools, the leader
  reviews each result, and everything shares a small memory board. A built-in `craft` team (image, design, video,
  3D, documents) and an `apps` team with **one agent per installed Craft app**.
- **`@` mentions and `/` commands.** Mention agents, teams, Craft apps, skills, folders and file groups (`@all`,
  `@web`, `@team:apps`, `@skill:...`, `@pictures` and more) and type commands such as `/stop`, `/retry`, `/team`, `/model`,
  `/think`, `/summarize`, `/rename` or `/data`. The popup ranks and scrolls the matches.
- **Tools you control.** File tools confined to a workspace folder, every MCP tool, and the Craft apps' control
  channels. Reads never ask, writes wait for **Allow / Deny** in manual mode, and paid, code-running and deleting tools
  always ask, even in Auto.
- **17 Craft apps, found automatically.** PhotoCraft, VectorCraft, FilmCraft, LightCraft, PdfCraft, EffectCraft,
  DesignCraft, FigCraft, TextureCraft, MaterialCraft, WordCraft, SoundCraft, DeckCraft, CADCraft, GridCraft, SolidCraft
  and Craft Anything. Headless by default, live in the app window on request, pictures they return are shown in the chat.
- **Cheap and reliable.** A tool router sends only the tools the chat needs (a greeting costs a few thousand prompt
  tokens, not tens of thousands), history is fitted to the context window, requests are retried with readable
  notices, and the cost of every reply is shown.
- **Skills.** The agentskills.io format with 17 bundled Craft skills.
- **Voice input** through [Handy](https://github.com/cjpais/Handy) (optional) with spoken `@` and `/` commands.
- **Blip,** a small pixel mascot that thinks, works, waits for your OK, listens, cheers and droops with the state of
  the chat. Colors, accessory, size and movement can be changed or switched off.
- **Everything is a command.** The UI, the CLI, the control channel and the MCP server all call the same command bus
  (`workcraft-cli commands`).

<!--
Screenshots: add real images to docs/images/ and uncomment before publishing (scripts/publish.ps1 check warns when they are missing).
<p align="center">
  <img src="docs/images/chat-light.png" alt="WorkCraft AI chat in the light theme" width="49%">
  <img src="docs/images/team-dark.png" alt="Team mode with the activity panel in the dark theme" width="49%">
</p>
-->

## Alpha warning

- There is **no stable release**. Every build is `0.1.0-alpha.N` and is published as a GitHub *pre-release*.
- Data formats and settings can change between alphas. Chats are plain JSONL and stay readable, but a future alpha may need a migration.
- The binaries are **not code-signed**. Windows SmartScreen may warn you. Build from source if you prefer.
- Keys you add in Settings are stored in plain text in `~/workcraft/chat/config.json` (like the workspace does), or read from
  environment variables. Nothing sends them anywhere but the provider you chose.
- Small local models are weak at planning and tool use. Hosted models work noticeably better for Craft tasks.

## Install

### Windows: download

1. Open the [Releases](https://github.com/visoire/workcraft-ai/releases) page and pick the newest **pre-release**.
2. Download `WorkCraft.exe` (the desktop app) and, if you want it, `WorkCraft-CLI.exe` (terminal and MCP server). Check
   them against `SHA256SUMS.txt` from the same release.
3. Run `WorkCraft.exe`. For a local model start [Ollama](https://ollama.com); for a hosted one add a key in
   **Settings > Providers** or set `DEEPSEEK_API_KEY`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY` and so on before you start.

### Build from source (any platform)

Needs Rust 1.90 or newer (and the MSVC Build Tools on Windows; on Linux the GUI libraries listed in
[`.github/workflows/ci.yml`](.github/workflows/ci.yml)).

```sh
git clone https://github.com/visoire/workcraft-ai.git
cd workcraft-ai
cargo build --release
cargo run -p workcraft-app --bin workcraft          # the desktop app
cargo run -p workcraft-cli -- chat "Summarise notes.txt"
```

## Usage

```sh
workcraft                          # desktop app
workcraft-cli chat                 # interactive chat in the terminal
workcraft-cli chat "one question"  # one answer, then exit
workcraft-cli serve                # headless engine with the control channel
workcraft-cli mcp                  # MCP server on stdio
workcraft-cli mcp setup            # add WorkCraft to Claude Code, Cursor, Windsurf, OpenCode and others
claude mcp add workcraft -- workcraft-cli mcp
```

In the chat: type `@` to mention an agent, app, skill or folder, `/` for commands, middle-click to scroll with the mouse,
**Enter** to send and **Shift+Enter** for a new line. Settings is **Ctrl+,**. The full reference (data folder, config
files, providers, team mode, control protocol) is in [docs/USAGE.md](docs/USAGE.md), the design in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), the AI core in [docs/AI.md](docs/AI.md) and the Craft apps in
[docs/CRAFT-APPS.md](docs/CRAFT-APPS.md).

## Platforms

| Platform | Status |
| :--- | :--- |
| Windows 11 | Built, tested and used. This is the verified platform. |
| macOS | Not built or tested. It should compile (egui and pure Rust); expect rough edges. |
| Linux | Not built or tested outside CI. Needs the GUI libraries named above. |

Desktop only: there is no web, iPad or phone build. (For iPadOS see [WorkCraft Workspace](https://github.com/visoire/workcraft).)

## What has been verified

Verified on Windows 11: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo test --workspace` pass; every installed Craft app starts headless and passes one real workflow (`apps_headless`);
four live app windows were bridged and screenshotted; five Craft prompts completed 5 of 5 with a hosted model.

Not verified: the OpenAI-compatible and Anthropic adapters against a live API with keys of the maintainers, the Streamable HTTP MCP
client against a real server, live windows of most Craft apps, macOS and Linux builds, and the look of the new animations on every display.
The detailed list is in [docs/USAGE.md](docs/USAGE.md#what-has-been-verified-and-what-has-not).

## Contributing

Contributions are welcome, whether it is a bug report, a Craft app that misbehaves, a doc fix or a pull request. Start
with [CONTRIBUTING.md](CONTRIBUTING.md); it is short. Please follow the [Code of Conduct](CODE_OF_CONDUCT.md).
For security problems read [SECURITY.md](SECURITY.md) and do not open a public issue.

Good first steps: try it on macOS or Linux and report what breaks, run it against a Craft app you own, or improve a skill.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
Unless you explicitly state otherwise, any contribution you submit for inclusion is dual licensed as above, without
additional terms or conditions. The bundled Inter font is under the SIL Open Font License 1.1.

## Credits

Started and maintained by [visoire](https://github.com/visoire), with Claude (Anthropic) helping to write part of the code.
The Craft apps are made by [storytold](https://github.com/storytold), the ArtCraft team and contributors (SolidCraft by
[tangency-labs](https://github.com/tangency-labs/solidcraft)). WorkCraft is an independent community project: it is not
an official ArtCraft product and is not affiliated with, endorsed by or sponsored by any company it names. The projects
this one learned from are listed in [SOURCES.md](SOURCES.md) and [USED-REPOS.md](USED-REPOS.md).
