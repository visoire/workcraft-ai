//! Settings: appearance, models, providers (local and cloud, with keys), agents and approvals, MCP
//! servers, skills. Every change goes through the same engine commands the control channel uses
//! (`config.set`, `model.set`, `provider.save`, ...).
//!
//! Layout: the window is split into two fixed rectangles (navigation, content) and every row stacks
//! top to bottom, so long text wraps instead of running into its neighbours.

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, Rect, RichText, Sense,
    Stroke, UiBuilder, Vec2, pos2, vec2,
};
use serde_json::{Value, json};
use workcraft_core::paths;

use crate::app::{AgentEdit, App, ProviderEdit, SettingsTab, SkillEdit, TeamEdit};
use crate::providers::{PRESETS, is_local, preset};
use crate::theme::{Palette, ThemePref, agent_color, pal, semi, soft};
use crate::view_side::split_model;
use crate::widgets::{self, Icon, icon_button, paint_icon, short};

const TABS: [(SettingsTab, Icon, &str); 6] = [
    (SettingsTab::General, Icon::Settings, "General"),
    (SettingsTab::Models, Icon::Sparkle, "Models"),
    (SettingsTab::Providers, Icon::Key, "Providers"),
    (SettingsTab::Agents, Icon::Users, "Agents & approvals"),
    (SettingsTab::Tools, Icon::Plug, "Tools & MCP"),
    (SettingsTab::Skills, Icon::Terminal, "Skills"),
];

const NAV_W: f32 = 210.0;

/// A titled block: heading, optional help text that wraps, then the controls.
fn section(ui: &mut egui::Ui, title: &str, help: &str) {
    let p = pal(ui);
    ui.add_space(18.0);
    ui.label(semi(title).size(14.0).color(p.text));
    if !help.is_empty() {
        ui.add_space(2.0);
        ui.add(
            egui::Label::new(
                RichText::new(help)
                    .size(12.5)
                    .color(p.muted)
                    .line_height(Some(18.0)),
            )
            .wrap(),
        );
    }
    ui.add_space(8.0);
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    let p = pal(ui);
    ui.add_space(6.0);
    ui.label(RichText::new(text).size(12.5).color(p.muted));
    ui.add_space(2.0);
}

/// A single-line text field with the surface fill and a hairline border.
fn input(ui: &mut egui::Ui, text: &mut String, hint: &str, password: bool) -> egui::Response {
    let p = pal(ui);
    egui::Frame::new()
        .fill(p.bg)
        .stroke(Stroke::new(1.0, p.border_strong))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 0))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text(RichText::new(hint).color(p.faint))
                    .password(password)
                    .frame(egui::Frame::NONE)
                    .min_size(vec2(0.0, 32.0))
                    .align(Align2::LEFT_CENTER)
                    .desired_width(ui.available_width()),
            )
        })
        .inner
}

/// A multi-line text field with the surface fill and a hairline border.
fn multiline_input(
    ui: &mut egui::Ui,
    text: &mut String,
    hint: &str,
    rows: usize,
) -> egui::Response {
    let p = pal(ui);
    egui::Frame::new()
        .fill(p.bg)
        .stroke(Stroke::new(1.0, p.border_strong))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(text)
                    .hint_text(RichText::new(hint).color(p.faint))
                    .frame(egui::Frame::NONE)
                    .desired_rows(rows)
                    .desired_width(ui.available_width()),
            )
        })
        .inner
}

fn card<R>(
    ui: &mut egui::Ui,
    p: &Palette,
    highlight: bool,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(
            1.0,
            if highlight {
                soft(p.accent, 0x90)
            } else {
                p.border
            },
        ))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A coloured dot and a status line that wraps.
fn status_line(ui: &mut egui::Ui, color: Color32, text: &str) {
    let p = pal(ui);
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(10.0, 18.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 3.5, color);
        ui.add(egui::Label::new(RichText::new(text).size(12.5).color(p.muted)).wrap());
    });
}

/// The key field with a show / hide button. Never pre-filled: a saved key stays in the engine.
fn key_field(ui: &mut egui::Ui, e: &mut ProviderEdit) {
    ui.horizontal(|ui| {
        let w = ui.available_width() - 42.0;
        ui.allocate_ui(vec2(w, 34.0), |ui| {
            input(ui, &mut e.key, "Paste your API key", !e.show_key);
        });
        let tip = if e.show_key {
            "Hide the key"
        } else {
            "Show the key"
        };
        if widgets::icon_button_sized(ui, Icon::Eye, tip, 32.0, e.show_key).clicked() {
            e.show_key = !e.show_key;
        }
    });
}

/// What a provider card decided this frame.
#[derive(Default)]
struct CardOut {
    save: bool,
    forget: Option<(String, String)>,
    remove: Option<String>,
}

impl App {
    pub(crate) fn settings(&mut self, ctx: &egui::Context) {
        let p = crate::theme::palette(ctx.theme() == egui::Theme::Dark);
        let screen = ctx.content_rect().size();
        let w = (screen.x - 80.0).clamp(640.0, 940.0);
        let h = (screen.y - 80.0).clamp(440.0, 700.0);
        let modal = egui::Modal::new(egui::Id::unique("settings"))
            .backdrop_color(Color32::from_black_alpha(if p.dark { 150 } else { 70 }))
            .frame(
                egui::Frame::new()
                    .fill(p.raised)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(CornerRadius::same(16))
                    .shadow(ctx.global_style().visuals.window_shadow)
                    .inner_margin(Margin::ZERO),
            )
            .show(ctx, |ui| {
                let (outer, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                let nav = Rect::from_min_size(outer.min, vec2(NAV_W, h));
                let content = Rect::from_min_max(pos2(nav.right(), outer.top()), outer.max);
                ui.painter().rect_filled(
                    nav,
                    CornerRadius {
                        nw: 16,
                        sw: 16,
                        ne: 0,
                        se: 0,
                    },
                    p.sidebar,
                );
                ui.painter()
                    .vline(nav.right(), nav.y_range(), Stroke::new(1.0, p.border));

                // Navigation.
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(nav.shrink(12.0))
                        .layout(Layout::top_down(Align::Min)),
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add_space(6.0);
                            ui.label(semi("Settings").size(17.0).color(p.text));
                        });
                        ui.add_space(12.0);
                        for (tab, icon, label) in TABS {
                            let on = self.settings_tab == tab;
                            let (rect, resp) = widgets::row(ui, on, 36.0);
                            let fg = if on { p.text } else { p.muted };
                            paint_icon(
                                ui.painter(),
                                Rect::from_center_size(
                                    pos2(rect.left() + 18.0, rect.center().y),
                                    Vec2::splat(15.0),
                                ),
                                icon,
                                fg,
                            );
                            ui.painter().text(
                                pos2(rect.left() + 36.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                label,
                                FontId::proportional(13.5),
                                fg,
                            );
                            if resp.clicked() && !on {
                                self.settings_tab = tab;
                                self.settings_scroll = Some(0.0);
                                self.settings_error = None;
                                self.provider_edit = None;
                            }
                        }
                    },
                );

                // Content: a header, then the tab in a scroll area.
                let inner = Rect::from_min_max(
                    content.min + vec2(28.0, 16.0),
                    content.max - vec2(16.0, 16.0),
                );
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(inner)
                        .layout(Layout::top_down(Align::Min)),
                    |ui| {
                        ui.horizontal(|ui| {
                            let title = TABS
                                .iter()
                                .find(|t| t.0 == self.settings_tab)
                                .map_or("", |t| t.2);
                            ui.label(semi(title).size(18.0).color(p.text));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if icon_button(ui, Icon::Close, "Close (Esc)").clicked() {
                                    self.show_settings = false;
                                }
                            });
                        });
                        if let Some(e) = self.settings_error.clone() {
                            ui.add_space(6.0);
                            egui::Frame::new()
                                .fill(p.danger_soft)
                                .corner_radius(CornerRadius::same(8))
                                .inner_margin(Margin::symmetric(10, 6))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width() - 12.0);
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(e).size(12.5).color(p.danger),
                                        )
                                        .wrap(),
                                    );
                                });
                        }
                        let mut sa = egui::ScrollArea::vertical()
                            .id_salt(("settings-scroll", self.settings_tab as u8))
                            .auto_shrink([false, false]);
                        if let Some(y) = self.settings_scroll.take() {
                            sa = sa.vertical_scroll_offset(y);
                        }
                        sa.show(ui, |ui| {
                            // Leave room for the scroll bar on the right.
                            ui.set_width(ui.available_width() - 12.0);
                            match self.settings_tab {
                                SettingsTab::General => self.tab_general(ui),
                                SettingsTab::Models => self.tab_models(ui),
                                SettingsTab::Providers => self.tab_providers(ui),
                                SettingsTab::Agents => self.tab_agents(ui),
                                SettingsTab::Tools => self.tab_tools(ui),
                                SettingsTab::Skills => self.tab_skills(ui),
                            }
                            ui.add_space(16.0);
                        });
                    },
                );
            });
        if modal.should_close() {
            self.show_settings = false;
            self.provider_edit = None;
        }
    }

    fn tab_general(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        section(
            ui,
            "Appearance",
            "System follows the light or dark setting of your computer. The switch in the sidebar flips between light and dark.",
        );
        let sel = match self.theme {
            ThemePref::System => 0,
            ThemePref::Light => 1,
            ThemePref::Dark => 2,
        };
        if let Some(i) = widgets::segmented(
            ui,
            &[
                (Some(Icon::Monitor), "System"),
                (Some(Icon::Sun), "Light"),
                (Some(Icon::Moon), "Dark"),
            ],
            sel,
            "theme",
        ) {
            let t = [ThemePref::System, ThemePref::Light, ThemePref::Dark][i];
            let ctx = ui.ctx().clone();
            self.set_theme(&ctx, t);
        }

        section(ui, "Layout", "");
        let mut sb = self.show_sidebar;
        let mut ac = self.show_activity;
        let a = ui.checkbox(&mut sb, "Show the chat list").changed();
        let b = ui.checkbox(&mut ac, "Show the activity panel").changed();
        if a || b {
            self.show_sidebar = sb;
            self.show_activity = ac;
            self.save_prefs();
        }

        section(
            ui,
            "Blue Blip mascot",
            "Colors, accessory, size, how much Blip moves and whether it talks. Blip thinks, works, waits for your OK, listens, cheers when a job is done and droops when something fails.",
        );
        card(ui, &p, false, |ui| {
            ui.horizontal(|ui| {
                if crate::blip::paint_mascot(ui, &self.blip, 68.0, None, self.now) {
                    self.blip.on_click(self.now);
                }
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label(semi("Color theme").size(12.5).color(p.text));
                    ui.horizontal_wrapped(|ui| {
                        for th in crate::blip::BlipTheme::ALL {
                            let active = self.blip.theme == th;
                            if widgets::chip(
                                ui,
                                Icon::Sparkle,
                                th.label(),
                                active,
                                "Set Blip theme",
                            )
                            .clicked()
                            {
                                self.blip.theme = th;
                                crate::theme::set_accent(ui.ctx(), th.accent());
                                self.save_prefs();
                            }
                        }
                    });
                    ui.add_space(6.0);
                    ui.label(semi("Default accessory").size(12.5).color(p.text));
                    ui.horizontal_wrapped(|ui| {
                        for acc in crate::blip::BlipAccessory::ALL {
                            let active = self.blip.accessory == acc;
                            if widgets::chip(
                                ui,
                                Icon::Check,
                                acc.label(),
                                active,
                                "Set Blip default accessory",
                            )
                            .clicked()
                            {
                                self.blip.accessory = acc;
                                self.blip.temp_accessory = None;
                                self.save_prefs();
                            }
                        }
                    });
                    ui.add_space(6.0);
                    ui.label(semi("Movement").size(12.5).color(p.text));
                    ui.horizontal_wrapped(|ui| {
                        for an in crate::blip::BlipAnim::ALL {
                            let tip = match an {
                                crate::blip::BlipAnim::Full => {
                                    "Full animation, 30 frames a second while something happens"
                                }
                                crate::blip::BlipAnim::Calm => "Softer moves, 15 frames a second",
                                crate::blip::BlipAnim::Off => {
                                    "A still picture, no animation and no extra redraws"
                                }
                            };
                            if widgets::chip(
                                ui,
                                Icon::Sparkle,
                                an.label(),
                                self.blip.anim == an,
                                tip,
                            )
                            .clicked()
                            {
                                self.blip.anim = an;
                                self.save_prefs();
                            }
                        }
                    });
                    ui.add_space(6.0);
                    ui.label(semi("Size").size(12.5).color(p.text));
                    ui.horizontal_wrapped(|ui| {
                        for sz in crate::blip::BlipSize::ALL {
                            if widgets::chip(
                                ui,
                                Icon::Check,
                                sz.label(),
                                self.blip.size == sz,
                                "Set Blip size",
                            )
                            .clicked()
                            {
                                self.blip.size = sz;
                                self.save_prefs();
                            }
                        }
                    });
                    ui.add_space(6.0);
                    let mut talk = self.blip.bubbles;
                    if ui.checkbox(&mut talk, "Speech bubbles").changed() {
                        self.blip.bubbles = talk;
                        self.save_prefs();
                    }
                });
            });
            ui.add_space(8.0);
            ui.label(semi("Test animations").size(12.0).color(p.muted));
            ui.horizontal_wrapped(|ui| {
                for st in crate::blip::BlipState::ALL {
                    let active = self.blip.active_state(self.now) == st;
                    if widgets::button(ui, None, st.label(), active).clicked() {
                        self.blip.trigger(st, 3.0, self.now);
                        self.blip
                            .say(format!("State: {}", st.label()), 2.5, self.now);
                    }
                }
            });
        });

        section(
            ui,
            "Voice input (Handy 0.10.0 & Speech)",
            "Click the microphone icon next to Send (or press Ctrl+Shift+M) to speak commands like \"@gridcraft read column B\" or \"Summarize the current document\".",
        );
        card(ui, &p, false, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                paint_icon(ui.painter(), r, Icon::Mic, p.accent_text);
                if let Some(exe) = &self.voice.handy_info.exe_path {
                    let ver = self.voice.handy_info.version.clone();
                    ui.label(
                        semi(format!(
                            "Handy {} ready \u{00b7} {}",
                            ver.as_deref().unwrap_or(""),
                            self.voice.handy_info.model_display
                        ))
                        .size(13.0)
                        .color(p.text),
                    )
                    .on_hover_text(exe.display().to_string());
                    if ver.as_deref().is_some_and(|v| {
                        !crate::voice::version_at_least(v, crate::voice::HANDY_TARGET_VERSION)
                    }) {
                        widgets::pill(
                            ui,
                            &format!("Update to {}", crate::voice::HANDY_TARGET_VERSION),
                            p.warning,
                            crate::theme::soft(p.warning, 0x26),
                        );
                    } else {
                        widgets::pill(
                            ui,
                            "Detected",
                            p.success,
                            crate::theme::soft(p.success, 0x26),
                        );
                    }
                } else {
                    ui.label(
                        semi("Windows Speech / Web Speech API ready")
                            .size(13.0)
                            .color(p.text),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button(
                        ui,
                        Icon::Refresh,
                        "Refresh microphones & Handy runtime",
                    )
                    .clicked()
                    {
                        self.voice.refresh_devices();
                    }
                });
            });
            ui.add_space(6.0);
            ui.label(semi("Speech recognition engine").size(12.0).color(p.muted));
            ui.horizontal_wrapped(|ui| {
                for pref in [
                    crate::voice::VoiceEnginePref::Auto,
                    crate::voice::VoiceEnginePref::HandyNemotron,
                    crate::voice::VoiceEnginePref::WindowsSpeech,
                    crate::voice::VoiceEnginePref::WebSpeechApi,
                ] {
                    let active = self.voice.engine_pref == pref;
                    if widgets::chip(ui, Icon::Mic, pref.label(), active, "Select voice engine")
                        .clicked()
                    {
                        self.voice.engine_pref = pref;
                        self.save_prefs();
                    }
                }
            });
            ui.add_space(6.0);
            ui.label(semi("Microphone device").size(12.0).color(p.muted));
            ui.horizontal_wrapped(|ui| {
                let mics = self.voice.mics.clone();
                for m in &mics {
                    let active = self.voice.selected_mic == m.id;
                    if widgets::chip(ui, Icon::Mic, &m.name, active, "Select input microphone")
                        .clicked()
                    {
                        self.voice.selected_mic = m.id;
                    }
                }
            });
            ui.add_space(6.0);
            let mut auto_stop = self.voice.auto_stop_on_silence;
            let mut send_cmd = self.voice.voice_send_command;
            let c1 = ui
                .checkbox(
                    &mut auto_stop,
                    "Automatically finish recording after 2.3s of silence",
                )
                .changed();
            let c2 = ui
                .checkbox(
                    &mut send_cmd,
                    "Allow a spoken \"send message\" at the end to send immediately",
                )
                .changed();
            if c1 || c2 {
                self.voice.auto_stop_on_silence = auto_stop;
                self.voice.voice_send_command = send_cmd;
                self.save_prefs();
            }
        });

        section(
            ui,
            "Data folder",
            "Chats, settings, agents, skills and mcp.json live here.",
        );
        let dir = paths::data_dir().display().to_string();
        egui::Frame::new()
            .fill(p.surface)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(
                    egui::Label::new(RichText::new(&dir).monospace().size(12.0).color(p.muted))
                        .wrap(),
                );
            });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let copied = self.just_copied("data-dir");
            if widgets::button(
                ui,
                Some(if copied { Icon::Check } else { Icon::Copy }),
                if copied { "Copied" } else { "Copy path" },
                false,
            )
            .clicked()
            {
                ui.ctx().copy_text(dir.clone());
                self.copied = Some(("data-dir".into(), self.now));
            }
            if widgets::button(ui, Some(Icon::Folder), "Open folder", false)
                .on_hover_text("Show the data folder in the file manager")
                .clicked()
            {
                crate::app::open_folder(&paths::data_dir());
            }
            if widgets::button(ui, Some(Icon::Download), "Export all chats (.json)", false)
                .clicked()
            {
                self.export_all();
            }
        });

        section(ui, "Keyboard", "");
        egui::Grid::new("keys")
            .num_columns(2)
            .spacing(vec2(14.0, 8.0))
            .show(ui, |ui| {
                for (k, what) in [
                    ("Enter", "Send"),
                    ("Shift+Enter", "New line"),
                    ("Ctrl+Shift+M", "Start or finish voice input"),
                    ("Esc", "Stop the reply / cancel voice input"),
                    ("Ctrl+Shift+O", "New chat"),
                    ("Ctrl+Shift+S", "Show or hide the chat list"),
                    ("Ctrl+,", "Settings"),
                ] {
                    widgets::pill(ui, k, p.text, p.surface);
                    ui.label(RichText::new(what).size(13.0).color(p.muted));
                    ui.end_row();
                }
            });
    }

    fn tab_models(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        section(
            ui,
            "Default model",
            "Used by new chats. The picker above the chat changes the model of the open chat only.",
        );
        egui::Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::same(4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt("default-model")
                    .max_height(200.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if self.models.is_empty() {
                            ui.label(
                                RichText::new("No models yet. Set up a provider first.")
                                    .size(13.0)
                                    .color(p.faint),
                            );
                        }
                        for m in self.models.clone() {
                            let on = self.draft.default_model == m;
                            let (rect, resp) = widgets::row(ui, on, 32.0);
                            let (prov, name) = split_model(&m);
                            let lr = Rect::from_center_size(
                                pos2(rect.left() + 18.0, rect.center().y),
                                Vec2::splat(16.0),
                            );
                            widgets::paint_provider_logo(
                                ui,
                                lr,
                                self.assets.provider(prov),
                                &self.provider_name(prov),
                            );
                            ui.painter()
                                .with_clip_rect(Rect::from_min_max(
                                    rect.min,
                                    pos2(rect.right() - 30.0, rect.bottom()),
                                ))
                                .text(
                                    pos2(rect.left() + 36.0, rect.center().y),
                                    Align2::LEFT_CENTER,
                                    format!("{name}   \u{00b7}  {}", self.provider_name(prov)),
                                    FontId::proportional(13.5),
                                    p.text,
                                );
                            if on {
                                paint_icon(
                                    ui.painter(),
                                    Rect::from_center_size(
                                        pos2(rect.right() - 16.0, rect.center().y),
                                        Vec2::splat(14.0),
                                    ),
                                    Icon::Check,
                                    p.accent_text,
                                );
                            }
                            if resp.clicked() {
                                self.draft.default_model = m.clone();
                            }
                        }
                    });
            });

        section(
            ui,
            "Reasoning",
            "Thinking on or off, for models that support it.",
        );
        let sel = match self.draft.think {
            None => 0,
            Some(true) => 1,
            Some(false) => 2,
        };
        if let Some(i) = widgets::segmented(
            ui,
            &[(None, "Model default"), (None, "On"), (None, "Off")],
            sel,
            "think",
        ) {
            self.draft.think = [None, Some(true), Some(false)][i];
        }

        section(
            ui,
            "Context window",
            "Tokens a local model keeps in mind. Empty lets WorkCraft decide. Bigger needs more memory.",
        );
        ui.horizontal(|ui| {
            ui.allocate_ui(vec2(150.0, 34.0), |ui| {
                input(ui, &mut self.draft.num_ctx, "Automatic", false)
            });
            for n in [8192u32, 16384, 32768, 65536] {
                let on = self.draft.num_ctx == n.to_string();
                if widgets::chip(ui, Icon::Plus, &format!("{}k", n / 1024), on, "").clicked() {
                    self.draft.num_ctx = if on { String::new() } else { n.to_string() };
                }
            }
        });

        section(
            ui,
            "Tool steps per message",
            "How many rounds of tool use one message may take before WorkCraft stops.",
        );
        ui.add(egui::Slider::new(&mut self.draft.max_steps, 1..=200));

        section(
            ui,
            "Instructions",
            "Added to the system prompt of every chat.",
        );
        egui::Frame::new()
            .fill(p.bg)
            .stroke(Stroke::new(1.0, p.border_strong))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.draft.system_prompt)
                        .hint_text(
                            RichText::new("For example: answer in British English, keep it short.")
                                .color(p.faint),
                        )
                        .frame(egui::Frame::NONE)
                        .desired_rows(4)
                        .desired_width(ui.available_width()),
                );
            });
        ui.add_space(14.0);
        let bad_ctx = !self.draft.num_ctx.trim().is_empty()
            && self.draft.num_ctx.trim().parse::<u32>().is_err();
        ui.horizontal(|ui| {
            if widgets::button(ui, Some(Icon::Check), "Save", true).clicked() && !bad_ctx {
                let num_ctx = self
                    .draft
                    .num_ctx
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .map_or(Value::Null, |n| json!(n));
                let think = self.draft.think.map_or(Value::Null, |b| json!(b));
                self.bridge.run(
                    "config-saved",
                    "config.set",
                    json!({ "think": think, "num_ctx": num_ctx, "max_steps": self.draft.max_steps, "system_prompt": self.draft.system_prompt }),
                );
                let m = self.draft.default_model.clone();
                if !m.is_empty() && self.config["model"].as_str() != Some(m.as_str()) {
                    self.bridge
                        .run("model-default", "model.set", json!({ "model": m }));
                    self.config["model"] = json!(m);
                }
                self.copied = Some(("settings-saved".into(), self.now));
            }
            if bad_ctx {
                ui.label(
                    RichText::new("The context window must be a whole number.")
                        .size(12.5)
                        .color(p.danger),
                );
            } else if self.just_copied("settings-saved") {
                ui.label(RichText::new("Saved").size(12.5).color(p.success));
            }
        });
    }

    /// One card per provider: logo, name, what it is, whether it works, and an editor to set it up.
    fn tab_providers(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        ui.add_space(10.0);
        ui.add(
            egui::Label::new(
                RichText::new("Use models on this computer, or add a cloud provider with your own API key. Keys are saved in config.json in the data folder on this computer, are sent only to that provider, and never appear in chats. A key in an environment variable works too.")
                    .size(12.5)
                    .color(p.muted)
                    .line_height(Some(18.0)),
            )
            .wrap(),
        );
        let mut edit = self.provider_edit.take();
        let mut out = CardOut::default();

        section(ui, "On this computer", "");
        for pr in PRESETS.iter().filter(|x| x.local) {
            self.provider_card(ui, pr.id, &mut edit, &mut out);
            ui.add_space(8.0);
        }
        section(ui, "Cloud providers", "");
        for pr in PRESETS.iter().filter(|x| !x.local) {
            self.provider_card(ui, pr.id, &mut edit, &mut out);
            ui.add_space(8.0);
        }
        section(
            ui,
            "Your own endpoints",
            "Any server that speaks the OpenAI Chat Completions format: vLLM, llama.cpp server, a company gateway.",
        );
        let customs: Vec<String> = self
            .provider_info
            .iter()
            .filter_map(|v| v["id"].as_str())
            .filter(|id| preset(id).is_none())
            .map(str::to_string)
            .collect();
        for id in &customs {
            self.provider_card(ui, id, &mut edit, &mut out);
            ui.add_space(8.0);
        }
        let adding = edit.as_ref().is_some_and(|e| e.custom);
        if adding {
            let mut cancel = false;
            if let Some(e) = edit.as_mut() {
                card(ui, &p, true, |ui| {
                    ui.label(semi("New endpoint").size(14.0).color(p.text));
                    field_label(ui, "Name");
                    input(ui, &mut e.name, "For example: Lab server", false);
                    field_label(ui, "Address");
                    input(ui, &mut e.base_url, "https://example.com/v1", false);
                    field_label(ui, "API key (leave empty if the server needs none)");
                    key_field(ui, e);
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if widgets::button(ui, Some(Icon::Check), "Add endpoint", true).clicked() {
                            out.save = true;
                        }
                        if widgets::button(ui, None, "Cancel", false).clicked() {
                            cancel = true;
                        }
                    });
                });
            }
            if cancel {
                edit = None;
            }
        } else if widgets::button(ui, Some(Icon::Plus), "Add an endpoint", false).clicked() {
            edit = Some(ProviderEdit {
                id: "custom".into(),
                kind: "openai".into(),
                custom: true,
                ..ProviderEdit::default()
            });
        }

        self.provider_edit = edit;
        if out.save {
            self.save_provider();
        }
        if let Some((id, kind)) = out.forget {
            self.forget_key(&id, &kind);
        }
        if let Some(id) = out.remove {
            self.bridge
                .run("provider-removed", "provider.remove", json!({ "id": id }));
        }
    }

    fn provider_card(
        &self,
        ui: &mut egui::Ui,
        id: &str,
        edit: &mut Option<ProviderEdit>,
        out: &mut CardOut,
    ) {
        let p = pal(ui);
        let pre = preset(id);
        let info = self.provider_info.iter().find(|v| v["id"] == id);
        let name = self.provider_name(id);
        let in_registry = self.providers.iter().any(|x| x == id);
        let models = self
            .models
            .iter()
            .filter(|m| split_model(m).0 == id)
            .count();
        let error = self.model_errors.get(id);
        let key_from = info.and_then(|v| v["key_from"].as_str()).unwrap_or("none");
        let hint = info.and_then(|v| v["key_hint"].as_str()).unwrap_or("");
        let from_settings = info.is_some_and(|v| v["source"] == "settings");
        let broken = info.is_some_and(|v| v["broken"] == true);
        let local = is_local(id);
        let editing = edit.as_ref().is_some_and(|e| !e.custom && e.id == id);
        let text_of = |k: &str| info.and_then(|v| v[k].as_str()).map(str::to_string);

        card(ui, &p, editing, |ui| {
            ui.horizontal_top(|ui| {
                widgets::provider_logo(ui, self.assets.provider(id), &name, 30.0);
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.horizontal(|ui| {
                        ui.label(semi(&name).size(14.0).color(p.text));
                    });
                    let blurb = pre.map_or_else(
                        || text_of("base_url").unwrap_or_default(),
                        |x| x.blurb.to_string(),
                    );
                    if !blurb.is_empty() {
                        ui.add(
                            egui::Label::new(RichText::new(blurb).size(12.5).color(p.muted)).wrap(),
                        );
                    }
                    let (color, text) = if broken {
                        (
                            p.danger,
                            "This entry could not be loaded. Check the address.".to_string(),
                        )
                    } else if let Some(e) = error {
                        (
                            p.danger,
                            short(&crate::providers::friendly_error(id, e), 180),
                        )
                    } else if in_registry && models > 0 {
                        let key = match key_from {
                            "saved" => format!(" \u{00b7} key {hint}"),
                            "env" => " \u{00b7} key from an environment variable".to_string(),
                            _ => String::new(),
                        };
                        (p.success, format!("Ready \u{00b7} {models} models{key}"))
                    } else if in_registry {
                        (p.warning, "Connected, no models listed yet".to_string())
                    } else if local {
                        (p.faint, "Not set up".to_string())
                    } else {
                        (p.faint, "Needs an API key".to_string())
                    };
                    status_line(ui, color, &text);
                });
            });

            if editing {
                let mut cancel = false;
                if let Some(e) = edit.as_mut() {
                    ui.add_space(10.0);
                    if !local {
                        field_label(
                            ui,
                            if key_from == "saved" {
                                "API key (a key is saved; type a new one to replace it)"
                            } else {
                                "API key"
                            },
                        );
                        key_field(ui, e);
                        if let Some(pr) = pre {
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(format!("Get a key at {}", pr.docs))
                                    .size(12.0)
                                    .color(p.faint),
                            );
                        }
                    }
                    field_label(ui, "Address");
                    input(
                        ui,
                        &mut e.base_url,
                        pre.map_or("https://example.com/v1", |x| x.base_url),
                        false,
                    );
                    if !local {
                        field_label(ui, "Or read the key from this environment variable");
                        input(
                            ui,
                            &mut e.key_env,
                            pre.map_or("MY_API_KEY", |x| x.key_env),
                            false,
                        );
                    }
                    ui.add_space(12.0);
                    ui.horizontal_wrapped(|ui| {
                        if widgets::button(ui, Some(Icon::Check), "Save", true).clicked() {
                            out.save = true;
                        }
                        if widgets::button(ui, None, "Cancel", false).clicked() {
                            cancel = true;
                        }
                        if key_from == "saved"
                            && widgets::button(ui, Some(Icon::Trash), "Forget the key", false)
                                .clicked()
                        {
                            out.forget = Some((id.to_string(), e.kind.clone()));
                        }
                        if from_settings
                            && widgets::button(ui, Some(Icon::Trash), "Remove", false).clicked()
                        {
                            out.remove = Some(id.to_string());
                        }
                    });
                }
                if cancel {
                    *edit = None;
                }
            } else {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.add_space(42.0);
                    let (label, icon, primary) = if in_registry || from_settings {
                        ("Edit", Icon::Edit, false)
                    } else if local {
                        ("Set up", Icon::Plus, false)
                    } else {
                        ("Add key", Icon::Key, true)
                    };
                    if widgets::button(ui, Some(icon), label, primary).clicked() {
                        let kind = text_of("kind")
                            .or_else(|| pre.map(|x| x.kind.to_string()))
                            .unwrap_or_else(|| "openai".into());
                        *edit = Some(ProviderEdit {
                            id: id.to_string(),
                            kind,
                            name: text_of("name")
                                .or_else(|| pre.map(|x| x.name.to_string()))
                                .unwrap_or_default(),
                            base_url: text_of("base_url")
                                .or_else(|| pre.map(|x| x.base_url.to_string()))
                                .unwrap_or_default(),
                            key_env: text_of("key_env")
                                .or_else(|| pre.map(|x| x.key_env.to_string()))
                                .unwrap_or_default(),
                            ..ProviderEdit::default()
                        });
                    }
                });
            }
        });
    }

    #[allow(clippy::too_many_lines)]
    fn tab_agents(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        section(
            ui,
            "Approvals",
            "Ask first: anything that writes a file or changes a Craft document waits for your Allow. Auto: it runs right away. Reading never asks.",
        );
        let sel = usize::from(self.auto_approve);
        if let Some(i) = widgets::segmented(
            ui,
            &[
                (Some(Icon::Shield), "Ask first"),
                (Some(Icon::Bolt), "Auto"),
            ],
            sel,
            "approval",
        ) {
            self.set_auto_approve(i == 1);
        }

        section(
            ui,
            "Team mode",
            "Which team runs when Team mode is on. The Craft team has five specialists; the apps team has one agent for every Craft app that is installed and switched on.",
        );
        let preset_sel = usize::from(self.team_preset == "apps");
        if let Some(i) = widgets::segmented(
            ui,
            &[
                (Some(Icon::Users), "Craft team (5 specialists)"),
                (Some(Icon::Plug), "One agent per app"),
            ],
            preset_sel,
            "team-preset",
        ) {
            self.team_preset = if i == 1 { "apps" } else { "craft" }.to_string();
            self.save_prefs();
        }

        section(
            ui,
            "Team configuration",
            "In Team mode the leader plans, the specialists work in parallel up to the concurrency limit, and the leader reviews each result.",
        );
        let team_name = self.team_raw["name"].as_str().unwrap_or("Craft Team");
        let leader_name = self.team_raw["leader"]["name"]
            .as_str()
            .unwrap_or("Craft Lead");
        let leader_scope = self.team_raw["leader"]["scope"]
            .as_str()
            .unwrap_or("Plans and coordinates multi-app tasks across the Craft suite.");
        let concurrency = self.team_raw["concurrency"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(3);
        let review = self.team_raw["review"].as_bool().unwrap_or(true);

        let editing_team = self.team_edit.is_some();
        let mut save_team_clicked = false;
        let mut reset_team_clicked = false;
        let mut cancel_team_clicked = false;

        card(ui, &p, editing_team, |ui| {
            ui.horizontal(|ui| {
                ui.label(semi(team_name).size(13.5).color(p.text));
                widgets::pill(
                    ui,
                    &format!("Concurrency: {concurrency}"),
                    p.muted,
                    p.pressed,
                );
                if review {
                    widgets::pill(ui, "Leader review on", p.accent_text, p.accent_soft);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if !editing_team
                        && widgets::button(ui, Some(Icon::Edit), "Customize team", false).clicked()
                    {
                        self.team_edit = Some(TeamEdit {
                            name: team_name.to_string(),
                            leader_name: leader_name.to_string(),
                            leader_scope: leader_scope.to_string(),
                            leader_prompt: self.team_raw["leader"]["prompt"]
                                .as_str()
                                .unwrap_or_default()
                                .to_string(),
                            concurrency,
                            review,
                            plan_approval: self.team_raw["plan_approval"].as_bool(),
                        });
                    }
                });
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("Leader: {leader_name} — {leader_scope}"))
                    .size(12.5)
                    .color(p.muted),
            );

            if let Some(te) = self.team_edit.as_mut() {
                ui.add_space(10.0);
                field_label(ui, "Team name");
                input(ui, &mut te.name, "Craft Team", false);
                field_label(ui, "Leader name");
                input(ui, &mut te.leader_name, "Craft Lead", false);
                field_label(ui, "Leader role / scope");
                input(
                    ui,
                    &mut te.leader_scope,
                    "Plans and coordinates multi-app tasks across the Craft suite.",
                    false,
                );
                field_label(ui, "Leader system prompt");
                multiline_input(
                    ui,
                    &mut te.leader_prompt,
                    "Instructions for the team leader when decomposing tasks and reviewing specialist outputs...",
                    4,
                );
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Max parallel specialists:")
                            .size(12.5)
                            .color(p.muted),
                    );
                    for c in [1_u32, 2, 3, 4, 6] {
                        if widgets::chip(
                            ui,
                            Icon::Users,
                            &c.to_string(),
                            te.concurrency == c,
                            "Maximum number of specialists running at the same time",
                        )
                        .clicked()
                        {
                            te.concurrency = c;
                        }
                    }
                    ui.add_space(8.0);
                    if widgets::chip(
                        ui,
                        if te.review {
                            Icon::Check
                        } else {
                            Icon::Sparkle
                        },
                        if te.review {
                            "Review each step: On"
                        } else {
                            "Review each step: Off"
                        },
                        te.review,
                        "Have the leader review specialist outputs before moving on",
                    )
                    .clicked()
                    {
                        te.review = !te.review;
                    }
                    ui.add_space(8.0);
                    let (plan_label, plan_tip) = match te.plan_approval {
                        None => (
                            "Approve the plan: when asking first",
                            "The plan waits for you when approvals are on Ask first",
                        ),
                        Some(true) => (
                            "Approve the plan: always",
                            "The plan always waits for you, even with Auto approvals",
                        ),
                        Some(false) => ("Approve the plan: never", "The plan starts at once"),
                    };
                    if widgets::chip(
                        ui,
                        Icon::Shield,
                        plan_label,
                        te.plan_approval.is_some(),
                        plan_tip,
                    )
                    .clicked()
                    {
                        te.plan_approval = match te.plan_approval {
                            None => Some(true),
                            Some(true) => Some(false),
                            Some(false) => None,
                        };
                    }
                });
                ui.add_space(12.0);
                ui.horizontal_wrapped(|ui| {
                    if widgets::button(ui, Some(Icon::Check), "Save team", true).clicked() {
                        save_team_clicked = true;
                    }
                    if widgets::button(ui, None, "Cancel", false).clicked() {
                        cancel_team_clicked = true;
                    }
                    if widgets::button(ui, Some(Icon::Refresh), "Reset to default", false).clicked()
                    {
                        reset_team_clicked = true;
                    }
                });
            }
        });
        if cancel_team_clicked {
            self.team_edit = None;
        }
        if save_team_clicked {
            self.save_team();
        }
        if reset_team_clicked {
            self.reset_team();
        }

        section(
            ui,
            "Agents & specialist roles",
            "Create custom specialist roles or edit existing ones. Custom agents are automatically added to the team and can also be mentioned with @ in chat.",
        );
        if self.agent_edit.is_none()
            && widgets::button(ui, Some(Icon::Plus), "New custom agent / role", true).clicked()
        {
            self.agent_edit = Some(AgentEdit {
                id: String::new(),
                name: String::new(),
                scope: String::new(),
                prompt: "You are a specialist agent in WorkCraft. Focus on your assigned role and deliver clear, accurate results.".to_string(),
                model: String::new(),
                skills: String::new(),
                tools: "*".to_string(),
                approval: String::new(),
                is_new: true,
            });
        }

        let mut save_agent_clicked = false;
        let mut cancel_agent_clicked = false;
        let mut delete_agent_id: Option<String> = None;

        if let Some(ae) = self.agent_edit.as_mut() {
            ui.add_space(8.0);
            card(ui, &p, true, |ui| {
                ui.label(
                    semi(if ae.is_new {
                        "New custom agent / role"
                    } else {
                        "Edit agent / role"
                    })
                    .size(14.0)
                    .color(p.text),
                );
                field_label(ui, "Role name");
                input(ui, &mut ae.name, "e.g. Research Analyst", false);
                field_label(
                    ui,
                    "Agent ID (used for @mention, leave blank to generate from name)",
                );
                input(ui, &mut ae.id, "e.g. research-analyst", false);
                field_label(
                    ui,
                    "Role description / scope (when the team leader should delegate to this agent)",
                );
                input(
                    ui,
                    &mut ae.scope,
                    "e.g. Deep research, summarizing documents, and fact-checking.",
                    false,
                );
                field_label(ui, "System prompt / instructions");
                multiline_input(
                    ui,
                    &mut ae.prompt,
                    "Detailed instructions for how this agent should work...",
                    5,
                );
                field_label(
                    ui,
                    "Skills (comma-separated skill names, e.g. wordcraft, pdfcraft)",
                );
                input(ui, &mut ae.skills, "wordcraft, gridcraft", false);
                field_label(
                    ui,
                    "Allowed tools (* for all tools, or comma-separated tool names / prefixes)",
                );
                input(ui, &mut ae.tools, "*", false);
                field_label(
                    ui,
                    "Model override (optional, leave blank to use active chat model)",
                );
                input(ui, &mut ae.model, "provider/model", false);
                field_label(
                    ui,
                    "Approval (paid generation, code execution, deleting and signing always ask)",
                );
                ui.horizontal_wrapped(|ui| {
                    for (val, label, tip) in [
                        ("", "Follow the team", "Use the approval of the team"),
                        (
                            "manual",
                            "Ask first",
                            "Every change this agent makes waits for your Allow",
                        ),
                        ("auto", "Auto", "This agent's changes run without asking"),
                    ] {
                        if widgets::chip(ui, Icon::Shield, label, ae.approval == val, tip).clicked()
                        {
                            ae.approval = val.to_string();
                        }
                    }
                });
                ui.add_space(12.0);
                ui.horizontal_wrapped(|ui| {
                    if widgets::button(ui, Some(Icon::Check), "Save agent", true).clicked() {
                        save_agent_clicked = true;
                    }
                    if widgets::button(ui, None, "Cancel", false).clicked() {
                        cancel_agent_clicked = true;
                    }
                    if !ae.is_new
                        && ae.id != "leader"
                        && widgets::button(ui, Some(Icon::Trash), "Delete agent", false).clicked()
                    {
                        delete_agent_id = Some(ae.id.clone());
                    }
                });
            });
            ui.add_space(10.0);
        }
        if cancel_agent_clicked {
            self.agent_edit = None;
        }
        if save_agent_clicked {
            self.save_agent();
        }
        if let Some(id) = delete_agent_id {
            self.remove_agent(&id);
        }

        ui.add_space(8.0);
        let roster = self.roster.clone();
        if roster.is_empty() {
            ui.label(RichText::new("Loading the team...").color(p.faint));
        }
        for a in &roster {
            let tint = agent_color(&a.id, p.dark);
            let is_editing = self
                .agent_edit
                .as_ref()
                .is_some_and(|e| !e.is_new && e.id == a.id);
            card(ui, &p, is_editing, |ui| {
                ui.horizontal_top(|ui| {
                    widgets::avatar(ui, 32.0, self.assets.agent(&a.id, &a.skills), &a.name, tint);
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.horizontal(|ui| {
                            ui.label(semi(&a.name).size(13.5).color(p.text));
                            widgets::pill(ui, &format!("@{}", a.id), p.muted, p.pressed);
                            match a.approval.as_deref() {
                                Some("manual") => {
                                    widgets::pill(
                                        ui,
                                        "Asks first",
                                        p.warning,
                                        crate::theme::soft(p.warning, 0x26),
                                    );
                                }
                                Some("auto") => {
                                    widgets::pill(ui, "Auto", p.accent_text, p.accent_soft);
                                }
                                _ => {}
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if a.id != "leader"
                                    && widgets::button(ui, Some(Icon::Edit), "Edit", false)
                                        .clicked()
                                {
                                    self.agent_edit = Some(AgentEdit {
                                        id: a.id.clone(),
                                        name: a.name.clone(),
                                        scope: a.scope.clone(),
                                        prompt: a.prompt.clone(),
                                        model: a.model.clone().unwrap_or_default(),
                                        skills: a.skills.join(", "),
                                        tools: if a.tools.is_empty() {
                                            "*".to_string()
                                        } else {
                                            a.tools.join(", ")
                                        },
                                        approval: a.approval.clone().unwrap_or_default(),
                                        is_new: false,
                                    });
                                }
                            });
                        });
                        if !a.scope.is_empty() {
                            ui.add(
                                egui::Label::new(RichText::new(&a.scope).size(12.5).color(p.muted))
                                    .wrap(),
                            );
                        }
                        if let Some(m) = &a.model {
                            ui.label(
                                RichText::new(format!("Model: {m}"))
                                    .size(12.0)
                                    .color(p.faint),
                            );
                        }
                        let tools = if a.tools.is_empty() {
                            "Tools: none (plans and reviews only)".to_string()
                        } else {
                            format!("Tools: {}", short(&a.tools.join(", "), 140))
                        };
                        ui.add(
                            egui::Label::new(RichText::new(tools).size(11.5).color(p.faint)).wrap(),
                        );
                        let apps: Vec<&String> = a
                            .skills
                            .iter()
                            .filter(|s| self.assets.app(s).is_some())
                            .collect();
                        if !apps.is_empty() {
                            ui.horizontal_wrapped(|ui| {
                                for s in apps {
                                    if let Some(t) = self.assets.app(s) {
                                        widgets::image(ui, t, 20.0).on_hover_text(s.as_str());
                                    }
                                }
                            });
                        }
                    });
                });
            });
            ui.add_space(8.0);
        }
    }

    fn tab_tools(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        section(
            ui,
            "Craft apps (Connectors)",
            "Every Craft app found on this computer can connect as an MCP server. Headless works on files without a window, Live drives an open window, Off disconnects the app. Craft Anything stays Off until you switch it on.",
        );
        let craft = self.craft.clone();
        let total = craft.len();
        let installed_n = craft.iter().filter(|c| c["installed"] == true).count();
        let connected_n = craft.iter().filter(|c| c["connected"] == true).count();
        let mut rescan = false;
        let mut switch_on: Vec<String> = Vec::new();
        card(ui, &p, false, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    semi(format!("{installed_n} of {total} Craft apps found"))
                        .size(13.5)
                        .color(p.text),
                );
                widgets::pill(
                    ui,
                    &format!("{connected_n} connected"),
                    p.success,
                    crate::theme::soft(p.success, 0x26),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::button(ui, Some(Icon::Refresh), "Rescan", false)
                        .on_hover_text("Look for the installed Craft apps again")
                        .clicked()
                    {
                        rescan = true;
                    }
                    if widgets::button(ui, Some(Icon::Plug), "Switch all on", false)
                        .on_hover_text("Set every installed app that is Off to Headless (Craft Anything stays as it is)")
                        .clicked()
                    {
                        switch_on = craft
                            .iter()
                            .filter(|c| c["installed"] == true && c["mode"] == "off" && c["id"] != "craft-anything")
                            .filter_map(|c| c["id"].as_str().map(str::to_string))
                            .collect();
                    }
                });
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new("Looked in the standard install folders (Program Files, Program Files\\ArtCraft, %LOCALAPPDATA%, your profile folder) and on PATH. An app installed somewhere else: set its variable (for example GRIDCRAFT_CLI) to the path of its command-line tool.")
                    .size(11.5)
                    .color(p.faint),
            );
        });
        ui.add_space(6.0);
        let mut ordered: Vec<&serde_json::Value> =
            craft.iter().filter(|c| c["installed"] == true).collect();
        let n_installed = ordered.len();
        ordered.extend(craft.iter().filter(|c| c["installed"] != true));
        let mut set_mode: Option<(String, &'static str)> = None;
        let mut test_app: Option<String> = None;
        let mut open_app: Option<String> = None;
        for (idx, c) in ordered.into_iter().enumerate() {
            if idx == n_installed {
                section(
                    ui,
                    "Not found on this computer",
                    "These apps are not installed, or they are in a folder WorkCraft does not look in.",
                );
            }
            let id = c["id"].as_str().unwrap_or_default();
            let name = c["name"].as_str().unwrap_or(id);
            let kind = c["kind"].as_str().unwrap_or("");
            let version = c["version"].as_str().unwrap_or("");
            let cli_path = c["cli"].as_str().unwrap_or("");
            let installed = c["installed"] == true;
            let live = c["live"] == true;
            let connected = c["connected"] == true;
            let supports_live = c["supports_live"] == true;
            let mode = c["mode"].as_str().unwrap_or("headless");
            let port = c["port"].as_u64();
            let tool_count = c["tool_count"].as_u64().unwrap_or(0);
            let lean_count = c["lean_tool_count"].as_u64().unwrap_or(0);
            let last_error = c["last_error"].as_str().filter(|s| !s.is_empty());
            card(ui, &p, connected, |ui| {
                ui.horizontal(|ui| {
                    if let Some(t) = self.assets.app(id) {
                        widgets::image(ui, t, 24.0);
                    } else {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
                        paint_icon(ui.painter(), r, Icon::Plug, p.muted);
                    }
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        let head = ui.horizontal_wrapped(|ui| {
                            ui.label(semi(name).size(13.5).color(p.text));
                            if !kind.is_empty() {
                                widgets::pill(ui, kind, p.muted, p.pressed);
                            }
                            if !version.is_empty() {
                                ui.label(RichText::new(format!("v{version}")).size(11.5).color(p.faint));
                            }
                            if let Some(pt) = port {
                                widgets::pill(ui, &format!(":{pt}"), p.faint, p.hover);
                            }
                        });
                        if !cli_path.is_empty() {
                            head.response.on_hover_text(cli_path);
                        }
                        let (dot_c, st) = if let Some(err) = last_error {
                            (p.danger, format!("Error: {}", short(err, 90)))
                        } else if connected && live {
                            (
                                p.success,
                                if tool_count == 0 { "Connected (live window)".to_string() } else { format!("Connected (live window) \u{00b7} {lean_count}/{tool_count} tools") },
                            )
                        } else if connected {
                            (
                                p.success,
                                if tool_count == 0 { "Connected (headless), tools load on first use".to_string() } else { format!("Connected (headless MCP) \u{00b7} {lean_count}/{tool_count} tools") },
                            )
                        } else if !installed {
                            (p.faint, "Not installed".to_string())
                        } else if mode == "off" {
                            (p.faint, "Off (disabled)".to_string())
                        } else if mode == "live" {
                            (
                                p.warning,
                                "Live mode selected \u{00b7} waiting for app window or write".to_string(),
                            )
                        } else {
                            (
                                p.accent,
                                if lean_count == 0 { "Available on demand (headless)".to_string() } else { format!("Available on demand (headless) \u{00b7} {lean_count} lean tools") },
                            )
                        };
                        status_line(ui, dot_c, &st);
                        if id == "craft-anything" {
                            ui.label(
                                RichText::new(
                                    "Runs with --clean-room and human inbox approval by default.",
                                )
                                .size(11.5)
                                .color(p.faint),
                            );
                        }
                    });
                });
                if installed {
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(32.0);
                        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                        for (m_id, m_lbl, tip) in [
                            (
                                "headless",
                                "Headless",
                                "Run headlessly on files without opening a window (default)",
                            ),
                            (
                                "live",
                                "Live",
                                "Drive an open desktop window over --control",
                            ),
                            ("off", "Off", "Disable this Craft app's MCP connector"),
                        ] {
                            if m_id == "live" && !supports_live {
                                continue;
                            }
                            if widgets::chip(ui, Icon::Plug, m_lbl, mode == m_id, tip).clicked()
                                && mode != m_id
                            {
                                set_mode = Some((id.to_string(), m_id));
                            }
                        }
                        ui.add_space(6.0);
                        if widgets::button(ui, None, "Test", false)
                            .on_hover_text(
                                "Spawn or probe this app's MCP server and verify tools/list",
                            )
                            .clicked()
                        {
                            test_app = Some(id.to_string());
                        }
                        if supports_live
                            && widgets::button(
                                ui,
                                Some(Icon::Monitor),
                                if live { "Live" } else { "Open window" },
                                false,
                            )
                            .on_hover_text(
                                "Launch the desktop window with --control and bridge live MCP",
                            )
                            .clicked()
                        {
                            open_app = Some(id.to_string());
                        }
                    });
                }
            });
            ui.add_space(6.0);
        }
        if rescan {
            self.bridge.run("craft", "craft.list", json!({}));
        }
        for app in switch_on {
            self.bridge.run(
                "craft-mode",
                "craft.mode",
                json!({ "app": app, "mode": "headless" }),
            );
        }
        if let Some((app, mode)) = set_mode {
            self.bridge.run(
                "craft-mode",
                "craft.mode",
                json!({ "app": app, "mode": mode }),
            );
        }
        if let Some(app) = test_app {
            self.bridge
                .run("craft-test", "craft.test", json!({ "app": app }));
        }
        if let Some(app) = open_app {
            self.blip.on_tool_start("open_craft_app", self.now);
            self.bridge
                .run("craft-open", "craft.open", json!({ "app": app }));
        }

        ui.add_space(10.0);
        section(
            ui,
            "Sandbox roots & tool catalog",
            "Headless Craft apps are confined to the WorkCraft workspace plus any extra folders you allow below.",
        );
        let full_catalog = self.config["full_tool_catalog"] == true;
        card(ui, &p, false, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(semi("Full Craft tool catalogs").size(13.5).color(p.text));
                    ui.label(
                        RichText::new(
                            "Off (recommended): expose curated lean profiles (\u{2264}16 tools/app) to save context tokens. On: expose every raw MCP tool.",
                        )
                        .size(12.0)
                        .color(p.muted),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::chip(
                        ui,
                        if full_catalog { Icon::Check } else { Icon::Sparkle },
                        if full_catalog { "Full" } else { "Lean" },
                        full_catalog,
                        "Toggle between lean tool profiles and full tool catalogs",
                    )
                    .clicked()
                    {
                        self.bridge.run(
                            "config-saved",
                            "config.set",
                            json!({ "full_tool_catalog": !full_catalog }),
                        );
                    }
                });
            });
            ui.add_space(8.0);
            let ws = paths::data_dir().join("workspace").display().to_string();
            ui.label(
                RichText::new(format!("Default workspace root: {ws}"))
                    .monospace()
                    .size(11.5)
                    .color(p.faint),
            );
            let mut roots: Vec<String> = self.config["extra_roots"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            let mut remove_idx: Option<usize> = None;
            for (idx, r) in roots.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(r).monospace().size(12.0).color(p.text));
                    if widgets::icon_button_sized(
                        ui,
                        Icon::Close,
                        "Remove allowed folder",
                        22.0,
                        false,
                    )
                    .clicked()
                    {
                        remove_idx = Some(idx);
                    }
                });
            }
            let mut changed = false;
            if let Some(idx) = remove_idx {
                roots.remove(idx);
                changed = true;
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 90.0).max(120.0);
                ui.allocate_ui(vec2(w, 32.0), |ui| {
                    input(
                        ui,
                        &mut self.extra_root_input,
                        "Add extra allowed folder path...",
                        false,
                    );
                });
                if widgets::button(ui, Some(Icon::Plus), "Add", false).clicked() {
                    let candidate = self.extra_root_input.trim().to_string();
                    if !candidate.is_empty() && !roots.contains(&candidate) {
                        roots.push(candidate);
                        self.extra_root_input.clear();
                        changed = true;
                    }
                }
            });
            if changed {
                self.bridge.run(
                    "config-saved",
                    "config.set",
                    json!({ "extra_roots": roots }),
                );
                self.bridge.run("mcp", "mcp.reload", Value::Null);
            }
        });
        ui.add_space(10.0);

        let file = paths::data_dir().join("mcp.json").display().to_string();
        section(
            ui,
            "MCP servers (mcp.json)",
            "Custom servers listed in mcp.json (the Claude Desktop format), plus the enabled Craft apps above.",
        );
        egui::Frame::new()
            .fill(p.surface)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(
                    egui::Label::new(RichText::new(&file).monospace().size(12.0).color(p.muted))
                        .wrap(),
                );
            });
        ui.add_space(10.0);
        match &self.mcp_status {
            Some(s) => {
                let servers: Vec<&str> = s["servers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                ui.label(
                    RichText::new(format!(
                        "{} connected \u{00b7} {} tools",
                        servers.len(),
                        s["tools"]
                    ))
                    .size(13.0)
                    .color(p.muted),
                );
                ui.add_space(6.0);
                for n in &servers {
                    card(ui, &p, false, |ui| {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(10.0, 18.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 4.0, p.success);
                            if let Some(t) = self.assets.app(n) {
                                widgets::image(ui, t, 20.0);
                            } else {
                                let (r, _) =
                                    ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                                paint_icon(ui.painter(), r, Icon::Plug, p.muted);
                            }
                            ui.label(semi(*n).size(13.5).color(p.text));
                        });
                    });
                    ui.add_space(6.0);
                }
                for w in s["warnings"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    egui::Frame::new()
                        .fill(p.danger_soft)
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.add(
                                egui::Label::new(RichText::new(w).size(12.5).color(p.danger))
                                    .wrap(),
                            );
                        });
                    ui.add_space(6.0);
                }
                if servers.is_empty() {
                    ui.label(
                        RichText::new("No server is connected.")
                            .size(13.0)
                            .color(p.faint),
                    );
                    ui.add_space(6.0);
                }
            }
            None => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(14.0).color(p.accent));
                    ui.label(RichText::new("Connecting...").color(p.faint));
                });
            }
        }
        ui.add_space(4.0);
        if widgets::button(ui, Some(Icon::Refresh), "Reload servers", false).clicked() {
            self.mcp_status = None;
            self.bridge.run("mcp", "mcp.reload", Value::Null);
        }
        section(
            ui,
            "Built-in tools",
            "Files and folders anywhere on this computer (read_file, write_file, edit_file, copy_file, list_dir, search_files, file_info), Office document builders (create_document .docx, create_spreadsheet .xlsx/.csv, create_presentation .pptx), web page reader (fetch_url), undo checkpoints, shared memory, skills, and the Craft apps.",
        );
    }

    fn tab_skills(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        section(
            ui,
            "Skills",
            "Instructions and domain workflows the agents can read when a task needs them. Create custom skills below or edit any existing skill.",
        );
        if self.skill_edit.is_none()
            && widgets::button(ui, Some(Icon::Plus), "New custom skill", true).clicked()
        {
            self.skill_edit = Some(SkillEdit {
                name: String::new(),
                description: String::new(),
                body: "# Skill Instructions\n\nDescribe the steps, guidelines, or examples the agent should follow when using this skill.\n".to_string(),
                is_new: true,
            });
        }

        let mut save_skill_clicked = false;
        let mut cancel_skill_clicked = false;
        let mut delete_skill_name: Option<String> = None;

        if let Some(se) = self.skill_edit.as_mut() {
            ui.add_space(8.0);
            card(ui, &p, true, |ui| {
                ui.label(
                    semi(if se.is_new {
                        "New custom skill"
                    } else {
                        "Edit skill"
                    })
                    .size(14.0)
                    .color(p.text),
                );
                field_label(ui, "Skill name (used for @skill:name)");
                input(ui, &mut se.name, "e.g. brand-voice", false);
                field_label(
                    ui,
                    "Short description (shown to the agent in the skill list)",
                );
                input(
                    ui,
                    &mut se.description,
                    "e.g. Guidelines and tone of voice for company documents and reports.",
                    false,
                );
                field_label(ui, "Skill instructions (Markdown saved in SKILL.md)");
                multiline_input(
                    ui,
                    &mut se.body,
                    "# Skill Instructions\n\nWrite detailed Markdown instructions...",
                    8,
                );
                ui.add_space(12.0);
                ui.horizontal_wrapped(|ui| {
                    if widgets::button(ui, Some(Icon::Check), "Save skill", true).clicked() {
                        save_skill_clicked = true;
                    }
                    if widgets::button(ui, None, "Cancel", false).clicked() {
                        cancel_skill_clicked = true;
                    }
                    if !se.is_new
                        && widgets::button(ui, Some(Icon::Trash), "Delete skill", false).clicked()
                    {
                        delete_skill_name = Some(se.name.clone());
                    }
                });
            });
            ui.add_space(10.0);
        }
        if cancel_skill_clicked {
            self.skill_edit = None;
        }
        if save_skill_clicked {
            self.save_skill();
        }
        if let Some(name) = delete_skill_name {
            self.remove_skill(&name);
        }

        ui.add_space(8.0);
        if self.skills.is_empty() {
            ui.label(RichText::new("No skills yet.").color(p.faint));
        }
        let skills_list = self.skills.clone();
        let mut edit_skill_item: Option<(String, String)> = None;
        for (n, d) in &skills_list {
            let is_editing = self
                .skill_edit
                .as_ref()
                .is_some_and(|e| !e.is_new && e.name == *n);
            card(ui, &p, is_editing, |ui| {
                ui.horizontal(|ui| {
                    if let Some(t) = self.assets.app(n) {
                        widgets::image(ui, t, 20.0);
                    }
                    ui.label(semi(n.as_str()).size(13.5).color(p.text));
                    widgets::pill(ui, &format!("@skill:{n}"), p.muted, p.pressed);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::button(ui, Some(Icon::Edit), "Edit", false).clicked() {
                            edit_skill_item = Some((n.clone(), d.clone()));
                        }
                    });
                });
                if !d.is_empty() {
                    ui.add_space(2.0);
                    ui.add(
                        egui::Label::new(RichText::new(short(d, 240)).size(12.5).color(p.muted))
                            .wrap(),
                    );
                }
            });
            ui.add_space(6.0);
        }
        if let Some((name, desc)) = edit_skill_item {
            self.open_skill_edit(&name, &desc);
        }
    }
}
