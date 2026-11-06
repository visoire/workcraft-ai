---
name: materialcraft
description: >
  MaterialCraft is ArtCraft's procedural node-graph PBR material authoring tool (.sbs packages, .mcmat,
  .sbs). Use this skill when creating tileable procedural materials, wiring noise/pattern/filter
  nodes, tuning material parameters, rendering 2D/3D material previews, or exporting PBR texture
  maps (basecolor, normal, roughness, metallic, AO, height) for TextureCraft or 3D engines.
---

# MaterialCraft

## Identity & Architecture

- **Product**: MaterialCraft (`materialcraft`, `materialcraft-cli`, port `7991`).
- **Role**: Node-based procedural PBR material generator and Substance `.sbs` / `.mcmat` package authoring studio.
- **Formats**:
  - **Read**: `.sbs`, `.mcmat`, `.png`, `.jpg`, `.tga`, `.tif`, `.exr`, `.hdr` (`.mcraft` is no longer supported since MaterialCraft 0.2.0).
  - **Write**: `.sbs`, `.mcmat`, `.png`, `.jpg`, `.tga`, `.tif`, `.webp`, `.exr`, `.hdr`.

## WorkCraft MCP Surface (`materialcraft__*`)

### Lean Tool Profile (14 tools)
- **Packages & Graphs**: `package_open` (`path`), `package_save` (`path`), `graph_new` (`name`, `size`, `template`), `session_inspect` ().
- **Node Authoring**: `library_list` (`category`, `filter`), `node_add` (`def`, `at`, `connectFrom`, `params`), `node_connect` (`from`, `fromPort`, `to`, `toPort`), `node_param` (`id`, `param`, `value`), `bitmap_import` (`path`, `name`).
- **Render & Export**: `view_render` (`width`, `height`, `scene`, `view`, `node`, `port`, `hq`, `tiling`, `displacement`, `yaw`, `pitch`, `full`), `outputs_export` (`dir`, `format`, `depth`, `graph`, `pattern`).
- **Generic Commands**: `command_list` (`filter`), `command_run` (`id`, `params`), `command_batch` (`steps`).

### Live-Window Only Tools (`Live` mode on port `7991`)
- `view_screenshot`, `ui_screenshot`, `ui_inspect`, `ui_menu_invoke`, `ui_input`.
-Note: Live bridging uses a bare port (`--bridge 7991`) and `MATERIALCRAFT_CONTROL_TOKEN` / temp token file authentication, which WorkCraft configures automatically. In `Live` mode, use `view_screenshot` / `ui_screenshot` for previews; use `Headless` mode for `view_render` and `outputs_export`.

## Critical Path & Sandbox Rules

- MaterialCraft headless MCP runs with `--read-root <workspace> --write-root <workspace>` and requires **workspace-relative forward-slash paths** (`"materials/brick.sbs"`, `"exports/brick"`, or `"."`).
- WorkCraft automatically rewrites paths inside the workspace to relative forward-slash form and blocks paths outside allowed roots (`denied_by_root`).
- **Never invent node definition ids**: call `materialcraft__library_list` (`{"filter": "noise"}` or `{"category": "generator"}`) first to inspect exact `def`, port names, and parameter names.

## Standard Agent Workflow

1. **Start a Graph**:
   - Call `materialcraft__graph_new` (`{"name": "weathered_stone", "size": 1024, "template": "pbr"}`) or open an existing package with `materialcraft__package_open`.
   - Call `materialcraft__session_inspect` to see existing output nodes and node ids.
2. **Discover & Wire Nodes**:
   - Query `materialcraft__library_list` to find generators, noises, blends, normal-from-height, and levels/warp filters.
   - Add nodes via `materialcraft__node_add` (`{"def": "...", "params": {...}}`), wire ports via `materialcraft__node_connect` (`{"from": n1, "to": out_id}`), and tweak parameters via `materialcraft__node_param`.
3. **Preview & Iterate Visually**:
   - Call `materialcraft__view_render` (`{"width": 512, "height": 512}`) to inspect the 3D shaderball/plane preview image before exporting.
4. **Export & Handoff**:
   - Export PBR maps with `materialcraft__outputs_export` (`{"dir": "exports/stone", "format": "png"}`) and save the editable graph with `materialcraft__package_save` (`{"path": "materials/stone.sbs"}`).
   - To apply exported maps or `.mcmat` directly onto a 3D mesh in TextureCraft, use `craft_handoff` (`from_app: "materialcraft"`, `to_app: "texturecraft"`).

