//! Artifacts: the pictures and files the tools of this chat produced, in the side panel, with a full-size viewer.
//!
//! They are read from the tool results already in the chat (`[saved image: <path> (WxH)]` lines and `"path"` values
//! of exports and saves), so the list needs no extra bookkeeping and survives reopening a chat. Only files that
//! exist on disk are listed. The scan runs when the chat changes, never per frame.

use std::path::Path;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, Stroke,
    Vec2, pos2, vec2,
};
use workcraft_core::Role;

use crate::app::{Action, App};
use crate::theme::{pal, semi};
use crate::widgets::{self, Icon, paint_icon, short};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Artifact {
    pub path: String,
    pub name: String,
    /// A format the viewer can decode (PNG, JPEG, WebP, GIF, BMP).
    pub image: bool,
}

const IMAGE_EXTS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];
const FILE_EXTS: [&str; 24] = [
    "svg", "pdf", "docx", "xlsx", "pptx", "csv", "md", "txt", "step", "stp", "stl", "obj", "3mf",
    "glb", "dxf", "wav", "mp3", "mp4", "mov", "sc", "fcproj", "ecproj", "scraft", "html",
];

fn ext_of(path: &str) -> String {
    Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Paths a tool result mentions: saved pictures and `"path"` values.
fn paths_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("[saved image: ") {
        let after = &rest[i + "[saved image: ".len()..];
        let end = after
            .find(" (")
            .or_else(|| after.find(']'))
            .unwrap_or(after.len());
        out.push(after[..end].trim().to_string());
        rest = &after[end..];
    }
    let mut rest = text;
    while let Some(i) = rest.find("\"path\"") {
        let after = &rest[i + 6..];
        let Some(colon) = after.find(':') else { break };
        let after = after[colon + 1..].trim_start();
        let Some(body) = after.strip_prefix('"') else {
            rest = after;
            continue;
        };
        let mut s = String::new();
        let mut chars = body.chars();
        let mut used = 0usize;
        while let Some(c) = chars.next() {
            used += c.len_utf8();
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('\\') => {
                        s.push('\\');
                        used += 1;
                    }
                    Some('/') => {
                        s.push('/');
                        used += 1;
                    }
                    Some(o) => {
                        s.push('\\');
                        s.push(o);
                        used += o.len_utf8();
                    }
                    None => break,
                },
                c => s.push(c),
            }
        }
        out.push(s);
        rest = &body[used.min(body.len())..];
    }
    out
}

fn is_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    p.starts_with('/') || (b.len() > 2 && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
}

/// The artifacts of a chat, newest first, each once, only files that exist.
pub(crate) fn collect(messages: &[workcraft_core::Message]) -> Vec<Artifact> {
    let mut seen = std::collections::HashSet::new();
    let mut found: Vec<Artifact> = Vec::new();
    for m in messages.iter().filter(|m| m.role == Role::Tool) {
        for p in paths_in(&m.content) {
            let ext = ext_of(&p);
            let image = IMAGE_EXTS.contains(&ext.as_str());
            if !is_absolute(&p)
                || !(image || FILE_EXTS.contains(&ext.as_str()))
                || !seen.insert(p.clone())
            {
                continue;
            }
            if !Path::new(&p).is_file() {
                continue;
            }
            let name = Path::new(&p)
                .file_name()
                .map_or_else(|| p.clone(), |f| f.to_string_lossy().into_owned());
            found.push(Artifact {
                path: p,
                name,
                image,
            });
        }
    }
    found.reverse();
    found
}

impl App {
    /// Refresh the list when the chat changed (cheap check by session and message count).
    pub(crate) fn refresh_artifacts(&mut self) {
        let key = (self.current.clone(), self.messages.len());
        if self.artifacts.0 != key {
            self.artifacts = (key, collect(&self.messages));
        }
    }

    pub(crate) fn artifacts_tab(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let list = self.artifacts.1.clone();
        if list.is_empty() {
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Pictures and files the tools make in this chat show here: renders, previews, exports. Click one to look at it.",
                )
                .size(12.5)
                .color(p.faint),
            );
            return;
        }
        let ctx = ui.ctx().clone();
        let mut actions: Vec<Action> = Vec::new();
        let mut open: Option<String> = None;
        let images: Vec<&Artifact> = list.iter().filter(|a| a.image).collect();
        let files: Vec<&Artifact> = list.iter().filter(|a| !a.image).collect();
        if !images.is_empty() {
            widgets::caption(ui, "Pictures");
            let gap = 8.0;
            let w = ((ui.available_width() - gap) / 2.0).floor();
            for pair in images.chunks(2) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for a in pair {
                        let (slot, resp) =
                            ui.allocate_exact_size(vec2(w, w * 0.78 + 20.0), Sense::click());
                        let t = ui
                            .ctx()
                            .animate_bool_with_time(resp.id, resp.hovered(), 0.12);
                        let card = slot.translate(vec2(0.0, -2.0 * t));
                        ui.painter()
                            .rect_filled(card, CornerRadius::same(10), p.surface);
                        ui.painter().rect_stroke(
                            card,
                            CornerRadius::same(10),
                            Stroke::new(1.0, p.border.lerp_to_gamma(p.accent, t)),
                            egui::StrokeKind::Inside,
                        );
                        let pic = Rect::from_min_size(
                            card.min + vec2(4.0, 4.0),
                            vec2(w - 8.0, w * 0.78 - 8.0),
                        );
                        if let Some((tex, aspect)) = self.thumb_for_path(&ctx, &a.path) {
                            let mut size = pic.size();
                            if size.x / size.y > aspect {
                                size.x = size.y * aspect;
                            } else {
                                size.y = size.x / aspect;
                            }
                            ui.painter().image(
                                tex.id(),
                                Rect::from_center_size(pic.center(), size),
                                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        ui.painter().text(
                            pos2(card.left() + 8.0, card.bottom() - 10.0),
                            Align2::LEFT_CENTER,
                            short(&a.name, 22),
                            FontId::proportional(11.5),
                            p.muted,
                        );
                        if resp.hovered() {
                            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
                        }
                        if resp.on_hover_text(&a.path).clicked() {
                            open = Some(a.path.clone());
                        }
                    }
                });
                ui.add_space(gap);
            }
        }
        if !files.is_empty() {
            ui.add_space(4.0);
            widgets::caption(ui, "Files");
            for a in files {
                let (rect, resp) = widgets::row(ui, false, 34.0);
                let ir = Rect::from_min_size(
                    pos2(rect.left() + 8.0, rect.center().y - 8.0),
                    Vec2::splat(16.0),
                );
                paint_icon(ui.painter(), ir, Icon::File, p.muted);
                ui.painter().text(
                    pos2(ir.right() + 8.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    short(&a.name, 26),
                    FontId::proportional(13.0),
                    p.text,
                );
                ui.painter().text(
                    pos2(rect.right() - 8.0, rect.center().y),
                    Align2::RIGHT_CENTER,
                    ext_of(&a.path),
                    FontId::proportional(11.0),
                    p.faint,
                );
                if resp
                    .on_hover_text(format!("{}\nClick to open", a.path))
                    .clicked()
                {
                    actions.push(Action::OpenPath(a.path.clone()));
                }
            }
        }
        if let Some(path) = open {
            self.lightbox = Some(path);
        }
        if !actions.is_empty() {
            self.apply(actions, &ctx);
        }
    }

    /// The full-size viewer: a dark layer over the window with the picture, its name and three buttons.
    pub(crate) fn lightbox_view(&mut self, ctx: &egui::Context) {
        let Some(path) = self.lightbox.clone() else {
            return;
        };
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.lightbox = None;
            return;
        }
        // A larger texture than the thumbnails: the viewer is for looking.
        if self.lightbox_tex.as_ref().is_none_or(|(p, _)| *p != path) {
            self.lightbox_tex = image::open(&path).ok().map(|img| {
                let img = if img.width() > 2000 || img.height() > 2000 {
                    img.thumbnail(2000, 2000)
                } else {
                    img
                };
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let tex = ctx.load_texture(
                    format!("lightbox:{path}"),
                    egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
                    egui::TextureOptions::LINEAR,
                );
                (path.clone(), tex)
            });
        }
        let screen = ctx.content_rect();
        let mut close = false;
        let mut open_os = false;
        let mut copy = false;
        egui::Area::new(egui::Id::unique("lightbox"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, backdrop) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, Color32::from_black_alpha(224));
                let name = Path::new(&path)
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let mut over_ui = false;
                if let Some((_, tex)) = &self.lightbox_tex {
                    let avail =
                        vec2(rect.width() * 0.88, rect.height() - 150.0).max(vec2(80.0, 80.0));
                    let sz = tex.size_vec2();
                    let k = (avail.x / sz.x).min(avail.y / sz.y).min(1.6);
                    let img_r = Rect::from_center_size(
                        pos2(rect.center().x, rect.center().y + 6.0),
                        sz * k,
                    );
                    ui.painter().rect_filled(
                        img_r.expand(6.0),
                        CornerRadius::same(8),
                        Color32::from_gray(32),
                    );
                    ui.painter().image(
                        tex.id(),
                        img_r,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    over_ui |= ui.rect_contains_pointer(img_r.expand(6.0));
                } else {
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "This picture cannot be shown here. Use Open.",
                        FontId::proportional(15.0),
                        Color32::from_gray(200),
                    );
                }
                let bar = Rect::from_min_size(
                    pos2(rect.center().x - 240.0, rect.top() + 24.0),
                    vec2(480.0, 34.0),
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(bar)
                        .layout(Layout::left_to_right(Align::Center)),
                    |ui| {
                        ui.label(semi(short(&name, 44)).size(14.0).color(Color32::WHITE));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if widgets::icon_button_sized(
                                ui,
                                Icon::Close,
                                "Close (Esc)",
                                30.0,
                                false,
                            )
                            .clicked()
                            {
                                close = true;
                            }
                            if widgets::chip(
                                ui,
                                Icon::Copy,
                                "Copy path",
                                false,
                                "Copy the file path",
                            )
                            .clicked()
                            {
                                copy = true;
                            }
                            if widgets::chip(
                                ui,
                                Icon::Folder,
                                "Open",
                                false,
                                "Open with the system viewer",
                            )
                            .clicked()
                            {
                                open_os = true;
                            }
                        });
                    },
                );
                over_ui |= ui.rect_contains_pointer(bar.expand(4.0));
                if backdrop.clicked() && !over_ui {
                    close = true;
                }
            });
        if copy {
            ctx.copy_text(path.clone());
            self.notify("Copied path to clipboard.");
        }
        if open_os {
            self.apply(vec![Action::OpenPath(path)], ctx);
        }
        if close {
            self.lightbox = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_found_in_saved_image_lines_and_json() {
        let text = "[image: image/png] {\"w\":1}\n[saved image: C:\\w\\.workcraft\\artifacts\\a-0.png (640x480)]\n{\n  \"bytes\": 10,\n  \"path\": \"C:\\\\w\\\\logo.svg\"\n}";
        let p = paths_in(text);
        assert_eq!(
            p,
            ["C:\\w\\.workcraft\\artifacts\\a-0.png", "C:\\w\\logo.svg"]
        );
        assert!(paths_in("{\"path\":null}").is_empty());
    }

    #[test]
    fn only_absolute_known_existing_files_are_listed_once() {
        let dir = std::env::temp_dir().join(format!("wc-art-{}", workcraft_core::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("a.png");
        let svg = dir.join("b.svg");
        let exe = dir.join("c.exe");
        for f in [&png, &svg, &exe] {
            std::fs::write(f, b"x").unwrap();
        }
        let missing = dir.join("gone.png");
        let esc = |p: &Path| p.display().to_string().replace('\\', "\\\\");
        let mut m1 = workcraft_core::Message::tool("1", format!("{{\"path\": \"{}\"}}", esc(&svg)));
        let mut m2 = workcraft_core::Message::tool(
            "2",
            format!(
                "[saved image: {} (1x1)]\n{{\"path\": \"{}\"}} {{\"path\": \"{}\"}} {{\"path\": \"{}\"}} {{\"path\": \"relative.png\"}}",
                png.display(),
                svg.display().to_string().replace('\\', "\\\\"),
                exe.display().to_string().replace('\\', "\\\\"),
                missing.display().to_string().replace('\\', "\\\\")
            ),
        );
        m1.role = Role::Tool;
        m2.role = Role::Tool;
        let list = collect(&[m1, m2]);
        let names: Vec<&str> = list.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            ["a.png", "b.svg"],
            "newest first, no duplicates, no .exe, no missing, no relative path"
        );
        assert!(list[0].image && !list[1].image);
        let _ = std::fs::remove_dir_all(dir);
    }
}
