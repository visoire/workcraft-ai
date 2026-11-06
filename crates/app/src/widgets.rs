//! Small drawn pieces the screens share: line icons, icon buttons, pills, avatars, a segmented switch.
//! Icons are painted with strokes so they stay sharp at any scale and follow the theme colour.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, RichText, Sense, Shape,
    Stroke, StrokeKind, TextureHandle, Vec2, pos2, vec2,
};

use crate::theme::{pal, semi, soft};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Plus,
    Send,
    Stop,
    Copy,
    Check,
    Edit,
    Trash,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    Sidebar,
    Panel,
    Settings,
    Search,
    Close,
    Sun,
    Moon,
    Monitor,
    Sparkle,
    Shield,
    Bolt,
    Plug,
    Terminal,
    Users,
    Refresh,
    Person,
    Key,
    Eye,
    Folder,
    Undo,
    Download,
    Globe,
    File,
    Image,
    Mic,
    MicOff,
}

/// The outline of a crescent: the part of circle 1 outside circle 2. Falls back to circle 1 when the
/// circles do not cross.
fn crescent(c1: Pos2, r1: f32, c2: Pos2, r2: f32) -> Vec<Pos2> {
    use std::f32::consts::{PI, TAU};
    let d = c1.distance(c2);
    if d >= r1 + r2 || d <= (r1 - r2).abs() || d == 0.0 {
        return arc(c1, r1, 0.0, TAU, 40);
    }
    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h = (r1 * r1 - a * a).max(0.0).sqrt();
    let dir = (c2 - c1) / d;
    let mid = c1 + dir * a;
    let perp = vec2(-dir.y, dir.x);
    let (i1, i2) = (mid + perp * h, mid - perp * h);
    let ang = |c: Pos2, q: Pos2| (q.y - c.y).atan2(q.x - c.x);
    // Sweep from `from` to `to` on circle (c, r), the way whose middle points along `want`.
    let sweep = |c: Pos2, r: f32, from: Pos2, to: Pos2, want: f32| {
        let (s, e) = (ang(c, from), ang(c, to));
        let mut delta = (e - s).rem_euclid(TAU);
        let mid_a = s + delta / 2.0;
        let off = ((mid_a - want + PI).rem_euclid(TAU) - PI).abs();
        if off > PI / 2.0 {
            delta -= TAU;
        }
        arc(c, r, s, s + delta, 28)
    };
    let away = ang(c2, c1); // pointing from c2 through c1: the side of circle 1 away from circle 2
    let mut pts = sweep(c1, r1, i1, i2, away);
    // Back along the inner circle, inside circle 1.
    let mut inner = sweep(c2, r2, i2, i1, ang(c2, c1));
    inner.remove(0);
    inner.pop();
    pts.extend(inner);
    pts
}

fn arc(c: Pos2, r: f32, from: f32, to: f32, n: usize) -> Vec<Pos2> {
    (0..=n)
        .map(|i| {
            let a = from + (to - from) * i as f32 / n as f32;
            pos2(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect()
}

/// Paint `icon` centred in `rect` (drawn on a 16 unit grid, scaled to the rect).
pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    use std::f32::consts::{PI, TAU};
    let s = rect.width().min(rect.height()) / 16.0;
    let o = rect.center() - vec2(8.0 * s, 8.0 * s);
    let p = |x: f32, y: f32| pos2(o.x + x * s, o.y + y * s);
    let st = Stroke::new(1.5 * s.max(0.8), color);
    let line = |pts: Vec<Pos2>| {
        painter.add(Shape::line(pts, st));
    };
    match icon {
        Icon::Plus => {
            line(vec![p(8.0, 3.0), p(8.0, 13.0)]);
            line(vec![p(3.0, 8.0), p(13.0, 8.0)]);
        }
        Icon::Send => {
            line(vec![p(8.0, 13.0), p(8.0, 3.5)]);
            line(vec![p(4.0, 7.5), p(8.0, 3.5), p(12.0, 7.5)]);
        }
        Icon::Stop => {
            painter.rect_filled(
                Rect::from_min_max(p(4.5, 4.5), p(11.5, 11.5)),
                CornerRadius::same((1.5 * s) as u8),
                color,
            );
        }
        Icon::Copy => {
            let r = CornerRadius::same((2.0 * s) as u8);
            painter.rect_stroke(
                Rect::from_min_max(p(5.5, 5.5), p(13.0, 13.0)),
                r,
                st,
                StrokeKind::Middle,
            );
            line(vec![p(10.5, 3.0), p(4.5, 3.0), p(3.0, 4.5), p(3.0, 10.5)]);
        }
        Icon::Check => line(vec![p(3.5, 8.5), p(6.5, 11.5), p(12.5, 4.5)]),
        Icon::Edit => {
            line(vec![
                p(3.5, 12.5),
                p(4.0, 10.0),
                p(10.5, 3.5),
                p(12.5, 5.5),
                p(6.0, 12.0),
                p(3.5, 12.5),
            ]);
            line(vec![p(9.0, 5.0), p(11.0, 7.0)]);
        }
        Icon::Trash => {
            line(vec![p(2.5, 4.5), p(13.5, 4.5)]);
            line(vec![p(6.0, 4.5), p(6.5, 2.5), p(9.5, 2.5), p(10.0, 4.5)]);
            line(vec![p(4.0, 4.5), p(4.8, 13.5), p(11.2, 13.5), p(12.0, 4.5)]);
        }
        Icon::ChevronDown => line(vec![p(4.5, 6.5), p(8.0, 10.0), p(11.5, 6.5)]),
        Icon::ChevronLeft => line(vec![p(9.5, 4.0), p(5.5, 8.0), p(9.5, 12.0)]),
        Icon::ChevronRight => line(vec![p(6.5, 4.0), p(10.5, 8.0), p(6.5, 12.0)]),
        Icon::Sidebar | Icon::Panel => {
            painter.rect_stroke(
                Rect::from_min_max(p(2.0, 3.0), p(14.0, 13.0)),
                CornerRadius::same((2.5 * s) as u8),
                st,
                StrokeKind::Middle,
            );
            let x = if icon == Icon::Sidebar { 6.0 } else { 10.0 };
            line(vec![p(x, 3.0), p(x, 13.0)]);
        }
        Icon::Settings => {
            for (y, k) in [(4.0, 10.5), (8.0, 5.5), (12.0, 9.0)] {
                line(vec![p(2.5, y), p(13.5, y)]);
                painter.circle_filled(p(k, y), 1.9 * s, color);
            }
        }
        Icon::Search => {
            painter.circle_stroke(p(7.0, 7.0), 4.2 * s, st);
            line(vec![p(10.2, 10.2), p(13.5, 13.5)]);
        }
        Icon::Close => {
            line(vec![p(4.0, 4.0), p(12.0, 12.0)]);
            line(vec![p(12.0, 4.0), p(4.0, 12.0)]);
        }
        Icon::Sun => {
            painter.circle_stroke(p(8.0, 8.0), 2.8 * s, st);
            for i in 0..8 {
                let a = i as f32 * TAU / 8.0;
                let (c, si) = (a.cos(), a.sin());
                line(vec![
                    p(8.0 + 4.8 * c, 8.0 + 4.8 * si),
                    p(8.0 + 6.3 * c, 8.0 + 6.3 * si),
                ]);
            }
        }
        Icon::Moon => {
            painter.add(Shape::closed_line(
                crescent(p(7.6, 8.4), 5.6 * s, p(10.6, 5.4), 4.6 * s),
                st,
            ));
        }
        Icon::Person => {
            painter.circle_stroke(p(8.0, 5.4), 2.7 * s, st);
            line(arc(p(8.0, 15.2), 5.4 * s, PI * 1.08, PI * 1.92, 16));
        }
        Icon::Monitor => {
            painter.rect_stroke(
                Rect::from_min_max(p(2.0, 3.0), p(14.0, 11.0)),
                CornerRadius::same((1.5 * s) as u8),
                st,
                StrokeKind::Middle,
            );
            line(vec![p(5.5, 13.5), p(10.5, 13.5)]);
            line(vec![p(8.0, 11.0), p(8.0, 13.5)]);
        }
        Icon::Sparkle => {
            let pts = vec![
                p(8.0, 1.8),
                p(9.4, 6.6),
                p(14.2, 8.0),
                p(9.4, 9.4),
                p(8.0, 14.2),
                p(6.6, 9.4),
                p(1.8, 8.0),
                p(6.6, 6.6),
            ];
            painter.add(Shape::closed_line(pts, st));
        }
        Icon::Shield => {
            let pts = vec![
                p(8.0, 1.8),
                p(13.0, 3.8),
                p(12.6, 8.6),
                p(8.0, 14.2),
                p(3.4, 8.6),
                p(3.0, 3.8),
            ];
            painter.add(Shape::closed_line(pts, st));
            line(vec![p(5.8, 8.0), p(7.5, 9.7), p(10.4, 6.4)]);
        }
        Icon::Bolt => {
            let pts = vec![
                p(9.5, 1.8),
                p(3.5, 9.0),
                p(7.8, 9.0),
                p(6.5, 14.2),
                p(12.5, 7.0),
                p(8.2, 7.0),
            ];
            painter.add(Shape::closed_line(pts, st));
        }
        Icon::Plug => {
            line(vec![p(6.0, 2.0), p(6.0, 5.0)]);
            line(vec![p(10.0, 2.0), p(10.0, 5.0)]);
            line(vec![
                p(3.5, 5.0),
                p(12.5, 5.0),
                p(12.5, 7.5),
                p(10.0, 10.5),
                p(6.0, 10.5),
                p(3.5, 7.5),
                p(3.5, 5.0),
            ]);
            line(vec![p(8.0, 10.5), p(8.0, 14.0)]);
        }
        Icon::Terminal => {
            line(vec![p(3.0, 4.5), p(6.5, 8.0), p(3.0, 11.5)]);
            line(vec![p(8.5, 12.0), p(13.0, 12.0)]);
        }
        Icon::Users => {
            painter.circle_stroke(p(6.0, 5.5), 2.4 * s, st);
            line(arc(p(6.0, 14.0), 4.2 * s, PI * 1.05, PI * 1.95, 14));
            painter.circle_stroke(p(11.5, 6.0), 1.9 * s, st);
            line(arc(p(11.8, 13.6), 3.2 * s, PI * 1.2, PI * 1.95, 10));
        }
        Icon::Refresh => {
            line(arc(p(8.0, 8.0), 5.0 * s, -PI * 0.2, PI * 1.45, 24));
            line(vec![p(12.6, 2.6), p(12.2, 5.6), p(9.2, 5.2)]);
        }
        Icon::Key => {
            painter.circle_stroke(p(5.2, 10.8), 2.9 * s, st);
            line(vec![p(7.3, 8.7), p(13.2, 2.8)]);
            line(vec![p(11.0, 5.0), p(12.8, 6.8)]);
            line(vec![p(9.4, 6.6), p(10.8, 8.0)]);
        }
        Icon::Eye => {
            // Two arcs of the same circle meeting at the corners (1.5, 8) and (14.5, 8).
            let (half, r) = (6.5f32, 8.0f32);
            let d = (r * r - half * half).sqrt();
            let a = (half / r).asin(); // half the arc angle
            let mut lens = arc(p(8.0, 8.0 + d), r * s, -PI / 2.0 - a, -PI / 2.0 + a, 16);
            lens.extend(arc(p(8.0, 8.0 - d), r * s, PI / 2.0 - a, PI / 2.0 + a, 16));
            painter.add(Shape::closed_line(lens, st));
            painter.circle_stroke(p(8.0, 8.0), 2.1 * s, st);
        }
        Icon::Folder => {
            line(vec![
                p(2.0, 4.5),
                p(6.2, 4.5),
                p(7.5, 6.0),
                p(14.0, 6.0),
                p(14.0, 12.8),
                p(2.0, 12.8),
                p(2.0, 4.5),
            ]);
        }
        Icon::Undo => {
            line(vec![p(6.2, 3.3), p(2.6, 6.6), p(6.2, 9.9)]);
            let mut path = vec![p(3.0, 6.6), p(10.0, 6.6)];
            path.extend(arc(p(10.0, 9.7), 3.1 * s, -PI * 0.5, PI * 0.5, 14));
            path.push(p(5.2, 12.8));
            line(path);
        }
        Icon::Download => {
            line(vec![p(8.0, 2.5), p(8.0, 10.5)]);
            line(vec![p(5.0, 7.5), p(8.0, 10.5), p(11.0, 7.5)]);
            line(vec![p(3.0, 13.0), p(13.0, 13.0)]);
        }
        Icon::Globe => {
            painter.circle_stroke(p(8.0, 8.0), 5.5 * s, st);
            line(vec![p(2.6, 8.0), p(13.4, 8.0)]);
            line(vec![p(8.0, 2.5), p(8.0, 13.5)]);
        }
        Icon::File => {
            line(vec![
                p(4.0, 2.5),
                p(9.8, 2.5),
                p(12.5, 5.2),
                p(12.5, 13.5),
                p(4.0, 13.5),
                p(4.0, 2.5),
            ]);
            line(vec![p(9.5, 2.5), p(9.5, 5.5), p(12.5, 5.5)]);
        }
        Icon::Image => {
            painter.rect_stroke(
                Rect::from_min_max(p(2.5, 3.0), p(13.5, 13.0)),
                CornerRadius::same((1.5 * s) as u8),
                st,
                StrokeKind::Middle,
            );
            painter.circle_filled(p(5.6, 6.2), 1.2 * s, color);
            line(vec![
                p(3.0, 12.2),
                p(7.2, 8.2),
                p(9.8, 10.5),
                p(11.5, 9.0),
                p(13.2, 11.2),
            ]);
        }
        Icon::Mic => {
            painter.rect_stroke(
                Rect::from_min_max(p(6.0, 2.0), p(10.0, 9.0)),
                CornerRadius::same((2.0 * s).round().clamp(1.0, 255.0) as u8),
                st,
                StrokeKind::Middle,
            );
            line(arc(p(8.0, 7.4), 4.0 * s, 0.0, PI, 16));
            line(vec![p(8.0, 11.4), p(8.0, 13.8)]);
            line(vec![p(5.5, 13.8), p(10.5, 13.8)]);
        }
        Icon::MicOff => {
            painter.rect_stroke(
                Rect::from_min_max(p(6.0, 2.0), p(10.0, 9.0)),
                CornerRadius::same((2.0 * s).round().clamp(1.0, 255.0) as u8),
                st,
                StrokeKind::Middle,
            );
            line(arc(p(8.0, 7.4), 4.0 * s, 0.0, PI, 16));
            line(vec![p(8.0, 11.4), p(8.0, 13.8)]);
            line(vec![p(5.5, 13.8), p(10.5, 13.8)]);
            line(vec![p(2.6, 2.6), p(13.4, 13.4)]);
        }
    }
}

/// A light / dark switch: a track with a knob that carries a sun or a moon and slides across.
pub fn theme_switch(ui: &mut egui::Ui, dark: bool) -> Response {
    let p = pal(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(54.0, 28.0), Sense::click());
    let k = ui
        .ctx()
        .animate_bool_with_time(egui::Id::unique("theme-switch"), dark, 0.18);
    let r = CornerRadius::same(255);
    let track = if resp.hovered() { p.pressed } else { p.surface };
    ui.painter().rect_filled(rect, r, track);
    ui.painter()
        .rect_stroke(rect, r, Stroke::new(1.0, p.border), StrokeKind::Inside);
    // The idle icons sit on the track: sun on the left, moon on the right.
    let left = Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), Vec2::splat(13.0));
    let right = Rect::from_center_size(
        pos2(rect.right() - 14.0, rect.center().y),
        Vec2::splat(13.0),
    );
    paint_icon(ui.painter(), left, Icon::Sun, p.faint);
    paint_icon(ui.painter(), right, Icon::Moon, p.faint);
    let x = egui::lerp(rect.left() + 14.0..=rect.right() - 14.0, k);
    let knob = pos2(x, rect.center().y);
    ui.painter().circle_filled(
        knob + vec2(0.0, 1.0),
        11.5,
        Color32::from_black_alpha(if p.dark { 90 } else { 30 }),
    );
    ui.painter()
        .circle_filled(knob, 11.0, if p.dark { p.hover } else { p.raised });
    let ir = Rect::from_center_size(knob, Vec2::splat(13.0));
    if k > 0.5 {
        paint_icon(ui.painter(), ir, Icon::Moon, p.text);
    } else {
        paint_icon(ui.painter(), ir, Icon::Sun, p.text);
    }
    resp.on_hover_text(if dark {
        "Dark theme. Click for light."
    } else {
        "Light theme. Click for dark."
    })
}

/// A provider logo: one-colour logos follow the text colour, colour logos are drawn as they are, and a
/// provider without a logo gets its first letter on a tile.
pub fn provider_logo(
    ui: &mut egui::Ui,
    logo: Option<(&TextureHandle, bool)>,
    name: &str,
    size: f32,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_provider_logo(ui, rect, logo, name);
    resp
}

pub fn paint_provider_logo(
    ui: &egui::Ui,
    rect: Rect,
    logo: Option<(&TextureHandle, bool)>,
    name: &str,
) {
    let p = pal(ui);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    match logo {
        Some((t, true)) => {
            ui.painter().image(t.id(), rect, uv, p.text);
        }
        Some((t, false)) => {
            ui.painter().image(t.id(), rect, uv, Color32::WHITE);
        }
        None => {
            let r = CornerRadius::same((rect.width() * 0.28) as u8);
            ui.painter().rect_filled(rect, r, p.pressed);
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                name.chars()
                    .next()
                    .unwrap_or('?')
                    .to_uppercase()
                    .to_string(),
                FontId::proportional(rect.height() * 0.58),
                p.text,
            );
        }
    }
}

/// A square icon button with a soft hover background.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, tip: &str) -> Response {
    icon_button_sized(ui, icon, tip, 30.0, false)
}

pub fn icon_button_sized(
    ui: &mut egui::Ui,
    icon: Icon,
    tip: &str,
    size: f32,
    active: bool,
) -> Response {
    let p = pal(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if resp.is_pointer_button_down_on() {
            p.pressed
        } else if resp.hovered() || active {
            p.hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, CornerRadius::same(8), bg);
        let fg = if resp.hovered() || active {
            p.text
        } else {
            p.muted
        };
        paint_icon(ui.painter(), rect.shrink(size * 0.24), icon, fg);
    }
    if tip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tip)
    }
}

/// A rounded button with an optional leading icon. `primary` fills it with the accent.
pub fn button(ui: &mut egui::Ui, icon: Option<Icon>, text: &str, primary: bool) -> Response {
    let p = pal(ui);
    let font = FontId::proportional(13.5);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, Color32::PLACEHOLDER);
    let icon_w = if icon.is_some() { 20.0 } else { 0.0 };
    let size = vec2(galley.size().x + icon_w + 24.0, 32.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = resp.hovered();
        let (bg, fg, border) = if primary {
            let bg = if hovered { p.accent_text } else { p.accent };
            (bg, p.on_accent, Color32::TRANSPARENT)
        } else {
            (
                if hovered {
                    p.hover
                } else {
                    Color32::TRANSPARENT
                },
                p.text,
                p.border_strong,
            )
        };
        let r = CornerRadius::same(10);
        ui.painter().rect_filled(rect, r, bg);
        if border != Color32::TRANSPARENT {
            ui.painter()
                .rect_stroke(rect, r, Stroke::new(1.0, border), StrokeKind::Inside);
        }
        let mut x = rect.left() + 12.0;
        if let Some(i) = icon {
            paint_icon(
                ui.painter(),
                Rect::from_center_size(pos2(x + 7.0, rect.center().y), Vec2::splat(15.0)),
                i,
                fg,
            );
            x += icon_w;
        }
        ui.painter()
            .galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
    }
    resp
}

/// A full-width row button, as in a sidebar list: `selected` keeps the hover fill.
pub fn row(ui: &mut egui::Ui, selected: bool, height: f32) -> (Rect, Response) {
    let p = pal(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let bg = if selected {
        p.pressed
    } else if resp.hovered() {
        p.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), bg);
    (rect, resp)
}

/// A small rounded label, for statuses and badges.
pub fn pill(ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) -> Response {
    egui::Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(255))
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(11.5)
                    .color(fg)
                    .variation("wght", 550.0),
            )
        })
        .response
}

/// A filled dot, gently pulsing when `live`.
pub fn dot(ui: &egui::Ui, center: Pos2, color: Color32, live: bool) {
    if live {
        let t = ui.input(|i| i.time) as f32;
        let k = (t * 3.0).sin() * 0.5 + 0.5;
        ui.painter()
            .circle_filled(center, 3.5 + 3.0 * k, soft(color, (60.0 * (1.0 - k)) as u8));
        ui.ctx().request_repaint();
    }
    ui.painter().circle_filled(center, 3.5, color);
}

/// A round avatar: an image if there is one, else the first letter on the agent colour.
pub fn avatar(
    ui: &mut egui::Ui,
    size: f32,
    tex: Option<&TextureHandle>,
    letter: &str,
    color: Color32,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_avatar(ui, rect, tex, letter, color);
    resp
}

pub fn paint_avatar(
    ui: &egui::Ui,
    rect: Rect,
    tex: Option<&TextureHandle>,
    letter: &str,
    color: Color32,
) {
    // No rings or glows: the WorkCraft mark and the Craft app icons already carry their own shape,
    // so they are drawn plainly. An agent without an icon gets a quiet tile with its initial.
    let p = pal(ui);
    let painter = ui.painter();
    match tex {
        Some(t) => {
            painter.image(
                t.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            let r = CornerRadius::same((rect.width() * 0.26) as u8);
            painter.rect_filled(rect, r, p.surface);
            painter.rect_stroke(
                rect,
                r,
                Stroke::new(1.0, p.border_strong),
                StrokeKind::Inside,
            );
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                letter
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_uppercase()
                    .to_string(),
                FontId::proportional(rect.height() * 0.5),
                color,
            );
        }
    }
}

/// A square image (an app icon) at `size`.
pub fn image(ui: &mut egui::Ui, tex: &TextureHandle, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    ui.painter().image(
        tex.id(),
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    resp
}

/// Two or more options in one rounded track; returns the clicked index.
pub fn segmented(
    ui: &mut egui::Ui,
    options: &[(Option<Icon>, &str)],
    selected: usize,
    id: &str,
) -> Option<usize> {
    let p = pal(ui);
    let font = FontId::proportional(13.0);
    let widths: Vec<f32> = options
        .iter()
        .map(|(i, t)| {
            let g =
                ui.painter()
                    .layout_no_wrap((*t).to_string(), font.clone(), Color32::PLACEHOLDER);
            g.size().x + 24.0 + if i.is_some() { 20.0 } else { 0.0 }
        })
        .collect();
    let total = widths.iter().sum::<f32>() + 6.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 32.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(10), p.surface);
    // The thumb slides between options.
    let mut x = rect.left() + 3.0;
    let starts: Vec<f32> = widths
        .iter()
        .map(|w| {
            let s = x;
            x += w;
            s
        })
        .collect();
    let target = starts.get(selected).copied().unwrap_or(rect.left() + 3.0);
    let ax = ui
        .ctx()
        .animate_value_with_time(egui::Id::unique((id, "thumb")), target, 0.16);
    let aw = widths.get(selected).copied().unwrap_or(0.0);
    let thumb = Rect::from_min_size(pos2(ax, rect.top() + 3.0), vec2(aw, 26.0));
    ui.painter().rect_filled(
        thumb,
        CornerRadius::same(8),
        if p.dark { p.pressed } else { p.raised },
    );
    if !p.dark {
        ui.painter().rect_stroke(
            thumb,
            CornerRadius::same(8),
            Stroke::new(1.0, p.border),
            StrokeKind::Inside,
        );
    }
    let mut clicked = None;
    for (i, ((icon, text), (w, s))) in options.iter().zip(widths.iter().zip(starts)).enumerate() {
        let r = Rect::from_min_size(pos2(s, rect.top() + 3.0), vec2(*w, 26.0));
        let resp = ui.interact(r, egui::Id::unique((id, i)), Sense::click());
        if resp.clicked() && i != selected {
            clicked = Some(i);
        }
        let fg = if i == selected || resp.hovered() {
            p.text
        } else {
            p.muted
        };
        let mut tx = r.left() + 12.0;
        if let Some(ic) = icon {
            paint_icon(
                ui.painter(),
                Rect::from_center_size(pos2(tx + 7.0, r.center().y), Vec2::splat(14.0)),
                *ic,
                fg,
            );
            tx += 20.0;
        }
        ui.painter().text(
            pos2(tx, r.center().y),
            Align2::LEFT_CENTER,
            *text,
            font.clone(),
            fg,
        );
    }
    clicked
}

/// A section caption in small caps style.
pub fn caption(ui: &mut egui::Ui, text: &str) {
    let p = pal(ui);
    ui.label(semi(text).size(11.5).color(p.faint));
}

/// A toggle chip (icon + text, highlighted while `on`).
pub fn chip(ui: &mut egui::Ui, icon: Icon, text: &str, on: bool, tip: &str) -> Response {
    let p = pal(ui);
    let font = FontId::proportional(12.5);
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 38.0, 30.0), Sense::click());
    let (bg, fg, border) = if on {
        (p.accent_soft, p.accent_text, soft(p.accent, 0x60))
    } else if resp.hovered() {
        (p.hover, p.text, p.border_strong)
    } else {
        (Color32::TRANSPARENT, p.muted, p.border)
    };
    let r = CornerRadius::same(255);
    ui.painter().rect_filled(rect, r, bg);
    ui.painter()
        .rect_stroke(rect, r, Stroke::new(1.0, border), StrokeKind::Inside);
    paint_icon(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 17.0, rect.center().y), Vec2::splat(14.0)),
        icon,
        fg,
    );
    ui.painter().galley(
        pos2(rect.left() + 29.0, rect.center().y - g.size().y / 2.0),
        g,
        fg,
    );
    if tip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tip)
    }
}

/// Text cut to `max` characters on one line.
#[must_use]
pub fn short(s: &str, max: usize) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        one
    } else {
        format!("{}\u{2026}", one.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::short;

    #[test]
    fn crescent_is_circle_one_outside_circle_two() {
        use eframe::egui::pos2;
        let (c1, r1, c2, r2) = (pos2(7.6, 8.4), 5.6, pos2(10.6, 5.4), 4.6);
        let pts = super::crescent(c1, r1, c2, r2);
        assert!(pts.len() > 20);
        for q in &pts {
            // Every point is on or inside circle 1 and on or outside circle 2.
            assert!(q.distance(c1) <= r1 + 0.01, "{q:?} outside circle 1");
            assert!(q.distance(c2) >= r2 - 0.01, "{q:?} inside circle 2");
        }
        // The far side of circle 1 (away from circle 2) is part of the outline.
        let far = c1 + (c1 - c2).normalized() * r1;
        assert!(pts.iter().any(|q| q.distance(far) < 0.8));
    }

    #[test]
    fn short_cuts_and_flattens() {
        assert_eq!(short("a\n  b", 10), "a b");
        assert_eq!(short("abcdef", 3), "abc\u{2026}");
    }
}
