//! ArtCraft application registry, discovery, modes, sandbox roots, approval policy, quirks, and handoffs.

pub mod discover;
pub mod handoff;
pub mod live;
pub mod policy;
pub mod quirks;
pub mod router;
pub mod spec;

use std::collections::BTreeMap;

use serde_json::{Value, json};
use workcraft_mcp::McpConfig;

pub use discover::{
    Found, candidates_in_bases, default_install_bases, discover, discover_in_bases,
    discover_with_errors, find_gui, find_gui_for_spec, probe_version,
};
pub use handoff::{execute_handoff, shared_formats};
pub use live::{
    RootsContext, default_control_port, ensure_headless, ensure_live, ensure_live_with_roots,
    filmcraft_token_for, headless_mcp_args, is_port_listening, live_mcp_args, live_port_for,
    photocraft_token_file, server, server_for, spawned_gui_pid, take_spawned_gui_pid,
    verify_unique_ports,
};
pub use policy::{
    ApprovalLevel, CLOUD_TOOL_BUDGET, LOCAL_TOOL_BUDGET, apply_tool_budget, classify_tool_call,
    estimate_tool_tokens, filter_app_tools,
};
pub use router::{FIND_TOOLS, Routed, apps_in_history, apps_named_by_words, apps_named_in, route};
pub use spec::{
    AppSpec, CraftMode, LiveAuth, LiveBridgeKind, ON_BY_DEFAULT, RootsPolicy, SPECS, Tier, VerbMap,
    VerbTarget, find_spec, spec_by_id, specs,
};

/// Whether `id` should connect automatically at startup (`true` in `choices`, or in `ON_BY_DEFAULT`
/// when `choices` has no entry and `mode != Off`).
#[must_use]
pub fn enabled(id: &str, choices: &BTreeMap<String, bool>) -> bool {
    choices
        .get(id)
        .copied()
        .unwrap_or_else(|| ON_BY_DEFAULT.contains(&id))
}

/// Whether `id` may connect on-demand (via `@app`, specialist scope, `craft_activate`, or `<app>__*`).
/// Every installed app except those explicitly disabled (`choices[id] == false` or `mode == Off` or
/// `off_by_default` without explicit opt-in) is available on demand (D4, Q6).
#[must_use]
pub fn available_on_demand(
    id: &str,
    choices: &BTreeMap<String, bool>,
    modes: &BTreeMap<String, CraftMode>,
) -> bool {
    if modes.get(id) == Some(&CraftMode::Off) {
        return false;
    }
    if let Some(explicit) = choices.get(id) {
        return *explicit;
    }
    let Some(spec) = spec_by_id(id) else {
        return false;
    };
    !spec.off_by_default
}

/// Effective `CraftMode` for `id`, honoring `modes` and `choices`.
#[must_use]
pub fn mode_for(
    id: &str,
    choices: &BTreeMap<String, bool>,
    modes: &BTreeMap<String, CraftMode>,
) -> CraftMode {
    if choices.get(id) == Some(&false) {
        return CraftMode::Off;
    }
    if let Some(m) = modes.get(id).copied() {
        return m;
    }
    if choices.get(id) == Some(&true) {
        return CraftMode::Headless;
    }
    if spec_by_id(id).is_some_and(|s| s.off_by_default) {
        CraftMode::Off
    } else {
        CraftMode::Headless
    }
}

/// Resolve an app id, display name, or alias (e.g. `"WordCraft"`, `"word"`, `"craft-anything"`)
/// to its canonical `(id, name, supports_live)`.
#[must_use]
pub fn lookup_app(raw: &str) -> Option<(&'static str, &'static str, bool)> {
    let s = find_spec(raw)?;
    Some((s.id, s.name, s.supports_live()))
}

#[must_use]
pub fn is_known(id: &str) -> bool {
    spec_by_id(id).is_some()
}

/// Extract `@app` mentions from user chat text (e.g. `"@photocraft resize this"` or `"@craft-anything"`).
#[must_use]
pub fn extract_mentions(text: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    for token in text
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | ':' | '!' | '?'))
    {
        let Some(raw) = token.strip_prefix('@') else {
            continue;
        };
        let clean = raw.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_');
        if let Some(spec) = find_spec(clean)
            && !out.contains(&spec.id)
        {
            out.push(spec.id);
        }
    }
    out
}

/// Merge always-on Craft apps into `mcp.json` using full sandbox `roots` and `modes`.
#[must_use]
pub fn merge_with_roots(
    mut cfg: McpConfig,
    found: &[Found],
    choices: &BTreeMap<String, bool>,
    modes: &BTreeMap<String, CraftMode>,
    roots: &RootsContext,
) -> McpConfig {
    for f in found {
        let mode = mode_for(f.id, choices, modes);
        if mode != CraftMode::Off && enabled(f.id, choices) && !cfg.servers.contains_key(f.id) {
            cfg.servers
                .insert(f.id.to_string(), server_for(f, mode, roots));
        }
    }
    cfg
}

/// Backwards-compatible `merge` (headless default, no extra workspace roots).
#[must_use]
pub fn merge(cfg: McpConfig, found: &[Found], choices: &BTreeMap<String, bool>) -> McpConfig {
    merge_with_roots(
        cfg,
        found,
        choices,
        &BTreeMap::new(),
        &RootsContext::default(),
    )
}

/// Detailed report for `craft.list` and Settings > Tools & MCP (section 5.2).
#[must_use]
pub fn report_full(
    found: &[Found],
    override_errors: &BTreeMap<&'static str, String>,
    choices: &BTreeMap<String, bool>,
    modes: &BTreeMap<String, CraftMode>,
    connected: &[String],
    tool_counts: &BTreeMap<String, usize>,
    last_errors: &BTreeMap<String, String>,
) -> Value {
    json!(
        SPECS
            .iter()
            .map(|spec| {
                let f = found.iter().find(|f| f.id == spec.id);
                let is_conn = connected.iter().any(|c| c == spec.id);
                let mode = mode_for(spec.id, choices, modes);
                let ov_err = override_errors.get(spec.id);
                let hub_err = last_errors.get(spec.id);
                let live_listening = f.and_then(|x| x.live_port).is_some();

                let health = if let Some(e) = ov_err {
                    format!("error: {e}")
                } else if f.is_none() {
                    "not_installed".to_string()
                } else if let Some(e) = hub_err {
                    format!("error: {e}")
                } else if mode == CraftMode::Live && !live_listening {
                    "not_running".to_string()
                } else {
                    "ok".to_string()
                };

                json!({
                    "id": spec.id,
                    "name": spec.name,
                    "kind": spec.kind,
                    "installed": f.is_some(),
                    "version": f.and_then(|x| x.version.clone()),
                    "cli": f.map(|x| x.cli.display().to_string()),
                    "gui": f.and_then(|x| x.gui.as_ref().map(|g| g.display().to_string())),
                    "mode": mode.as_str(),
                    "port": spec.port,
                    "supports_live": spec.supports_live(),
                    "live": live_listening,
                    "enabled": enabled(spec.id, choices),
                    "available": f.is_some() && available_on_demand(spec.id, choices, modes),
                    "connected": is_conn,
                    "tools": tool_counts.get(spec.id).copied().unwrap_or(0),
                    "health": health,
                    "last_error": ov_err.cloned().or_else(|| hub_err.cloned()),
                })
            })
            .collect::<Vec<_>>()
    )
}

/// Backwards-compatible 3-argument `report` wrapper.
#[must_use]
pub fn report(found: &[Found], choices: &BTreeMap<String, bool>, connected: &[String]) -> Value {
    report_full(
        found,
        &BTreeMap::new(),
        choices,
        &BTreeMap::new(),
        connected,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use workcraft_mcp::ServerEntry;

    fn fake_found(id: &'static str, port: Option<u16>) -> Found {
        Found {
            id,
            name: "X",
            kind: "k",
            cli: PathBuf::from(format!("C:/Apps/{id}-cli.exe")),
            gui: None,
            version: Some("0.1.0".into()),
            live_port: port,
        }
    }

    #[test]
    fn all_17_specs_have_unique_ids_ports_and_valid_metadata() {
        assert_eq!(SPECS.len(), 17);
        verify_unique_ports().unwrap();
        let mut ids = BTreeSet::new();
        for s in &SPECS {
            assert!(ids.insert(s.id), "duplicate id {}", s.id);
            assert!(!s.name.is_empty());
            assert!(s.icon.starts_with("icons/") && s.icon.ends_with(".png"));
            assert!(!s.skill.is_empty());
            assert!(
                s.lean_tools.len() <= 16,
                "{} has {} lean tools (> 16)",
                s.id,
                s.lean_tools.len()
            );
            for tool in s.lean_tools {
                let full_len = s.id.len() + 2 + tool.len();
                assert!(full_len <= 64, "{}__{tool} exceeds 64 chars", s.id);
            }
        }
    }

    #[test]
    fn document_apps_are_on_by_default_and_choices_win() {
        let mut c = BTreeMap::new();
        assert!(enabled("wordcraft", &c));
        assert!(!enabled("filmcraft", &c));
        assert!(available_on_demand("photocraft", &c, &BTreeMap::new()));
        assert!(!available_on_demand("craft-anything", &c, &BTreeMap::new()));
        c.insert("wordcraft".to_string(), false);
        c.insert("filmcraft".to_string(), true);
        c.insert("craft-anything".to_string(), true);
        assert!(!enabled("wordcraft", &c) && enabled("filmcraft", &c));
        assert!(available_on_demand("craft-anything", &c, &BTreeMap::new()));
    }

    #[test]
    fn merge_keeps_mcp_json_and_adds_found_apps() {
        let mut cfg = McpConfig::default();
        cfg.servers.insert(
            "gridcraft".into(),
            ServerEntry {
                command: Some("my-grid".into()),
                ..ServerEntry::default()
            },
        );
        let m = merge(
            cfg,
            &[
                fake_found("wordcraft", None),
                fake_found("gridcraft", None),
                fake_found("pdfcraft", None),
            ],
            &BTreeMap::new(),
        );
        assert_eq!(
            m.servers["gridcraft"].command.as_deref(),
            Some("my-grid"),
            "mcp.json wins"
        );
        assert_eq!(m.servers["wordcraft"].args, vec!["mcp"]);
        assert_eq!(m.servers["pdfcraft"].args, vec!["mcp", "--compact"]);
        let live = server(&fake_found("wordcraft", Some(7981)));
        assert_eq!(live.args, vec!["mcp", "--connect", "127.0.0.1:7981"]);
    }

    #[test]
    fn discovery_in_fake_program_files_tree_finds_artcraft_subdir_apps() {
        let tmp =
            std::env::temp_dir().join(format!("wc-fake-pf-{}", uuid::Uuid::new_v4().simple()));
        let vc_dir = tmp.join("ArtCraft").join("VectorCraft");
        let tc_dir = tmp.join("TextureCraft");
        std::fs::create_dir_all(&vc_dir).unwrap();
        std::fs::create_dir_all(&tc_dir).unwrap();
        std::fs::write(vc_dir.join(discover::exe("vectorcraft-cli")), b"stub").unwrap();
        std::fs::write(tc_dir.join(discover::exe("texturecraft-cli")), b"stub").unwrap();

        let (found, errs) = discover_in_bases(std::slice::from_ref(&tmp), false, false);
        assert!(errs.is_empty());
        assert!(found.iter().any(|f| f.id == "vectorcraft"));
        assert!(found.iter().any(|f| f.id == "texturecraft"));
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[test]
    fn extract_mentions_handles_names_and_aliases() {
        let m = extract_mentions("Please use @photocraft and @craft-anything, then @Deck!");
        assert_eq!(m, vec!["photocraft", "craft-anything", "deckcraft"]);
    }

    #[test]
    fn soundcraft_headless_does_not_pass_demo_and_craft_anything_uses_clean_room() {
        let ws = std::env::temp_dir().join("wc-roots-check");
        let dd = std::env::temp_dir().join("wc-data-check");
        let roots = RootsContext::for_workspace(ws.clone(), dd.clone(), Vec::new());

        let sc = spec_by_id("soundcraft").unwrap();
        let sc_args = headless_mcp_args(sc, &roots);
        assert!(!sc_args.iter().any(|a| a == "--demo"));
        assert!(sc_args.iter().any(|a| a == "--automation-read-root"));

        let ca = spec_by_id("craft-anything").unwrap();
        let ca_args = headless_mcp_args(ca, &roots);
        assert!(ca_args.iter().any(|a| a == "--clean-room"));
        assert!(!ca_args.iter().any(|a| a == "--allow-run"));
        assert!(!ca_args.iter().any(|a| a == "--deep"));
        assert!(ca_args.iter().any(|a| a == "--inbox"));

        let mc = spec_by_id("materialcraft").unwrap();
        let mc_args = headless_mcp_args(mc, &roots);
        assert!(mc_args.iter().any(|a| a == "--read-root"));
        assert!(!mc_args.iter().any(|a| a == "--automation-read-root"));

        let _ = std::fs::remove_dir_all(ws);
        let _ = std::fs::remove_dir_all(dd);
    }
}
