---
name: solidcraft
description: >
  SolidCraft is ArtCraft's parametric 3D B-rep CAD (sketch solver, feature timeline, assemblies and joints). Use this
  skill to model parts and enclosures, edit an existing design, check volume and dimensions, take renders, and export
  STEP, STL, 3MF, OBJ or a DXF of a view. Native files are .sc.
---

# SolidCraft

## Identity

- **Product**: SolidCraft 0.30 (`solidcraft`, `solidcraft-cli`), WorkCraft control port `7992`. Tier C app, bonus to the
  ArtCraft 16. Source: `artcraft/apps/solidcraft-0.30.1` (`docs/mcp.md` is the reference for the tools).
- **Read** (`open`): `.sc` (and older `.solidcraft`), STEP (`.step`, `.stp`), STL and 3MF as mesh bodies.
- **Write**: `save` makes `.sc`. `export` makes `step`, `3mf`, `3mf-tagged`, `stl`, `stla`, `obj` and `urdf` and nothing
  else: **there is no GLB, SVG or IGES export.** A 2D DXF of a view is the command `file.export_view_dxf` (needs a
  `camera`, see below). For TextureCraft hand over `stl` or `obj`; for CADCraft hand over the DXF.

## WorkCraft tool surface (`solidcraft__*`)

Lean profile, 14 tools (15 in Live mode): `new_design`, `open`, `save`, `export`, `inspect_design`, `measure`,
`body_topology`, `set_parameter`, `screenshot`, `execute`, `batch`, `list_commands`, `skill_reference`, `undo`, and
`pending` in Live mode only. `describe_command`, `check_parameters`, `redo`, `running`, `compare_picture` and
`crash_reports` appear when `full_tool_catalog` is on.

`execute` and `batch` run engine commands (570 of them). `list_commands {"filter": "..."}` and `execute {"command": "engine.describe", "params": {"command": "<id>"}}` (a free read)
give shapes; never call `list_commands` without a filter (over 100 KB).

## First step: read the app's own guide

Call `solidcraft__skill_reference {"skill": "solidcraft-modeling"}` before building (15 KB). It has the command shapes,
plane axes, id rules and the known traps. Also available: `solidcraft-editing` (change an existing design),
`solidcraft-mechanisms` (assemblies, joints, motion), `solidcraft-checking` (regression checks).

## Verified build (headless, 14400 mm3 box)

```text
new_design {"name": "bracket"}
batch {"commands": [
  {"command": "sketch.create", "params": {"plane": "XY"}},
  {"command": "sketch.rectangle.two_point", "params": {"p0": [0,0], "p1": [40,30]}},
  {"command": "sketch.finish"},
  {"command": "solid.extrude", "params": {"distance": 12}}],
 "expect": {"volume_mm3": 14400, "bodies": 1}}
measure {"summary": true}          -> volume_mm3 14400, bbox 0..40, 0..30, 0..12
screenshot {"view": "iso", "width": 480, "height": 320}   -> a picture
export {"path": "<workspace>/bracket.step"}
save {"path": "<workspace>/bracket.sc"}
```

Compute the expected volume and bounding box first and pass them as `expect`. A mismatch comes back as an error with the
measured numbers, the design is kept as built (undo with `undo`). A cut that reports success but leaves the volume
unchanged has missed.

## Rules

1. **One batch per stage** (parameters + sketch + first feature, then fillets/shell, then holes/patterns). A batch is
   all-or-nothing and rolls back on the first failing step.
2. **Verify with numbers**, not with the reply: `measure {"summary": true}`, then one `screenshot`.
3. **Pass plain numbers for picks.** An edge pick farther than 5 % of the body from every edge is refused.
4. **Paths**: files are reachable only in the workspace and the folders WorkCraft passes with `--allow-path`, and the
   design's own folder when it is inside your home or temp folder and not hidden. A refused path says which folder to add.
5. **Long operations** can answer `still running`. Do not send the change again; in Live mode call `pending`, otherwise
   `inspect_design` to see whether it applied.
6. **Do not call `new_design` or `open` in Live mode without asking**: both discard the unsaved design in the user's window.
7. `file.send_to_slicer` and `file.open_in_easyprint` start another program and always ask for approval.

## DXF of a view (for CADCraft or DesignCraft)

```text
execute {"command": "file.export_view_dxf",
         "params": {"path": "<workspace>/front.dxf",
                    "camera": {"target": [20,15,6], "yaw": 0, "pitch": 0, "roll": 0, "distance": 100, "fov": 0}}}
```

`camera` is an orbit camera in millimetres and radians and **needs all of `target`, `yaw`, `pitch`, `roll`, `distance`,
`fov`** here (verified: leaving `roll` and `fov` out is `invalid camera`). `yaw 0, pitch 0` is the front view (looking
along +Y), `yaw` pi/2 looks from -X, `pitch` pi/2 looks straight down. `fov: 0` is orthographic.

## Handoff

| To | File | How |
| :--- | :--- | :--- |
| TextureCraft | `stl`, `obj` | `export`, then `texturecraft__mesh_import` |
| CADCraft, DesignCraft | `dxf` of a view | `file.export_view_dxf`, then `cadcraft__open` or `designcraft__place_image` |
| Another CAD | `step` | `export` |
| 3D printing | `3mf`, `stl` | `export` |
