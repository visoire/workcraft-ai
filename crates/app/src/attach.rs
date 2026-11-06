//! Files attached to a message: images (sent to vision models), text and Office documents (their text
//! goes into the message), and files too big for that (sent as a path the assistant reads in parts).
//!
//! Files load on a background thread, so a 50 MB PDF does not freeze the window.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use workcraft_agents::docs;
use workcraft_core::Attachment;

/// Images bigger than this are refused; anything up to it is scaled to [`IMAGE_SIDE`].
const IMAGE_MAX_BYTES: u64 = 40 * 1024 * 1024;
const IMAGE_SIDE: u32 = 2048;
/// Text longer than this goes as a path instead of inside the message.
pub const INLINE_CHARS: usize = 120_000;
/// Files bigger than this are not read at all, only referenced.
const READ_MAX_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    /// Text goes into the message.
    Inline,
    /// Too long: the message carries the path.
    Reference,
    Failed,
}

pub struct Attached {
    pub id: u64,
    pub name: String,
    pub path: Option<PathBuf>,
    pub size: u64,
    pub ext: String,
    pub kind: Kind,
    pub image: Option<Attachment>,
    pub thumb: Option<TextureHandle>,
    pub text: Option<String>,
    /// Why it failed, or what was done ("scaled to 2048 px").
    pub note: String,
    pub pending: bool,
}

#[must_use]
pub fn human_size(n: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

fn ext_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn is_image_ext(e: &str) -> bool {
    matches!(e, "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
}

/// The short label on the file tile.
#[must_use]
pub fn type_label(ext: &str) -> String {
    match ext {
        "docx" | "docm" | "dotx" | "doc" => "DOC".into(),
        "xlsx" | "xlsm" | "xls" | "xlsb" => "XLS".into(),
        "pptx" | "pptm" | "ppt" => "PPT".into(),
        "dir" => "DIR".into(),
        "" => "FILE".into(),
        e => e.chars().take(4).collect::<String>().to_uppercase(),
    }
}

/// A placeholder shown while the file loads.
#[must_use]
pub fn pending(id: u64, name: &str, path: Option<PathBuf>) -> Attached {
    Attached {
        id,
        name: name.to_string(),
        path,
        size: 0,
        ext: ext_of(name),
        kind: Kind::Inline,
        image: None,
        thumb: None,
        text: None,
        note: "Reading...".into(),
        pending: true,
    }
}

fn failed(
    id: u64,
    name: &str,
    path: Option<PathBuf>,
    size: u64,
    why: impl Into<String>,
) -> Attached {
    Attached {
        id,
        name: name.to_string(),
        path,
        size,
        ext: ext_of(name),
        kind: Kind::Failed,
        image: None,
        thumb: None,
        text: None,
        note: why.into(),
        pending: false,
    }
}

/// Decode, scale down to [`IMAGE_SIDE`] if needed, and return the model attachment and a thumbnail.
fn load_image(
    ctx: &egui::Context,
    id: u64,
    name: &str,
    bytes: &[u8],
) -> Result<(Attachment, TextureHandle, String), String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("not a readable image ({e})"))?;
    let (w, h) = (img.width(), img.height());
    let mut note = format!("{w} x {h}");
    let (data, mime) = if w.max(h) > IMAGE_SIDE {
        let small = img.resize(
            IMAGE_SIDE,
            IMAGE_SIDE,
            image::imageops::FilterType::Lanczos3,
        );
        let mut out = Cursor::new(Vec::new());
        small
            .to_rgb8()
            .write_to(&mut out, image::ImageFormat::Jpeg)
            .map_err(|e| e.to_string())?;
        note = format!("scaled from {w} x {h}");
        (out.into_inner(), "image/jpeg".to_string())
    } else {
        let mime = match ext_of(name).as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "png" => "image/png",
            _ => "",
        };
        if mime.is_empty() {
            // BMP and the like: send as PNG, which every vision model takes.
            let mut out = Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png)
                .map_err(|e| e.to_string())?;
            (out.into_inner(), "image/png".to_string())
        } else {
            (bytes.to_vec(), mime.to_string())
        }
    };
    let t = img.thumbnail(96, 96).to_rgba8();
    let ci =
        ColorImage::from_rgba_unmultiplied([t.width() as usize, t.height() as usize], t.as_raw());
    let tex = ctx.load_texture(format!("attach-{id}"), ci, TextureOptions::LINEAR);
    Ok((
        Attachment {
            name: name.to_string(),
            mime,
            data: base64::engine::general_purpose::STANDARD.encode(&data),
        },
        tex,
        note,
    ))
}

/// Load a file from disk (runs on a worker thread).
#[must_use]
pub fn from_path(ctx: &egui::Context, id: u64, path: &Path) -> Attached {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let size = std::fs::metadata(path).map_or(0, |m| m.len());
    let e = ext_of(&name);
    let p = Some(path.to_path_buf());
    if path.is_dir() {
        return Attached {
            ext: "dir".into(),
            kind: Kind::Reference,
            note: "folder".into(),
            ..failed(id, &name, p, 0, "")
        };
    }
    if is_image_ext(&e) {
        if size > IMAGE_MAX_BYTES {
            return failed(
                id,
                &name,
                p,
                size,
                format!("image larger than {}", human_size(IMAGE_MAX_BYTES)),
            );
        }
        return match std::fs::read(path) {
            Ok(b) => match load_image(ctx, id, &name, &b) {
                Ok((att, tex, note)) => Attached {
                    id,
                    name,
                    path: p,
                    size,
                    ext: e,
                    kind: Kind::Image,
                    image: Some(att),
                    thumb: Some(tex),
                    text: None,
                    note,
                    pending: false,
                },
                Err(why) => failed(id, &name, p, size, why),
            },
            Err(err) => failed(id, &name, p, size, err.to_string()),
        };
    }
    if size > READ_MAX_BYTES {
        return reference(id, &name, p, size, &e, "large file");
    }
    match docs::extract_text(path) {
        Ok(text) => {
            let chars = text.chars().count();
            if chars > INLINE_CHARS {
                reference(
                    id,
                    &name,
                    p,
                    size,
                    &e,
                    &format!("{} characters, read in parts", group(chars)),
                )
            } else {
                Attached {
                    id,
                    name,
                    path: p,
                    size,
                    ext: e,
                    kind: Kind::Inline,
                    image: None,
                    thumb: None,
                    note: format!("{} characters", group(chars)),
                    text: Some(text),
                    pending: false,
                }
            }
        }
        // A binary file WorkCraft cannot read itself (a .psd, a .zip): the assistant still gets the
        // path, so a Craft app can open it.
        Err(err) => {
            if docs::is_document(path) {
                failed(id, &name, p, size, short_err(&err.to_string()))
            } else {
                reference(id, &name, p, size, &e, "sent as a path")
            }
        }
    }
}

/// Load a dropped file that has bytes but no path (rare on desktop).
#[must_use]
pub fn from_bytes(ctx: &egui::Context, id: u64, name: &str, bytes: &[u8]) -> Attached {
    let e = ext_of(name);
    let size = bytes.len() as u64;
    if is_image_ext(&e) {
        return match load_image(ctx, id, name, bytes) {
            Ok((att, tex, note)) => Attached {
                id,
                name: name.to_string(),
                path: None,
                size,
                ext: e,
                kind: Kind::Image,
                image: Some(att),
                thumb: Some(tex),
                text: None,
                note,
                pending: false,
            },
            Err(why) => failed(id, name, None, size, why),
        };
    }
    // Write it to a temp file so the document readers can work on it.
    let tmp = std::env::temp_dir().join(format!("workcraft-drop-{id}-{name}"));
    if std::fs::write(&tmp, bytes).is_err() {
        return failed(id, name, None, size, "could not be read");
    }
    let mut a = from_path(ctx, id, &tmp);
    a.name = name.to_string();
    a
}

fn reference(
    id: u64,
    name: &str,
    path: Option<PathBuf>,
    size: u64,
    ext: &str,
    note: &str,
) -> Attached {
    Attached {
        id,
        name: name.to_string(),
        path,
        size,
        ext: ext.to_string(),
        kind: Kind::Reference,
        image: None,
        thumb: None,
        text: None,
        note: note.to_string(),
        pending: false,
    }
}

fn short_err(e: &str) -> String {
    let e = e.split(": ").last().unwrap_or(e);
    e.chars().take(90).collect()
}

fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

impl Attached {
    /// The part of the message text this attachment adds (images travel separately).
    #[must_use]
    pub fn message_block(&self) -> Option<String> {
        let path = self
            .path
            .as_ref()
            .map(|p| format!(" path=\"{}\"", attr(&p.display().to_string())))
            .unwrap_or_default();
        let head = format!(
            "<attachment name=\"{}\" type=\"{}\" size=\"{}\"{path}",
            attr(&self.name),
            type_label(&self.ext),
            human_size(self.size)
        );
        match self.kind {
            Kind::Inline => Some(format!(
                "{head}>\n{}\n</attachment>",
                self.text.as_deref().unwrap_or("").trim_end()
            )),
            Kind::Reference if self.ext == "dir" => Some(format!(
                "{head} note=\"folder attached; inspect its files with list_dir or search_files\"/>"
            )),
            Kind::Reference => Some(format!(
                "{head} note=\"not included in the message; read it with read_file (in parts) or open it with a Craft app\"/>"
            )),
            Kind::Image | Kind::Failed => None,
        }
    }
}

/// An attachment as the chat shows it after sending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub name: String,
    pub kind: String,
    pub size: String,
    pub path: Option<String>,
}

fn attr_value(tag: &str, key: &str) -> Option<String> {
    let k = format!("{key}=\"");
    let start = tag.find(&k)? + k.len();
    let end = tag[start..].find('"')? + start;
    Some(
        tag[start..end]
            .replace("&quot;", "\"")
            .replace("&lt;", "<")
            .replace("&amp;", "&"),
    )
}

/// Split a sent message into its own text and the attachment blocks WorkCraft added.
#[must_use]
pub fn split_message(content: &str) -> (String, Vec<Shown>) {
    let mut text = String::new();
    let mut shown = Vec::new();
    let mut rest = content;
    while let Some(i) = rest.find("<attachment ") {
        text.push_str(&rest[..i]);
        let after = &rest[i..];
        let Some(tag_end) = after.find('>') else {
            text.push_str(after);
            rest = "";
            break;
        };
        let tag = &after[..=tag_end];
        let kind = attr_value(tag, "type").unwrap_or_default();
        if kind != "NOTE" {
            shown.push(Shown {
                name: attr_value(tag, "name").unwrap_or_default(),
                kind,
                size: attr_value(tag, "size").unwrap_or_default(),
                path: attr_value(tag, "path"),
            });
        }
        if tag.ends_with("/>") {
            rest = &after[tag_end + 1..];
        } else if let Some(close) = after.find("</attachment>") {
            rest = &after[close + "</attachment>".len()..];
        } else {
            rest = "";
        }
    }
    text.push_str(rest);
    (text.trim().to_string(), shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_sizes_and_groups() {
        assert_eq!(type_label("docx"), "DOC");
        assert_eq!(type_label("pdf"), "PDF");
        assert_eq!(type_label(""), "FILE");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(group(1234567), "1,234,567");
    }

    #[test]
    fn sent_messages_split_back_into_text_and_chips() {
        let ctx = egui::Context::default();
        let dir = std::env::temp_dir().join(format!("wc-attach-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("notes & v2.md");
        std::fs::write(&f, "# Notes\nhello").unwrap();
        let a = from_path(&ctx, 1, &f);
        assert_eq!(a.kind, Kind::Inline, "{}", a.note);
        let msg = format!("Summarise this\n\n{}", a.message_block().unwrap());
        assert!(msg.contains("hello"));
        let (text, chips) = split_message(&msg);
        assert_eq!(text, "Summarise this");
        assert_eq!(chips[0].name, "notes & v2.md");
        assert_eq!(chips[0].kind, "MD");
        assert!(
            chips[0]
                .path
                .as_deref()
                .is_some_and(|p| p.ends_with("notes & v2.md"))
        );
        // A reference is self-closing and also splits out (including quotes in attributes).
        let r = reference(
            2,
            "big \"v2\".pdf",
            Some(PathBuf::from("C:/x/big.pdf")),
            9_000_000,
            "pdf",
            "large",
        )
        .message_block()
        .unwrap();
        let (t2, c2) = split_message(&format!("Read {r} please"));
        assert_eq!(
            (t2.as_str(), c2[0].name.as_str()),
            ("Read  please", "big \"v2\".pdf")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
