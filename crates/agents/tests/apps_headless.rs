//! L3 headless golden workflows and L7 failure injection against the REAL installed ArtCraft apps
//! (`PLAN.md` section 7).
//!
//! Every test is `#[ignore]` so the default `cargo test` stays offline and fast. Run them with
//!
//! ```text
//! cargo test -p workcraft-agents --test apps_headless -- --include-ignored --test-threads=4 --nocapture
//! ```
//!
//! Rules: each test uses its own scratch folder under the OS temp dir as the only sandbox root, talks to
//! `<app>-cli mcp` over stdio through `McpHub` (never opens a desktop window), never calls a paid tool,
//! and stops only the child process it started. An app that is not installed makes its test skip.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use workcraft_agents::craft::live::RootsContext;
use workcraft_agents::craft::{CraftMode, discover, server_for};
use workcraft_core::ToolResult;
use workcraft_mcp::{McpHub, ServerEntry};

struct Fx {
    hub: McpHub,
    id: &'static str,
    ws: PathBuf,
    scratch: PathBuf,
}

impl Fx {
    /// Start `id` headless inside a fresh scratch workspace. `None` when the app is not installed.
    async fn new(id: &'static str) -> Option<Self> {
        let scratch = std::env::temp_dir().join(format!(
            "wc-headless-{id}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let ws = scratch.join("workspace");
        let data = scratch.join("data");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let Some(found) = discover().into_iter().find(|f| f.id == id) else {
            eprintln!("SKIP {id}: not installed");
            let _ = std::fs::remove_dir_all(&scratch);
            return None;
        };
        let roots = RootsContext::for_workspace(ws.clone(), data.clone(), Vec::new());
        let mut entry = server_for(&found, CraftMode::Headless, &roots);
        for var in [
            "TEXTURECRAFT_NO_PREFS",
            "MATERIALCRAFT_NO_PREFS",
            "SOUNDCRAFT_NO_PREFS",
        ] {
            entry.env.insert(var.to_string(), "1".to_string());
        }
        entry.env.insert(
            "CRAFT_ANYTHING_CONFIG_DIR".to_string(),
            data.join("ca-cfg").to_string_lossy().into_owned(),
        );
        let hub = McpHub::new();
        let n = hub
            .connect_one(id, &entry)
            .await
            .unwrap_or_else(|e| panic!("{id}: headless connect failed: {e}"));
        eprintln!("[{id}] connected, {n} tools, args {:?}", entry.args);
        Some(Self {
            hub,
            id,
            ws,
            scratch,
        })
    }

    /// Absolute path of `name` inside the workspace.
    fn p(&self, name: &str) -> String {
        self.ws.join(name).to_string_lossy().into_owned()
    }

    fn path(&self, name: &str) -> PathBuf {
        self.ws.join(name)
    }

    async fn call(&self, tool: &str, args: Value) -> ToolResult {
        let full = format!("{}__{tool}", self.id);
        let r = self
            .hub
            .call(&full, args)
            .await
            .unwrap_or_else(|e| panic!("{full}: transport error: {e}"));
        let shown: String = r.text.chars().take(300).collect();
        eprintln!(
            "[{full}] ok={} images={} :: {}",
            r.ok,
            r.images.len(),
            shown.replace('\n', " ")
        );
        r
    }

    /// Call and require success.
    async fn ok(&self, tool: &str, args: Value) -> ToolResult {
        let r = self.call(tool, args.clone()).await;
        assert!(
            r.ok,
            "{}__{tool} {args} failed: {}",
            self.id,
            r.text.chars().take(800).collect::<String>()
        );
        r
    }

    async fn done(self) {
        self.hub.clear().await;
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

fn sized(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

fn head(p: &Path, n: usize) -> Vec<u8> {
    let mut b = std::fs::read(p).unwrap_or_default();
    b.truncate(n);
    b
}

fn assert_file(p: &Path, magic: &[u8]) {
    assert!(sized(p) > 0, "{} missing or empty", p.display());
    assert!(
        head(p, magic.len()) == magic,
        "{}: expected header {:?}, got {:?}",
        p.display(),
        magic,
        head(p, magic.len())
    );
}

/// A picture came back inline (MCP image part) or was written to `file`.
fn assert_picture(r: &ToolResult, file: Option<&Path>) {
    let inline = r.images.iter().any(|i| i.mime_type.starts_with("image/"));
    let on_disk = file.is_some_and(|f| sized(f) > 100);
    let in_text = r.text.contains("data:image/") || r.text.contains("png_base64");
    assert!(
        inline || on_disk || in_text,
        "no picture in result (images={}, text={:?})",
        r.images.len(),
        r.text.chars().take(200).collect::<String>()
    );
}

/// A 240x180 gradient test image in PNG or JPEG (chosen by extension).
fn make_image(path: &Path) {
    let img = image::RgbImage::from_fn(240, 180, |x, y| {
        image::Rgb([
            (x % 256) as u8,
            (y * 255 / 180) as u8,
            ((x + y) % 256) as u8,
        ])
    });
    img.save(path).unwrap();
}

/// 0.5 s of a 440 Hz sine, 16-bit mono PCM WAV.
fn make_wav(path: &Path) {
    let rate: u32 = 44_100;
    let n = rate / 2;
    let mut data = Vec::with_capacity(n as usize * 2);
    for i in 0..n {
        let s = (f64::from(i) * 440.0 * std::f64::consts::TAU / f64::from(rate)).sin();
        data.extend_from_slice(&((s * 12_000.0) as i16).to_le_bytes());
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    std::fs::write(path, out).unwrap();
}

/// Short MP4 shipped with the creative-apps plugin test inputs (override with `WORKCRAFT_TEST_CLIP`).
fn test_clip() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("WORKCRAFT_TEST_CLIP") {
        return Some(PathBuf::from(p));
    }
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../../../artcraft/mcp/creative-apps-claude-plugin-main/tests/inputs/test_clip.mp4",
    );
    p.is_file().then_some(p)
}

// ---------------------------------------------------------------------------------------------
// L3: one golden workflow per app
// ---------------------------------------------------------------------------------------------

#[tokio::test]
#[ignore = "real app"]
async fn wordcraft_new_type_render_save() {
    let Some(f) = Fx::new("wordcraft").await else {
        return;
    };
    f.ok("new_document", json!({})).await;
    f.ok("type_text", json!({"text": "Hello WorkCraft headless"}))
        .await;
    let t = f.ok("get_text", json!({})).await;
    assert!(t.text.contains("Hello WorkCraft"), "text: {}", t.text);
    let r = f.ok("render_page", json!({"page": 1})).await;
    assert_picture(&r, None);
    let out = f.path("out.docx");
    f.ok("save_document", json!({"path": f.p("out.docx")}))
        .await;
    assert_file(&out, b"PK");
    f.done().await;
}

#[tokio::test]
#[ignore = "real app"]
async fn gridcraft_formula_roundtrip() {
    let Some(f) = Fx::new("gridcraft").await else {
        return;
    };
    f.ok("new_workbook", json!({})).await;
    f.ok("set_cell", json!({"cell": "A1", "input": "10"})).await;
    f.ok("set_cell", json!({"cell": "A2", "input": "32"})).await;
    f.ok("set_cell", json!({"cell": "A3", "input": "=SUM(A1:A2)"}))
        .await;
    let c = f.ok("get_cell", json!({"cell": "A3"})).await;
    assert!(
        c.text.contains("42"),
        "A3 should evaluate to 42: {}",
        c.text
    );
    f.ok("save_workbook", json!({"path": f.p("out.xlsx")}))
        .await;
    assert_file(&f.path("out.xlsx"), b"PK");
    f.done().await;
}

#[tokio::test]
#[ignore = "real app"]
async fn deckcraft_slide_text_render_save() {
    let Some(f) = Fx::new("deckcraft").await else {
        return;
    };
    f.ok("new_presentation", json!({})).await;
    f.ok(
        "add_text_box",
        json!({"text": "WorkCraft deck", "x": 60, "y": 60, "w": 600, "h": 80}),
    )
    .await;
    f.ok("inspect_document", json!({})).await;
    // DeckCraft slides are 0-based.
    let r = f.ok("render_slide", json!({"slide": 0})).await;
    assert_picture(&r, None);
    f.ok("save_presentation", json!({"path": f.p("out.pptx")}))
        .await;
    assert_file(&f.path("out.pptx"), b"PK");
    // The text must have made it into the saved package (slide XML inside the zip).
    let mut zip = zip::ZipArchive::new(std::fs::File::open(f.path("out.pptx")).unwrap()).unwrap();
    let mut found = false;
    for i in 0..zip.len() {
        let mut e = zip.by_index(i).unwrap();
        let name = e.name().map(|n| n.to_string()).unwrap_or_default();
        if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut e, &mut s).unwrap();
            found |= s.contains("WorkCraft deck");
        }
    }
    assert!(found, "saved pptx does not contain the text box text");
    f.done().await;
}

#[tokio::test]
#[ignore = "real app"]
async fn pdfcraft_create_text_save_reopen() {
    let Some(f) = Fx::new("pdfcraft").await else {
        return;
    };
    let c = f
        .ok(
            "tool_call",
            json!({"name": "doc_create", "arguments": {"from": "blank", "pages": 2}}),
        )
        .await;
    let doc = extract_doc_id(&c.text);
    let mut add = json!({"page": 1, "text": "WorkCraft PDF", "at": [72, 72]});
    add["doc"] = json!(doc);
    f.ok(
        "tool_call",
        json!({"name": "page_add_text", "arguments": add}),
    )
    .await;
    f.ok(
        "tool_call",
        json!({"name": "doc_save", "arguments": {"doc": doc, "path": f.p("out.pdf")}}),
    )
    .await;
    assert_file(&f.path("out.pdf"), b"%PDF-");
    let o = f
        .ok(
            "tool_call",
            json!({"name": "doc_open", "arguments": {"path": f.p("out.pdf")}}),
        )
        .await;
    let doc2 = extract_doc_id(&o.text);
    let t = f
        .ok(
            "tool_call",
            json!({"name": "text_extract", "arguments": {"doc": doc2}}),
        )
        .await;
    assert!(t.text.contains("WorkCraft PDF"), "extract: {}", t.text);
    let r = f
        .ok(
            "tool_call",
            json!({"name": "page_render", "arguments": {"doc": doc2, "page": 1}}),
        )
        .await;
    assert_picture(&r, None);
    f.done().await;
}

/// Pull a document handle out of a PdfCraft result (`{"doc":"..."}` JSON or `doc: <id>` text).
fn extract_doc_id(text: &str) -> Value {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        for k in ["doc", "doc_id", "id", "handle"] {
            if let Some(x) = v.get(k).or_else(|| v.pointer(&format!("/result/{k}"))) {
                return x.clone();
            }
        }
    }
    panic!("no document id in PdfCraft result: {text}");
}

#[tokio::test]
#[ignore = "real app"]
async fn photocraft_blur_render_export() {
    let Some(f) = Fx::new("photocraft").await else {
        return;
    };
    make_image(&f.path("in.png"));
    f.ok("doc_open", json!({"path": f.p("in.png")})).await;
    f.ok(
        "command_run",
        json!({"id": "filter.blur.gaussianBlur", "params": {"radius": 6}}),
    )
    .await;
    let r = f.ok("doc_render_preview", json!({})).await;
    assert_picture(&r, None);
    f.ok(
        "doc_export",
        json!({"path": f.p("out.png"), "format": "png"}),
    )
    .await;
    assert_file(&f.path("out.png"), &[0x89, b'P', b'N', b'G']);
    assert_ne!(
        std::fs::read(f.path("in.png")).unwrap(),
        std::fs::read(f.path("out.png")).unwrap(),
        "blurred export must differ from the input"
    );
    f.done().await;
}

#[tokio::test]
#[ignore = "real app"]
async fn vectorcraft_shape_text_export() {
    let Some(f) = Fx::new("vectorcraft").await else {
        return;
    };
    f.ok(
        "draw_shape",
        json!({"shape": "rect", "x": 20, "y": 20, "width": 160, "height": 90, "fill": "#2f7d5b"}),
    )
    .await;
    f.ok(
        "add_text",
        json!({"text": "WorkCraft", "x": 30, "y": 70, "size": 24}),
    )
    .await;
    f.ok("inspect_document", json!({"limit": 20})).await;
    let r = f.ok("render_preview", json!({})).await;
    assert_picture(&r, None);
    f.ok("export", json!({"format": "svg", "path": f.p("out.svg")}))
        .await;
    let svg = std::fs::read_to_string(f.path("out.svg")).unwrap();
    assert!(svg.contains("<svg"), "svg: {}", &svg[..svg.len().min(200)]);
    f.ok("export", json!({"format": "png", "path": f.p("out.png")}))
        .await;
    assert_file(&f.path("out.png"), &[0x89, b'P', b'N', b'G']);
    f.done().await;
}

#[tokio::test]
#[ignore = "real app"]
async fn lightcraft_import_develop_render_export() {
    let Some(f) = Fx::new("lightcraft").await else {
        return;
    };
    make_image(&f.path("shot.jpg"));
    f.ok("import", json!({"paths": [f.p("shot.jpg")]})).await;
    let q = f.ok("query_photos", json!({"limit": 5})).await;
    let id = first_photo_id(&q.text);
    f.ok(
        "set_develop",
        json!({"id": id, "settings": {"exposure": 0.6}}),
    )
    .await;
    let r = f.ok("render_photo", json!({"id": id, "size": 256})).await;
    assert_picture(&r, None);
    f.ok(
        "export",
        json!({"id": id, "path": f.p("export.jpg"), "format": "jpeg"}),
    )
    .await;
    assert_file(&f.path("export.jpg"), &[0xFF, 0xD8]);
    f.done().await;
}

/// First photo id in a LightCraft `query_photos` result.
fn first_photo_id(text: &str) -> Value {
    let v: Value = serde_json::from_str(text).unwrap_or_else(|_| panic!("not JSON: {text}"));
    for ptr in [
        "/photos/0/id",
        "/items/0/id",
        "/results/0/id",
        "/0/id",
        "/ids/0",
    ] {
        if let Some(x) = v.pointer(ptr) {
            return x.clone();
        }
    }
    panic!("no photo id in: {text}");
}

#[tokio::test]
#[ignore = "real app"]
async fn designcraft_frame_image_render_export() {
    let Some(f) = Fx::new("designcraft").await else {
        return;
    };
    make_image(&f.path("pic.png"));
    f.ok(
        "new_document",
        json!({"width": 595, "height": 842, "pages": 1}),
    )
    .await;
    f.ok(
        "place_image",
        json!({"path": f.p("pic.png"), "x": 40, "y": 40, "width": 240}),
    )
    .await;
    // DesignCraft pages are 0-based.
    let r = f.ok("render_page", json!({"page": 0})).await;
    assert_picture(&r, None);
    f.ok("export_png", json!({"path": f.p("page.png"), "page": 0}))
        .await;
    assert_file(&f.path("page.png"), &[0x89, b'P', b'N', b'G']);
    f.done().await;
}

#[tokio::test]
#[ignore = "real app"]
async fn filmcraft_import_cut_render_frame() {
    let Some(f) = Fx::new("filmcraft").await else {
        return;
    };
    let Some(clip) = test_clip() else {
        eprintln!("SKIP filmcraft: test_clip.mp4 not found (set WORKCRAFT_TEST_CLIP)");
        f.done().await;
        return;
    };
    std::fs::copy(&clip, f.path("clip.mp4")).unwrap();
    f.ok("media_import", json!({"paths": [f.p("clip.mp4")]}))
        .await;
    f.ok("project_inspect", json!({})).await;
    f.ok(
        "command_run",
        json!({"id": "file.newSequenceFromClip", "params": {"items": [1]}}),
    )
    .await;
    let s = f.ok("sequence_inspect", json!({})).await;
    assert!(
        s.text.contains("clip.mp4") || s.text.contains("Iris"),
        "sequence: {}",
        s.text
    );
    let r = f.ok("render_frame", json!({"seconds": 0.5})).await;
    assert_picture(&r, None);
    f.done().await;
}

#[tokio::test]
#[ignore = "real app"]
async fn effectcraft_comp_layers_render_save() {
    let Some(f) = Fx::new("effectcraft").await else {
        return;
    };
    f.ok("open_project", json!({"new": true})).await;
    f.ok("command_run", json!({"id": "comp.new"})).await;
    f.ok("command_run", json!({"id": "layer.newSolid"})).await;
    let r = f.ok("render_frame", json!({"time": 0})).await;
    assert_picture(&r, None);
    f.ok("save_project", json!({"path": f.p("out.ecproj")}))
        .await;
    assert!(sized(&f.path("out.ecproj")) > 0);
    f.done().await;
}

#[tokio::test]
#[ignore = "real app"]
async fn soundcraft_import_inspect_bounce_save() {
    let Some(f) = Fx::new("soundcraft").await else {
        return;
    };
    make_wav(&f.path("tone.wav"));
    f.ok("new_session", json!({"name": "wc-test"})).await;
    f.ok("import_audio", json!({"path": f.p("tone.wav")})).await;
    let i = f.ok("inspect_session", json!({})).await;
    assert!(
        i.text.to_lowercase().contains("tone") || i.text.contains("clip"),
        "imported clip not visible: {}",
        i.text
    );
    f.ok(
        "bounce_mix",
        json!({"path": f.p("mix.wav"), "format": "wav"}),
    )
    .await;
    assert_file(&f.path("mix.wav"), b"RIFF");
    f.ok("save_session", json!({"path": f.p("out.scraft")}))
        .await;
    assert!(sized(&f.path("out.scraft")) > 0);
    f.done().await;
}

#[tokio::test]
#[ignore = "real app"]
async fn cadcraft_script_inspect_render_save() {
    let Some(f) = Fx::new("cadcraft").await else {
        return;
    };
    f.ok("new_drawing", json!({})).await;
    f.ok("script", json!({"text": "LINE 0,0 100,0\nCIRCLE 50,50 20"}))
        .await;
    let i = f.ok("inspect_drawing", json!({})).await;
    assert!(
        i.text.to_lowercase().contains("line") || i.text.to_lowercase().contains("circle"),
        "inspect: {}",
        i.text
    );
    let r = f.ok("render", json!({})).await;
    assert_picture(&r, None);
    f.ok("save", json!({"path": f.p("out.dxf")})).await;
    assert!(sized(&f.path("out.dxf")) > 0);
    f.done().await;
}

#[tokio::test]
#[ignore = "real app"]
async fn figcraft_nodes_screenshot_export_save() {
    let Some(f) = Fx::new("figcraft").await else {
        return;
    };
    f.ok(
        "create_node",
        json!({"type": "FRAME", "name": "Card", "x": 0, "y": 0, "width": 320, "height": 200}),
    )
    .await;
    f.ok(
        "create_node",
        json!({"type": "TEXT", "text": "WorkCraft", "x": 20, "y": 20}),
    )
    .await;
    let i = f.ok("inspect_document", json!({})).await;
    assert!(i.text.contains("Card"), "inspect: {}", i.text);
    let r = f.ok("screenshot", json!({})).await;
    assert_picture(&r, None);
    f.ok("export", json!({"format": "svg", "path": f.p("out.svg")}))
        .await;
    assert!(sized(&f.path("out.svg")) > 0);
    f.ok("save_file", json!({"path": f.p("out.figcraft")}))
        .await;
    assert!(sized(&f.path("out.figcraft")) > 0);
    f.done().await;
}

#[tokio::test]
#[ignore = "real app"]
async fn texturecraft_paint_render_export_save() {
    let Some(f) = Fx::new("texturecraft").await else {
        return;
    };
    f.ok("project_new", json!({"name": "wc"})).await;
    f.ok(
        "paint_stroke",
        json!({"points": [[0.2, 0.2], [0.6, 0.6]], "color": "#cc3333"}),
    )
    .await;
    let r = f
        .ok(
            "viewport_render",
            json!({"cpu": true, "width": 256, "height": 256}),
        )
        .await;
    assert_picture(&r, None);
    f.ok("textures_export", json!({"dir": "maps"})).await;
    assert!(f.ws.join("maps").is_dir(), "maps dir not created");
    f.ok("project_save", json!({"path": "wc.tcraft"})).await;
    assert!(sized(&f.path("wc.tcraft")) > 0);
    f.done().await;
}

#[tokio::test]
#[ignore = "real app"]
async fn materialcraft_graph_render_export_save() {
    let Some(f) = Fx::new("materialcraft").await else {
        return;
    };
    f.ok("graph_new", json!({"template": "pbr"})).await;
    f.ok("library_list", json!({})).await;
    let r = f.ok("view_render", json!({})).await;
    assert_picture(&r, None);
    f.ok("outputs_export", json!({"dir": "maps"})).await;
    assert!(f.ws.join("maps").is_dir(), "maps dir not created");
    // MaterialCraft 0.2.0: native .sbs packages; .mcraft is gone and .sbs is appended when missing
    f.ok("package_save", json!({"path": "wc.sbs"})).await;
    assert!(sized(&f.path("wc.sbs")) > 0);
    f.done().await;
}

#[tokio::test]
#[ignore = "real app"]
async fn solidcraft_sketch_extrude_inspect_export() {
    let Some(f) = Fx::new("solidcraft").await else {
        return;
    };
    // The app's own guide is served from the binary (no file access needed).
    let guide = f
        .ok("skill_reference", json!({"skill": "solidcraft-modeling"}))
        .await;
    assert!(guide.text.len() > 5_000, "skill_reference too short");

    f.ok("new_design", json!({"name": "wc"})).await;
    // A 40 x 30 x 12 box is 14400 mm3. `expect` makes the server check it, so a wrong build is an error.
    f.ok(
        "batch",
        json!({
            "commands": [
                {"command": "sketch.create", "params": {"plane": "XY"}},
                {"command": "sketch.rectangle.two_point", "params": {"p0": [0, 0], "p1": [40, 30]}},
                {"command": "sketch.finish"},
                {"command": "solid.extrude", "params": {"distance": 12}}
            ],
            "expect": {"volume_mm3": 14400, "bodies": 1, "bbox": {"min": [0, 0, 0], "max": [40, 30, 12]}}
        }),
    )
    .await;
    let m = f.ok("measure", json!({"summary": true})).await;
    let v: Value = serde_json::from_str(&m.text).unwrap();
    let vol = v["total"]["volume_mm3"]
        .as_f64()
        .or_else(|| v["bodies"][0]["volume_mm3"].as_f64())
        .expect("volume in measure reply");
    assert!((vol - 14_400.0).abs() < 1.0, "volume {vol}");

    // A wrong expectation must come back as an error, not as silent success.
    let bad = f
        .call(
            "execute",
            json!({"command": "solid.extrude", "params": {"distance": 1}, "expect": {"volume_mm3": 14400}}),
        )
        .await;
    assert!(
        !bad.ok,
        "a missed expectation must be an error: {}",
        bad.text
    );
    f.ok("undo", json!({})).await;

    let shot = f
        .ok(
            "screenshot",
            json!({"view": "iso", "width": 320, "height": 240}),
        )
        .await;
    assert_picture(&shot, None);

    // Read-only command by id, then the file formats the app really writes.
    f.ok(
        "execute",
        json!({"command": "engine.describe", "params": {"command": "solid.extrude"}}),
    )
    .await;
    for ext in ["step", "stl", "3mf", "obj"] {
        f.ok("export", json!({"path": f.p(&format!("out.{ext}"))}))
            .await;
        assert!(
            sized(&f.path(&format!("out.{ext}"))) > 0,
            "{ext} export empty"
        );
    }
    // There is no GLB export; the server says so instead of writing something else.
    // A bad argument is a JSON-RPC error (a transport-level Err in the hub), not a tool result.
    let glb = f
        .hub
        .call(
            "solidcraft__export",
            json!({"path": f.p("out.glb"), "format": "glb"}),
        )
        .await;
    assert!(glb.is_err() || glb.is_ok_and(|r| !r.ok));
    assert!(!f.path("out.glb").exists());
    f.ok("save", json!({"path": f.p("part.sc")})).await;
    assert!(sized(&f.path("part.sc")) > 0);

    // A 2D DXF of the front view (for CADCraft or DesignCraft).
    f.ok(
        "execute",
        json!({"command": "file.export_view_dxf", "params": {
            "path": f.p("front.dxf"),
            "camera": {"target": [20, 15, 6], "yaw": 0, "pitch": 0, "roll": 0, "distance": 100, "fov": 0}
        }}),
    )
    .await;
    assert!(sized(&f.path("front.dxf")) > 0, "dxf empty");

    // The STEP file we wrote opens again with the same volume.
    f.ok("open", json!({"path": f.p("out.step")})).await;
    let m2 = f.ok("measure", json!({"summary": true})).await;
    let v2: Value = serde_json::from_str(&m2.text).unwrap();
    let vol2 = v2["total"]["volume_mm3"]
        .as_f64()
        .or_else(|| v2["bodies"][0]["volume_mm3"].as_f64())
        .expect("volume after reopen");
    assert!((vol2 - 14_400.0).abs() < 5.0, "reopened STEP volume {vol2}");
    f.done().await;
}

#[tokio::test]
#[ignore = "real app"]
async fn craft_anything_clean_room_reads_only() {
    let Some(f) = Fx::new("craft-anything").await else {
        return;
    };
    f.ok("core_session_show", json!({})).await;
    f.ok(
        "core_brief_write",
        json!({"question": "What does the WorkCraft test fixture do?"}),
    )
    .await;
    f.ok("core_investigation_show", json!({})).await;
    // Paid generation is never invoked here. The server must also run with --clean-room.
    let args = f.hub.client_args("craft-anything").await.unwrap();
    assert!(
        args.iter().any(|a| a == "--clean-room"),
        "craft-anything must start clean-room: {args:?}"
    );
    assert!(!args.iter().any(|a| a == "--allow-run" || a == "--deep"));
    f.done().await;
}

// ---------------------------------------------------------------------------------------------
// Cross-app handoff (file based)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
#[ignore = "real app"]
async fn handoff_vectorcraft_png_into_deckcraft() {
    let (Some(v), Some(d)) = (Fx::new("vectorcraft").await, Fx::new("deckcraft").await) else {
        return;
    };
    v.ok(
        "draw_shape",
        json!({"shape": "rect", "x": 0, "y": 0, "width": 120, "height": 80, "fill": "#cc3333"}),
    )
    .await;
    v.ok("export", json!({"format": "png", "path": v.p("logo.png")}))
        .await;
    assert_file(&v.path("logo.png"), &[0x89, b'P', b'N', b'G']);
    // The receiving app reads the file from its own root, so stage it the way craft_handoff does.
    std::fs::copy(v.path("logo.png"), d.path("logo.png")).unwrap();
    d.ok("new_presentation", json!({})).await;
    d.ok("insert_picture", json!({"path": d.p("logo.png")}))
        .await;
    let i = d.ok("inspect_document", json!({})).await;
    assert!(
        i.text.to_lowercase().contains("picture") || i.text.to_lowercase().contains("image"),
        "picture not on slide: {}",
        i.text
    );
    v.done().await;
    d.done().await;
}

// ---------------------------------------------------------------------------------------------
// L7: failure injection
// ---------------------------------------------------------------------------------------------

#[tokio::test]
#[ignore = "real app"]
async fn l7_missing_binary_is_a_connect_error_not_a_hang() {
    let hub = McpHub::new();
    let entry = ServerEntry {
        command: Some(
            std::env::temp_dir()
                .join("definitely-not-installed-cli.exe")
                .to_string_lossy()
                .into_owned(),
        ),
        args: vec!["mcp".into()],
        ..ServerEntry::default()
    };
    let t = std::time::Instant::now();
    let r = hub.connect_one("ghost", &entry).await;
    assert!(r.is_err(), "connecting a missing binary must fail");
    assert!(t.elapsed().as_secs() < 10, "must fail fast");
    assert!(!hub.is_connected("ghost").await);
}

#[tokio::test]
#[ignore = "real app"]
async fn l7_closed_live_port_is_not_listening() {
    // Bind then drop to get a port that is certainly closed, then ask the registry helper.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    assert!(!workcraft_agents::craft::live::is_port_listening(port));
}

#[tokio::test]
#[ignore = "real app"]
async fn l7_path_outside_root_is_refused() {
    let Some(f) = Fx::new("photocraft").await else {
        return;
    };
    // A file that exists but lives outside the workspace root (the OS temp dir itself).
    let outside =
        std::env::temp_dir().join(format!("wc-outside-{}.png", uuid::Uuid::new_v4().simple()));
    make_image(&outside);
    let r = f
        .call("doc_open", json!({"path": outside.to_string_lossy()}))
        .await;
    let _ = std::fs::remove_file(&outside);
    assert!(
        !r.ok,
        "opening a file outside the read root must be refused, got: {}",
        r.text
    );
    f.done().await;
}

#[tokio::test]
#[ignore = "real app"]
async fn l7_killed_child_reconnects_once() {
    let Some(f) = Fx::new("cadcraft").await else {
        return;
    };
    f.ok("new_drawing", json!({})).await;
    let pid = f
        .hub
        .child_pid("cadcraft")
        .await
        .expect("stdio child must have a pid");
    // Kill exactly the child this test started.
    let status = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .output()
        .expect("taskkill");
    assert!(status.status.success(), "taskkill failed");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let t = std::time::Instant::now();
    let r = f.call("new_drawing", json!({})).await;
    assert!(
        r.ok,
        "call after child death should transparently reconnect: {}",
        r.text
    );
    assert!(t.elapsed().as_secs() < 20, "reconnect must be quick");
    let pid2 = f.hub.child_pid("cadcraft").await.unwrap();
    assert_ne!(pid, pid2, "a new child process must have been started");
    f.done().await;
}
