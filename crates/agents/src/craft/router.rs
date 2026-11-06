//! Which Craft app tools go into a request.
//!
//! Seventeen apps with 12 to 16 lean tools each are 200+ tools; with only the four document apps connected a
//! request already carries 90 tools. Measured with DeepSeek that was 36,000 prompt tokens before the user had
//! said anything, which does not fit the 16k window a local model has and is paid for on every round of a tool
//! loop. So a request carries the tools of the apps the question is about, and a way to ask for the rest.
//!
//! An app is **in** when
//! - the latest user message names it (`@photocraft`, "a spreadsheet", "export an svg", a file type);
//! - a tool of the app, or a `craft_*` verb with that `app`, was already called in this chat;
//! - an earlier `find_tools` call in this chat asked for it.
//!
//! The set is computed from the history alone, so it only grows within a chat and is the same from one round to
//! the next, which keeps the provider's prompt cache valid. The `craft_*` verbs and the built-in tools are
//! always there: they reach every installed app without its own tools being loaded.

use std::collections::BTreeSet;

use workcraft_core::{Message, ToolSpec};
use workcraft_mcp::hub::split_name;

use super::spec::{AppSpec, find_spec, specs};

/// The name of the tool that lists withheld tools.
pub const FIND_TOOLS: &str = "find_tools";

/// Words that point at an app besides its name, aliases and file types.
const EXTRA_WORDS: &[(&str, &[&str])] = &[
    (
        "wordcraft",
        &[
            "document",
            "docx",
            "word",
            "letter",
            "essay",
            "report",
            "memo",
            "odt",
            "rtf",
            "paragraph",
            "heading",
            "contract",
        ],
    ),
    (
        "gridcraft",
        &[
            "spreadsheet",
            "xlsx",
            "xls",
            "csv",
            "sheet",
            "workbook",
            "formula",
            "cell",
            "cells",
            "excel",
            "column",
            "row",
            "pivot",
            "budget",
        ],
    ),
    (
        "deckcraft",
        &[
            "presentation",
            "slides",
            "slide",
            "pptx",
            "deck",
            "powerpoint",
            "keynote",
        ],
    ),
    ("pdfcraft", &["pdf", "ocr", "redact", "scan"]),
    (
        "photocraft",
        &[
            "photo", "image", "picture", "png", "jpg", "jpeg", "blur", "crop", "filter", "layer",
            "retouch", "resize", "psd", "sharpen",
        ],
    ),
    (
        "vectorcraft",
        &[
            "vector",
            "svg",
            "logo",
            "illustration",
            "icon",
            "eps",
            "artboard",
        ],
    ),
    (
        "filmcraft",
        &[
            "video", "clip", "mp4", "mov", "timeline", "footage", "movie", "trim", "sequence",
            "premiere",
        ],
    ),
    (
        "lightcraft",
        &[
            "raw",
            "lightroom",
            "develop",
            "dng",
            "nef",
            "cr2",
            "arw",
            "exposure",
            "catalog",
        ],
    ),
    (
        "effectcraft",
        &[
            "motion",
            "vfx",
            "effects",
            "animation",
            "composite",
            "keyframe",
            "aftereffects",
            "lottie",
        ],
    ),
    (
        "designcraft",
        &[
            "layout",
            "indesign",
            "magazine",
            "brochure",
            "publishing",
            "typography",
            "flyer",
            "poster",
        ],
    ),
    (
        "figcraft",
        &[
            "interface",
            "wireframe",
            "mockup",
            "prototype",
            "figma",
            "ui",
        ],
    ),
    (
        "texturecraft",
        &[
            "texture", "painting", "mesh", "uv", "glb", "gltf", "bake", "3d", "pbr",
        ],
    ),
    (
        "materialcraft",
        &["material", "shader", "procedural", "substance", "tiling"],
    ),
    (
        "soundcraft",
        &[
            "audio", "sound", "wav", "mp3", "music", "daw", "podcast", "song", "flac",
        ],
    ),
    (
        "cadcraft",
        &[
            "cad",
            "dxf",
            "dwg",
            "drafting",
            "drawing",
            "blueprint",
            "floorplan",
        ],
    ),
    (
        "solidcraft",
        &[
            "solid",
            "stp",
            "stl",
            "3mf",
            "extrude",
            "fillet",
            "sketch",
            "enclosure",
            "bracket",
            "parametric",
            "brep",
        ],
    ),
    (
        "craft-anything",
        &[
            "reverse",
            "analyze",
            "rebuild",
            "binary",
            "apk",
            "decompile",
            "software",
        ],
    ),
];

/// The words of a text, and separately the extensions of anything that looks like a file name. A file type
/// ("report.docx") points at an app; the bare word "step" in "step by step" must not.
#[derive(Default)]
struct Text {
    plain: BTreeSet<String>,
    exts: BTreeSet<String>,
}

impl Text {
    fn is_empty(&self) -> bool {
        self.plain.is_empty() && self.exts.is_empty()
    }
}

fn words(text: &str) -> Text {
    let mut out = Text::default();
    for raw in text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                ',' | ';' | '(' | ')' | '"' | '\'' | '`' | '!' | '?' | ':'
            )
    }) {
        let raw = raw.trim_start_matches('@').trim_end_matches('.');
        if raw.is_empty() {
            continue;
        }
        let lower = raw.to_ascii_lowercase();
        if let Some((_, ext)) = lower.rsplit_once('.')
            && !ext.is_empty()
            && ext.len() <= 5
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
        {
            out.exts.insert(ext.to_string());
        }
        for w in lower.split(|c: char| !c.is_ascii_alphanumeric() && c != '-') {
            if !w.is_empty() {
                out.plain.insert(w.to_string());
                // "spreadsheets" and "slides" point where "spreadsheet" and "slide" do.
                if let Some(s) = w.strip_suffix('s')
                    && s.len() > 2
                {
                    out.plain.insert(s.to_string());
                }
            }
        }
    }
    out
}

fn app_words(spec: &AppSpec) -> BTreeSet<String> {
    let mut w: BTreeSet<String> = BTreeSet::new();
    w.insert(spec.id.to_string());
    w.insert(spec.name.to_ascii_lowercase().replace(' ', "-"));
    w.extend(spec.aliases.iter().map(|a| a.to_ascii_lowercase()));
    if let Some((_, extra)) = EXTRA_WORDS.iter().find(|(id, _)| *id == spec.id) {
        w.extend(extra.iter().map(|s| (*s).to_string()));
    }
    w
}

fn app_exts(spec: &AppSpec) -> BTreeSet<String> {
    spec.reads_formats
        .iter()
        .chain(spec.writes_formats)
        .map(|f| (*f).to_ascii_lowercase())
        .collect()
}

/// The apps the plain words of `text` point at, ignoring file types. A file type such as `.png` belongs to many
/// apps; this is for deciding which one specialist a request is about.
#[must_use]
pub fn apps_named_by_words(text: &str) -> BTreeSet<&'static str> {
    let t = words(text);
    specs()
        .iter()
        .filter(|sp| !app_words(sp).is_disjoint(&t.plain))
        .map(|sp| sp.id)
        .collect()
}

/// The apps `text` points at.
#[must_use]
pub fn apps_named_in(text: &str) -> BTreeSet<&'static str> {
    let t = words(text);
    if t.is_empty() {
        return BTreeSet::new();
    }
    specs()
        .iter()
        .filter(|sp| !app_words(sp).is_disjoint(&t.plain) || !app_exts(sp).is_disjoint(&t.exts))
        .map(|sp| sp.id)
        .collect()
}

/// The apps already in use in this chat: their tools or the `craft_*` verbs were called, or `find_tools` was
/// asked for them.
#[must_use]
pub fn apps_in_history(history: &[Message]) -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();
    for c in history.iter().flat_map(|m| &m.tool_calls) {
        if let Some((srv, _)) = split_name(&c.name)
            && let Some(sp) = find_spec(srv)
        {
            out.insert(sp.id);
        } else if c.name.starts_with("craft_")
            && let Some(sp) = c
                .arguments
                .get("app")
                .and_then(|a| a.as_str())
                .and_then(find_spec)
        {
            out.insert(sp.id);
        } else if c.name == FIND_TOOLS
            && let Some(q) = c.arguments.get("query").and_then(|q| q.as_str())
        {
            out.extend(apps_named_in(q));
        }
    }
    out
}

/// What the router decided.
#[derive(Debug, Default)]
pub struct Routed {
    pub tools: Vec<ToolSpec>,
    /// Apps whose tools are in the request.
    pub active: BTreeSet<&'static str>,
    /// Number of Craft tools left out.
    pub withheld: usize,
}

/// Keep every tool that is not a Craft app's, and the tools of the apps this question needs.
#[must_use]
pub fn route(all: Vec<ToolSpec>, history: &[Message], user_text: &str) -> Routed {
    let mut active: BTreeSet<&'static str> = apps_named_in(user_text);
    active.extend(apps_in_history(history));
    let mut r = Routed::default();
    for t in all {
        match split_name(&t.name).and_then(|(srv, _)| find_spec(srv)) {
            Some(sp) if !active.contains(sp.id) => r.withheld += 1,
            _ => r.tools.push(t),
        }
    }
    r.active = active;
    r
}

/// Tools of Craft apps that match `query`, as `(name, one-line description)`; at most `limit`.
/// An app named in the query lists its whole lean set first.
#[must_use]
pub fn search(all: &[ToolSpec], query: &str, limit: usize) -> Vec<(String, String)> {
    let named = apps_named_in(query);
    let q = words(query);
    let mut scored: Vec<(usize, &ToolSpec)> = all
        .iter()
        .filter_map(|t| {
            let (srv, sub) = split_name(&t.name)?;
            let sp = find_spec(srv)?;
            let hay = format!(
                "{} {}",
                sub.replace('_', " "),
                t.description.to_ascii_lowercase()
            );
            let hits = q
                .plain
                .iter()
                .filter(|w| w.len() > 2 && hay.contains(w.as_str()))
                .count();
            let app_bonus = if named.contains(sp.id) { 3 } else { 0 };
            (hits + app_bonus > 0).then_some((hits + app_bonus, t))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, t)| {
            let first = t.description.split(['.', '\n']).next().unwrap_or("").trim();
            (t.name.clone(), first.chars().take(110).collect())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use workcraft_core::ToolCall;

    fn tool(name: &str, desc: &str) -> ToolSpec {
        ToolSpec::new(name, desc, json!({"type": "object"}))
    }

    fn catalog() -> Vec<ToolSpec> {
        vec![
            tool("read_file", "Read a file"),
            tool("craft_open", "Open a document in a Craft app"),
            tool("wordcraft__type_text", "Type text into the document"),
            tool("gridcraft__set_cell", "Set the value of a cell"),
            tool("deckcraft__add_slide", "Add a slide"),
            tool("photocraft__doc_open", "Open an image"),
            tool("solidcraft__execute", "Run a CAD command"),
            tool("my_server__lookup", "A server the user added"),
        ]
    }

    fn names(r: &Routed) -> Vec<&str> {
        r.tools.iter().map(|t| t.name.as_str()).collect()
    }

    #[test]
    fn words_point_at_apps() {
        assert_eq!(
            apps_named_in("make a spreadsheet of the budget"),
            ["gridcraft"].into_iter().collect()
        );
        assert!(apps_named_in("export sums.xlsx").contains("gridcraft"));
        assert!(apps_named_in("blur in.png").contains("photocraft"));
        assert!(apps_named_in("a 40 mm bracket, export STEP").contains("solidcraft"));
        assert!(apps_named_in("@deckcraft add slides").contains("deckcraft"));
        assert!(apps_named_in("hello there").is_empty());
        // Plurals and file names in a sentence.
        assert!(apps_named_in("three presentations").contains("deckcraft"));
        assert!(apps_named_in("open report.docx.").contains("wordcraft"));
    }

    #[test]
    fn only_the_named_app_and_non_craft_tools_go_out() {
        let r = route(catalog(), &[], "type a letter");
        assert_eq!(
            names(&r),
            [
                "read_file",
                "craft_open",
                "wordcraft__type_text",
                "my_server__lookup"
            ]
        );
        assert_eq!(r.withheld, 4);
        let none = route(catalog(), &[], "hello");
        assert_eq!(
            names(&none),
            ["read_file", "craft_open", "my_server__lookup"]
        );
    }

    #[test]
    fn apps_used_or_found_earlier_stay_in() {
        let mut a = Message::assistant("");
        a.tool_calls.push(ToolCall {
            id: "1".into(),
            name: "photocraft__doc_open".into(),
            arguments: json!({}),
        });
        a.tool_calls.push(ToolCall {
            id: "2".into(),
            name: "craft_new".into(),
            arguments: json!({"app": "solidcraft"}),
        });
        a.tool_calls.push(ToolCall {
            id: "3".into(),
            name: FIND_TOOLS.into(),
            arguments: json!({"query": "set a cell in a sheet"}),
        });
        let r = route(catalog(), &[Message::user("go on"), a], "and now?");
        assert_eq!(
            r.active,
            ["gridcraft", "photocraft", "solidcraft"]
                .into_iter()
                .collect()
        );
        assert!(
            names(&r).contains(&"gridcraft__set_cell")
                && !names(&r).contains(&"deckcraft__add_slide")
        );
    }

    #[test]
    fn the_set_is_stable_from_round_to_round() {
        // Same history and question: same tools in the same order (the prompt cache depends on it).
        let a = route(catalog(), &[], "type a letter");
        let b = route(catalog(), &[], "type a letter");
        assert_eq!(names(&a), names(&b));
    }

    #[test]
    fn find_tools_lists_matches_with_the_named_app_first() {
        let hits = search(&catalog(), "slide in a presentation", 5);
        assert_eq!(hits[0].0, "deckcraft__add_slide");
        let none = search(&catalog(), "zzz", 5);
        assert!(none.is_empty());
        let cad = search(&catalog(), "cad command", 5);
        assert_eq!(cad[0].0, "solidcraft__execute");
    }
}
