//! L4 live-window tests (`PLAN.md` section 7.3): start the real desktop app with `--control <port>`
//! through the registry, bridge its MCP server, run a read, a write and a screenshot, then stop
//! **only the process this test started**.
//!
//! These open real windows on the desktop, so they are `#[ignore]`d. Run them one at a time:
//!
//! ```text
//! cargo test -p workcraft-agents --test apps_live -- --include-ignored --test-threads=1 --nocapture
//! ```
//!
//! Safety rules: a test is skipped when the app is already running (never drive or kill the user's
//! own window), nothing is saved outside a scratch folder, and the window is closed by PID only.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use workcraft_agents::craft::find_spec;
use workcraft_agents::craft::live::{
    RootsContext, ensure_live_with_roots, live_port_for, take_spawned_gui_pid,
};
use workcraft_core::ToolResult;
use workcraft_mcp::McpHub;

struct Live {
    hub: McpHub,
    id: &'static str,
    scratch: PathBuf,
}

impl Live {
    /// Start the window for `id`. `None` (skip) when not installed or already running.
    async fn start(id: &'static str) -> Option<Self> {
        let spec = find_spec(id).expect("spec");
        if live_port_for(id, spec.name).is_some() {
            eprintln!(
                "SKIP {id}: a {} window is already running; not touching the user's session",
                spec.name
            );
            return None;
        }
        let scratch =
            std::env::temp_dir().join(format!("wc-live-{id}-{}", uuid::Uuid::new_v4().simple()));
        let ws = scratch.join("workspace");
        let data = scratch.join("data");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let roots = RootsContext::for_workspace(ws, data, Vec::new());
        let hub = McpHub::new();
        match ensure_live_with_roots(id, None, &roots, &hub).await {
            Ok(msg) => eprintln!("[{id}] {msg}"),
            Err(e) => {
                eprintln!("SKIP {id}: {e}");
                Self::kill_own(id);
                let _ = std::fs::remove_dir_all(&scratch);
                return None;
            }
        }
        Some(Self { hub, id, scratch })
    }

    fn kill_own(id: &str) {
        if let Some(pid) = take_spawned_gui_pid(id) {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .output();
            eprintln!("[{id}] stopped own window pid {pid}");
        }
    }

    async fn call(&self, tool: &str, args: Value) -> ToolResult {
        let full = format!("{}__{tool}", self.id);
        let mut r;
        let mut tries = 0;
        loop {
            r = self
                .hub
                .call(&full, args.clone())
                .await
                .unwrap_or_else(|e| panic!("{full}: transport error: {e}"));
            // A window that was just launched answers "still starting" until its UI is ready.
            if !r.ok && r.text.contains("still starting") && tries < 30 {
                tries += 1;
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            break;
        }
        let shown: String = r.text.chars().take(240).collect();
        eprintln!(
            "[{full}] ok={} images={} :: {}",
            r.ok,
            r.images.len(),
            shown.replace('\n', " ")
        );
        r
    }

    async fn finish(self) {
        self.hub.clear().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        // `Drop` stops the window and removes the scratch folder.
    }
}

/// Runs even when an assertion panics, so a failed test never leaves a window behind.
impl Drop for Live {
    fn drop(&mut self) {
        Self::kill_own(self.id);
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

fn pictured(r: &ToolResult) -> bool {
    r.images.iter().any(|i| i.mime_type.starts_with("image/"))
        || r.text.contains("data:image/")
        || r.text.contains("base64")
}

#[tokio::test]
#[ignore = "opens a real window"]
async fn photocraft_live_new_document_inspect_screenshot() {
    let Some(l) = Live::start("photocraft").await else {
        return;
    };
    let n = l
        .call(
            "doc_new",
            json!({"width": 320, "height": 200, "name": "wc-live"}),
        )
        .await;
    assert!(n.ok, "doc_new live: {}", n.text);
    let i = l.call("doc_inspect", json!({})).await;
    assert!(i.ok, "doc_inspect live: {}", i.text);
    let s = l.call("ui_screenshot", json!({})).await;
    assert!(s.ok && pictured(&s), "live screenshot: {}", s.text);
    l.finish().await;
}

#[tokio::test]
#[ignore = "opens a real window"]
async fn vectorcraft_live_shape_inspect_screenshot() {
    let Some(l) = Live::start("vectorcraft").await else {
        return;
    };
    // A fresh window has no document; `file.new` makes one (found with command_list).
    let n = l
        .call(
            "command_run",
            json!({"id": "file.new", "params": {"name": "wc-live"}}),
        )
        .await;
    assert!(n.ok, "file.new live: {}", n.text);
    let d = l
        .call(
            "draw_shape",
            json!({"shape": "rect", "x": 20, "y": 20, "width": 120, "height": 80, "fill": "#2f7d5b"}),
        )
        .await;
    assert!(d.ok, "draw_shape live: {}", d.text);
    let i = l.call("inspect_document", json!({"limit": 10})).await;
    assert!(i.ok, "inspect_document live: {}", i.text);
    let s = l.call("ui_screenshot", json!({})).await;
    assert!(s.ok && pictured(&s), "live screenshot: {}", s.text);
    l.finish().await;
}

#[tokio::test]
#[ignore = "opens a real window"]
async fn solidcraft_live_build_measure_screenshot() {
    let Some(l) = Live::start("solidcraft").await else {
        return;
    };
    let n = l.call("new_design", json!({"name": "wc-live"})).await;
    assert!(n.ok, "new_design live: {}", n.text);
    let b = l
        .call(
            "batch",
            json!({"commands": [
                {"command": "sketch.create", "params": {"plane": "XY"}},
                {"command": "sketch.rectangle.two_point", "params": {"p0": [0, 0], "p1": [40, 30]}},
                {"command": "sketch.finish"},
                {"command": "solid.extrude", "params": {"distance": 12}}
            ], "expect": {"volume_mm3": 14400, "bodies": 1}}),
        )
        .await;
    assert!(b.ok, "batch live: {}", b.text);
    let m = l.call("measure", json!({"summary": true})).await;
    assert!(m.ok && m.text.contains("14400"), "measure live: {}", m.text);
    // Bridged, `source: "model"` renders the model instead of the whole app window.
    let s = l
        .call(
            "screenshot",
            json!({"source": "model", "view": "iso", "width": 320, "height": 240}),
        )
        .await;
    assert!(s.ok && pictured(&s), "live screenshot: {}", s.text);
    l.finish().await;
}

#[tokio::test]
#[ignore = "opens a real window"]
async fn filmcraft_live_inspect_screenshot() {
    let Some(l) = Live::start("filmcraft").await else {
        return;
    };
    let i = l.call("project_inspect", json!({})).await;
    assert!(i.ok, "project_inspect live: {}", i.text);
    let s = l.call("ui_screenshot", json!({})).await;
    assert!(s.ok && pictured(&s), "live screenshot: {}", s.text);
    l.finish().await;
}
