//! Cross-app file handoff matrix and `craft_handoff` staging (`PLAN.md` section 5.4).

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use workcraft_core::{Error, Result};

use crate::craft::spec::{AppSpec, RootsPolicy, find_spec};

static HANDOFF_SEQ: AtomicU64 = AtomicU64::new(1);

/// Recommended receiving tool call in `to_app` for an asset with extension `ext`.
fn receiving_call(to: &AppSpec, staged_path: &Path, workspace: &Path, ext: &str) -> Value {
    let display_path = if matches!(to.roots, RootsPolicy::RelativeReadWrite) {
        staged_path
            .strip_prefix(workspace)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| staged_path.to_string_lossy().replace('\\', "/"))
    } else {
        staged_path.to_string_lossy().into_owned()
    };

    match (to.id, ext) {
        ("deckcraft", "png" | "jpg" | "jpeg" | "svg") => json!({
            "tool": "deckcraft__insert_picture",
            "arguments": { "slide": 0, "path": display_path }
        }),
        ("designcraft", "png" | "jpg" | "jpeg" | "svg" | "pdf") => json!({
            "tool": "designcraft__place_image",
            "arguments": { "path": display_path }
        }),
        ("wordcraft", "png" | "jpg" | "jpeg") => json!({
            "tool": "wordcraft__execute",
            "arguments": { "command": "insert.picture", "params": { "path": display_path } }
        }),
        ("pdfcraft", "png" | "jpg" | "jpeg") => json!({
            "tool": "pdfcraft__tool_call",
            "arguments": { "name": "page_add_image", "arguments": { "path": display_path } }
        }),
        ("pdfcraft", "pdf") => json!({
            "tool": "pdfcraft__doc_open",
            "arguments": { "path": display_path }
        }),
        ("photocraft", _) => json!({
            "tool": "photocraft__doc_open",
            "arguments": { "path": display_path }
        }),
        ("vectorcraft", _) => json!({
            "tool": "vectorcraft__open_file",
            "arguments": { "path": display_path }
        }),
        ("figcraft", "fcraft" | "fig" | "svg") => json!({
            "tool": "figcraft__open_file",
            "arguments": { "path": display_path }
        }),
        ("texturecraft", "tcmat" | "mcmat" | "mcraft") => json!({
            "tool": "texturecraft__material_load",
            "arguments": { "path": display_path }
        }),
        (
            "texturecraft",
            "glb" | "gltf" | "fbx" | "obj" | "stl" | "ply" | "usd" | "usda" | "usdc" | "usdz",
        ) => json!({
            "tool": "texturecraft__mesh_import",
            "arguments": { "path": display_path }
        }),
        ("texturecraft", _) => json!({
            "tool": "texturecraft__resource_import",
            "arguments": { "path": display_path }
        }),
        ("materialcraft", "sbs" | "mcraft" | "mcmat") => json!({
            "tool": "materialcraft__package_open",
            "arguments": { "path": display_path }
        }),
        ("materialcraft", _) => json!({
            "tool": "materialcraft__bitmap_import",
            "arguments": { "path": display_path }
        }),
        ("filmcraft", _) => json!({
            "tool": "filmcraft__media_import",
            "arguments": { "paths": [display_path] }
        }),
        ("soundcraft", "scraft") => json!({
            "tool": "soundcraft__open_session",
            "arguments": { "path": display_path }
        }),
        ("soundcraft", _) => json!({
            "tool": "soundcraft__import_audio",
            "arguments": { "path": display_path }
        }),
        ("solidcraft", _) => json!({
            "tool": "solidcraft__open",
            "arguments": { "path": display_path }
        }),
        ("cadcraft", _) => json!({
            "tool": "cadcraft__open",
            "arguments": { "path": display_path }
        }),
        _ => json!({
            "tool": "craft_open",
            "arguments": { "app": to.id, "path": display_path }
        }),
    }
}

/// Compatible interchange formats that `from` writes and `to` reads.
#[must_use]
pub fn shared_formats(from: &AppSpec, to: &AppSpec) -> Vec<&'static str> {
    from.writes_formats
        .iter()
        .copied()
        .filter(|fmt| to.reads_formats.contains(fmt))
        .collect()
}

/// Stage `source_file` into `<workspace>/handoffs/<from>-<to>-<n>.<ext>`, validating format
/// compatibility without silent re-encoding, and return the staged path + next receiving call.
pub fn execute_handoff(
    from_raw: &str,
    to_raw: &str,
    source_file: &Path,
    as_format: Option<&str>,
    workspace: &Path,
) -> Result<Value> {
    let from = find_spec(from_raw)
        .ok_or_else(|| Error::NotFound(format!("unknown source Craft app '{from_raw}'")))?;
    let to = find_spec(to_raw)
        .ok_or_else(|| Error::NotFound(format!("unknown target Craft app '{to_raw}'")))?;

    if !source_file.is_file() {
        return Err(Error::NotFound(format!(
            "handoff file does not exist: {}",
            source_file.display()
        )));
    }

    let file_ext = source_file
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    let requested_ext = as_format
        .map(|f| f.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| file_ext.clone());

    let shared = shared_formats(from, to);
    if requested_ext != file_ext {
        if !from.writes_formats.contains(&requested_ext.as_str()) {
            return Err(Error::Invalid(format!(
                "{} cannot export '.{requested_ext}' (supported exports: {})",
                from.name,
                from.writes_formats.join(", ")
            )));
        }
        if !to.reads_formats.contains(&requested_ext.as_str()) {
            return Err(Error::Invalid(format!(
                "{} does not read '.{requested_ext}' (shared formats with {}: {})",
                to.name,
                from.name,
                shared.join(", ")
            )));
        }
        return Err(Error::Invalid(format!(
            "source file is '.{file_ext}' while as_format requested '.{requested_ext}'; export '.{requested_ext}' from {} first (WorkCraft never silently re-encodes)",
            from.name
        )));
    }

    if !to.reads_formats.contains(&file_ext.as_str()) {
        let hint = if shared.is_empty() {
            format!("{} reads: {}", to.name, to.reads_formats.join(", "))
        } else {
            format!(
                "export one of [{}] from {} first",
                shared.join(", "),
                from.name
            )
        };
        return Err(Error::Invalid(format!(
            "{} cannot open '.{file_ext}' directly ({hint})",
            to.name
        )));
    }

    let handoffs_dir = workspace.join("handoffs");
    std::fs::create_dir_all(&handoffs_dir)?;
    let n = HANDOFF_SEQ.fetch_add(1, Ordering::Relaxed);
    let dest_name = format!("{}-{}-{n}.{file_ext}", from.id, to.id);
    let dest = handoffs_dir.join(dest_name);
    std::fs::copy(source_file, &dest)?;

    let next_call = receiving_call(to, &dest, workspace, &file_ext);
    Ok(json!({
        "ok": true,
        "from_app": from.id,
        "to_app": to.id,
        "format": file_ext,
        "staged_path": dest.to_string_lossy(),
        "relative_path": format!("handoffs/{}", dest.file_name().unwrap_or_default().to_string_lossy()),
        "shared_formats": shared,
        "next_call": next_call,
    }))
}
