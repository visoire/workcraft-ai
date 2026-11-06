//! Lean tool profiles, global tool budget, and computed approval policy (D4, D6, section 8).

use std::collections::BTreeSet;

use serde_json::Value;
use workcraft_core::{ToolSpec, looks_read_only};
use workcraft_mcp::hub::split_name;

use crate::craft::spec::{AppSpec, CraftMode, find_spec};

/// Default maximum tools sent in one model turn for local vs cloud models (D4, T3.2).
pub const LOCAL_TOOL_BUDGET: usize = 60;
pub const CLOUD_TOOL_BUDGET: usize = 120;

/// Craft Anything's default Clean-room Essentials toolset (29 tools). Tools outside this set that
/// appear dynamically after `enable_toolset` are passed through even in lean mode (T2.7).
const CRAFT_ANYTHING_DEFAULT_SET: &[&str] = &[
    "core_target_close",
    "core_session_show",
    "core_targets_list",
    "core_target_select",
    "core_evidence_show",
    "core_evidence_export",
    "core_evidence_import",
    "core_unknowns_show",
    "core_unknown_add",
    "core_unknown_edit",
    "core_unknown_verify",
    "core_person_ask",
    "core_action_approve",
    "assets_image_generate",
    "assets_model_generate",
    "core_investigation_show",
    "core_brief_write",
    "core_plan_update",
    "core_hypothesis_record",
    "core_hypothesis_update",
    "core_finding_record",
    "core_finding_edit",
    "core_handoff_write",
    "core_journal_read",
    "core_report_save",
    "core_report_write",
    "core_evidence_browse",
    "rebuild_check",
    "list_toolsets",
    "enable_toolset",
];

/// Filter one app's raw `ToolSpec`s according to its lean profile and current `CraftMode` (T3.1).
#[must_use]
pub fn filter_app_tools(
    spec: &AppSpec,
    mode: CraftMode,
    full_catalog: bool,
    tools: Vec<ToolSpec>,
) -> Vec<ToolSpec> {
    tools
        .into_iter()
        .filter(|t| {
            let sub = split_name(&t.name).map_or(t.name.as_str(), |(_, s)| s);
            if mode == CraftMode::Headless && spec.live_only_tools.contains(&sub) {
                return false;
            }
            if full_catalog || spec.lean_tools.is_empty() {
                return true;
            }
            if spec.lean_tools.contains(&sub) {
                return true;
            }
            // Dynamic toolsets enabled at runtime in Craft Anything pass through automatically.
            if spec.id == "craft-anything" && !CRAFT_ANYTHING_DEFAULT_SET.contains(&sub) {
                return true;
            }
            false
        })
        .collect()
}

/// Rough token estimate of a `ToolSpec`'s JSON schema + description (~4 chars per token).
#[must_use]
pub fn estimate_tool_tokens(spec: &ToolSpec) -> usize {
    let schema_len = spec.input_schema.to_string().len();
    (spec.name.len() + spec.description.len() + schema_len).div_ceil(4)
}

/// Priority tier when enforcing the global tool budget (lower number = kept first).
fn tool_priority(spec: &ToolSpec, mentioned_apps: &BTreeSet<String>) -> u8 {
    if let Some((srv, _)) = split_name(&spec.name) {
        if mentioned_apps.contains(srv) {
            return 1;
        }
        if let Some(app) = find_spec(srv) {
            return if app.always_on_default { 3 } else { 2 };
        }
        // User-configured custom MCP server in `mcp.json`.
        return 2;
    }
    // Built-in WorkCraft tools and `craft_*` facade verbs always stay available.
    0
}

/// Enforce the model-class global tool cap (60 local / 120 cloud), returning the kept tools and an
/// optional system-prompt notice naming what was trimmed (T3.2).
#[must_use]
pub fn apply_tool_budget(
    mut tools: Vec<ToolSpec>,
    is_local_model: bool,
    custom_cap: Option<usize>,
    mentioned_apps: &BTreeSet<String>,
) -> (Vec<ToolSpec>, Option<String>) {
    let cap = custom_cap.unwrap_or(if is_local_model {
        LOCAL_TOOL_BUDGET
    } else {
        CLOUD_TOOL_BUDGET
    });
    if tools.len() <= cap {
        return (tools, None);
    }

    // Stable sort by priority while preserving original order within each priority bucket.
    let mut indexed: Vec<(usize, u8, ToolSpec)> = tools
        .drain(..)
        .enumerate()
        .map(|(i, t)| {
            let pri = tool_priority(&t, mentioned_apps);
            (i, pri, t)
        })
        .collect();
    indexed.sort_by_key(|(i, pri, _)| (*pri, *i));

    let total = indexed.len();
    let mut dropped_servers = BTreeSet::new();
    for (_, _, t) in indexed.iter().skip(cap) {
        if let Some((srv, _)) = split_name(&t.name) {
            dropped_servers.insert(srv.to_string());
        }
    }
    indexed.truncate(cap);
    indexed.sort_by_key(|(i, _, _)| *i);
    let kept: Vec<ToolSpec> = indexed.into_iter().map(|(_, _, t)| t).collect();

    let kept_len = kept.len();
    let dropped = total - kept_len;
    let servers_note = if dropped_servers.is_empty() {
        String::new()
    } else {
        format!(
            " (from {})",
            dropped_servers.into_iter().collect::<Vec<_>>().join(", ")
        )
    };
    let note = format!(
        "Tool budget: showing {kept_len}/{total} tools for this model; {dropped} lower-priority tools{servers_note} were omitted. Use `craft_*` verbs (`craft_open`, `craft_inspect`, `craft_render`, `craft_run`) or `@app` mentions to focus an app."
    );
    (kept, Some(note))
}

/// Computed approval requirement for a tool call (D6, T3.3, T3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalLevel {
    /// Pure read or in-memory preview: never asks.
    Free,
    /// Mutates a document, UI, or workspace file: asks in `ApprovalMode::Manual`, runs in `Auto`.
    AskInManual { destructive: bool },
    /// Paid cloud API, arbitrary code execution, or irreversible deletion: asks EVEN in `Auto`.
    Gated { reason: String },
}

/// Check if `args` specifies an output file path for dual-mode `render` / `screenshot` tools.
fn has_output_path_arg(args: &Value) -> bool {
    let Some(obj) = args.as_object() else {
        return false;
    };
    ["path", "out", "output", "file", "file_path", "save_path"]
        .iter()
        .any(|k| {
            obj.get(*k)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
        })
}

/// Classify a single engine command ID for generic command runners (`command_run`, `execute`, etc.).
fn classify_command_id(cmd_id: &str) -> ApprovalLevel {
    let c = cmd_id.trim();
    let lower = c.to_ascii_lowercase();

    // 1. Gated command IDs (paid/network downloads, arbitrary script eval, permanent deletion, printing, signing).
    if lower.contains("deletepermanently")
        || lower.contains("emptyrecentlydeleted")
        || lower == "lens.downloaddatabase"
        || lower.contains("downloadmodel")
        || lower.starts_with("script.")
        || lower.ends_with(".eval")
        || lower == "file.send_to_slicer"
        || lower == "file.open_in_easyprint"
        || lower == "lisp"
        || lower.starts_with("js.")
        || lower.starts_with("sign.")
        || lower.ends_with(".print")
        || lower == "file.print"
        || lower == "doc.print"
    {
        return ApprovalLevel::Gated {
            reason: format!(
                "Command '{c}' performs script execution, external download, print, signature, or permanent deletion."
            ),
        };
    }

    // 2. Known read-only command ID families across the 17 Craft engines.
    if lower.starts_with("inspect.")
        || lower.ends_with(".inspect")
        || lower.starts_with("query.")
        || lower.ends_with(".list")
        || lower.ends_with(".presets")
        || lower.ends_with(".links")
        || lower.ends_with(".exposed")
        // SolidCraft: catalog, schema, revision and topology reads.
        || matches!(
            lower.as_str(),
            "engine.commands"
                | "engine.describe"
                | "document.revision"
                | "model.edges"
                | "model.faces"
                | "model.threads"
                | "file.slicers"
        )
        || lower.starts_with("view.zoom")
        || lower.starts_with("view.pan")
        || lower.starts_with("view.fit")
        || lower.starts_with("camera.")
        || lower.starts_with("display.")
        || lower == "jobs.list"
        || lower == "ui.inspect"
        || lower == "ui.menu.list"
    {
        return ApprovalLevel::Free;
    }

    let destructive =
        lower.contains("delete") || lower.contains("clear") || lower.contains("remove");
    ApprovalLevel::AskInManual { destructive }
}

/// Extract command IDs invoked inside generic runner / batch tools (`T3.4`).
fn extract_generic_command_ids(subtool: &str, args: &Value) -> Option<Vec<String>> {
    match subtool {
        "command_run" | "execute" | "execute_command" | "run_command" => {
            let id = args
                .get("id")
                .or_else(|| args.get("command"))
                .or_else(|| args.get("name"))
                .and_then(Value::as_str)?;
            Some(vec![id.to_string()])
        }
        "tool_call" => {
            let name = args.get("name").and_then(Value::as_str)?;
            Some(vec![name.to_string()])
        }
        "command_batch" | "batch" => {
            let arr = args
                .get("steps")
                .or_else(|| args.get("commands"))
                .and_then(Value::as_array)?;
            let mut ids = Vec::with_capacity(arr.len());
            for step in arr {
                if let Some(id) = step
                    .get("id")
                    .or_else(|| step.get("command"))
                    .or_else(|| step.get("name"))
                    .and_then(Value::as_str)
                {
                    ids.push(id.to_string());
                }
            }
            Some(ids)
        }
        _ => None,
    }
}

/// Compute the approval level for an MCP or `craft_*` tool call (D6, T3.3, T3.4).
#[must_use]
pub fn classify_tool_call(full_name: &str, args: &Value, spec: Option<&ToolSpec>) -> ApprovalLevel {
    // Built-in `craft_*` facade verbs:
    if split_name(full_name).is_none() {
        return match full_name {
            "craft_status" | "craft_activate" | "craft_inspect" | "craft_commands" => {
                ApprovalLevel::Free
            }
            "craft_render" => {
                if has_output_path_arg(args) {
                    ApprovalLevel::AskInManual { destructive: false }
                } else {
                    ApprovalLevel::Free
                }
            }
            "craft_run" => {
                if let Some(cmd) = args.get("command").and_then(Value::as_str) {
                    classify_command_id(cmd)
                } else {
                    ApprovalLevel::AskInManual { destructive: false }
                }
            }
            "craft_batch" => {
                if let Some(ids) = extract_generic_command_ids("batch", args) {
                    fold_command_levels(&ids)
                } else {
                    ApprovalLevel::AskInManual { destructive: false }
                }
            }
            _ => {
                if spec.is_some_and(|s| s.read_only) {
                    ApprovalLevel::Free
                } else {
                    ApprovalLevel::AskInManual {
                        destructive: spec.is_some_and(|s| s.destructive),
                    }
                }
            }
        };
    }

    let (srv, subtool) = split_name(full_name).unwrap_or(("", full_name));
    let sub_lower = subtool.to_ascii_lowercase();

    // 1. Gated tools (ask even in Auto mode, with explicit reason).
    if matches!(subtool, "assets_image_generate" | "assets_model_generate") {
        return ApprovalLevel::Gated {
            reason: format!(
                "'{full_name}' calls a paid external generation provider and incurs API cost."
            ),
        };
    }
    if matches!(subtool, "run_script" | "script_ui" | "script" | "js_run")
        || sub_lower.starts_with("js_")
    {
        return ApprovalLevel::Gated {
            reason: format!("'{full_name}' executes script code inside {srv}."),
        };
    }
    if matches!(
        subtool,
        "cmd_photo_deletePermanently"
            | "cmd_library_emptyRecentlyDeleted"
            | "redact_apply"
            | "doc_protect"
            | "doc_print"
    ) || sub_lower.starts_with("sign_")
    {
        return ApprovalLevel::Gated {
            reason: format!(
                "'{full_name}' performs an irreversible, security-sensitive, or physical print operation."
            ),
        };
    }
    if srv == "craft-anything"
        && (sub_lower.contains("_exec")
            || sub_lower.contains("_spawn")
            || sub_lower.contains("_attach")
            || sub_lower.contains("_inject")
            || sub_lower.contains("_install"))
    {
        return ApprovalLevel::Gated {
            reason: format!("'{full_name}' executes or attaches to target code on this device."),
        };
    }

    // 2. Generic command tools (`command_run`, `execute`, `batch`, `tool_call`, ...).
    if let Some(ids) = extract_generic_command_ids(subtool, args)
        && !ids.is_empty()
    {
        if subtool == "tool_call" && ids.len() == 1 && looks_read_only(&ids[0]) {
            return ApprovalLevel::Free;
        }
        return fold_command_levels(&ids);
    }

    // 3. Render / screenshot tools: free when returning an image in-memory, ask when saving a file.
    if (sub_lower.contains("render") || sub_lower.contains("screenshot"))
        && !sub_lower.contains("export")
    {
        return if has_output_path_arg(args) {
            ApprovalLevel::AskInManual { destructive: false }
        } else {
            ApprovalLevel::Free
        };
    }

    // 4. Explicit MCP `readOnlyHint` annotation.
    let destructive = spec.is_some_and(|s| s.destructive)
        || sub_lower.contains("delete")
        || sub_lower.contains("remove")
        || sub_lower.contains("clear");
    if let Some(hint) = spec.and_then(|s| s.read_only_hint) {
        return if hint && !destructive {
            ApprovalLevel::Free
        } else {
            ApprovalLevel::AskInManual { destructive }
        };
    }

    // 5. Conservative name heuristic (`looks_read_only`), else write.
    if looks_read_only(subtool) && !destructive {
        ApprovalLevel::Free
    } else {
        ApprovalLevel::AskInManual { destructive }
    }
}

fn fold_command_levels(ids: &[String]) -> ApprovalLevel {
    let mut any_write = false;
    let mut any_destructive = false;
    for id in ids {
        match classify_command_id(id) {
            ApprovalLevel::Gated { reason } => return ApprovalLevel::Gated { reason },
            ApprovalLevel::AskInManual { destructive } => {
                any_write = true;
                any_destructive |= destructive;
            }
            ApprovalLevel::Free => {}
        }
    }
    if any_write {
        ApprovalLevel::AskInManual {
            destructive: any_destructive,
        }
    } else {
        ApprovalLevel::Free
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn solidcraft_reads_are_free_and_launching_other_programs_is_gated() {
        for cmd in [
            "engine.describe",
            "engine.commands",
            "document.revision",
            "model.faces",
            "joint.inspect",
        ] {
            assert_eq!(
                classify_tool_call("solidcraft__execute", &json!({"command": cmd}), None),
                ApprovalLevel::Free,
                "{cmd}"
            );
        }
        assert!(matches!(
            classify_tool_call(
                "solidcraft__execute",
                &json!({"command": "solid.extrude"}),
                None
            ),
            ApprovalLevel::AskInManual { .. }
        ));
        assert!(matches!(
            classify_tool_call(
                "solidcraft__execute",
                &json!({"command": "file.send_to_slicer"}),
                None
            ),
            ApprovalLevel::Gated { .. }
        ));
        // A batch is as strict as its strictest step.
        assert!(matches!(
            classify_tool_call(
                "solidcraft__batch",
                &json!({"commands": [{"command": "engine.commands"}, {"command": "file.open_in_easyprint"}]}),
                None
            ),
            ApprovalLevel::Gated { .. }
        ));
    }
}
