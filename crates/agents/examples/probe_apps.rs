//! Probe every installed Craft app's `<app>-cli mcp` server over stdio and write contract
//! snapshots to `crates/agents/tests/snapshots/tools-<app>.json`.
//!
//! Run with:
//!   cargo run -p workcraft-agents --example probe_apps

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use workcraft_mcp::{McpClient, ServerEntry};

struct ProbeTarget {
    id: &'static str,
    label: &'static str,
    folder: &'static str,
    cli: &'static str,
    args_kind: ArgsKind,
}

#[derive(Clone, Copy)]
enum ArgsKind {
    Plain,
    CompactOnly,
    CompactWithRoots,
    Roots,
    HeadlessWithRoots,
    ShortRoots,
    CleanRoomInbox,
    SolidHeadless,
}

const TARGETS: &[ProbeTarget] = &[
    ProbeTarget {
        id: "photocraft",
        label: "PhotoCraft",
        folder: "PhotoCraft",
        cli: "photocraft-cli",
        args_kind: ArgsKind::Roots,
    },
    ProbeTarget {
        id: "vectorcraft",
        label: "VectorCraft",
        folder: "VectorCraft",
        cli: "vectorcraft-cli",
        args_kind: ArgsKind::HeadlessWithRoots,
    },
    ProbeTarget {
        id: "filmcraft",
        label: "FilmCraft",
        folder: "FilmCraft",
        cli: "filmcraft-cli",
        args_kind: ArgsKind::Plain,
    },
    ProbeTarget {
        id: "lightcraft",
        label: "LightCraft",
        folder: "LightCraft",
        cli: "lightcraft-cli",
        args_kind: ArgsKind::CompactOnly,
    },
    ProbeTarget {
        id: "pdfcraft",
        label: "PdfCraft",
        folder: "PdfCraft",
        cli: "pdfcraft-cli",
        args_kind: ArgsKind::CompactWithRoots,
    },
    ProbeTarget {
        id: "effectcraft",
        label: "EffectCraft",
        folder: "EffectCraft",
        cli: "effectcraft-cli",
        args_kind: ArgsKind::Plain,
    },
    ProbeTarget {
        id: "designcraft",
        label: "DesignCraft",
        folder: "DesignCraft",
        cli: "designcraft-cli",
        args_kind: ArgsKind::Roots,
    },
    ProbeTarget {
        id: "figcraft",
        label: "FigCraft",
        folder: "FigCraft",
        cli: "figcraft-cli",
        args_kind: ArgsKind::Roots,
    },
    ProbeTarget {
        id: "texturecraft",
        label: "TextureCraft",
        folder: "TextureCraft",
        cli: "texturecraft-cli",
        args_kind: ArgsKind::ShortRoots,
    },
    ProbeTarget {
        id: "materialcraft",
        label: "MaterialCraft",
        folder: "MaterialCraft",
        cli: "materialcraft-cli",
        args_kind: ArgsKind::ShortRoots,
    },
    ProbeTarget {
        id: "wordcraft",
        label: "WordCraft",
        folder: "WordCraft",
        cli: "wordcraft-cli",
        args_kind: ArgsKind::Roots,
    },
    ProbeTarget {
        id: "soundcraft",
        label: "SoundCraft",
        folder: "SoundCraft",
        cli: "soundcraft-cli",
        args_kind: ArgsKind::Roots,
    },
    ProbeTarget {
        id: "deckcraft",
        label: "DeckCraft",
        folder: "DeckCraft",
        cli: "deckcraft-cli",
        args_kind: ArgsKind::Roots,
    },
    ProbeTarget {
        id: "cadcraft",
        label: "CADCraft",
        folder: "CADCraft",
        cli: "cadcraft-cli",
        args_kind: ArgsKind::Plain,
    },
    ProbeTarget {
        id: "gridcraft",
        label: "GridCraft",
        folder: "GridCraft",
        cli: "gridcraft-cli",
        args_kind: ArgsKind::Plain,
    },
    ProbeTarget {
        id: "craft-anything",
        label: "Craft Anything",
        folder: "Craft Anything",
        cli: "craft-anything-cli",
        args_kind: ArgsKind::CleanRoomInbox,
    },
    ProbeTarget {
        id: "solidcraft",
        label: "SolidCraft",
        folder: "SolidCraft",
        cli: "solidcraft-cli",
        args_kind: ArgsKind::SolidHeadless,
    },
];

fn find_cli(target: &ProbeTarget) -> Option<PathBuf> {
    let env_key = format!("{}_CLI", target.id.to_ascii_uppercase().replace('-', "_"));
    if let Ok(explicit) = std::env::var(&env_key) {
        let p = PathBuf::from(explicit.trim().trim_matches('"'));
        if p.is_file() {
            return Some(p);
        }
    }
    let exe = if cfg!(windows) {
        format!("{}.exe", target.cli)
    } else {
        target.cli.to_string()
    };
    let mut candidates = Vec::new();
    for var in ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"] {
        if let Ok(base) = std::env::var(var) {
            let b = PathBuf::from(&base);
            candidates.push(b.join(target.folder).join(&exe));
            candidates.push(b.join("ArtCraft").join(target.folder).join(&exe));
            candidates.push(b.join("Programs").join(target.folder).join(&exe));
            candidates.push(
                b.join("Programs")
                    .join("ArtCraft")
                    .join(target.folder)
                    .join(&exe),
            );
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            if dir.is_absolute() {
                candidates.push(dir.join(&exe));
            }
        }
    }
    for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        candidates.push(PathBuf::from(dir).join(&exe));
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn build_args(kind: ArgsKind, scratch: &Path) -> Vec<String> {
    let s = scratch.display().to_string();
    match kind {
        ArgsKind::Plain => vec!["mcp".into()],
        ArgsKind::CompactOnly => vec!["mcp".into(), "--compact".into()],
        ArgsKind::CompactWithRoots => vec![
            "mcp".into(),
            "--compact".into(),
            "--automation-read-root".into(),
            s.clone(),
            "--automation-write-root".into(),
            s,
        ],
        ArgsKind::Roots => vec![
            "mcp".into(),
            "--automation-read-root".into(),
            s.clone(),
            "--automation-write-root".into(),
            s,
        ],
        ArgsKind::HeadlessWithRoots => vec![
            "mcp".into(),
            "--headless".into(),
            "--automation-read-root".into(),
            s.clone(),
            "--automation-write-root".into(),
            s,
        ],
        ArgsKind::ShortRoots => vec![
            "mcp".into(),
            "--read-root".into(),
            s.clone(),
            "--write-root".into(),
            s,
        ],
        ArgsKind::CleanRoomInbox => {
            let inbox = scratch.join("inbox");
            let _ = std::fs::create_dir_all(&inbox);
            vec![
                "mcp".into(),
                "--clean-room".into(),
                "--inbox".into(),
                inbox.display().to_string(),
            ]
        }
        ArgsKind::SolidHeadless => {
            vec!["mcp".into(), "--headless".into(), "--allow-path".into(), s]
        }
    }
}

#[tokio::main]
async fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let snapshots_dir = manifest_dir.join("tests").join("snapshots");
    if let Err(e) = std::fs::create_dir_all(&snapshots_dir) {
        eprintln!("failed to create {}: {e}", snapshots_dir.display());
        std::process::exit(1);
    }

    let scratch = std::env::temp_dir().join(format!("wc-probe-{}", std::process::id()));
    if let Err(e) = std::fs::create_dir_all(&scratch) {
        eprintln!("failed to create scratch {}: {e}", scratch.display());
        std::process::exit(1);
    }

    println!(
        "{:<16} {:<10} {:>6} {:>9} {:>10}  CLI",
        "APP", "VERSION", "TOOLS", "ANNOTATED", "SCHEMA_KB"
    );
    println!("{}", "-".repeat(96));

    let mut found = 0usize;
    let mut failed = 0usize;

    for target in TARGETS {
        let Some(cli_path) = find_cli(target) else {
            println!("{:<16} {:<10} {:>6}", target.id, "MISSING", "-");
            continue;
        };

        let entry = ServerEntry {
            command: Some(cli_path.display().to_string()),
            args: build_args(target.args_kind, &scratch),
            ..ServerEntry::default()
        };

        let client = match McpClient::connect(target.id, &entry).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{:<16} ERROR connecting: {e}", target.id);
                failed += 1;
                continue;
            }
        };

        let info = client.server_info().unwrap_or_else(|| json!({}));
        let version = info
            .pointer("/serverInfo/version")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        let raw_tools = match client.list_tools_raw().await {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{:<16} ERROR tools/list: {e}", target.id);
                failed += 1;
                continue;
            }
        };

        let schema_bytes: usize = raw_tools
            .iter()
            .map(|t| serde_json::to_string(t).map_or(0, |s| s.len()))
            .sum();

        let mut annotated_count = 0usize;
        let mut tool_records = Vec::with_capacity(raw_tools.len());

        for t in &raw_tools {
            let name = t
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let annotations = t.get("annotations").cloned();
            if annotations
                .as_ref()
                .and_then(Value::as_object)
                .is_some_and(|o| !o.is_empty())
            {
                annotated_count += 1;
            }
            let mut required: Vec<String> = t
                .pointer("/inputSchema/required")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            required.sort();

            let mut props: Vec<String> = t
                .pointer("/inputSchema/properties")
                .and_then(Value::as_object)
                .map(|obj| obj.keys().cloned().collect())
                .unwrap_or_default();
            props.sort();

            tool_records.push(json!({
                "name": name,
                "required": required,
                "properties": props,
                "annotations": annotations,
            }));
        }

        println!(
            "{:<16} {:<10} {:>6} {:>9} {:>9.1}K  {}",
            target.id,
            version,
            raw_tools.len(),
            annotated_count,
            schema_bytes as f64 / 1024.0,
            cli_path.display()
        );

        let snapshot = json!({
            "app": target.id,
            "label": target.label,
            "server_info": info.get("serverInfo").cloned().unwrap_or_else(|| json!({})),
            "capabilities": info.get("capabilities").cloned().unwrap_or_else(|| json!({})),
            "tool_count": raw_tools.len(),
            "annotated_count": annotated_count,
            "schema_bytes": schema_bytes,
            "tools": tool_records,
        });

        let snap_path = snapshots_dir.join(format!("tools-{}.json", target.id));
        match serde_json::to_string_pretty(&snapshot) {
            Ok(pretty) => {
                if let Err(e) = std::fs::write(&snap_path, format!("{pretty}\n")) {
                    eprintln!("failed to write {}: {e}", snap_path.display());
                    failed += 1;
                }
            }
            Err(e) => {
                eprintln!("failed to serialize snapshot for {}: {e}", target.id);
                failed += 1;
            }
        }

        drop(client);
        found += 1;
    }

    let _ = std::fs::remove_dir_all(&scratch);
    println!(
        "\nProbed {found}/{} apps ({failed} failed). Snapshots saved to {}",
        TARGETS.len(),
        snapshots_dir.display()
    );
    if failed > 0 {
        std::process::exit(1);
    }
}
