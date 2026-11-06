//! Images built into the app: the WorkCraft mark and the Craft app icons (64 px copies of the ArtCraft
//! icons). A file that fails to decode is simply left out; the UI draws a letter badge instead.

use std::collections::HashMap;

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

const MARK_BLUE: &[u8] = include_bytes!("../assets/mark-blue.png");
const WORDMARK_BLUE: &[u8] = include_bytes!("../assets/wordmark-blue.png");
const WINDOW_ICON: &[u8] = include_bytes!("../assets/window-icon.png");

/// Craft app name -> icon. The names match the MCP server names (`photocraft__...` tools).
const APPS: [(&str, &[u8]); 17] = [
    (
        "photocraft",
        include_bytes!("../assets/icons/photocraft.png"),
    ),
    (
        "vectorcraft",
        include_bytes!("../assets/icons/vectorcraft.png"),
    ),
    (
        "lightcraft",
        include_bytes!("../assets/icons/lightcraft.png"),
    ),
    ("filmcraft", include_bytes!("../assets/icons/filmcraft.png")),
    (
        "effectcraft",
        include_bytes!("../assets/icons/effectcraft.png"),
    ),
    (
        "soundcraft",
        include_bytes!("../assets/icons/soundcraft.png"),
    ),
    ("wordcraft", include_bytes!("../assets/icons/wordcraft.png")),
    ("gridcraft", include_bytes!("../assets/icons/gridcraft.png")),
    ("deckcraft", include_bytes!("../assets/icons/deckcraft.png")),
    ("pdfcraft", include_bytes!("../assets/icons/pdfcraft.png")),
    (
        "designcraft",
        include_bytes!("../assets/icons/designcraft.png"),
    ),
    ("cadcraft", include_bytes!("../assets/icons/cadcraft.png")),
    ("figcraft", include_bytes!("../assets/icons/figcraft.png")),
    (
        "texturecraft",
        include_bytes!("../assets/icons/texturecraft.png"),
    ),
    (
        "materialcraft",
        include_bytes!("../assets/icons/materialcraft.png"),
    ),
    (
        "solidcraft",
        include_bytes!("../assets/icons/solidcraft.png"),
    ),
    (
        "craft-anything",
        include_bytes!("../assets/icons/craft-anything.png"),
    ),
];

fn decode(bytes: &[u8]) -> Option<ColorImage> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}

fn load(ctx: &egui::Context, name: &str, bytes: &[u8]) -> Option<TextureHandle> {
    decode(bytes).map(|i| ctx.load_texture(name, i, TextureOptions::LINEAR))
}

/// Provider logos. `true`: a one-colour logo (white on transparent) that is tinted to the theme;
/// `false`: a colour logo drawn as it is.
const PROVIDERS: [(&str, &[u8], bool); 10] = [
    (
        "anthropic",
        include_bytes!("../assets/providers/anthropic.png"),
        true,
    ),
    (
        "openai",
        include_bytes!("../assets/providers/openai.png"),
        true,
    ),
    (
        "gemini",
        include_bytes!("../assets/providers/gemini.png"),
        true,
    ),
    (
        "deepseek",
        include_bytes!("../assets/providers/deepseek.png"),
        true,
    ),
    (
        "ollama",
        include_bytes!("../assets/providers/ollama.png"),
        true,
    ),
    ("xai", include_bytes!("../assets/providers/xai.png"), true),
    (
        "mistral",
        include_bytes!("../assets/providers/mistral.png"),
        false,
    ),
    (
        "groq",
        include_bytes!("../assets/providers/groq.png"),
        false,
    ),
    (
        "openrouter",
        include_bytes!("../assets/providers/openrouter.png"),
        false,
    ),
    (
        "together",
        include_bytes!("../assets/providers/together.png"),
        false,
    ),
];

pub struct Assets {
    pub mark: Option<TextureHandle>,
    /// The full `[W]orkcraft` wordmark (both mark and text in one image), with its width/height aspect ratio.
    pub wordmark: Option<(TextureHandle, f32)>,
    apps: HashMap<&'static str, TextureHandle>,
    providers: HashMap<&'static str, (TextureHandle, bool)>,
}

impl Assets {
    pub fn load(ctx: &egui::Context) -> Self {
        let wordmark = decode(WORDMARK_BLUE).map(|img| {
            let aspect = (img.width() as f32 / img.height().max(1) as f32).max(1.0);
            (
                ctx.load_texture("wordmark-blue", img, TextureOptions::LINEAR),
                aspect,
            )
        });
        Self {
            mark: load(ctx, "mark-blue", MARK_BLUE),
            wordmark,
            apps: APPS
                .iter()
                .filter_map(|(n, b)| load(ctx, n, b).map(|t| (*n, t)))
                .collect(),
            providers: PROVIDERS
                .iter()
                .filter_map(|(n, b, mono)| load(ctx, n, b).map(|t| (*n, (t, *mono))))
                .collect(),
        }
    }

    /// The logo of a provider id (`google` shares Gemini's), and whether it is one-colour.
    pub fn provider(&self, id: &str) -> Option<(&TextureHandle, bool)> {
        let id = if id == "google" { "gemini" } else { id };
        self.providers.get(id).map(|(t, m)| (t, *m))
    }

    /// The icon of a Craft app, by app name or by a tool name such as `photocraft__command_run`.
    pub fn app(&self, name: &str) -> Option<&TextureHandle> {
        let app = name.split("__").next().unwrap_or(name);
        self.apps.get(app.to_ascii_lowercase().as_str())
    }

    /// The icon that stands for an agent: the mark for the leader, the first Craft app it works with
    /// for a specialist.
    pub fn agent(&self, id: &str, skills: &[String]) -> Option<&TextureHandle> {
        match id {
            "leader" => self.mark.as_ref(),
            "image" => self.app("photocraft"),
            "design" => self.app("vectorcraft"),
            "video" => self.app("filmcraft"),
            "document" => self.app("wordcraft"),
            "three-d" => self.app("solidcraft"),
            "explorer" => self.app("craft-anything"),
            _ => skills.iter().find_map(|s| self.app(s)),
        }
    }
}

/// The window and taskbar icon.
pub fn window_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory_with_format(WINDOW_ICON, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    Some(egui::IconData {
        width: img.width(),
        height: img.height(),
        rgba: img.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_image_decodes() {
        assert!(decode(MARK_BLUE).is_some());
        assert!(decode(WORDMARK_BLUE).is_some());
        assert!(window_icon().is_some());
        for (n, b) in APPS {
            assert!(decode(b).is_some(), "{n}");
        }
        for (n, b, _) in PROVIDERS {
            assert!(decode(b).is_some(), "{n}");
        }
    }
}
