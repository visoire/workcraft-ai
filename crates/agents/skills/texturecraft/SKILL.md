---
name: texturecraft
description: >
  TextureCraft is ArtCraft's native pure-Rust 3D texture painting and mesh baking studio (.tcraft,
  .tcmat). Use this skill when importing 3D meshes (GLB/glTF/OBJ/FBX/STL/USD), painting PBR strokes
  and fill layers, applying procedural MaterialCraft (.mcmat) materials, baking mesh maps (AO,
  curvature, normal, thickness), rendering viewport previews, or exporting PBR texture sets.
---

# TextureCraft

## Identity & Architecture

- **Product**: TextureCraft (`texturecraft`, `texturecraft-cli`, port `7990`).
- **Role**: Non-destructive 3D PBR texture painting, fill/procedural layer stacks, smart masks, mesh map baking, and PBR channel-packed texture export.
- **Native formats**: `.tcraft` (project archive), `.tcmat` (material preset). Also loads `.mcmat` / `.mcraft` materials exported from MaterialCraft.
- **Mesh formats**:
  - **Read**: `.glb`, `.gltf`, `.fbx`, `.obj`, `.stl`, `.ply`, `.usd`, `.usda`, `.usdc`, `.usdz`.
  - **Write**: `.glb`, `.obj`, `.usda`, `.usdc`, `.usdz`.
- **Texture formats**: `.png`, `.jpg`, `.tga`, `.tif`, `.exr`, `.hdr`.

## WorkCraft MCP Surface (`texturecraft__*`)

### Lean Tool Profile (16 tools)
- **Project & Session**: `project_new` (`mesh`, `name`, `resolution`, `template`), `project_open` (`path`), `project_save` (`path`), `session_inspect` ().
- **Meshes & Materials**: `mesh_import` (`path`, `resolution`), `mesh_export` (`path`, `format`), `material_load` (`path`), `material_save` (`path`, `layer`, `name`), `resource_import` (`path`).
- **Painting & Baking**: `paint_stroke` (`points`, `color`, `roughness`, `metallic`, `height`, `size`, `hardness`, `viewport`), `bake` (`maps`, `resolution`, `cpu`, `ts`).
- **Preview & Export**: `viewport_render` (`width`, `height`, `yaw`, `pitch`, `samples`, `bounces`, `cpu`, `view`), `textures_export` (`dir`, `preset`, `format`, `size`, `padding`, `ts`).
- **Generic Commands**: `command_list` (`filter`), `command_run` (`id`, `params`), `command_batch` (`steps`).

### Live-Window Only Tools (`Live` mode on port `7990`)
- `viewport_screenshot`, `ui_screenshot`, `ui_inspect`, `ui_menu_invoke`.
- **Important**: When `texturecraft-cli mcp --bridge 7990` is bridged to a live GUI window, headless file/render tools (`viewport_render`, `textures_export`) may be restricted by the live bridge; prefer `Headless` mode for automated batch baking/export, or use `viewport_screenshot` / `ui_screenshot` for visual inspection in `Live` mode.

## Critical Path & Sandbox Rules

1. **Relative Forward-Slash Paths**:
   - In headless mode, TextureCraft runs with `--read-root <workspace> --write-root <workspace>` and rejects absolute paths (`C:\...` or `/...`) and `..` segments.
   - WorkCraft automatically normalizes paths inside your workspace to relative forward-slash paths (`assets/crate.glb`, `exports/pbr`, or `"."` for the workspace root), and rejects paths outside configured roots with a clear `denied_by_root` error.
   - When writing paths yourself, prefer clean workspace-relative forward-slash paths (for example `"meshes/prop.glb"` or `dir: "out/textures"`).
2. **Built-in Primitive Projects**:
   - `project_new` works even without an external mesh file (defaults to a UV-mapped primitive or accepts `template` / `resolution`), making it easy to create procedural/painted test materials before importing custom geometry.

## Standard Agent Workflow

1. **Create or Open**:
   - Call `texturecraft__project_new` (`{"name": "Crate", "resolution": 1024}`) or import a mesh with `texturecraft__mesh_import` (`{"path": "meshes/crate.glb", "resolution": 1024}`).
2. **Inspect Layer & Texture Set State**:
   - Call `texturecraft__session_inspect` to verify active texture sets, UV layout, resolution, and layer stack ids.
3. **Add Fill Layers / Materials / Paint**:
   - Discover layer/mask commands via `texturecraft__command_list` (`{"filter": "layer"}`), apply `.tcmat`/`.mcmat` presets via `texturecraft__material_load`, or paint strokes via `texturecraft__paint_stroke` (`points: [[0.5, 0.5], [0.6, 0.5]]` in UV or 3D space).
   - Bake mesh maps if smart masks need curvature/AO: `texturecraft__bake` (`{"maps": ["ao", "curvature"], "resolution": 512, "cpu": true}`).
4. **Visual Verification**:
   - Call `texturecraft__viewport_render` (`{"width": 512, "height": 512, "cpu": true}`) — WorkCraft saves and forwards the rendered PNG so vision models can inspect lighting, seams, and roughness before export.
5. **Save & Export**:
   - Export channel-packed or standard PBR maps with `texturecraft__textures_export` (`{"dir": "exports/crate_pbr", "format": "png", "size": 1024}`) and save `.tcraft` via `texturecraft__project_save` (`{"path": "crate.tcraft"}`).

