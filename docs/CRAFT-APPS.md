# Craft apps in WorkCraft chat

How WorkCraft chat connects to the 17 Craft apps (the 16 of the ArtCraft lineup plus SolidCraft), what the model
sees, what asks for approval, and what has been tested against the real installed apps. Design background and the
measurements behind it: [PLAN.md](../PLAN.md). Architecture: [ARCHITECTURE.md](ARCHITECTURE.md).

Everything marked *measured* was run on Windows 11 on 2026-11-06 with the apps installed on this machine. Linux and
macOS install layouts are coded from the apps' documentation and **not verified**.

## The idea in one paragraph

Every app ships a maintained MCP server (`<app>-cli mcp`). WorkCraft does **not** reimplement those servers. It finds the
installed app, starts the server with safe sandbox roots, shows the model a lean set of tools, classifies every call
as read, write or gated, saves the pictures the apps return, and keeps one writer per app in Team mode. One registry,
`crates/agents/src/craft/spec.rs`, holds every per-app fact; adding an app is one entry there plus an icon and a skill.

## Versions checked on 2026-11-06

All 17 apps in `artcraft/apps` were probed (`cargo run -p workcraft-agents --example probe_apps`), `apps_contract` passed against the installed apps and the 22 workflows of
`apps_headless` passed. Changes that mattered: **MaterialCraft 0.2.0** saves native `.sbs` packages (`.mcraft` is no longer supported), **Craft Anything 0.5.0** adds
`core_finding_edit` (in the default set, outside the lean profile) and its skill now describes the working method of its own setup skill.

## Coverage

| # | App | Version | Tier | Kind | Headless workflow tested | Live window tested |
| -: | :--- | :--- | :--- | :--- | :--- | :--- |
| 1 | PhotoCraft | 0.21.0 | A | photos | yes: open, blur, preview, export (output differs from input) | yes |
| 2 | VectorCraft | 0.22.0 | A | vector art | yes: shape, text, preview, SVG and PNG export | yes |
| 3 | FilmCraft | 0.18.0 | A | video | yes: import MP4, sequence from clip, frame render | yes |
| 4 | LightCraft | 0.19.0 | A | RAW photos | yes: import, develop, render, JPEG export | not tested |
| 5 | PdfCraft | 0.19.0 | existing | PDF | yes: create, add text, save, reopen, extract, render | no live bridge upstream |
| 6 | EffectCraft | 0.22.0 | B | motion | yes: new project, comp, layer, frame, save | not tested |
| 7 | DesignCraft | 0.18.0 | A | layout | yes: document, place image, page render, PNG export | not tested |
| 8 | FigCraft | 0.16.0 | B | interface design | yes: frame and text nodes, screenshot, SVG export, save | not tested |
| 9 | TextureCraft | 0.4.0 | B | 3D texture painting | yes: project, stroke, CPU render, map export, save | not tested |
| 10 | MaterialCraft | 0.2.0 | B | procedural materials | yes: PBR graph, render, map export, package save | not tested |
| 11 | WordCraft | 0.16.0 | existing | documents | yes: type, read back, page render, `.docx` | not tested |
| 12 | SoundCraft | 0.18.0 | B | audio | yes: import WAV, inspect, bounce, save | not tested |
| 13 | DeckCraft | 0.16.0 | existing | presentations | yes: text box, render, `.pptx` contains the text | not tested |
| 14 | CADCraft | 0.18.0 | B | CAD | yes: script, inspect, render, DXF save | not tested |
| 15 | GridCraft | 0.17.0 | existing | spreadsheets | yes: formula `=SUM` evaluates to 42, `.xlsx` | not tested |
| 16 | Craft Anything | 0.5.0 | C | software exploration | read-only: session, brief, investigation; started `--clean-room` | no live bridge upstream |
| + | SolidCraft | 0.30.1 | C | solid CAD | yes: guide, box with `expect` (14400 mm3 measured), undo, picture, STEP/STL/3MF/OBJ, `.sc`, DXF of a view, STEP reopens with the same volume | yes |

"Tested" means a real `<app>-cli mcp` process was started and the call sequence in
`crates/agents/tests/apps_headless.rs` passed. Live means the real desktop window was started with `--control <port>`,
bridged, read, written and screenshotted (`crates/agents/tests/apps_live.rs`). Not tested is not the same as broken:
those apps are covered by the headless workflow and the contract snapshot, not by a window run.

## Discovery

For each app, in this order: the `<APP>_CLI` environment variable (for example `PHOTOCRAFT_CLI`; an invalid value is an
error, never a silent fallback), the `craft_apps` override in `config.json`, `Program Files\<App>`,
`Program Files\ArtCraft\<App>` (where the ArtCraft Launcher installs eight apps), `%LOCALAPPDATA%\Programs`, then `PATH`.
`craft.list` (or the `craft_status` tool) prints version, paths, mode, port, health and the tool count. On this machine
`craft.list` finds **17 of 17**.

## Modes

| Mode | What runs | When |
| :--- | :--- | :--- |
| `headless` (default) | `<app>-cli mcp` over stdio, no window | batch work, reads, CI. Never opens a window |
| `live` | the desktop window started with `--control <port>` and the MCP server bridged to it | the user wants to watch or hand-edit; `ui_*` tools |
| `off` | not connected | Craft Anything until it is switched on |

Switch with `craft.mode {app, mode}`, the chips in Settings > Tools & MCP, or `craft_activate`. A window is only opened
by `craft.open`, `open_craft_app` or the Live mode; a plain read against an app that is not running in Live mode returns
a typed `app_not_running` error with a hint, it does not start a window.

Apps connect **on demand**: all installed apps are available; WordCraft, GridCraft, DeckCraft and PdfCraft are on by
default, the rest connect when you `@mention` them, an agent's scope names them, or the model calls `craft_activate`.

Two apps have no live bridge upstream and stay headless: PdfCraft and Craft Anything (`craft.open` can still show the
window). TextureCraft and MaterialCraft split their tools: batch render and export are headless-only, viewport
screenshots are live-only (the lean profile hides the ones that do not apply to the current mode).

## Ports and authentication

Fixed per app, all different, none of them 7878. `verify_unique_ports()` refuses to launch on a clash.

| App | Port | Bridge flag | How the CLI authenticates |
| :--- | -: | :--- | :--- |
| LightCraft | 7980 | `--connect` | none |
| WordCraft | 7981 | `--connect HOST:PORT` | `control-key.local-<port>` |
| GridCraft | 7982 | `--connect PORT` | `~/.gridcraft/control-<port>.token` |
| DeckCraft | 7983 | `--connect PORT` | none |
| PhotoCraft | 7984 | `--bridge HOST:PORT` | `--control-token-file` (64 hex, in `%TEMP%`) |
| VectorCraft | 7985 | `--connect HOST:PORT` | none |
| DesignCraft | 7986 | `--connect PORT` | none handled by WorkCraft |
| SoundCraft | 7987 | `--connect PORT` | none |
| CADCraft | 7988 | `--connect HOST:PORT` | `%LOCALAPPDATA%\cadcraft-control-<port>.token` |
| FigCraft | 7989 | `--attach PORT` | none |
| TextureCraft | 7990 | `--bridge PORT` | `%TEMP%\texturecraft-control-<port>.token` |
| MaterialCraft | 7991 | `--bridge PORT` | `%TEMP%\materialcraft-control-<port>.token` |
| SolidCraft | 7992 | `--connect HOST:PORT` | none |
| FilmCraft | 9876 | `--bridge HOST:PORT` | `FILMCRAFT_CONTROL_TOKEN` (64 hex, mandatory) |
| EffectCraft | 9877 | `--bridge PORT` | none |
| PdfCraft, Craft Anything | n/a | no bridge | n/a |

Windows with a bridge were exercised for PhotoCraft (token file), VectorCraft and FilmCraft (env token).
The other rows come from the apps' CLI help and docs.

## Sandbox roots

Headless servers are started with the **workspace and the folders you added in Settings** as their only read and write
roots, never the home folder. PhotoCraft, WordCraft, PdfCraft: `--automation-read-root/--automation-write-root`;
VectorCraft, DeckCraft, DesignCraft, SoundCraft, FigCraft: repeatable read roots plus one write root; TextureCraft and
MaterialCraft: `--read-root/--write-root` and **paths relative to the root** (WorkCraft rewrites absolute paths inside the
root); SolidCraft: `--allow-path`. GridCraft, LightCraft, FilmCraft, EffectCraft and CADCraft take no root flags
(they have none); Craft Anything gets a project file under `.workcraft/` and the approval inbox. A file outside the roots is refused by the app (tested: PhotoCraft refuses a PNG in
the temp folder) and WorkCraft reports it as `denied_by_root`.

## What the model sees: lean profiles and the budget

A full `tools/list` of all apps is about 178k tokens. WorkCraft shows each app's lean profile instead (at most 16 tools
and 5,000 tokens, enforced by `tests/apps_contract.rs`). Everything else stays reachable through the app's own generic
tool (`command_run`, `execute`, `run_command`, `tool_call`). *Measured* lean sizes:

| App | Lean tools | Tokens | App | Lean tools | Tokens |
| :--- | -: | -: | :--- | -: | -: |
| WordCraft | 12 | 1362 | CADCraft | 12 | 1078 |
| GridCraft | 16 | 3718 | FigCraft | 10 | 4685 |
| DeckCraft | 16 | 2147 | TextureCraft | 16 | 2676 |
| PdfCraft | 16 | 2585 | MaterialCraft | 14 | 2231 |
| PhotoCraft | 13 | 3407 | SolidCraft | 14 | 4666 |
| VectorCraft | 12 | 4932 | Craft Anything | 16 | 2351 |
| LightCraft | 15 | 3771 | SoundCraft | 9 | 550 |
| DesignCraft | 13 | 2119 | FilmCraft | 9 | 997 |
| EffectCraft | 16 | 2984 | | | |

Since alpha.4 a request does not even carry all of these: only the apps the chat is about (see [AI.md](AI.md)). Set `full_tool_catalog`
in the config to disable the lean filter and `tool_routing: false` to send every connected tool. A global cap keeps the list usable for small models:
**60 tools for local models (Ollama, LM Studio), 120 for hosted models**, with apps you `@mention` kept first and a line
in the system prompt saying what was dropped.

## Uniform verbs

Apps name the same action differently (`doc_open`, `open_file`, `open_presentation`, `package_open`...). The `craft_*`
tools take an `app` argument and route to the right native tool:

`craft_status`, `craft_activate`, `craft_open`, `craft_new`, `craft_save`, `craft_inspect`, `craft_render` (returns a
picture), `craft_export`, `craft_commands`, `craft_run`, `craft_batch`, `craft_handoff`.

Native names (`photocraft__doc_open`) stay available. Craft Anything has no document verbs; use its `core_*` tools.

## Pictures

MCP images (renders, previews, screenshots) are kept, not turned into `[image: image/png]`. Each is saved to
`<workspace>/.workcraft/artifacts/<app>-<call>-<n>.<ext>`, the path goes into the text result, the tool card shows a
clickable thumbnail, and vision models (Claude, GPT-class, Gemini, vision-capable local models) receive the picture
itself. Non-vision models get the path and a description. Limits: 4 MiB per image, 8 per call. Some servers wrap the
picture differently (DesignCraft data URI, CADCraft `{png_base64}`); `craft/quirks.rs` normalizes both.

## Approvals

Reads never ask. In manual mode every write asks. The decision uses, in order: the server's `readOnlyHint` and
`destructiveHint`, the app's own risk table for generic tools (by command id, for example `filter.*` is a write and
`view.zoom*` a read), then a name heuristic where `render`, `screenshot`, `measure`, `check`, `pending`, `running`
count as reads. Unknown means write.

**Gated tools ask even in Auto mode**, with a reason on the card:

| Gate | Examples |
| :--- | :--- |
| Paid cloud generation | Craft Anything `assets_image_generate`, `assets_model_generate` |
| Running code or scripts | `script`, `eval`, `lisp`, `command_line`, `script_ui`, `script.*` and `js.*` ids inside `command_run` |
| Permanent deletion | `cmd_photo_deletePermanently`, `cmd_library_emptyRecentlyDeleted`, `redact_apply`, `doc_protect` |
| Signing and printing | `sign_*`, `doc_print` |

Craft Anything starts only with `--clean-room` and an inbox folder. Its `core_person_ask` and `core_action_approve`
requests appear as WorkCraft approval cards. `--allow-run` and `--deep` stay off unless you switch them on in Settings.

## Handoff between apps

`craft_handoff {from_app, to_app, file}` checks that the receiving app reads the format, converts only when the source
app can write it (it never re-encodes silently), stages the file in `<workspace>/handoffs/` and returns the call the
receiving app should make next. Tested: VectorCraft PNG export placed on a DeckCraft slide.

| From | To | Format | Receiving tool |
| :--- | :--- | :--- | :--- |
| PhotoCraft, LightCraft, VectorCraft | DeckCraft, WordCraft, DesignCraft, FigCraft, PdfCraft | PNG, JPG, SVG | `insert_picture`, `place_image`, ... |
| LightCraft | PhotoCraft | JPG, TIFF, DNG | `doc_open` |
| VectorCraft, CADCraft | DesignCraft, FigCraft, PdfCraft | SVG, PDF | `place_image`, `doc_open` |
| MaterialCraft | TextureCraft | PNG maps | `material_load`, `resource_import` |
| SolidCraft | TextureCraft | STL, OBJ (SolidCraft has no GLB export) | `mesh_import` |
| SolidCraft | CADCraft, DesignCraft | DXF of a view (`execute file.export_view_dxf`) | `open`, `place_image` |
| SoundCraft | FilmCraft | WAV | `media_import` |
| EffectCraft | FilmCraft | MP4, MOV | `media_import` |
| DesignCraft, WordCraft | PdfCraft | PDF | `doc_open` |

## Finding the apps

`craft.list` (and Settings, Craft apps (Connectors), with its Rescan button) looks for each app's command-line tool in this order: the `<APP>_CLI`
variable (for example `GRIDCRAFT_CLI`; a broken path is an error, never a silent fallback), the install folders of the system
(`Program Files\<App>`, `Program Files\ArtCraft\<App>`, the same under `%LOCALAPPDATA%` and `%LOCALAPPDATA%\Programs`, `%USERPROFILE%\ArtCraft\<App>`; on macOS
`/Applications/<App>.app`; on Linux `/opt`, `/usr/local` and the usual bin folders), `PATH`, and last a look one level into every folder there whose name belongs
to the app or to ArtCraft (`ArtCraft Apps\wordcraft`). SolidCraft's tool is also found as `solvecraft-cli`. The version comes from `<cli> --version`.

## One agent per app

`config::app_agents()` builds one agent for every app except Craft Anything (the explorer covers it): id `<app>-agent`, the tools of its own app plus the
shared read and handoff tools, no file writing of its own, and the app's skill. They live in the `apps` team (`config::apps_team`, built when a team run
starts from the apps that are installed and on), can be mentioned with `@<app>-agent` and are edited like any agent (`agents/<id>.json` wins). An agent
may carry `"approval": "manual" | "auto"`; a team may carry `"plan_approval": true | false`. The gated tools ask whatever is set.

## Team mode

Specialists: `image` (PhotoCraft, LightCraft), `design` (DesignCraft, FigCraft, VectorCraft), `video` (FilmCraft,
EffectCraft, SoundCraft), `three-d` (TextureCraft, MaterialCraft, SolidCraft, CADCraft), `document` (WordCraft, GridCraft,
DeckCraft, PdfCraft) and an optional `explorer` (Craft Anything, off by default). Each sees only its apps' tools. A
per-app writer lease means two agents never write to the same app at once; reads and writes to different apps run in
parallel (tested with a scripted model).

## Running the tests

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo test --workspace                                                    # offline, a few seconds
$env:WORKCRAFT_LIVE_APPS = "1"
cargo test -p workcraft-agents --test apps_contract -- --include-ignored  # re-probes all 17 installed servers
cargo test -p workcraft-agents --test apps_headless -- --include-ignored --test-threads=4
cargo test -p workcraft-agents --test apps_live -- --include-ignored --test-threads=1   # opens real windows
```

| Suite | File | What | Measured on 2026-11-06 |
| :--- | :--- | :--- | :--- |
| L1 unit | `src/**` | registry invariants, classifier, quirks, handoff | passes |
| L2 contract | `tests/apps_contract.rs` | snapshots in `tests/snapshots/`, lean profiles exist, token caps, live drift check | 2 passed, 17 of 17 servers, about 2 s |
| L3 headless | `tests/apps_headless.rs` | one workflow per app, handoff | 22 passed (17 apps, 1 handoff, 4 failure cases), 8 s |
| L4 live | `tests/apps_live.rs` | PhotoCraft, VectorCraft, FilmCraft, SolidCraft windows | 4 passed |
| L5 engine | `tests/engine.rs` | scripted model, approvals, gates, leases, vision, Craft Anything inbox | 19 passed |
| L7 failure | in `apps_headless.rs` | missing binary, closed port, path outside root, killed child | 4 passed |

Live tests skip when the app is already running (they never drive or kill your own window), and stop only the
process they started, by PID, also when an assertion fails.

When an app updates, `apps_contract` fails on a removed or renamed tool or a new required argument; refresh
`tests/snapshots/` with `cargo run -p workcraft-agents --example probe_apps`.

## Real model runs (L6)

Five prompts through `workcraft-cli chat --auto`, each in a fresh scratch data folder with the real apps (headless), one
per app: VectorCraft (logo SVG), PhotoCraft (blur an image), SolidCraft (40 x 30 x 12 box, report the volume, export STEP),
GridCraft (`=SUM`, save `.xlsx`), FilmCraft (import a clip, report). Run on 2026-11-06, first with alpha.2 and again with alpha.4.

| Model | alpha.2: files made | alpha.4: files made | alpha.4 per prompt |
| :--- | :--- | :--- | :--- |
| DeepSeek `deepseek-flash` / `deepseek-chat` (hosted) | 4 of 4, FilmCraft answered | 4 of 4, FilmCraft answered | 10 to 24 s, $0.003 to $0.008 |
| Ollama `qwen2.5:7b` | 0 of 4 | **3 of 4** (`logo.svg`, `blurred.png`, `sums.xlsx`); SolidCraft and FilmCraft not finished | 135 to 171 s |
| Ollama `qwen3.5:4b` | 0 of 4 | **2 of 4** (`logo.svg`, `sums.xlsx`); FilmCraft imported and answered; PhotoCraft and SolidCraft not finished | 77 to 138 s |

The jump for the local models has one cause: the alpha.2 requests were about 15,000 estimated tokens (more for a real tokenizer) of
tools and prompt, Ollama ran them with its default window of 4,096 tokens and **silently cut off the start of the prompt**, which is the
system prompt and the tool list. The models were answering without seeing their tools. Since alpha.4 the window is stated (16,384),
a request carries only the tools of the apps the question is about, and an overflow is an error. See [AI.md](AI.md).

Other things these runs found, all fixed: a tool call whose JSON had one closing brace too many (DeepSeek), `null` for every optional
argument (small models), calls to tools an app does not have (the error now lists the real ones), `skill` for `name` in `read_skill`,
one successful call repeated 50 times. The local models are still unreliable at multi-step work: 3 of 4 is one run each, not a rate.
The files DeepSeek produced were checked for existence and size and the values (volume 14,400 mm3, A3 = 42) were read from the app;
nobody looked at the SVG or PNG as a picture.
## Known limits

- The headless workflows check the app's own results and the files it writes. They do not prove the picture "looks
  right"; a person or a vision model has to look at the saved artifacts.
- Only four windows were driven live (PhotoCraft, VectorCraft, FilmCraft, SolidCraft). Bridges for the other apps follow the apps' documented flags.
- Local models finish the simple Craft tasks about half of the time (see "Real model runs"); a hosted model is far more reliable.
- Linux and macOS discovery and launch paths are untested.
