//! Colours, fonts and widget styling for the light and dark themes. Every screen reads its colours from
//! [`Palette`], so the two themes stay in step. The surfaces are neutral grays (in the spirit of Open WebUI
//! and LibreChat); the WorkCraft blue only marks primary actions and selection.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, RichText,
    Shadow, Stroke, TextStyle, Theme,
};

/// Inter (SIL Open Font License 1.1), the face Open WebUI and LibreChat use.
const INTER: &[u8] = include_bytes!("../assets/Inter-Variable.ttf");

/// The blue of the WorkCraft logo (the default accent).
#[cfg_attr(not(test), allow(dead_code))]
pub const BRAND: Color32 = Color32::from_rgb(0x2C, 0x84, 0xFC);

/// The accent colour in use. It follows the colour theme chosen for Blip (Settings, Blip), so the buttons,
/// selection and links match the mascot.
static ACCENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x002C_84FC);

/// The accent colour in use now.
#[must_use]
pub fn brand() -> Color32 {
    let v = ACCENT.load(std::sync::atomic::Ordering::Relaxed);
    rgb(v & 0x00FF_FFFF)
}

/// Make `c` the accent and restyle both themes so every widget picks it up.
pub fn set_accent(ctx: &egui::Context, c: Color32) {
    let v = (u32::from(c.r()) << 16) | (u32::from(c.g()) << 8) | u32::from(c.b());
    if ACCENT.swap(v, std::sync::atomic::Ordering::Relaxed) != v {
        ctx.style_mut_of(Theme::Dark, |s| style(s, &palette(true)));
        ctx.style_mut_of(Theme::Light, |s| style(s, &palette(false)));
        ctx.request_repaint();
    }
}

/// The accent as readable text or a thin line on the background.
fn accent_text(c: Color32, dark: bool) -> Color32 {
    if dark {
        c.lerp_to_gamma(Color32::WHITE, 0.38)
    } else {
        c.lerp_to_gamma(Color32::BLACK, 0.22)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemePref {
    System,
    Light,
    Dark,
}

impl ThemePref {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    /// The chat surface.
    pub bg: Color32,
    pub sidebar: Color32,
    /// Cards, the user bubble, the composer.
    pub surface: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    /// Menus, popups and windows.
    pub raised: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    /// Accent used for text and thin lines (readable on the background).
    pub accent_text: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub code_bg: Color32,
    pub code_head: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub danger_soft: Color32,
    pub warning_soft: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

#[must_use]
pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            dark,
            bg: rgb(0x212121),
            sidebar: rgb(0x171717),
            surface: rgb(0x2a2a2a),
            hover: rgb(0x303030),
            pressed: rgb(0x3a3a3a),
            raised: rgb(0x262626),
            border: rgb(0x343434),
            border_strong: rgb(0x474747),
            text: rgb(0xececec),
            muted: rgb(0xb4b4b4),
            faint: rgb(0x8a8a8a),
            accent: brand(),
            accent_text: accent_text(brand(), true),
            accent_soft: soft(brand(), 0x2e),
            on_accent: Color32::WHITE,
            code_bg: rgb(0x181818),
            code_head: rgb(0x262626),
            success: rgb(0x34c38f),
            warning: rgb(0xf5b041),
            danger: rgb(0xf26a6a),
            danger_soft: Color32::from_rgba_unmultiplied(0xf2, 0x6a, 0x6a, 0x26),
            warning_soft: Color32::from_rgba_unmultiplied(0xf5, 0xb0, 0x41, 0x1f),
        }
    } else {
        Palette {
            dark,
            bg: rgb(0xffffff),
            sidebar: rgb(0xf9f9f9),
            surface: rgb(0xf3f3f3),
            hover: rgb(0xececec),
            pressed: rgb(0xe3e3e3),
            raised: rgb(0xffffff),
            border: rgb(0xe5e5e5),
            border_strong: rgb(0xd0d0d0),
            text: rgb(0x0d0d0d),
            muted: rgb(0x5d5d5d),
            faint: rgb(0x8f8f8f),
            accent: brand(),
            accent_text: accent_text(brand(), false),
            accent_soft: soft(brand(), 0x1f),
            on_accent: Color32::WHITE,
            code_bg: rgb(0xf7f7f7),
            code_head: rgb(0xeeeeee),
            success: rgb(0x0e8a6b),
            warning: rgb(0xb45309),
            danger: rgb(0xd92d2d),
            danger_soft: Color32::from_rgba_unmultiplied(0xd9, 0x2d, 0x2d, 0x1a),
            warning_soft: Color32::from_rgba_unmultiplied(0xf5, 0x9e, 0x0b, 0x1f),
        }
    }
}

/// The palette of the theme `ui` is drawn in.
#[must_use]
pub fn pal(ui: &egui::Ui) -> Palette {
    palette(ui.visuals().dark_mode)
}

/// Semibold text (Inter is variable, so this is a real weight, not a fake bold).
pub fn semi(t: impl Into<RichText>) -> RichText {
    t.into().variation("wght", 600.0)
}

pub fn medium(t: impl Into<RichText>) -> RichText {
    t.into().variation("wght", 500.0)
}

/// A colour per agent: fixed for the built-in team, stable by name for any other agent.
#[must_use]
pub fn agent_color(id: &str, dark: bool) -> Color32 {
    // (dark theme, light theme)
    // Calm, distinct accents: the leader wears the WorkCraft blue, each specialist the colour of the
    // kind of app it drives. They mark agents with a thin strip or a dot, never a big fill.
    const FIXED: [(&str, u32, u32); 7] = [
        ("leader", 0x5ea4ff, 0x2c84fc),
        ("image", 0xf0a04b, 0xc2700a),
        ("design", 0xa87ffb, 0x7248d6),
        ("video", 0xef7b7b, 0xcf4545),
        ("document", 0x4cc79a, 0x0f8f63),
        ("three-d", 0x58c4d8, 0x14889c),
        ("explorer", 0xc9c26a, 0x8a8020),
    ];
    const OTHERS: [(u32, u32); 5] = [
        (0x9aa7ff, 0x5b67d8),
        (0x58c4d8, 0x14889c),
        (0xd9a6ff, 0x9353c9),
        (0xc9c26a, 0x8a8020),
        (0xb0b8c4, 0x5f6b7a),
    ];
    let pick = |(d, l): (u32, u32)| rgb(if dark { d } else { l });
    if id == "leader" {
        // The leader wears the accent.
        return if dark {
            accent_text(brand(), true)
        } else {
            brand()
        };
    }
    if let Some((_, d, l)) = FIXED.iter().find(|(n, ..)| *n == id) {
        return pick((*d, *l));
    }
    let h = id.bytes().fold(5381u32, |h, b| {
        h.wrapping_mul(33).wrapping_add(u32::from(b))
    });
    pick(OTHERS[h as usize % OTHERS.len()])
}

/// The same colour, see-through, for backgrounds behind it.
#[must_use]
pub fn soft(c: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
}

fn fonts() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    let mut inter = FontData::from_static(INTER);
    inter.tweak.coords = egui::epaint::text::VariationCoords::new([("wght", 400.0)]);
    f.font_data.insert("inter".into(), Arc::new(inter));
    f.families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "inter".into());
    f
}

fn style(s: &mut egui::Style, p: &Palette) {
    use FontFamily::{Monospace, Proportional};
    s.text_styles = [
        (TextStyle::Small, FontId::new(12.0, Proportional)),
        (TextStyle::Body, FontId::new(14.5, Proportional)),
        (TextStyle::Button, FontId::new(14.0, Proportional)),
        (TextStyle::Heading, FontId::new(20.0, Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, Monospace)),
    ]
    .into();
    s.spacing.item_spacing = egui::vec2(8.0, 6.0);
    s.spacing.button_padding = egui::vec2(10.0, 5.0);
    s.spacing.interact_size.y = 28.0;
    s.spacing.menu_margin = Margin::same(6);
    s.spacing.window_margin = Margin::same(18);
    s.spacing.scroll.floating = true;
    s.spacing.scroll.bar_width = 8.0;
    s.spacing.scroll.floating_width = 3.0;
    s.interaction.tooltip_delay = 0.35;

    let v = &mut s.visuals;
    v.dark_mode = p.dark;
    v.panel_fill = p.bg;
    v.window_fill = p.raised;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(12);
    let shade = if p.dark { 110 } else { 36 };
    v.window_shadow = Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(shade),
    };
    v.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(shade),
    };
    v.extreme_bg_color = p.surface;
    v.text_edit_bg_color = Some(p.surface);
    v.faint_bg_color = p.surface;
    v.code_bg_color = p.code_bg;
    v.hyperlink_color = p.accent_text;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.selection.bg_fill = soft(p.accent, if p.dark { 0x66 } else { 0x40 });
    v.selection.stroke = Stroke::new(1.0, p.text);
    v.collapsing_header_frame = false;
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.indent_has_left_vline = false;
    v.button_frame = true;
    v.interact_cursor = Some(egui::CursorIcon::PointingHand);

    let r = CornerRadius::same(8);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.bg;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = p.surface;
    w.inactive.weak_bg_fill = p.surface;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.border_strong);
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.pressed;
    w.active.weak_bg_fill = p.pressed;
    w.active.bg_stroke = Stroke::new(1.0, p.border_strong);
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;

    w.open = w.hovered;
}

/// Fonts and both themes. Call once at start; [`set_theme`] picks which one shows.
pub fn install(ctx: &egui::Context, pref: ThemePref) {
    ctx.set_fonts(fonts());
    ctx.style_mut_of(Theme::Dark, |s| style(s, &palette(true)));
    ctx.style_mut_of(Theme::Light, |s| style(s, &palette(false)));
    set_theme(ctx, pref);
}

pub fn set_theme(ctx: &egui::Context, pref: ThemePref) {
    ctx.set_theme(match pref {
        ThemePref::System => egui::ThemePreference::System,
        ThemePref::Light => egui::ThemePreference::Light,
        ThemePref::Dark => egui::ThemePreference::Dark,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_colors_are_stable_and_fixed_for_the_built_in_team() {
        assert_eq!(agent_color("leader", false), BRAND);
        assert_eq!(
            agent_color("custom-x", false),
            agent_color("custom-x", false)
        );
        assert_ne!(agent_color("leader", true), agent_color("leader", false));
    }

    #[test]
    fn theme_pref_round_trips() {
        for p in [ThemePref::System, ThemePref::Light, ThemePref::Dark] {
            assert_eq!(ThemePref::parse(p.as_str()), p);
        }
        assert_eq!(ThemePref::parse("nonsense"), ThemePref::System);
    }
}
