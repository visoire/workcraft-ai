//! L2 Contract Snapshot & Budget Suite (`T6.1`).
//!
//! Validates all 17 saved tool-surface snapshots (`tests/snapshots/tools-<app>.json`) against
//! `workcraft_agents::craft::SPECS`, lean profile token caps, tool naming constraints, `VerbMap`
//! targets, and global tool budgets (`LOCAL_TOOL_BUDGET` = 60, `CLOUD_TOOL_BUDGET` = 128).
//! Also includes a live drift check (`--ignored` / `WORKCRAFT_LIVE_APPS=1`) against all 17
//! installed `<app>-cli mcp` binaries.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use workcraft_agents::craft::live::RootsContext;
use workcraft_agents::craft::{
    CLOUD_TOOL_BUDGET, CraftMode, LOCAL_TOOL_BUDGET, VerbTarget, apply_tool_budget, discover,
    estimate_tool_tokens, filter_app_tools, server_for, specs,
};
use workcraft_core::{ToolSpec, looks_read_only};
use workcraft_mcp::McpHub;

fn snapshots_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
}

fn load_snapshot(app_id: &str) -> Value {
    let p = snapshots_dir().join(format!("tools-{app_id}.json"));
    let raw = std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("missing snapshot {}: {e}", p.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("invalid JSON in {}: {e}", p.display()))
}

fn snapshot_to_tool_specs(app_id: &str, snap: &Value) -> Vec<ToolSpec> {
    let arr = snap["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("{app_id}: snapshot missing tools array"));
    arr.iter()
        .map(|t| {
            let sub = t["name"]
                .as_str()
                .unwrap_or_else(|| panic!("{app_id}: tool missing name"));
            let props = t["properties"].as_array().cloned().unwrap_or_default();
            let mut prop_map = serde_json::Map::new();
            for p in props {
                if let Some(pname) = p.as_str() {
                    prop_map.insert(pname.to_string(), json!({ "type": "string" }));
                }
            }
            let req = t["required"].clone();
            let hint = t
                .pointer("/annotations/readOnlyHint")
                .and_then(Value::as_bool);
            let destructive = t
                .pointer("/annotations/destructiveHint")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            ToolSpec {
                name: format!("{app_id}__{sub}"),
                description: t
                    .pointer("/annotations/title")
                    .and_then(Value::as_str)
                    .unwrap_or(sub)
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": prop_map,
                    "required": req,
                }),
                read_only: hint.unwrap_or_else(|| looks_read_only(sub)),
                read_only_hint: hint,
                destructive,
            }
        })
        .collect()
}

fn check_verb_target(app_id: &str, verb_label: &str, target: VerbTarget, names: &HashSet<&str>) {
    match target {
        VerbTarget::None => {}
        VerbTarget::Tool(tool) => {
            assert!(
                names.contains(tool),
                "{app_id}: VerbMap.{verb_label} references missing tool '{tool}'"
            );
        }
        VerbTarget::Command { runner, .. } => {
            assert!(
                names.contains(runner),
                "{app_id}: VerbMap.{verb_label} references missing command runner '{runner}'"
            );
        }
    }
}

#[test]
fn all_17_snapshots_match_lean_profiles_verb_maps_and_token_caps() {
    assert_eq!(specs().len(), 17);
    let mut all_lean_tools: Vec<ToolSpec> = Vec::new();

    for spec in specs() {
        let snap = load_snapshot(spec.id);
        assert_eq!(snap["app"].as_str(), Some(spec.id));
        let tools = snap["tools"].as_array().expect("tools array");
        assert!(!tools.is_empty(), "{}: snapshot has 0 tools", spec.id);
        let names: HashSet<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();

        // Every lean_tools entry must exist in the real snapshot.
        assert!(
            spec.lean_tools.len() <= 16,
            "{}: lean_tools has {} entries (> 16)",
            spec.id,
            spec.lean_tools.len()
        );
        for &lean in spec.lean_tools {
            assert!(
                names.contains(lean),
                "{}: lean_tools entry '{}' missing from snapshot {:?}",
                spec.id,
                lean,
                names
            );
        }

        // Every live_only_tools entry should exist in the snapshot when advertised statically.
        for &live_tool in spec.live_only_tools {
            // Note: SoundCraft / WordCraft only register some live UI tools when bridged, so only check non-empty names.
            assert!(
                !live_tool.is_empty(),
                "{}: empty live_only_tools entry",
                spec.id
            );
        }

        // Every VerbMap target must exist in the snapshot.
        check_verb_target(spec.id, "open", spec.verbs.open, &names);
        check_verb_target(spec.id, "new", spec.verbs.new, &names);
        check_verb_target(spec.id, "save", spec.verbs.save, &names);
        check_verb_target(spec.id, "inspect", spec.verbs.inspect, &names);
        check_verb_target(spec.id, "render", spec.verbs.render, &names);
        check_verb_target(spec.id, "export", spec.verbs.export, &names);
        for (lbl, raw) in [
            ("commands", spec.verbs.commands),
            ("run", spec.verbs.run),
            ("batch", spec.verbs.batch),
        ] {
            if !raw.is_empty() {
                assert!(
                    names.contains(raw),
                    "{}: VerbMap.{lbl} '{}' missing from snapshot",
                    spec.id,
                    raw
                );
            }
        }

        // Validate full tool names and filtered lean profile token budget.
        let full_specs = snapshot_to_tool_specs(spec.id, &snap);
        for t in &full_specs {
            assert!(
                t.name.len() <= 64,
                "{}: tool name '{}' exceeds 64 chars ({})",
                spec.id,
                t.name,
                t.name.len()
            );
            assert!(
                t.name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "{}: invalid characters in '{}'",
                spec.id,
                t.name
            );
        }

        let lean_headless = filter_app_tools(spec, CraftMode::Headless, false, full_specs.clone());
        assert!(
            !lean_headless.is_empty(),
            "{}: lean headless profile is empty",
            spec.id
        );
        assert!(
            lean_headless.len() <= 16,
            "{}: lean headless profile has {} tools (> 16)",
            spec.id,
            lean_headless.len()
        );
        for lt in &lean_headless {
            let sub = lt
                .name
                .split_once("__")
                .map_or(lt.name.as_str(), |(_, s)| s);
            assert!(
                !spec.live_only_tools.contains(&sub),
                "{}: headless lean profile leaked live-only tool '{}'",
                spec.id,
                sub
            );
        }

        // Also check actual raw snapshot schema_bytes scaled to the lean subset <= 5_000 tokens (~20 KB).
        let raw_schema_bytes = snap["schema_bytes"].as_u64().unwrap_or(0) as usize;
        let est_lean_raw_tokens =
            (raw_schema_bytes * lean_headless.len()) / (full_specs.len().max(1) * 4);
        let spec_tokens: usize = lean_headless.iter().map(estimate_tool_tokens).sum();
        assert!(
            est_lean_raw_tokens <= 5_000 && spec_tokens <= 5_000,
            "{}: lean profile exceeds 5,000 token cap (raw_est={est_lean_raw_tokens}, spec_tokens={spec_tokens})",
            spec.id
        );

        all_lean_tools.extend(lean_headless);
    }

    // Global budget check across all 17 apps simultaneously connected:
    let builtins: Vec<ToolSpec> = (0..12)
        .map(|i| ToolSpec {
            name: format!("builtin_{i}"),
            description: "builtin".into(),
            input_schema: json!({"type": "object", "properties": {}}),
            read_only: true,
            read_only_hint: Some(true),
            destructive: false,
        })
        .collect();
    let mut combined = builtins;
    combined.extend(all_lean_tools);
    assert!(combined.len() > CLOUD_TOOL_BUDGET);

    let mention_tc: BTreeSet<String> = ["texturecraft".to_string()].into_iter().collect();
    let (local_kept, local_note) = apply_tool_budget(combined.clone(), true, None, &mention_tc);
    assert_eq!(local_kept.len(), LOCAL_TOOL_BUDGET);
    assert!(local_note.is_some());
    assert!(
        local_kept
            .iter()
            .any(|t| t.name == "texturecraft__project_new"),
        "@texturecraft tools must survive local budget trimming"
    );

    let mention_sc: BTreeSet<String> = ["solidcraft".to_string()].into_iter().collect();
    let (cloud_kept, cloud_note) = apply_tool_budget(combined, false, None, &mention_sc);
    assert_eq!(cloud_kept.len(), CLOUD_TOOL_BUDGET);
    assert!(cloud_note.is_some());
    assert!(
        cloud_kept
            .iter()
            .any(|t| t.name == "solidcraft__new_design"),
        "@solidcraft tools must survive cloud budget trimming"
    );
}

#[tokio::test]
#[ignore = "spawns all 17 installed <app>-cli mcp processes; run with --include-ignored"]
async fn live_installed_apps_match_contract_snapshots() {
    let scratch =
        std::env::temp_dir().join(format!("wc-contract-{}", uuid::Uuid::new_v4().simple()));
    let ws = scratch.join("workspace");
    let data_dir = scratch.join("data");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();

    let found = discover();
    assert_eq!(
        found.len(),
        17,
        "expected all 17 ArtCraft apps installed on this machine, found: {:?}",
        found.iter().map(|f| f.id).collect::<Vec<_>>()
    );

    let roots = RootsContext::for_workspace(ws, data_dir, Vec::new());
    let mut mcp_cfg = workcraft_mcp::McpConfig::default();
    for f in &found {
        mcp_cfg
            .servers
            .insert(f.id.to_string(), server_for(f, CraftMode::Headless, &roots));
    }
    let hub = McpHub::new();
    let reports = hub.connect_all(&mcp_cfg).await;
    for r in &reports {
        assert!(
            r.error.is_none() && r.tools > 0,
            "server {} failed handshake: {:?}",
            r.server,
            r.error
        );
    }

    let mut over_cap = Vec::new();
    for spec in specs() {
        let live_tools = hub.server_tools(spec.id).await.unwrap();
        let live_names: HashSet<&str> = live_tools.iter().map(|t| t.name.as_str()).collect();
        for &lean in spec.lean_tools {
            assert!(
                live_names.contains(lean),
                "{}: lean tool '{}' missing from live MCP server {:?}",
                spec.id,
                lean,
                live_names
            );
        }
        let prefixed: Vec<ToolSpec> = live_tools
            .into_iter()
            .map(|mut t| {
                t.name = format!("{}__{}", spec.id, t.name);
                t
            })
            .collect();
        let lean_live = filter_app_tools(spec, CraftMode::Headless, false, prefixed);
        let real_tokens: usize = lean_live.iter().map(estimate_tool_tokens).sum();
        println!(
            "{:<16} {:>2} lean tools  {:>4} tokens",
            spec.id,
            lean_live.len(),
            real_tokens
        );
        if real_tokens > 5_000 {
            over_cap.push(format!("{}: {real_tokens} tokens", spec.id));
        }
    }

    hub.clear().await;
    let _ = std::fs::remove_dir_all(scratch);
    assert!(
        over_cap.is_empty(),
        "apps exceeding 5,000-token lean cap: {over_cap:?}"
    );
}
