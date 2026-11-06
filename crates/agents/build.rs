//! Embed the bundled skills (`skills/<name>/...`) so a fresh install has the Craft app skills without
//! any download. The generated file lists `(relative path, include_bytes!)` pairs.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn main() {
    let root =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default()).join("skills");
    println!("cargo:rerun-if-changed=skills");
    let mut files = Vec::new();
    walk(&root, &mut files);
    let mut code = String::from("pub const BUNDLED: &[(&str, &[u8])] = &[\n");
    for f in files {
        let Ok(rel) = f.strip_prefix(&root) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let abs = f.to_string_lossy().replace('\\', "/");
        let _ = writeln!(code, "    ({rel:?}, include_bytes!({abs:?})),");
    }
    code.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join("bundled_skills.rs");
    let _ = std::fs::write(out, code);
}
