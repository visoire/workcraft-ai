//! Per-app pre/post call quirks extracted from `tools.rs` (T1.3).

use serde_json::{Value, json};
use workcraft_core::{Result, ToolResult};
use workcraft_mcp::McpHub;

use crate::craft::spec::CraftMode;

/// Pre-call rewrite/intercept for Craft MCP tools. Returns `Some(result)` when the call was
/// handled directly (e.g. `wordcraft__create_document` alias), or `None` to proceed with `args`.
pub async fn pre_call(
    srv: &str,
    subtool: &str,
    args: &mut Value,
    mode: CraftMode,
    hub: &McpHub,
) -> Result<Option<ToolResult>> {
    if srv == "deckcraft" {
        normalize_deckcraft_mcp_args(subtool, args);
        if subtool == "run_command"
            && let Some(sel_ids) = extract_deckcraft_selection_ids(args)
        {
            let _ = hub
                .call(
                    "deckcraft__run_command",
                    json!({ "command": "edit.select", "params": { "ids": sel_ids } }),
                )
                .await;
        }
    }

    if srv == "wordcraft" {
        if subtool == "create_document" {
            let _ = hub.call("wordcraft__new_document", json!({})).await;
            let title = args.get("title").and_then(Value::as_str).unwrap_or("");
            let body = args
                .get("content")
                .or_else(|| args.get("text"))
                .or_else(|| args.get("markdown"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let combined = match (!title.is_empty(), !body.is_empty()) {
                (true, true) => format!("{title}\n\n{body}"),
                (true, false) => format!("{title}\n\n"),
                (false, true) => body.to_string(),
                (false, false) => String::new(),
            };
            if !combined.is_empty() {
                let _ = hub
                    .call("wordcraft__type_text", json!({ "text": combined }))
                    .await;
            }
            let msg = if mode == CraftMode::Live {
                "Created a new document in the open WordCraft desktop window."
            } else {
                "Created a new document in WordCraft."
            };
            return Ok(Some(ToolResult::ok(msg.to_string())));
        }

        if subtool == "open_document"
            && args
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .is_empty()
        {
            let inspect = hub
                .call("wordcraft__inspect_document", json!({}))
                .await
                .unwrap_or_else(|_| ToolResult::ok("{}"));
            let where_msg = if mode == CraftMode::Live {
                "WordCraft desktop window is open on screen."
            } else {
                "WordCraft headless session is ready."
            };
            return Ok(Some(ToolResult::ok(format!(
                "{where_msg}\n{}",
                inspect.text
            ))));
        }
    }

    Ok(None)
}

/// Post-call retries and message clarifications (e.g. `tool_search` multi-word retry, WordCraft status).
pub async fn post_call(
    srv: &str,
    subtool: &str,
    full_name: &str,
    args: &Value,
    mode: CraftMode,
    mut res: ToolResult,
    hub: &McpHub,
) -> ToolResult {
    // If `tool_search` (e.g. `pdfcraft__tool_search`) was given a multi-word query and returned 0 tools,
    // retry automatically with the first keyword so the model gets matching tools.
    if subtool == "tool_search"
        && res.ok
        && res.text.contains("\"count\": 0")
        && let Some(first_word) = args.get("query").and_then(Value::as_str).and_then(|q| {
            let fw = q.split_whitespace().next()?;
            (fw.len() < q.trim().len()).then(|| fw.to_string())
        })
    {
        let mut retry_args = args.clone();
        if let Some(obj) = retry_args.as_object_mut() {
            obj.insert("query".into(), Value::String(first_word));
        }
        if let Ok(r2) = hub.call(full_name, retry_args).await
            && r2.ok
            && !r2.text.contains("\"count\": 0")
        {
            res = r2;
        }
    }

    if res.ok && srv == "wordcraft" {
        match (subtool, mode) {
            ("type_text", CraftMode::Live) => {
                res.text = format!(
                    "Typed text into the open WordCraft desktop window.\n{}",
                    res.text
                );
            }
            ("type_text", _) => {
                res.text = format!("Typed text into WordCraft.\n{}", res.text);
            }
            ("new_document", CraftMode::Live) => {
                res.text = format!(
                    "Created a new document in the open WordCraft desktop window.\n{}",
                    res.text
                );
            }
            ("new_document", _) => {
                res.text = format!("Created a new document in WordCraft.\n{}", res.text);
            }
            _ => {}
        }
    }

    // Surface async polling guidance for SolidCraft `still running:` responses (T2.6).
    if srv == "solidcraft" && res.text.contains("still running") {
        res.text.push_str(
            "\n[Hint: call `solidcraft__running` or `solidcraft__pending` to poll until this operation finishes.]",
        );
    }

    res
}

/// Normalize DeckCraft command names and parameters in `deckcraft__run_command` and `deckcraft__batch`
/// so common LLM variations (`picture.insert`, `add_text_box`, `select`, `shape.delete`, `shape.sendToBack`,
/// and `{x, y, w, h}` instead of `rect: [x, y, w, h]`) work without failing.
pub fn normalize_deckcraft_mcp_args(subtool: &str, args: &mut Value) {
    if subtool == "run_command" {
        normalize_single_deckcraft_cmd(args);
    } else if subtool == "batch"
        && let Some(cmds) = args.get_mut("commands").and_then(Value::as_array_mut)
    {
        let mut expanded = Vec::with_capacity(cmds.len());
        for mut item in std::mem::take(cmds) {
            normalize_single_deckcraft_cmd(&mut item);
            if let Some(ids) = extract_deckcraft_selection_ids(&item) {
                expanded.push(json!({
                    "command": "edit.select",
                    "params": { "ids": ids }
                }));
            }
            expanded.push(item);
        }
        *cmds = expanded;
    }
}

fn normalize_single_deckcraft_cmd(cmd_obj: &mut Value) {
    let Some(obj) = cmd_obj.as_object_mut() else {
        return;
    };
    let raw_cmd = obj
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let canonical = match raw_cmd {
        "picture.insert" | "insert_picture" | "add_picture" | "image.insert" => {
            Some("insert.picture")
        }
        "add_text_box" | "insert_text_box" | "text.addBox" | "text.insert" => {
            Some("insert.textBox")
        }
        "add_shape" | "insert_shape" | "shape.insert" => Some("insert.shape"),
        "add_slide" | "insert_slide" => Some("slide.add"),
        "delete_slide" | "remove_slide" => Some("slide.delete"),
        "select" | "shape.select" => Some("edit.select"),
        "shape.delete" | "delete" | "delete_shape" | "remove" => Some("edit.delete"),
        "shape.sendToBack" | "sendToBack" | "send_to_back" => Some("arrange.sendToBack"),
        "shape.bringToFront" | "bringToFront" | "bring_to_front" => Some("arrange.bringToFront"),
        "shape.sendBackward" | "sendBackward" | "send_backward" => Some("arrange.sendBackward"),
        "shape.bringForward" | "bringForward" | "bring_forward" => Some("arrange.bringForward"),
        _ => None,
    };
    if let Some(c) = canonical {
        obj.insert("command".into(), Value::String(c.into()));
    }
    let cmd_name = obj
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if let Some(params) = obj.get_mut("params").and_then(Value::as_object_mut) {
        if matches!(
            cmd_name.as_str(),
            "insert.picture" | "insert.textBox" | "insert.shape" | "shape.setRect"
        ) && !params.contains_key("rect")
        {
            let x = params.get("x").and_then(Value::as_f64);
            let y = params.get("y").and_then(Value::as_f64);
            let w = params
                .get("w")
                .or_else(|| params.get("width"))
                .and_then(Value::as_f64);
            let h = params
                .get("h")
                .or_else(|| params.get("height"))
                .and_then(Value::as_f64);
            if let (Some(x), Some(y), Some(w), Some(h)) = (x, y, w, h) {
                params.insert("rect".into(), json!([x, y, w, h]));
            }
        }
        if cmd_name == "edit.select"
            && !params.contains_key("ids")
            && let Some(id) = params.get("id").cloned()
        {
            params.insert("ids".into(), json!([id]));
        }
    }
}

#[must_use]
pub fn extract_deckcraft_selection_ids(cmd_obj: &Value) -> Option<Vec<Value>> {
    let cmd = cmd_obj.get("command").and_then(Value::as_str)?;
    if !matches!(
        cmd,
        "edit.delete"
            | "arrange.sendToBack"
            | "arrange.bringToFront"
            | "arrange.sendBackward"
            | "arrange.bringForward"
    ) {
        return None;
    }
    let params = cmd_obj.get("params")?.as_object()?;
    if let Some(ids) = params.get("ids").and_then(Value::as_array)
        && !ids.is_empty()
    {
        return Some(ids.clone());
    }
    if let Some(id) = params.get("id") {
        return Some(vec![id.clone()]);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deckcraft_commands_and_batch_aliases_are_normalized() {
        let mut run_args = json!({
            "command": "picture.insert",
            "params": { "slide": 0, "path": "flag.png", "x": 450, "y": 150, "w": 400, "h": 300 }
        });
        normalize_deckcraft_mcp_args("run_command", &mut run_args);
        assert_eq!(run_args["command"], "insert.picture");
        assert_eq!(
            run_args["params"]["rect"],
            json!([450.0, 150.0, 400.0, 300.0])
        );

        let mut batch_args = json!({
            "commands": [
                { "command": "select", "params": { "id": 2 } },
                { "command": "add_text_box", "params": { "slide": 0, "text": "Title", "x": 60, "y": 40, "w": 840, "h": 60 } },
                { "command": "shape.delete", "params": { "id": 1 } },
                { "command": "shape.sendToBack", "params": { "id": 10 } }
            ]
        });
        normalize_deckcraft_mcp_args("batch", &mut batch_args);
        let cmds = batch_args["commands"].as_array().unwrap();
        assert_eq!(cmds.len(), 6);
        assert_eq!(cmds[0]["command"], "edit.select");
        assert_eq!(cmds[0]["params"]["ids"], json!([2]));
        assert_eq!(cmds[1]["command"], "insert.textBox");
        assert_eq!(cmds[1]["params"]["rect"], json!([60.0, 40.0, 840.0, 60.0]));
        assert_eq!(cmds[2]["command"], "edit.select");
        assert_eq!(cmds[2]["params"]["ids"], json!([1]));
        assert_eq!(cmds[3]["command"], "edit.delete");
        assert_eq!(cmds[4]["command"], "edit.select");
        assert_eq!(cmds[4]["params"]["ids"], json!([10]));
        assert_eq!(cmds[5]["command"], "arrange.sendToBack");
    }
}
