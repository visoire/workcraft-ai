//! The chat list on the left and the bar above the conversation (model picker, Single / Team switch).

use chrono::{Local, TimeZone};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, Stroke,
    StrokeKind, Vec2, pos2, vec2,
};

use crate::app::{App, SettingsTab};
use crate::theme::{pal, semi};
use crate::widgets::{self, Icon, icon_button, paint_icon, short};

/// Which date group a chat falls in, by its last change (ms since the epoch).
pub(crate) fn date_group(updated_ms: i64, now: chrono::DateTime<Local>) -> &'static str {
    let Some(t) = Local.timestamp_millis_opt(updated_ms).single() else {
        return "Older";
    };
    let days = (now.date_naive() - t.date_naive()).num_days();
    match days {
        i64::MIN..=0 => "Today",
        1 => "Yesterday",
        2..=7 => "Previous 7 days",
        8..=30 => "Previous 30 days",
        _ => "Older",
    }
}

use crate::providers::is_local;

pub(crate) fn split_model(m: &str) -> (&str, &str) {
    m.split_once(':').unwrap_or(("ollama", m))
}

impl App {
    /// Display name of a provider: the name saved in the settings, else the preset name.
    pub(crate) fn provider_name(&self, id: &str) -> String {
        let saved = self
            .provider_info
            .iter()
            .find(|p| p["id"] == id)
            .and_then(|p| p["name"].as_str());
        crate::providers::label(id, saved)
    }

    pub(crate) fn sidebar(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        // Brand row: the wordmark image carries both the W mark and "orkcraft".
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            if let Some((tex, aspect)) = self.assets.wordmark.as_ref() {
                let h = 22.0;
                let (rect, _) =
                    ui.allocate_exact_size(vec2((h * aspect).round(), h), Sense::hover());
                ui.painter().image(
                    tex.id(),
                    rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                ui.label(semi("WorkCraft").size(16.0).color(p.text));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, Icon::Sidebar, "Hide sidebar (Ctrl+Shift+S)").clicked() {
                    self.show_sidebar = false;
                    self.save_prefs();
                }
            });
        });
        ui.add_space(8.0);

        // New chat.
        let (rect, resp) = widgets::row(ui, false, 36.0);
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.0, p.border_strong),
            StrokeKind::Inside,
        );
        paint_icon(
            ui.painter(),
            Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(15.0)),
            Icon::Edit,
            p.text,
        );
        ui.painter().text(
            pos2(rect.left() + 34.0, rect.center().y),
            Align2::LEFT_CENTER,
            "New chat",
            FontId::proportional(14.0),
            p.text,
        );
        ui.painter().text(
            pos2(rect.right() - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            "Ctrl+Shift+O",
            FontId::proportional(11.0),
            if resp.hovered() {
                p.faint
            } else {
                Color32::TRANSPARENT
            },
        );
        if resp.clicked() {
            self.new_chat();
        }
        ui.add_space(8.0);

        // Search.
        egui::Frame::new()
            .fill(p.surface)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(egui::Margin::symmetric(8, 2))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                    paint_icon(ui.painter(), r, Icon::Search, p.faint);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("Search chats")
                            .frame(egui::Frame::NONE)
                            .align(Align2::LEFT_CENTER)
                            .min_size(vec2(0.0, 28.0))
                            .desired_width(ui.available_width() - 4.0),
                    );
                });
            });
        ui.add_space(10.0);

        let bottom_h = 44.0;
        let mut open = None;
        let mut delete = None;
        let mut ask_delete = None;
        let mut cancel_delete = false;
        let needle = self.search.trim().to_lowercase();
        let now = Local::now();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height((ui.available_height() - bottom_h).max(60.0))
            .show(ui, |ui| {
                let mut last_group = "";
                let mut any = false;
                for s in self
                    .sessions
                    .iter()
                    .filter(|s| needle.is_empty() || s.title.to_lowercase().contains(&needle))
                {
                    any = true;
                    let g = date_group(s.updated, now);
                    if g != last_group {
                        if !last_group.is_empty() {
                            ui.add_space(8.0);
                        }
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);
                            widgets::caption(ui, g);
                        });
                        last_group = g;
                    }
                    let selected = self.current.as_deref() == Some(s.id.as_str());
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                    let row_hovered = ui.rect_contains_pointer(rect);
                    if selected {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(8), p.pressed);
                    } else if row_hovered {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(8), p.hover);
                    }
                    let confirming = self.pending_delete.as_deref() == Some(s.id.as_str());
                    let show_tools = row_hovered || confirming || selected;
                    let right_w = if confirming {
                        60.0
                    } else if show_tools {
                        30.0
                    } else {
                        0.0
                    };
                    let live = self.running.contains(&s.id) || (selected && self.busy);
                    let unseen = !selected && self.unseen_done.contains(&s.id);
                    let mut x = rect.left() + 10.0;
                    if live {
                        widgets::dot(ui, pos2(x + 3.0, rect.center().y), p.accent, true);
                        x += 14.0;
                    } else if unseen {
                        widgets::dot(ui, pos2(x + 3.0, rect.center().y), p.success, false);
                        x += 14.0;
                    }
                    let title = if s.title.trim().is_empty() {
                        "New chat"
                    } else {
                        s.title.as_str()
                    };
                    let galley = ui.painter().layout_no_wrap(
                        title.to_string(),
                        FontId::proportional(13.5),
                        if selected { p.text } else { p.muted },
                    );
                    let clip = Rect::from_min_max(
                        pos2(x, rect.top()),
                        pos2(rect.right() - right_w - 6.0, rect.bottom()),
                    );
                    ui.painter().with_clip_rect(clip).galley(
                        pos2(x, rect.center().y - galley.size().y / 2.0),
                        galley.clone(),
                        p.text,
                    );
                    if galley.size().x > clip.width() {
                        resp.clone().on_hover_text(title);
                    }
                    if confirming {
                        let ok = Rect::from_center_size(
                            pos2(rect.right() - 44.0, rect.center().y),
                            Vec2::splat(24.0),
                        );
                        let no = Rect::from_center_size(
                            pos2(rect.right() - 16.0, rect.center().y),
                            Vec2::splat(24.0),
                        );
                        let r_ok = ui
                            .interact(ok, egui::Id::unique(("del-ok", &s.id)), Sense::click())
                            .on_hover_text("Delete this chat");
                        let r_no = ui
                            .interact(no, egui::Id::unique(("del-no", &s.id)), Sense::click())
                            .on_hover_text("Keep it");
                        if r_ok.hovered() {
                            ui.painter()
                                .rect_filled(ok, CornerRadius::same(6), p.danger_soft);
                        }
                        if r_no.hovered() {
                            ui.painter().rect_filled(no, CornerRadius::same(6), p.hover);
                        }
                        paint_icon(ui.painter(), ok.shrink(5.0), Icon::Check, p.danger);
                        paint_icon(ui.painter(), no.shrink(5.0), Icon::Close, p.muted);
                        if r_ok.clicked() {
                            delete = Some(s.id.clone());
                        } else if r_no.clicked() {
                            cancel_delete = true;
                        }
                    } else if show_tools {
                        let tr = Rect::from_center_size(
                            pos2(rect.right() - 16.0, rect.center().y),
                            Vec2::splat(24.0),
                        );
                        let r = ui
                            .interact(tr, egui::Id::unique(("del", &s.id)), Sense::click())
                            .on_hover_text("Delete");
                        if r.hovered() {
                            ui.painter()
                                .rect_filled(tr, CornerRadius::same(6), p.pressed);
                        }
                        paint_icon(
                            ui.painter(),
                            tr.shrink(5.0),
                            Icon::Trash,
                            if r.hovered() { p.danger } else { p.faint },
                        );
                        if r.clicked() {
                            ask_delete = Some(s.id.clone());
                        } else if resp.clicked() {
                            open = Some(s.id.clone());
                        }
                    } else if resp.clicked() {
                        open = Some(s.id.clone());
                    }
                }
                if !any {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(if needle.is_empty() {
                                "Your chats appear here."
                            } else {
                                "No chat matches."
                            })
                            .size(13.0)
                            .color(p.faint),
                        );
                    });
                }
            });
        if let Some(id) = ask_delete {
            self.pending_delete = Some(id);
        }
        if cancel_delete {
            self.pending_delete = None;
        }
        if let Some(id) = delete {
            self.pending_delete = None;
            self.delete_session(&id);
        }
        if let Some(id) = open {
            self.pending_delete = None;
            self.open(&id);
        }

        // Bottom: settings and theme.
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.horizontal(|ui| {
                let w = ui.available_width() - 64.0;
                let (rect, resp) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click());
                if resp.hovered() {
                    ui.painter()
                        .rect_filled(rect, CornerRadius::same(8), p.hover);
                }
                paint_icon(
                    ui.painter(),
                    Rect::from_center_size(
                        pos2(rect.left() + 18.0, rect.center().y),
                        Vec2::splat(15.0),
                    ),
                    Icon::Settings,
                    p.muted,
                );
                ui.painter().text(
                    pos2(rect.left() + 34.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    "Settings",
                    FontId::proportional(13.5),
                    p.text,
                );
                if resp.on_hover_text("Ctrl+,").clicked() {
                    self.open_settings(SettingsTab::General);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::theme_switch(ui, p.dark).clicked() {
                        let ctx = ui.ctx().clone();
                        self.toggle_theme(&ctx);
                    }
                });
            });
            ui.add_space(2.0);
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter()
                .hline(r.x_range(), r.center().y, Stroke::new(1.0, p.border));
        });
    }

    pub(crate) fn top_bar(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if !self.show_sidebar {
                if icon_button(ui, Icon::Sidebar, "Show sidebar (Ctrl+Shift+S)").clicked() {
                    self.show_sidebar = true;
                    self.save_prefs();
                }
                if icon_button(ui, Icon::Edit, "New chat (Ctrl+Shift+O)").clicked() {
                    self.new_chat();
                }
                ui.add_space(4.0);
            }
            self.model_picker(ui);
            ui.add_space(4.0);
            let sel = usize::from(self.mode_team);
            if let Some(i) = widgets::segmented(
                ui,
                &[(Some(Icon::Person), "Single"), (Some(Icon::Users), "Team")],
                sel,
                "mode",
            ) {
                self.set_mode(i == 1);
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if widgets::icon_button_sized(
                    ui,
                    Icon::Panel,
                    if self.show_activity {
                        "Hide activity"
                    } else {
                        "Show activity"
                    },
                    30.0,
                    self.show_activity,
                )
                .clicked()
                {
                    self.show_activity = !self.show_activity;
                    self.save_prefs();
                }
                if self.current.is_some() && !self.messages.is_empty() {
                    let exp_resp = widgets::icon_button_sized(
                        ui,
                        Icon::Download,
                        "Export or copy this chat",
                        30.0,
                        false,
                    );
                    let exp_id = egui::Popup::default_response_id(&exp_resp);
                    let mut exp_action: Option<(&str, bool)> = None;
                    egui::Popup::menu(&exp_resp)
                        .width(210.0)
                        .gap(6.0)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            ui.set_width(200.0);
                            widgets::caption(ui, "Export chat");
                            ui.add_space(4.0);
                            for (label, fmt, copy) in [
                                ("Save as Markdown (.md)", "md", false),
                                ("Save as Web page (.html)", "html", false),
                                ("Save as JSON (.json)", "json", false),
                                ("Save as plain text (.txt)", "txt", false),
                                ("Copy as Markdown", "md", true),
                            ] {
                                let (rect, r) = widgets::row(ui, false, 30.0);
                                paint_icon(
                                    ui.painter(),
                                    Rect::from_center_size(
                                        pos2(rect.left() + 14.0, rect.center().y),
                                        Vec2::splat(14.0),
                                    ),
                                    if copy { Icon::Copy } else { Icon::Download },
                                    p.muted,
                                );
                                ui.painter().text(
                                    pos2(rect.left() + 30.0, rect.center().y),
                                    Align2::LEFT_CENTER,
                                    label,
                                    FontId::proportional(13.0),
                                    p.text,
                                );
                                if r.clicked() {
                                    exp_action = Some((fmt, copy));
                                }
                            }
                        });
                    if let Some((fmt, copy)) = exp_action {
                        egui::Popup::close_id(ui.ctx(), exp_id);
                        self.export(fmt, copy);
                    }
                }
                let can_undo = self.checkpoints.values().any(|v| v["restored"] != true);
                if can_undo
                    && widgets::chip(
                        ui,
                        Icon::Undo,
                        "Undo",
                        false,
                        "Restore the files changed by the last reply",
                    )
                    .clicked()
                {
                    self.undo_last();
                }
                if self.busy {
                    let secs = self.turn_started.map_or(0.0, |t| self.now - t);
                    ui.add(egui::Spinner::new().size(14.0).color(p.accent));
                    ui.label(
                        RichText::new(format!("Working {}", fmt_secs(secs)))
                            .size(12.5)
                            .color(p.muted),
                    );
                } else if let Some(u) = &self.last_usage {
                    ui.label(RichText::new(u).size(12.0).color(p.faint))
                        .on_hover_text("Tokens and cost of the last reply");
                }
            });
        });
    }

    fn model_picker(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let model = self.model.clone();
        let (prov, name) = split_model(&model);
        let name = if name.is_empty() {
            "Choose a model"
        } else {
            name
        };
        let g = ui.painter().layout_no_wrap(
            short(name, 34),
            FontId::proportional(14.5),
            Color32::PLACEHOLDER,
        );
        let w = g.size().x + 64.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click());
        let popup_id = egui::Popup::default_response_id(&resp);
        if std::mem::take(&mut self.open_picker) {
            egui::Popup::open_id(ui.ctx(), popup_id);
        }
        let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
        if resp.hovered() || open {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(10), p.hover);
        }
        let logo =
            Rect::from_center_size(pos2(rect.left() + 19.0, rect.center().y), Vec2::splat(18.0));
        widgets::paint_provider_logo(
            ui,
            logo,
            self.assets.provider(prov),
            &self.provider_name(prov),
        );
        ui.painter().galley(
            pos2(rect.left() + 36.0, rect.center().y - g.size().y / 2.0),
            g,
            p.text,
        );
        paint_icon(
            ui.painter(),
            Rect::from_center_size(
                pos2(rect.right() - 14.0, rect.center().y),
                Vec2::splat(13.0),
            ),
            Icon::ChevronDown,
            p.muted,
        );
        let resp = resp.on_hover_text(self.provider_name(prov));

        let mut chosen = None;
        let mut manage = false;
        let mut refresh = false;
        egui::Popup::menu(&resp)
            .width(360.0)
            .gap(6.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_width(350.0);
                egui::Frame::new()
                    .fill(p.surface)
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(8, 0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                            paint_icon(ui.painter(), r, Icon::Search, p.faint);
                            let te = ui.add(
                                egui::TextEdit::singleline(&mut self.model_filter)
                                    .hint_text("Search models")
                                    .frame(egui::Frame::NONE)
                                    .min_size(vec2(0.0, 32.0))
                                    .align(Align2::LEFT_CENTER)
                                    .desired_width(ui.available_width()),
                            );
                            if ui.memory(|m| m.focused().is_none()) {
                                te.request_focus();
                            }
                        });
                    });
                ui.add_space(4.0);
                let needle = self.model_filter.trim().to_lowercase();
                egui::ScrollArea::vertical()
                    .max_height(380.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let mut provs: Vec<String> = self.providers.clone();
                        // Local first, then the cloud APIs.
                        provs.sort_by_key(|p| (!is_local(p), self.provider_name(p)));
                        let mut shown = 0;
                        for pv in provs {
                            let list: Vec<&String> = self
                                .models
                                .iter()
                                .filter(|m| split_model(m).0 == pv.as_str())
                                .filter(|m| needle.is_empty() || m.to_lowercase().contains(&needle))
                                .collect();
                            let err = self.model_errors.get(pv.as_str());
                            if list.is_empty() && (err.is_none() || !needle.is_empty()) {
                                continue;
                            }
                            shown += 1;
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.add_space(6.0);
                                widgets::provider_logo(
                                    ui,
                                    self.assets.provider(&pv),
                                    &self.provider_name(&pv),
                                    15.0,
                                );
                                ui.label(semi(self.provider_name(&pv)).size(12.5).color(p.muted));
                            });
                            if let Some(e) = err {
                                ui.horizontal(|ui| {
                                    ui.add_space(6.0);
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(short(
                                                &crate::providers::friendly_error(&pv, e),
                                                120,
                                            ))
                                            .size(12.0)
                                            .color(p.danger),
                                        )
                                        .wrap(),
                                    );
                                });
                            }
                            for m in list {
                                let selected = *m == self.model;
                                let (rect, r) = widgets::row(ui, selected, 32.0);
                                let clip = Rect::from_min_max(
                                    rect.min,
                                    pos2(rect.right() - 30.0, rect.bottom()),
                                );
                                ui.painter().with_clip_rect(clip).text(
                                    pos2(rect.left() + 12.0, rect.center().y),
                                    Align2::LEFT_CENTER,
                                    split_model(m).1,
                                    FontId::proportional(13.5),
                                    p.text,
                                );
                                if selected {
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
                                if r.clicked() {
                                    chosen = Some(m.clone());
                                }
                            }
                        }
                        if shown == 0 {
                            ui.add_space(8.0);
                            let msg = if self.models.is_empty() && self.model_errors.is_empty() {
                                "Loading models..."
                            } else {
                                "No model matches."
                            };
                            ui.label(RichText::new(msg).size(13.0).color(p.faint));
                        }
                    });
                ui.add_space(6.0);
                let (r, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter()
                    .hline(r.x_range(), r.center().y, Stroke::new(1.0, p.border));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if widgets::button(ui, Some(Icon::Plus), "Add a provider", false).clicked() {
                        manage = true;
                    }
                    if widgets::button(ui, Some(Icon::Refresh), "Refresh", false).clicked() {
                        refresh = true;
                    }
                });
            });
        if refresh {
            self.bridge
                .run("models", "model.list", serde_json::Value::Null);
        }
        if let Some(m) = chosen {
            self.model_filter.clear();
            self.set_model(m);
            egui::Popup::close_id(ui.ctx(), popup_id);
        }
        if manage {
            egui::Popup::close_id(ui.ctx(), popup_id);
            self.open_settings(SettingsTab::Providers);
        }
    }
}

pub(crate) fn fmt_secs(s: f64) -> String {
    let s = s.max(0.0) as u64;
    if s < 60 {
        format!("{s}s")
    } else {
        format!("{}m {:02}s", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_group_by_day() {
        let now = Local::now();
        let ms = now.timestamp_millis();
        assert_eq!(date_group(ms, now), "Today");
        assert_eq!(date_group(ms - 86_400_000, now), "Yesterday");
        assert_eq!(date_group(ms - 4 * 86_400_000, now), "Previous 7 days");
        assert_eq!(date_group(ms - 20 * 86_400_000, now), "Previous 30 days");
        assert_eq!(date_group(0, now), "Older");
    }

    #[test]
    fn models_split_and_label() {
        assert_eq!(split_model("ollama:qwen3:4b"), ("ollama", "qwen3:4b"));
        assert_eq!(fmt_secs(75.2), "1m 15s");
    }
}
