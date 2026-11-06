//! The conversation: messages in a centred column, tool calls as cards, thinking as a collapsible block,
//! and the composer with approvals above it.

use std::collections::HashMap;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, Pos2, Rect, RichText,
    Sense, Stroke, StrokeKind, TextWrapMode, Vec2, pos2, vec2,
};
use workcraft_core::{Message, Role, ToolCall};

use crate::app::{Action, App};
use crate::markdown::{Block, Span, parse_blocks};
use crate::theme::{Palette, agent_color, medium, pal, semi, soft};
use crate::tree::{Step, visible_path};
use crate::view_side::fmt_secs;
use crate::widgets::{self, Icon, paint_icon, short};

/// Width of the text column, as in Open WebUI and LibreChat.
const COLUMN: f32 = 780.0;

/// Lay out `add` in a column of at most [`COLUMN`] points, centred.
pub(crate) fn column<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let avail = ui.available_width();
    // Keep at least 24 points from the panel edges and the scroll bar.
    let w = (avail - 48.0).clamp(200.0, COLUMN);
    let pad = ((avail - w) / 2.0).max(0.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(pad);
        ui.vertical(|ui| {
            ui.set_width(w);
            ui.spacing_mut().item_spacing.x = 8.0;
            add(ui)
        })
        .inner
    })
    .inner
}

/// `server__tool` -> ("server", "tool"); a built-in tool has no server.
pub(crate) fn split_tool(name: &str) -> (&str, &str) {
    name.split_once("__").unwrap_or(("", name))
}

pub(crate) fn tool_title(name: &str) -> String {
    match split_tool(name) {
        ("", t) => t.replace('_', " "),
        (s, t) => format!("{s} \u{00b7} {}", t.replace('_', " ")),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallState {
    Running,
    Waiting,
    Ok,
    Failed,
}

impl App {
    pub(crate) fn chat(&mut self, ui: &mut egui::Ui) {
        let path = visible_path(&self.messages, &self.choice);
        let by_id: HashMap<&str, &Message> =
            self.messages.iter().map(|m| (m.id.as_str(), m)).collect();
        // Results and image attachments of tool calls, so a call card can show its output and previews.
        let results: HashMap<&str, &str> = self
            .messages
            .iter()
            .filter(|m| m.role == Role::Tool)
            .filter_map(|m| m.tool_call_id.as_deref().map(|id| (id, m.content.as_str())))
            .collect();
        let tool_atts: HashMap<&str, &[workcraft_core::Attachment]> = self
            .messages
            .iter()
            .filter(|m| m.role == Role::Tool && !m.attachments.is_empty())
            .filter_map(|m| {
                m.tool_call_id
                    .as_deref()
                    .map(|id| (id, m.attachments.as_slice()))
            })
            .collect();
        let mut actions: Vec<Action> = Vec::new();
        let mut blip_clicked = false;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                autoscroll_tick(&self.autoscroll, ui);
                ui.add_space(12.0);
                column(ui, |ui| {
                    let visible: Vec<&Step> = path
                        .iter()
                        .filter(|s| {
                            by_id
                                .get(s.id.as_str())
                                .is_some_and(|m| m.role != Role::Tool && m.role != Role::System)
                        })
                        .collect();
                    if visible.is_empty() && !self.busy {
                        blip_clicked = self.empty_state(ui, &mut actions);
                    }
                    let last = visible.last().map(|s| s.id.clone());
                    let mut turn_id: Option<String> = None;
                    // The agent of the message above, so a run of steps by one agent shares one header.
                    let mut prev_agent: Option<Option<String>> = None;
                    for (idx, step) in visible.iter().enumerate() {
                        if let Some(m) = by_id.get(step.id.as_str()) {
                            if m.role == Role::User {
                                turn_id = Some(m.id.clone());
                            }
                            let next_is_user = visible
                                .get(idx + 1)
                                .and_then(|ns| by_id.get(ns.id.as_str()))
                                .is_none_or(|nm| nm.role == Role::User);
                            let is_last = last.as_deref() == Some(step.id.as_str());
                            let show_turn_end = if m.role != Role::User && next_is_user {
                                turn_id.as_deref()
                            } else {
                                None
                            };
                            let continues = m.role != Role::User
                                && prev_agent.as_ref().is_some_and(|a| *a == m.agent);
                            prev_agent = (m.role != Role::User).then(|| m.agent.clone());
                            self.message_view(
                                ui,
                                step,
                                m,
                                &results,
                                &tool_atts,
                                is_last,
                                show_turn_end,
                                continues,
                                &mut actions,
                            );
                        }
                    }
                    if self.busy {
                        self.live_view(ui, &by_id, &mut actions);
                    }
                    ui.add_space(24.0);
                });
            });
        if blip_clicked {
            self.blip.on_click(self.now);
        }
        let ctx = ui.ctx().clone();
        self.apply(actions, &ctx);
    }

    fn empty_state(&self, ui: &mut egui::Ui, out: &mut Vec<Action>) -> bool {
        let p = pal(ui);
        let h = ui.available_height();
        ui.add_space((h * 0.16).clamp(24.0, 120.0));
        let mut clicked = false;
        ui.vertical_centered(|ui| {
            let acc = if self.mode_team {
                crate::blip::BlipAccessory::Crown
            } else {
                crate::blip::BlipAccessory::Headset
            };
            clicked = crate::blip::paint_mascot(ui, &self.blip, 72.0, Some(acc), self.now);
            ui.add_space(10.0);
            let title = if self.mode_team {
                "What should the team work on?"
            } else {
                "How can I help you today?"
            };
            ui.label(semi(title).size(26.0).color(p.text));
            ui.add_space(4.0);
            let sub = if self.mode_team {
                "A leader plans the work, specialists do it in the Craft apps, and you approve what changes."
            } else {
                "Ask a question, use @mentions or /commands, or drop files on the chat box below."
            };
            ui.label(RichText::new(sub).size(14.0).color(p.muted));
        });
        ui.add_space(28.0);
        let ideas: [(&str, &str); 4] = if self.mode_team {
            [
                (
                    "Launch kit",
                    "Write a product launch brief, then a 5 slide deck and a short social video plan from it.",
                ),
                (
                    "Photo story",
                    "Pick the best three photos in my workspace, grade them consistently and lay them out on one page.",
                ),
                (
                    "Report",
                    "Turn the notes in my workspace into a formatted report with a summary table.",
                ),
                (
                    "Podcast edit",
                    "Plan the edit of a 20 minute interview: cuts, music bed and a chapter list.",
                ),
            ]
        } else {
            [
                (
                    "Explain",
                    "Explain how Single and Team mode differ in WorkCraft.",
                ),
                (
                    "Write",
                    "Draft a friendly email that announces our new release.",
                ),
                (
                    "Code",
                    "Write a Rust function that groups dates into Today, Yesterday and older.",
                ),
                ("Plan", "Make a one week plan to learn photo retouching."),
            ]
        };
        let gap = 10.0;
        let w = ((ui.available_width() - gap) / 2.0).floor();
        for pair in ideas.chunks(2) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (title, prompt) in pair {
                    let (slot, resp) = ui.allocate_exact_size(vec2(w, 64.0), Sense::click());
                    // Hover fades in and the card lifts two points: small, but it makes the cards feel alive.
                    let t = ui
                        .ctx()
                        .animate_bool_with_time(resp.id, resp.hovered(), 0.14);
                    let rect = slot.translate(vec2(0.0, -2.0 * t));
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(12),
                        p.hover.gamma_multiply(t),
                    );
                    ui.painter().rect_stroke(
                        rect,
                        CornerRadius::same(12),
                        Stroke::new(1.0, p.border.lerp_to_gamma(p.border_strong, t)),
                        StrokeKind::Inside,
                    );
                    if resp.hovered() {
                        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
                    }
                    ui.painter().text(
                        pos2(rect.left() + 14.0, rect.top() + 18.0),
                        Align2::LEFT_CENTER,
                        *title,
                        FontId::proportional(13.5),
                        p.text,
                    );
                    let g = ui.painter().layout(
                        short(prompt, 70),
                        FontId::proportional(12.5),
                        p.faint,
                        w - 28.0,
                    );
                    ui.painter().with_clip_rect(rect.shrink(2.0)).galley(
                        pos2(rect.left() + 14.0, rect.top() + 30.0),
                        g,
                        p.faint,
                    );
                    if resp.clicked() {
                        out.push(Action::Prompt((*prompt).to_string()));
                    }
                }
            });
            ui.add_space(gap);
        }
        // The Craft apps found on this computer, so a new chat shows what it can reach.
        let apps: Vec<(&str, &str)> = self
            .craft
            .iter()
            .filter(|c| c["installed"] == true)
            .filter_map(|c| Some((c["id"].as_str()?, c["name"].as_str()?)))
            .collect();
        if !apps.is_empty() {
            ui.add_space(6.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{} Craft apps found. Type @ to pick one.",
                        apps.len()
                    ))
                    .size(12.5)
                    .color(p.faint),
                );
                ui.add_space(6.0);
                let size = 24.0;
                let step = size + 8.0;
                let total = step * apps.len() as f32 - 8.0;
                let (strip, _) = ui.allocate_exact_size(
                    vec2(total.min(ui.available_width()), size),
                    Sense::hover(),
                );
                for (i, (id, name)) in apps.iter().enumerate() {
                    let r = Rect::from_min_size(
                        pos2(strip.left() + step * i as f32, strip.top()),
                        Vec2::splat(size),
                    );
                    if r.right() > strip.right() + 1.0 {
                        break;
                    }
                    let resp =
                        ui.interact(r, ui.scope_id().with(("app-strip", *id)), Sense::hover());
                    let t = ui
                        .ctx()
                        .animate_bool_with_time(resp.id, resp.hovered(), 0.12);
                    let r = r.expand(2.0 * t);
                    if let Some(tex) = self.assets.app(id) {
                        ui.painter().image(
                            tex.id(),
                            r,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE.gamma_multiply(0.72 + 0.28 * t),
                        );
                    }
                    resp.on_hover_text(*name);
                }
            });
        }
        clicked
    }

    fn call_state(
        &self,
        id: &str,
        results: &HashMap<&str, &str>,
    ) -> (CallState, Option<String>, Option<f64>) {
        if let Some(t) = self.tools.iter().find(|t| t.id == id) {
            let dur = t.finished.map(|f| f - t.started);
            return match &t.result {
                Some((ok, out)) => (
                    if *ok {
                        CallState::Ok
                    } else {
                        CallState::Failed
                    },
                    Some(out.clone()),
                    dur,
                ),
                None if self
                    .approvals
                    .iter()
                    .any(|a| a.tool == t.name && a.agent == t.agent) =>
                {
                    (CallState::Waiting, None, None)
                }
                None => (CallState::Running, None, None),
            };
        }
        match results.get(id) {
            Some(out) => {
                let failed = out.starts_with("error") || out.starts_with("Error");
                (
                    if failed {
                        CallState::Failed
                    } else {
                        CallState::Ok
                    },
                    Some((*out).to_string()),
                    None,
                )
            }
            None => (CallState::Ok, None, None),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn message_view(
        &self,
        ui: &mut egui::Ui,
        step: &Step,
        msg: &Message,
        results: &HashMap<&str, &str>,
        tool_atts: &HashMap<&str, &[workcraft_core::Attachment]>,
        is_last: bool,
        turn_end: Option<&str>,
        continues: bool,
        out: &mut Vec<Action>,
    ) {
        let p = pal(ui);
        let top = ui.cursor().top();
        let left = ui.max_rect().left();
        let right = ui.max_rect().right();
        ui.add_space(if continues { 2.0 } else { 10.0 });
        let (user_clean, user_files) = if msg.role == Role::User {
            crate::attach::split_message(&msg.content)
        } else {
            (String::new(), Vec::new())
        };
        match msg.role {
            Role::User => {
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    let maxw = (ui.available_width() * 0.78).max(160.0);
                    egui::Frame::new()
                        .fill(p.surface)
                        .corner_radius(CornerRadius::same(18))
                        .inner_margin(Margin::symmetric(16, 10))
                        .show(ui, |ui| {
                            ui.set_max_width(maxw);
                            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                if !user_clean.is_empty() {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&user_clean)
                                                .size(14.5)
                                                .color(p.text)
                                                .line_height(Some(22.0)),
                                        )
                                        .selectable(true)
                                        .wrap(),
                                    );
                                }
                                if !msg.attachments.is_empty() || !user_files.is_empty() {
                                    if !user_clean.is_empty() {
                                        ui.add_space(6.0);
                                    }
                                    ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                                        for a in &msg.attachments {
                                            file_badge(ui, &p, "IMG", &a.name, "");
                                        }
                                        for f in &user_files {
                                            file_badge(ui, &p, &f.kind, &f.name, &f.size);
                                        }
                                    });
                                }
                            });
                        });
                });
            }
            _ => {
                ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                    let agent = msg.agent.as_deref();
                    match agent {
                        _ if continues => {
                            // Same agent as the message above: no second avatar and name.
                            ui.add_space(30.0);
                        }
                        None => {
                            let st = if is_last {
                                self.blip.active_state(self.now)
                            } else {
                                crate::blip::BlipState::Idle
                            };
                            crate::blip::paint_avatar(ui, 30.0, st, self.now);
                        }
                        Some(a) => {
                            let color = agent_color(a, p.dark);
                            let tex = self.assets.agent(a, self.agent_skills(a));
                            widgets::avatar(ui, 30.0, tex, &self.agent_name(agent), color);
                        }
                    }
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        if !continues {
                            ui.horizontal(|ui| {
                                ui.label(semi(self.agent_name(agent)).size(14.0).color(p.text));
                                if let Some(m) = &msg.model {
                                    ui.label(RichText::new(&m.model).size(12.0).color(p.faint));
                                }
                            });
                            ui.add_space(4.0);
                        }
                        if let Some(r) = msg.reasoning.as_deref().filter(|r| !r.trim().is_empty()) {
                            thought_block(ui, &msg.id, r, false);
                        }
                        markdown(ui, &msg.content, &msg.id, self, out);
                        for c in &msg.tool_calls {
                            let (state, output, dur) = self.call_state(&c.id, results);
                            let atts = tool_atts.get(c.id.as_str()).copied().unwrap_or(&[]);
                            self.tool_card(ui, c, state, output.as_deref(), dur, atts, out);
                        }
                        if let Some(tid) = turn_end
                            && let Some(cp) = self.checkpoints.get(tid)
                            && cp["files"].as_array().is_some_and(|f| !f.is_empty())
                        {
                            ui.add_space(4.0);
                            turn_checkpoint_bar(ui, &p, tid, cp, out);
                        }
                    });
                });
            }
        }
        // Actions under the message: visible on hover and for the last message.
        let copy_text = if msg.role == Role::User && !user_clean.is_empty() {
            &user_clean
        } else {
            &msg.content
        };
        // A step that only called tools needs no row of buttons of its own; the last message of the answer has it.
        let quiet = msg.role != Role::User
            && !is_last
            && turn_end.is_none()
            && msg.content.trim().is_empty();
        let row_h = if quiet { 4.0 } else { 28.0 };
        let block = Rect::from_min_max(pos2(left, top), pos2(right, ui.cursor().top() + row_h));
        let show = is_last || ui.rect_contains_pointer(block);
        let layout = if msg.role == Role::User {
            Layout::right_to_left(Align::Center)
        } else {
            Layout::left_to_right(Align::Center)
        };
        let (row_rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
        // The version switcher of a branched message always shows; copy and edit on hover.
        if show || step.count > 1 {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(row_rect).layout(layout));
            let ui = &mut child;
            ui.spacing_mut().item_spacing.x = 4.0;
            if msg.role != Role::User {
                ui.add_space(38.0);
            }
            let key = format!("msg-{}", msg.id);
            let copied = self.just_copied(&key);
            if show
                && widgets::icon_button_sized(
                    ui,
                    if copied { Icon::Check } else { Icon::Copy },
                    if copied { "Copied" } else { "Copy" },
                    26.0,
                    false,
                )
                .clicked()
            {
                out.push(Action::Copy {
                    key,
                    text: copy_text.clone(),
                });
            }
            if show
                && msg.role != Role::User
                && let Some(tid) = turn_end
                && !self.busy
                && let Some(q) = self
                    .messages
                    .iter()
                    .find(|m| m.id == tid && m.attachments.is_empty())
                && crate::attach::split_message(&q.content).1.is_empty()
                && widgets::icon_button_sized(
                    ui,
                    Icon::Refresh,
                    "Regenerate (asks again as a new version)",
                    26.0,
                    false,
                )
                .clicked()
            {
                out.push(Action::Regenerate {
                    from: q.parent.clone(),
                    text: crate::attach::split_message(&q.content).0,
                });
            }
            if show
                && msg.role == Role::User
                && let Some(parent) = &msg.parent
                && widgets::icon_button_sized(ui, Icon::Edit, "Edit (starts a branch)", 26.0, false)
                    .clicked()
            {
                out.push(Action::Branch {
                    from: Some(parent.clone()),
                    text: copy_text.clone(),
                });
            }
            if show
                && msg.role == Role::User
                && msg.parent.is_none()
                && widgets::icon_button_sized(ui, Icon::Edit, "Edit (starts a branch)", 26.0, false)
                    .clicked()
            {
                out.push(Action::Branch {
                    from: None,
                    text: copy_text.clone(),
                });
            }
            if step.count > 1 {
                let parent_key = step.parent.clone().unwrap_or_default();
                let (prev, next) = if msg.role == Role::User {
                    (Icon::ChevronRight, Icon::ChevronLeft)
                } else {
                    (Icon::ChevronLeft, Icon::ChevronRight)
                };
                // In a right-to-left row the first button drawn is the rightmost.
                let first_is_next = msg.role == Role::User;
                let mut nav = |ui: &mut egui::Ui, forward: bool, icon: Icon| {
                    let ok = if forward {
                        step.index + 1 < step.count
                    } else {
                        step.index > 0
                    };
                    if widgets::icon_button_sized(
                        ui,
                        icon,
                        if forward {
                            "Next version"
                        } else {
                            "Previous version"
                        },
                        24.0,
                        false,
                    )
                    .clicked()
                        && ok
                    {
                        let i = if forward {
                            step.index + 1
                        } else {
                            step.index - 1
                        };
                        out.push(Action::Choose {
                            parent_key: parent_key.clone(),
                            id: step.siblings[i].clone(),
                        });
                    }
                };
                if first_is_next {
                    nav(ui, true, prev);
                    ui.label(
                        RichText::new(format!("{}/{}", step.index + 1, step.count))
                            .size(12.0)
                            .color(p.faint),
                    );
                    nav(ui, false, next);
                } else {
                    nav(ui, false, prev);
                    ui.label(
                        RichText::new(format!("{}/{}", step.index + 1, step.count))
                            .size(12.0)
                            .color(p.faint),
                    );
                    nav(ui, true, next);
                }
            }
        }
    }

    /// The answer while it streams: thinking, text, and tool calls not yet in a message.
    fn live_view(&self, ui: &mut egui::Ui, by_id: &HashMap<&str, &Message>, out: &mut Vec<Action>) {
        let p = pal(ui);
        ui.add_space(10.0);
        let known: std::collections::HashSet<&str> = by_id
            .values()
            .flat_map(|m| m.tool_calls.iter().map(|c| c.id.as_str()))
            .collect();
        let speaker = if self.mode_team { Some("leader") } else { None };
        ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
            crate::blip::paint_avatar(ui, 30.0, self.blip.active_state(self.now), self.now);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(semi(self.agent_name(speaker)).size(14.0).color(p.text));
                    if let Some(l) = self.live.get(speaker.unwrap_or(""))
                        && !l.detail.is_empty()
                        && self.mode_team
                    {
                        ui.label(
                            RichText::new(short(&l.detail, 50))
                                .size(12.0)
                                .color(p.faint),
                        );
                    }
                });
                let text: String = self
                    .stream
                    .iter()
                    .filter(|(a, _)| a.is_empty() || *a == "leader")
                    .map(|(_, t)| t.as_str())
                    .collect();
                if !self.reasoning.is_empty() {
                    thought_block(ui, "live", &self.reasoning, text.is_empty());
                }
                if !text.is_empty() {
                    markdown(ui, &text, "live", self, out);
                }
                for t in self.tools.iter().filter(|t| !known.contains(t.id.as_str())) {
                    let call = ToolCall {
                        id: t.id.clone(),
                        name: t.name.clone(),
                        arguments: serde_json::from_str(&t.args).unwrap_or_default(),
                    };
                    let (state, output, dur) = self.call_state(&t.id, &HashMap::new());
                    self.tool_card(ui, &call, state, output.as_deref(), dur, &[], out);
                }
                if text.is_empty() && self.reasoning.is_empty() {
                    typing_dots(ui, &p);
                }
            });
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn tool_card(
        &self,
        ui: &mut egui::Ui,
        call: &ToolCall,
        state: CallState,
        output: Option<&str>,
        dur: Option<f64>,
        atts: &[workcraft_core::Attachment],
        out: &mut Vec<Action>,
    ) {
        let p = pal(ui);
        ui.add_space(4.0);
        let id = ui.make_persistent_id(("tool", &call.id));
        let mut cs =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
        let border = match state {
            CallState::Waiting => soft(p.warning, 0x80),
            CallState::Failed => soft(p.danger, 0x70),
            _ => p.border,
        };
        let row_meta = self.tools.iter().find(|t| t.id == call.id);
        let craft_app = {
            let (srv, _) = split_tool(&call.name);
            if !srv.is_empty() {
                Some(srv)
            } else if call.name.starts_with("craft_") {
                call.arguments
                    .get("app")
                    .or_else(|| call.arguments.get("to_app"))
                    .and_then(serde_json::Value::as_str)
            } else {
                None
            }
        };
        let mode_badge = row_meta.and_then(|r| r.mode.as_deref()).or_else(|| {
            let app_id = craft_app?;
            let c = self
                .craft
                .iter()
                .find(|c| c["id"].as_str() == Some(app_id))?;
            if c["live"] == true {
                Some("live")
            } else if c["connected"] == true {
                Some("headless")
            } else {
                None
            }
        });
        egui::Frame::new()
            .fill(if p.dark { p.raised } else { p.bg })
            .stroke(Stroke::new(1.0, border))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                if resp.clicked() {
                    cs.toggle(ui);
                }
                let mut x = rect.left();
                let icon_r = Rect::from_min_size(pos2(x, rect.center().y - 9.0), Vec2::splat(18.0));
                match craft_app.and_then(|a| self.assets.app(a)) {
                    Some(t) => {
                        ui.painter().image(
                            t.id(),
                            icon_r,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    _ => paint_icon(ui.painter(), icon_r.shrink(1.0), Icon::Terminal, p.muted),
                }
                x += 26.0;
                let verb = match state {
                    CallState::Running => "Running",
                    CallState::Waiting => "Wants to run",
                    CallState::Ok => "Used",
                    CallState::Failed => "Failed",
                };
                let g1 = ui.painter().layout_no_wrap(
                    format!("{verb} "),
                    FontId::proportional(13.0),
                    p.muted,
                );
                let w1 = g1.size().x;
                ui.painter()
                    .galley(pos2(x, rect.center().y - g1.size().y / 2.0), g1, p.muted);
                let g2 = ui.painter().layout_no_wrap(
                    short(&tool_title(&call.name), 44),
                    FontId::proportional(13.0),
                    p.text,
                );
                let w2 = g2.size().x;
                ui.painter().galley(
                    pos2(x + w1, rect.center().y - g2.size().y / 2.0),
                    g2,
                    p.text,
                );
                if let Some(m) = mode_badge {
                    let badge_x = x + w1 + w2 + 8.0;
                    if badge_x + 56.0 < rect.right() - 48.0 {
                        let (fg, bg) = if m == "live" {
                            (p.success, soft(p.success, 0x24))
                        } else {
                            (p.muted, p.pressed)
                        };
                        let gm = ui.painter().layout_no_wrap(
                            m.to_string(),
                            FontId::proportional(10.5),
                            fg,
                        );
                        let br = Rect::from_min_size(
                            pos2(badge_x, rect.center().y - 9.0),
                            vec2(gm.size().x + 12.0, 18.0),
                        );
                        ui.painter().rect_filled(br, CornerRadius::same(9), bg);
                        ui.painter().galley(
                            pos2(br.left() + 6.0, br.center().y - gm.size().y / 2.0),
                            gm,
                            fg,
                        );
                    }
                }
                // Right side: status and chevron.
                let chev = Rect::from_center_size(
                    pos2(rect.right() - 8.0, rect.center().y),
                    Vec2::splat(12.0),
                );
                let open = cs.is_open();
                paint_icon(
                    ui.painter(),
                    chev,
                    if open {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    },
                    p.faint,
                );
                let sx = rect.right() - 26.0;
                match state {
                    CallState::Running => {
                        egui::Spinner::new().size(13.0).color(p.accent).paint_at(
                            ui,
                            Rect::from_center_size(
                                pos2(sx - 4.0, rect.center().y),
                                Vec2::splat(13.0),
                            ),
                        );
                    }
                    CallState::Waiting => {
                        widgets::dot(ui, pos2(sx - 4.0, rect.center().y), p.warning, true);
                    }
                    CallState::Ok => {
                        let label = dur.map(fmt_secs).unwrap_or_default();
                        if !label.is_empty() {
                            ui.painter().text(
                                pos2(sx - 14.0, rect.center().y),
                                Align2::RIGHT_CENTER,
                                label,
                                FontId::proportional(11.5),
                                p.faint,
                            );
                        }
                        paint_icon(
                            ui.painter(),
                            Rect::from_center_size(
                                pos2(sx - 4.0, rect.center().y),
                                Vec2::splat(13.0),
                            ),
                            Icon::Check,
                            p.success,
                        );
                    }
                    CallState::Failed => {
                        paint_icon(
                            ui.painter(),
                            Rect::from_center_size(
                                pos2(sx - 4.0, rect.center().y),
                                Vec2::splat(12.0),
                            ),
                            Icon::Close,
                            p.danger,
                        );
                    }
                }
                // Inline image thumbnails from rendered/saved tool artifacts or persisted attachments.
                let artifact_paths: &[String] = row_meta.map_or(&[], |r| r.artifacts.as_slice());
                if !artifact_paths.is_empty() || !atts.is_empty() {
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                        let ctx = ui.ctx().clone();
                        if !artifact_paths.is_empty() {
                            for path in artifact_paths {
                                if let Some((tex, aspect)) = self.thumb_for_path(&ctx, path) {
                                    let h = 132.0;
                                    let w = (h * aspect).clamp(64.0, 280.0);
                                    let (tr, tresp) =
                                        ui.allocate_exact_size(vec2(w, h), Sense::click());
                                    ui.painter()
                                        .rect_filled(tr, CornerRadius::same(8), p.code_bg);
                                    ui.painter().image(
                                        tex.id(),
                                        tr.shrink(2.0),
                                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                        Color32::WHITE,
                                    );
                                    ui.painter().rect_stroke(
                                        tr,
                                        CornerRadius::same(8),
                                        Stroke::new(1.0, p.border),
                                        StrokeKind::Inside,
                                    );
                                    if tresp
                                        .on_hover_text(format!("{path}\nClick to open"))
                                        .clicked()
                                    {
                                        out.push(Action::OpenPath(path.clone()));
                                    }
                                }
                            }
                        } else {
                            for (idx, att) in atts.iter().enumerate() {
                                if !att.mime.starts_with("image/") {
                                    continue;
                                }
                                let key = format!("{}:{idx}", call.id);
                                if let Some((tex, aspect)) =
                                    self.thumb_for_b64(&ctx, &key, &att.data)
                                {
                                    let h = 132.0;
                                    let w = (h * aspect).clamp(64.0, 280.0);
                                    let (tr, tresp) =
                                        ui.allocate_exact_size(vec2(w, h), Sense::hover());
                                    ui.painter()
                                        .rect_filled(tr, CornerRadius::same(8), p.code_bg);
                                    ui.painter().image(
                                        tex.id(),
                                        tr.shrink(2.0),
                                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                        Color32::WHITE,
                                    );
                                    ui.painter().rect_stroke(
                                        tr,
                                        CornerRadius::same(8),
                                        Stroke::new(1.0, p.border),
                                        StrokeKind::Inside,
                                    );
                                    tresp.on_hover_text(&att.name);
                                }
                            }
                        }
                    });
                }
                cs.show_body_unindented(ui, |ui| {
                    ui.add_space(4.0);
                    let args = serde_json::to_string_pretty(&call.arguments).unwrap_or_default();
                    mono_block(ui, "Input", &args, &format!("in-{}", call.id), self, out);
                    if let Some(o) = output {
                        ui.add_space(4.0);
                        mono_block(ui, "Output", o, &format!("out-{}", call.id), self, out);
                    }
                });
            });
    }

    // ------------------------------------------------------------------ composer

    /// While files are dragged over the window: highlight the chat composer box (not the full window).
    pub(crate) fn drop_overlay(&self, ctx: &egui::Context) {
        if self.show_settings {
            return;
        }
        let hovered = ctx.input(|i| i.raw.hovered_files.clone());
        if hovered.is_empty() {
            return;
        }
        let Some(rect) = self.composer_rect else {
            return;
        };
        let p = crate::theme::palette(ctx.theme() == egui::Theme::Dark);
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("drop"),
        ));
        painter.rect_filled(rect, CornerRadius::same(24), soft(p.accent_soft, 0xeb));
        painter.rect_stroke(
            rect,
            CornerRadius::same(24),
            Stroke::new(2.0, p.accent),
            StrokeKind::Inside,
        );
        let n = hovered.len();
        let title = if n == 1 {
            "Drop here to attach file or folder".to_string()
        } else {
            format!("Drop here to attach {n} items")
        };
        painter.text(
            rect.center() - vec2(0.0, 10.0),
            Align2::CENTER_CENTER,
            title,
            FontId::proportional(15.5),
            p.text,
        );
        painter.text(
            rect.center() + vec2(0.0, 12.0),
            Align2::CENTER_CENTER,
            "PDF, DOCX, XLSX, PPTX, images, code, or folders",
            FontId::proportional(12.5),
            p.muted,
        );
    }

    pub(crate) fn composer_area(&mut self, ui: &mut egui::Ui) {
        let mut actions = Vec::new();
        column(ui, |ui| {
            self.approval_cards(ui, &mut actions);
            self.banner_view(ui);
            self.composer(ui);
            let p = pal(ui);
            ui.add_space(4.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("Enter sends \u{00b7} Shift+Enter new line \u{00b7} @ mention app/path \u{00b7} / commands \u{00b7} Drop files here")
                        .size(11.5)
                        .color(p.faint),
                );
            });
        });
        let ctx = ui.ctx().clone();
        self.apply(actions, &ctx);
    }

    /// Floating overlays anchored above the composer so opening/closing the `@` popup or notice
    /// never resizes the bottom panel or causes a layout flicker.
    pub(crate) fn floating_chat_overlays(&mut self, ctx: &egui::Context) {
        let Some(comp_rect) = self.composer_rect else {
            return;
        };
        let has_notice = self.notice.is_some();
        let has_suggest =
            active_token(&self.input).is_some_and(|(_, tok)| !self.suggestions(&tok).is_empty());
        if !has_notice && !has_suggest {
            self.suggest_sel = 0;
            return;
        }
        egui::Area::new(egui::Id::unique("chat-floating-overlays"))
            .order(egui::Order::Foreground)
            .pivot(Align2::LEFT_BOTTOM)
            .fixed_pos(pos2(comp_rect.left(), comp_rect.top() - 8.0))
            .show(ctx, |ui| {
                ui.set_width(comp_rect.width());
                self.notice_view(ui);
                self.suggest_popup(ui);
            });
    }

    /// Blip next to the composer while the chat is in use: it shows what is going on (its speech bubble) where the
    /// eyes already are. In an empty chat the big Blip of the start screen does this job.
    pub(crate) fn blip_dock(&mut self, ctx: &egui::Context) {
        let Some(comp_rect) = self.composer_rect else {
            return;
        };
        // Blip stays next to the composer for as long as the chat is in use, so it never blinks in and out after a
        // reply. In an empty chat the big Blip of the start screen takes this job.
        if self.messages.is_empty() && !self.busy {
            return;
        }
        let mut clicked = false;
        egui::Area::new(egui::Id::unique("blip-dock"))
            .order(egui::Order::Foreground)
            .pivot(Align2::RIGHT_BOTTOM)
            .fixed_pos(pos2(comp_rect.right() - 6.0, comp_rect.top() - 6.0))
            .interactable(true)
            .show(ctx, |ui| {
                clicked = crate::blip::paint_mascot(ui, &self.blip, 52.0, None, self.now);
            });
        if clicked {
            self.blip.on_click(self.now);
        }
        // Keep drawing while it moves or talks; a slow tick is enough while it only blinks.
        let busy = self.busy || self.blip.is_animating(self.now);
        ctx.request_repaint_after(std::time::Duration::from_millis(if busy {
            60
        } else {
            400
        }));
    }

    fn notice_view(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let Some((msg, _)) = self.notice.clone() else {
            return;
        };
        let mut dismiss = false;
        let mut copy_path: Option<String> = None;
        egui::Frame::new()
            .fill(p.raised)
            .stroke(Stroke::new(1.0, p.border_strong))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let (badge_r, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
                    ui.painter()
                        .circle_filled(badge_r.center(), 12.0, soft(p.success, 0x26));
                    paint_icon(ui.painter(), badge_r.shrink(5.0), Icon::Check, p.success);

                    if let Some(raw_path) = msg.strip_prefix("Saved to ") {
                        let path = std::path::Path::new(raw_path.trim());
                        let file_name = path
                            .file_name()
                            .map(|f| f.to_string_lossy().into_owned())
                            .unwrap_or_else(|| raw_path.to_string());
                        let parent_dir = path
                            .parent()
                            .map(|d| d.display().to_string())
                            .unwrap_or_default();
                        let text_w = (ui.available_width() - 120.0).max(120.0);
                        ui.allocate_ui_with_layout(
                            vec2(text_w, 24.0),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                ui.label(semi("Saved").size(13.0).color(p.success));
                                ui.label(semi(short(&file_name, 42)).size(13.0).color(p.text))
                                    .on_hover_text(raw_path);
                                if !parent_dir.is_empty() {
                                    ui.label(
                                        RichText::new(format!("in {}", short(&parent_dir, 40)))
                                            .size(12.0)
                                            .color(p.muted),
                                    )
                                    .on_hover_text(raw_path);
                                }
                            },
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if widgets::icon_button_sized(ui, Icon::Close, "Dismiss", 24.0, false)
                                .clicked()
                            {
                                dismiss = true;
                            }
                            if widgets::chip(
                                ui,
                                Icon::Copy,
                                "Copy path",
                                false,
                                "Copy full file path to clipboard",
                            )
                            .clicked()
                            {
                                copy_path = Some(raw_path.trim().to_string());
                            }
                        });
                    } else {
                        let text_w = (ui.available_width() - 40.0).max(120.0);
                        ui.allocate_ui_with_layout(
                            vec2(text_w, 22.0),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.label(RichText::new(short(&msg, 120)).size(13.0).color(p.text));
                            },
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if widgets::icon_button_sized(ui, Icon::Close, "Dismiss", 24.0, false)
                                .clicked()
                            {
                                dismiss = true;
                            }
                        });
                    }
                });
            });
        ui.add_space(6.0);
        if let Some(p_str) = copy_path {
            ui.ctx().copy_text(p_str);
            self.notify("Copied path to clipboard.");
        } else if dismiss {
            self.notice = None;
        }
    }

    fn suggest_popup(&mut self, ui: &mut egui::Ui) {
        let Some((tok_start, tok)) = active_token(&self.input) else {
            self.suggest_sel = 0;
            return;
        };
        let items = self.suggestions(&tok);
        if items.is_empty() {
            return;
        }
        let len = items.len();
        if self.suggest_sel >= len {
            self.suggest_sel = 0;
        }
        let p = pal(ui);
        let up = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
        let down = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
        let tab = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
        if up {
            self.suggest_sel = if self.suggest_sel == 0 {
                len - 1
            } else {
                self.suggest_sel - 1
            };
        }
        if down {
            self.suggest_sel = (self.suggest_sel + 1) % len;
        }
        let mut chosen: Option<usize> = if tab { Some(self.suggest_sel) } else { None };

        egui::Frame::new()
            .fill(p.raised)
            .stroke(Stroke::new(1.0, p.border_strong))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let heading = if tok.starts_with('@') {
                        "Mentions \u{00b7} Craft apps, agents, folders, skills & teams"
                    } else {
                        "Commands & MCP tools"
                    };
                    ui.label(semi(heading).size(11.5).color(p.faint));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new("\u{2191}\u{2193} navigate \u{00b7} Tab select")
                                .size(11.0)
                                .color(p.faint),
                        );
                    });
                });
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("suggest-scroll")
                    .max_height(270.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (i, item) in items.iter().enumerate() {
                            let sel = i == self.suggest_sel;
                            let (rect, resp) = ui.allocate_exact_size(
                                vec2(ui.available_width(), 30.0),
                                Sense::click(),
                            );
                            let hovered = ui.rect_contains_pointer(rect);
                            if sel || hovered {
                                ui.painter().rect_filled(
                                    rect,
                                    CornerRadius::same(8),
                                    if sel { p.accent_soft } else { p.hover },
                                );
                            }
                            let icon_r = Rect::from_center_size(
                                pos2(rect.left() + 15.0, rect.center().y),
                                Vec2::splat(18.0),
                            );
                            if let Some(app_id) = item.app_icon.as_deref()
                                && let Some(tex) = self.assets.app(app_id)
                            {
                                ui.painter().image(
                                    tex.id(),
                                    icon_r,
                                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                    Color32::WHITE,
                                );
                            } else if let Some(col) = item.icon_color {
                                ui.painter().rect_filled(
                                    icon_r,
                                    CornerRadius::same(5),
                                    soft(col, 0x26),
                                );
                                paint_icon(ui.painter(), icon_r.shrink(3.0), item.icon, col);
                            } else {
                                paint_icon(
                                    ui.painter(),
                                    icon_r.shrink(2.0),
                                    item.icon,
                                    if sel { p.accent_text } else { p.muted },
                                );
                            }
                            ui.painter().text(
                                pos2(rect.left() + 32.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                &item.trigger,
                                FontId::proportional(13.0),
                                if sel { p.accent_text } else { p.text },
                            );
                            ui.painter().text(
                                pos2(rect.left() + 215.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                short(&item.desc, 58),
                                FontId::proportional(12.0),
                                p.muted,
                            );
                            if sel && (up || down) {
                                resp.scroll_to_me(None);
                            }
                            if resp.clicked() {
                                chosen = Some(i);
                            }
                        }
                    });
            });

        if let Some(idx) = chosen
            && let Some(item) = items.get(idx)
        {
            let ctx = ui.ctx().clone();
            self.apply_suggestion(tok_start, tok.len(), item, &ctx);
        }
    }

    fn suggestions(&self, tok: &str) -> Vec<SuggestItem> {
        let q = tok.get(1..).unwrap_or("").to_ascii_lowercase();
        let mut out = Vec::new();
        if tok.starts_with('@') {
            for c in &self.craft {
                let id = c["id"].as_str().unwrap_or_default();
                let name = c["name"].as_str().unwrap_or(id);
                let installed = c["installed"].as_bool().unwrap_or(false);
                let connected = c["connected"].as_bool().unwrap_or(false);
                let live = c["live"].as_bool().unwrap_or(false);
                let mode = c["mode"].as_str().unwrap_or("headless");
                if installed
                    && (q.is_empty() || id.contains(&q) || name.to_ascii_lowercase().contains(&q))
                    && (!q.is_empty() || out.len() < 8)
                {
                    let status = if live {
                        "live"
                    } else if connected {
                        "headless"
                    } else if mode == "off" {
                        "off \u{00b7} mention to enable"
                    } else {
                        "available on-demand"
                    };
                    out.push(SuggestItem {
                        icon: Icon::Plug,
                        app_icon: Some(id.to_string()),
                        icon_color: None,
                        trigger: format!("@{id}"),
                        desc: format!("{name} ({status}) \u{2014} connect & use its tools"),
                        kind: SuggestKind::Insert(format!("@{id} ")),
                    });
                }
            }
            let specials = [
                (
                    "@all",
                    Icon::Plug,
                    Color32::from_rgb(0x63, 0x66, 0xf1),
                    "Any installed Craft app may be used",
                    SuggestKind::Insert("@all ".to_string()),
                ),
                (
                    "@team:apps",
                    Icon::Users,
                    Color32::from_rgb(0x8b, 0x5c, 0xf6),
                    "Team mode with one agent per Craft app",
                    SuggestKind::Insert("@team:apps ".to_string()),
                ),
                (
                    "@team:craft",
                    Icon::Users,
                    Color32::from_rgb(0x7c, 0x3a, 0xed),
                    "Team mode with the five Craft specialists",
                    SuggestKind::Insert("@team:craft ".to_string()),
                ),
                (
                    "@web",
                    Icon::Globe,
                    Color32::from_rgb(0x02, 0x84, 0xc7),
                    "Allow fetching web pages for this answer",
                    SuggestKind::Insert("@web ".to_string()),
                ),
                (
                    "@artifacts",
                    Icon::Image,
                    Color32::from_rgb(0xec, 0x48, 0x99),
                    "The pictures and files the Craft apps made",
                    SuggestKind::Insert("@artifacts ".to_string()),
                ),
                (
                    "@pictures",
                    Icon::Folder,
                    Color32::from_rgb(0xdb, 0x27, 0x77),
                    "Reference your PC Pictures folder",
                    SuggestKind::Insert("@pictures ".to_string()),
                ),
                (
                    "@videos",
                    Icon::Folder,
                    Color32::from_rgb(0x7c, 0x3a, 0xed),
                    "Reference your PC Videos folder",
                    SuggestKind::Insert("@videos ".to_string()),
                ),
                (
                    "@music",
                    Icon::Folder,
                    Color32::from_rgb(0x0e, 0xa5, 0xe9),
                    "Reference your PC Music folder",
                    SuggestKind::Insert("@music ".to_string()),
                ),
                (
                    "@home",
                    Icon::Folder,
                    Color32::from_rgb(0x64, 0x74, 0x8b),
                    "Reference your user folder",
                    SuggestKind::Insert("@home ".to_string()),
                ),
                (
                    "@workspace",
                    Icon::Folder,
                    Color32::from_rgb(0x8b, 0x5c, 0xf6),
                    "Reference the active workspace folder",
                    SuggestKind::Insert("@workspace ".to_string()),
                ),
                (
                    "@documents",
                    Icon::Folder,
                    Color32::from_rgb(0x3b, 0x82, 0xf6),
                    "Reference your PC Documents folder",
                    SuggestKind::Insert("@documents ".to_string()),
                ),
                (
                    "@desktop",
                    Icon::Folder,
                    Color32::from_rgb(0x0d, 0x94, 0x88),
                    "Reference your PC Desktop folder",
                    SuggestKind::Insert("@desktop ".to_string()),
                ),
                (
                    "@downloads",
                    Icon::Folder,
                    Color32::from_rgb(0x1f, 0x8b, 0x4c),
                    "Reference your PC Downloads folder",
                    SuggestKind::Insert("@downloads ".to_string()),
                ),
                (
                    "@folder",
                    Icon::Folder,
                    Color32::from_rgb(0xd9, 0x77, 0x06),
                    "Pick any folder on your PC to attach\u{2026}",
                    SuggestKind::PickFolder,
                ),
                (
                    "@file",
                    Icon::File,
                    Color32::from_rgb(0x2b, 0x6c, 0xd4),
                    "Pick files or Office docs on your PC\u{2026}",
                    SuggestKind::PickFiles,
                ),
                (
                    "@photo",
                    Icon::Image,
                    Color32::from_rgb(0xec, 0x48, 0x99),
                    "Pick photos or images on your PC\u{2026}",
                    SuggestKind::PickPhotos,
                ),
            ];
            for (trig, icon, col, desc, kind) in specials {
                if q.is_empty() || trig[1..].contains(&q) {
                    out.push(SuggestItem {
                        icon,
                        app_icon: None,
                        icon_color: Some(col),
                        trigger: trig.to_string(),
                        desc: desc.to_string(),
                        kind,
                    });
                }
            }
            let mut agents_shown = 0;
            for a in &self.roster {
                if a.id == "leader" {
                    continue;
                }
                if q.is_empty() && agents_shown >= 4 {
                    break;
                }
                if q.is_empty()
                    || a.id.to_ascii_lowercase().contains(&q)
                    || a.name.to_ascii_lowercase().contains(&q)
                {
                    agents_shown += 1;
                    let app_ic = a.skills.first().cloned();
                    out.push(SuggestItem {
                        icon: Icon::Users,
                        app_icon: app_ic,
                        icon_color: Some(agent_color(&a.id, true)),
                        trigger: format!("@{}", a.id),
                        desc: format!("Agent \u{00b7} {}", a.name),
                        kind: SuggestKind::Insert(format!("@{} ", a.id)),
                    });
                }
            }
            let mut skills_shown = 0;
            for (sname, sdesc) in &self.skills {
                if q.is_empty() && skills_shown >= 4 {
                    break;
                }
                if q.is_empty() || sname.to_ascii_lowercase().contains(&q) {
                    skills_shown += 1;
                    out.push(SuggestItem {
                        icon: Icon::Sparkle,
                        app_icon: Some(sname.clone()),
                        icon_color: Some(Color32::from_rgb(0xf5, 0x9e, 0x0b)),
                        trigger: format!("@skill:{sname}"),
                        desc: short(sdesc, 56),
                        kind: SuggestKind::Insert(format!("@skill:{sname} ")),
                    });
                }
            }
        } else if tok.starts_with('/') {
            let mut cmds: Vec<(&str, Icon, Color32, &str, SuggestKind)> = vec![
                (
                    "/undo",
                    Icon::Undo,
                    Color32::from_rgb(0xf5, 0x9e, 0x0b),
                    "Undo file changes from the latest turn",
                    SuggestKind::UndoLatest,
                ),
                (
                    "/new",
                    Icon::Plus,
                    crate::theme::brand(),
                    "Start a fresh conversation",
                    SuggestKind::NewChat,
                ),
                (
                    "/export md",
                    Icon::Download,
                    Color32::from_rgb(0x3b, 0x82, 0xf6),
                    "Export conversation as Markdown (.md)",
                    SuggestKind::Export("md"),
                ),
                (
                    "/export html",
                    Icon::Download,
                    Color32::from_rgb(0xd9, 0x6b, 0x27),
                    "Export conversation as HTML (.html)",
                    SuggestKind::Export("html"),
                ),
                (
                    "/export json",
                    Icon::Download,
                    Color32::from_rgb(0xca, 0x8a, 0x04),
                    "Export conversation as JSON (.json)",
                    SuggestKind::Export("json"),
                ),
                (
                    "/copy",
                    Icon::Copy,
                    Color32::from_rgb(0x0d, 0x94, 0x88),
                    "Copy whole conversation as Markdown",
                    SuggestKind::CopyChat,
                ),
                (
                    "/docx",
                    Icon::File,
                    Color32::from_rgb(0x2b, 0x6c, 0xd4),
                    "Create a Word document (.docx)",
                    SuggestKind::Insert("Create a polished .docx document at ".to_string()),
                ),
                (
                    "/xlsx",
                    Icon::File,
                    Color32::from_rgb(0x1f, 0x8b, 0x4c),
                    "Create an Excel spreadsheet (.xlsx)",
                    SuggestKind::Insert(
                        "Create an .xlsx spreadsheet with formulas at ".to_string(),
                    ),
                ),
                (
                    "/pptx",
                    Icon::File,
                    Color32::from_rgb(0xd9, 0x6b, 0x27),
                    "Create a PowerPoint presentation (.pptx)",
                    SuggestKind::Insert("Create a .pptx slide deck at ".to_string()),
                ),
                (
                    "/web",
                    Icon::Globe,
                    Color32::from_rgb(0x02, 0x84, 0xc7),
                    "Fetch and read a web page URL",
                    SuggestKind::Insert("Fetch and summarize https://".to_string()),
                ),
                (
                    "/team",
                    Icon::Users,
                    Color32::from_rgb(0x8b, 0x5c, 0xf6),
                    "Switch to Team mode",
                    SuggestKind::SetTeam(true),
                ),
                (
                    "/single",
                    Icon::Person,
                    crate::theme::brand(),
                    "Switch to Single mode",
                    SuggestKind::SetTeam(false),
                ),
                (
                    "/auto",
                    Icon::Bolt,
                    Color32::from_rgb(0x16, 0xa3, 0x4a),
                    "Switch tool approval to Auto mode",
                    SuggestKind::SetAuto(true),
                ),
                (
                    "/ask",
                    Icon::Shield,
                    Color32::from_rgb(0xd9, 0x77, 0x06),
                    "Switch tool approval to Ask first",
                    SuggestKind::SetAuto(false),
                ),
                (
                    "/mcp",
                    Icon::Plug,
                    Color32::from_rgb(0x63, 0x66, 0xf1),
                    "Open Craft apps & MCP connectors settings",
                    SuggestKind::OpenSettings(crate::app::SettingsTab::Tools),
                ),
                (
                    "/skills",
                    Icon::Sparkle,
                    Color32::from_rgb(0xf5, 0x9e, 0x0b),
                    "Open Skills & plugins settings",
                    SuggestKind::OpenSettings(crate::app::SettingsTab::Skills),
                ),
            ];
            cmds.extend([
                (
                    "/stop",
                    Icon::Stop,
                    Color32::from_rgb(0xdc, 0x26, 0x26),
                    "Stop the reply in this chat",
                    SuggestKind::Stop,
                ),
                (
                    "/retry",
                    Icon::Refresh,
                    Color32::from_rgb(0x0d, 0x94, 0x88),
                    "Ask the last question again as a new version",
                    SuggestKind::Retry,
                ),
                (
                    "/team apps",
                    Icon::Users,
                    Color32::from_rgb(0x8b, 0x5c, 0xf6),
                    "Team mode with one agent per Craft app",
                    SuggestKind::SetPreset("apps"),
                ),
                (
                    "/team craft",
                    Icon::Users,
                    Color32::from_rgb(0x7c, 0x3a, 0xed),
                    "Team mode with the five Craft specialists",
                    SuggestKind::SetPreset("craft"),
                ),
                (
                    "/rename",
                    Icon::Edit,
                    Color32::from_rgb(0x64, 0x74, 0x8b),
                    "Rename this chat: /rename New title",
                    SuggestKind::Insert("/rename ".to_string()),
                ),
                (
                    "/model",
                    Icon::Sparkle,
                    Color32::from_rgb(0xf5, 0x9e, 0x0b),
                    "Pick the model: /model provider:name",
                    SuggestKind::Insert("/model ".to_string()),
                ),
                (
                    "/think",
                    Icon::Sparkle,
                    Color32::from_rgb(0xd9, 0x77, 0x06),
                    "Reasoning on or off: /think on | /think off",
                    SuggestKind::Insert("/think ".to_string()),
                ),
                (
                    "/data",
                    Icon::Folder,
                    Color32::from_rgb(0xd9, 0x77, 0x06),
                    "Open the data folder",
                    SuggestKind::OpenData,
                ),
                (
                    "/rescan",
                    Icon::Refresh,
                    Color32::from_rgb(0x63, 0x66, 0xf1),
                    "Look for installed Craft apps again",
                    SuggestKind::Rescan,
                ),
                (
                    "/apps",
                    Icon::Plug,
                    Color32::from_rgb(0x63, 0x66, 0xf1),
                    "Open the Craft app connectors",
                    SuggestKind::OpenSettings(crate::app::SettingsTab::Tools),
                ),
                (
                    "/agents",
                    Icon::Users,
                    Color32::from_rgb(0x8b, 0x5c, 0xf6),
                    "Open agents and approvals",
                    SuggestKind::OpenSettings(crate::app::SettingsTab::Agents),
                ),
                (
                    "/settings",
                    Icon::Settings,
                    Color32::from_rgb(0x64, 0x74, 0x8b),
                    "Open settings",
                    SuggestKind::OpenSettings(crate::app::SettingsTab::General),
                ),
                (
                    "/summarize",
                    Icon::File,
                    Color32::from_rgb(0x2b, 0x6c, 0xd4),
                    "Summarize an attached or mentioned file",
                    SuggestKind::Insert("Summarize the key points of ".to_string()),
                ),
                (
                    "/explain",
                    Icon::Sparkle,
                    Color32::from_rgb(0xf5, 0x9e, 0x0b),
                    "Explain a file, a formula or an error",
                    SuggestKind::Insert("Explain in plain words: ".to_string()),
                ),
                (
                    "/improve",
                    Icon::Edit,
                    Color32::from_rgb(0x16, 0xa3, 0x4a),
                    "Improve a document without changing its meaning",
                    SuggestKind::Insert("Improve the wording and layout of ".to_string()),
                ),
                (
                    "/render",
                    Icon::Image,
                    Color32::from_rgb(0xec, 0x48, 0x99),
                    "Render a preview of the open file",
                    SuggestKind::Insert("Render a preview of ".to_string()),
                ),
                (
                    "/convert",
                    Icon::Refresh,
                    Color32::from_rgb(0x0e, 0xa5, 0xe9),
                    "Convert a file to another format",
                    SuggestKind::Insert("Convert ".to_string()),
                ),
                (
                    "/handoff",
                    Icon::Plug,
                    Color32::from_rgb(0x63, 0x66, 0xf1),
                    "Pass a result from one Craft app to another",
                    SuggestKind::Insert(
                        "Take the result of the last step and hand it to ".to_string(),
                    ),
                ),
                (
                    "/help",
                    Icon::Sparkle,
                    Color32::from_rgb(0x64, 0x74, 0x8b),
                    "List every command",
                    SuggestKind::Help,
                ),
            ]);
            for (trig, icon, col, desc, kind) in cmds {
                if q.is_empty() || trig[1..].contains(&q) {
                    out.push(SuggestItem {
                        icon,
                        app_icon: None,
                        icon_color: Some(col),
                        trigger: trig.to_string(),
                        desc: desc.to_string(),
                        kind,
                    });
                }
            }
            for tool in &self.mcp_tools {
                if q.is_empty() || tool.to_ascii_lowercase().contains(&q) {
                    out.push(SuggestItem {
                        icon: Icon::Terminal,
                        app_icon: Some(tool.clone()),
                        icon_color: None,
                        trigger: format!("/{tool}"),
                        desc: format!("Run MCP tool {}", tool_title(tool)),
                        kind: SuggestKind::Insert(format!("Use tool `{tool}` to ")),
                    });
                }
            }
        }
        // Best match first: the exact name, then names that start with the text, then the rest (each group keeps its order).
        if !q.is_empty() {
            out.sort_by_key(|i| {
                let name = i
                    .trigger
                    .trim_start_matches(['@', '/'])
                    .to_ascii_lowercase();
                if name == q {
                    0
                } else if name.starts_with(&q) {
                    1
                } else {
                    2
                }
            });
        }
        out.truncate(30);
        out
    }

    fn apply_suggestion(
        &mut self,
        start: usize,
        len: usize,
        item: &SuggestItem,
        ctx: &egui::Context,
    ) {
        match &item.kind {
            SuggestKind::Insert(rep) => {
                replace_token(&mut self.input, start, len, rep);
            }
            SuggestKind::PickFiles => {
                replace_token(&mut self.input, start, len, "");
                self.pick_files(ctx);
            }
            SuggestKind::PickPhotos => {
                replace_token(&mut self.input, start, len, "");
                self.pick_photos(ctx);
            }
            SuggestKind::PickFolder => {
                replace_token(&mut self.input, start, len, "");
                self.pick_folder(ctx);
            }
            SuggestKind::UndoLatest => {
                replace_token(&mut self.input, start, len, "");
                self.undo_last();
            }
            SuggestKind::NewChat => {
                replace_token(&mut self.input, start, len, "");
                self.new_chat();
            }
            SuggestKind::Export(fmt) => {
                replace_token(&mut self.input, start, len, "");
                self.export(fmt, false);
            }
            SuggestKind::CopyChat => {
                replace_token(&mut self.input, start, len, "");
                self.export("md", true);
            }
            SuggestKind::SetTeam(team) => {
                replace_token(&mut self.input, start, len, "");
                self.set_mode(*team);
            }
            SuggestKind::SetAuto(auto) => {
                replace_token(&mut self.input, start, len, "");
                self.set_auto_approve(*auto);
            }
            SuggestKind::OpenSettings(tab) => {
                replace_token(&mut self.input, start, len, "");
                self.open_settings(*tab);
            }
            SuggestKind::Stop => {
                replace_token(&mut self.input, start, len, "");
                self.run_typed_command("/stop");
            }
            SuggestKind::Retry => {
                replace_token(&mut self.input, start, len, "");
                self.retry_last();
            }
            SuggestKind::SetPreset(p) => {
                replace_token(&mut self.input, start, len, "");
                self.set_team_preset(p);
            }
            SuggestKind::OpenData => {
                replace_token(&mut self.input, start, len, "");
                self.run_typed_command("/data");
            }
            SuggestKind::Rescan => {
                replace_token(&mut self.input, start, len, "");
                self.run_typed_command("/rescan");
            }
            SuggestKind::Help => {
                replace_token(&mut self.input, start, len, "");
                self.run_typed_command("/help");
            }
        }
    }

    fn approval_cards(&self, ui: &mut egui::Ui, out: &mut Vec<Action>) {
        let p = pal(ui);
        for a in &self.approvals {
            let (bg, border, shield_fg) = if a.destructive {
                (p.danger_soft, soft(p.danger, 0x80), p.danger)
            } else {
                (p.warning_soft, soft(p.warning, 0x70), p.warning)
            };
            egui::Frame::new()
                .fill(bg)
                .stroke(Stroke::new(1.0, border))
                .corner_radius(CornerRadius::same(14))
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                        paint_icon(ui.painter(), r, Icon::Shield, shield_fg);
                        let who = self.agent_name(a.agent.as_deref());
                        if let Some(t) = self.assets.app(&a.tool).filter(|_| !split_tool(&a.tool).0.is_empty()) {
                            widgets::image(ui, t, 18.0);
                        }
                        if !a.session.is_empty() && self.current.as_deref() != Some(a.session.as_str()) {
                            widgets::pill(ui, "Other chat", p.muted, p.pressed);
                        }
                        ui.label(semi(who).size(13.5).color(p.text));
                        ui.label(RichText::new("wants to run").size(13.5).color(p.muted));
                        ui.label(semi(tool_title(&a.tool)).size(13.5).color(p.text));
                        if a.destructive {
                            widgets::pill(ui, "Destructive", p.danger, soft(p.danger, 0x24));
                        } else if a.reason.is_some() {
                            widgets::pill(ui, "Always gated", p.warning, soft(p.warning, 0x24));
                        }
                    });
                    if let Some(reason) = &a.reason {
                        ui.add_space(3.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(reason)
                                    .size(12.5)
                                    .color(if a.destructive { p.danger } else { p.text }),
                            )
                            .wrap(),
                        );
                    }
                    ui.add_space(4.0);
                    let preview: String = a.args.lines().take(6).collect::<Vec<_>>().join("\n");
                    egui::Frame::new()
                        .fill(p.code_bg)
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.add(egui::Label::new(RichText::new(short_lines(&preview, 600)).monospace().size(12.0).color(p.muted)).wrap());
                        });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if widgets::button(ui, Some(Icon::Check), "Allow", true).clicked() {
                            out.push(Action::Approve { id: a.id.clone(), allow: true });
                        }
                        if widgets::button(ui, Some(Icon::Close), "Deny", false).clicked() {
                            out.push(Action::Approve { id: a.id.clone(), allow: false });
                        }
                        if a.reason.is_none() {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .add(egui::Label::new(RichText::new("Allow everything (Auto mode)").size(12.5).color(p.muted)).sense(Sense::click()))
                                    .on_hover_text("Switch approval to Auto: normal writes run without asking (gated/destructive tools still ask).")
                                    .clicked()
                                {
                                    out.push(Action::AutoApprove);
                                }
                            });
                        }
                    });
                });
            ui.add_space(8.0);
        }
    }

    fn banner_view(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let Some(b) = self.banner.clone() else { return };
        let mut close = false;
        egui::Frame::new()
            .fill(p.danger_soft)
            .stroke(Stroke::new(1.0, soft(p.danger, 0x60)))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 34.0;
                    ui.allocate_ui(vec2(w, 0.0), |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(short(&b, 400)).size(13.0).color(p.danger),
                            )
                            .wrap(),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::icon_button_sized(ui, Icon::Close, "Dismiss", 24.0, false)
                            .clicked()
                        {
                            close = true;
                        }
                    });
                });
            });
        ui.add_space(8.0);
        if close {
            self.banner = None;
        }
    }

    fn composer(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let id = egui::Id::unique("composer-input");
        let focused = ui.ctx().memory(|m| m.has_focus(id));
        let trimmed_cmd = self.input.trim().to_string();
        // Enter sends, Shift+Enter adds a line. Consume the key before the editor sees it.
        // consume_key ignores extra modifiers, so Shift+Enter is excluded explicitly; the editor itself treats
        // Shift+Enter as its return key (see `return_key` below).
        let enter = focused
            && ui.input_mut(|i| {
                !i.modifiers.shift && i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
            });
        let shift_enter =
            focused && ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter));
        if shift_enter {
            insert_newline(ui.ctx(), id, &mut self.input);
        }
        let mut send = false;
        let mut stop = false;
        let mut toggle_voice = false;
        let mut cancel_voice = false;
        let mut dismiss_voice_err = false;
        let mut open_mic_settings = false;
        let mut remove = None;
        let mut cancel_branch = false;
        let frame_resp = egui::Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(
                if matches!(self.voice.state, crate::voice::VoiceState::Listening) {
                    1.5
                } else {
                    1.0
                },
                if matches!(self.voice.state, crate::voice::VoiceState::Listening) {
                    Color32::from_rgb(0xef, 0x44, 0x44)
                } else if focused {
                    p.border_strong
                } else {
                    p.border
                },
            ))
            .corner_radius(CornerRadius::same(24))
            .inner_margin(Margin {
                left: 14,
                right: 10,
                top: 10,
                bottom: 8,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if let crate::voice::VoiceState::Error(err) = &self.voice.state {
                    egui::Frame::new()
                        .fill(p.danger_soft)
                        .stroke(Stroke::new(1.0, soft(p.danger, 0x55)))
                        .corner_radius(CornerRadius::same(12))
                        .inner_margin(Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                let (ic_r, _) =
                                    ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                                paint_icon(ui.painter(), ic_r, Icon::MicOff, p.danger);
                                ui.label(
                                    RichText::new(short(&err.message, 115))
                                        .size(12.5)
                                        .color(p.danger),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if widgets::icon_button_sized(
                                        ui,
                                        Icon::Close,
                                        "Dismiss voice error",
                                        22.0,
                                        false,
                                    )
                                    .clicked()
                                    {
                                        dismiss_voice_err = true;
                                    }
                                    if err.permission_denied
                                        && widgets::chip(
                                            ui,
                                            Icon::Settings,
                                            "Mic Privacy Settings",
                                            false,
                                            "Open Windows Microphone Privacy Settings",
                                        )
                                        .clicked()
                                    {
                                        open_mic_settings = true;
                                    }
                                    if widgets::chip(
                                        ui,
                                        Icon::Mic,
                                        "Try again",
                                        false,
                                        "Retry microphone voice input",
                                    )
                                    .clicked()
                                    {
                                        toggle_voice = true;
                                    }
                                });
                            });
                        });
                    ui.add_space(6.0);
                }
                if self.branch_from.is_some() {
                    ui.horizontal(|ui| {
                        widgets::pill(
                            ui,
                            "Editing an earlier message: sending starts a new branch",
                            p.accent_text,
                            p.accent_soft,
                        );
                        if ui
                            .add(
                                egui::Label::new(RichText::new("Cancel").size(12.5).color(p.muted))
                                    .sense(Sense::click()),
                            )
                            .clicked()
                        {
                            cancel_branch = true;
                        }
                    });
                    ui.add_space(4.0);
                }
                if !self.chips.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                        for (i, c) in self.chips.iter().enumerate() {
                            if attachment_card(ui, &p, c) {
                                remove = Some(i);
                            }
                        }
                    });
                    ui.add_space(6.0);
                }
                egui::ScrollArea::vertical()
                    .id_salt("composer-scroll")
                    .max_height(220.0)
                    .show(ui, |ui| {
                        let hint = if self.mode_team {
                            "Describe the goal for the team (@app, @folder, /command)"
                        } else {
                            "Message WorkCraft (@wordcraft, @path, /docx, or drop files here)"
                        };
                        ui.add(
                            egui::TextEdit::multiline(&mut self.input)
                                .id(id)
                                .hint_text(RichText::new(hint).color(p.faint))
                                .frame(egui::Frame::NONE)
                                .font(egui::FontId::proportional(15.0))
                                .return_key(None)
                                .desired_rows(1)
                                .desired_width(ui.available_width()),
                        );
                    });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    self.plus_menu_button(ui);
                    if widgets::chip(
                        ui,
                        Icon::Sparkle,
                        "Think",
                        !self.think_off,
                        "Reasoning for models that support it",
                    )
                    .clicked()
                    {
                        let off = !self.think_off;
                        self.set_think_off(off);
                    }
                    let (icon, label, tip) = if self.auto_approve {
                        (
                            Icon::Bolt,
                            "Auto",
                            "Tools run without asking. Click to ask first.",
                        )
                    } else {
                        (
                            Icon::Shield,
                            "Ask first",
                            "Tools that change things wait for your OK. Click for Auto.",
                        )
                    };
                    if widgets::chip(ui, icon, label, self.auto_approve, tip).clicked() {
                        let a = !self.auto_approve;
                        self.set_auto_approve(a);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let empty = self.input.trim().is_empty() && self.chips.is_empty();
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
                        let (bg, fg, icon, tip) = if self.busy {
                            (p.raised, p.text, Icon::Stop, "Stop (Esc)")
                        } else if empty {
                            (p.pressed, p.faint, Icon::Send, "Type a message")
                        } else {
                            (
                                if resp.hovered() {
                                    p.accent_text
                                } else {
                                    p.accent
                                },
                                p.on_accent,
                                Icon::Send,
                                "Send (Enter)",
                            )
                        };
                        ui.painter().circle_filled(rect.center(), 16.0, bg);
                        if self.busy {
                            ui.painter().circle_stroke(
                                rect.center(),
                                16.0,
                                Stroke::new(2.0, p.border),
                            );
                            let t = (self.now * 4.2) as f32;
                            let arc_pts: Vec<Pos2> = (0..=18)
                                .map(|i| {
                                    let a = t + (i as f32 / 18.0) * std::f32::consts::PI * 0.85;
                                    pos2(
                                        rect.center().x + 16.0 * a.cos(),
                                        rect.center().y + 16.0 * a.sin(),
                                    )
                                })
                                .collect();
                            ui.painter().add(egui::Shape::line(
                                arc_pts,
                                Stroke::new(2.2, p.accent),
                            ));
                            ui.ctx().request_repaint();
                        }
                        paint_icon(ui.painter(), rect.shrink(10.0), icon, fg);
                        if resp.on_hover_text(tip).clicked() {
                            if self.busy {
                                stop = true;
                            } else {
                                send = true;
                            }
                        }

                        // Voice input button directly next to Send
                        let (v_rect, v_resp) =
                            ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
                        let rec_red = Color32::from_rgb(0xef, 0x44, 0x44);
                        match &self.voice.state {
                            crate::voice::VoiceState::Idle => {
                                let v_bg = if v_resp.hovered() {
                                    p.hover
                                } else {
                                    p.pressed
                                };
                                let v_fg = if v_resp.hovered() { p.text } else { p.muted };
                                ui.painter().circle_filled(v_rect.center(), 16.0, v_bg);
                                ui.painter().circle_stroke(
                                    v_rect.center(),
                                    16.0,
                                    Stroke::new(1.0, p.border),
                                );
                                paint_icon(ui.painter(), v_rect.shrink(9.5), Icon::Mic, v_fg);
                                let model_label = if self.voice.handy_info.exe_path.is_some() {
                                    format!(
                                        "Voice input ({}) \u{2014} Click to speak (Ctrl+Shift+M)",
                                        self.voice.handy_info.model_display
                                    )
                                } else {
                                    "Voice input \u{2014} Click to speak (Ctrl+Shift+M)".to_string()
                                };
                                if v_resp.on_hover_text(model_label).clicked() {
                                    toggle_voice = true;
                                }
                            }
                            crate::voice::VoiceState::Listening => {
                                let pulse = ((self.now * 5.5).sin() as f32 * 0.5 + 0.5)
                                    .clamp(0.0, 1.0);
                                let halo_r = 16.0 + pulse * 3.5;
                                let halo_a = ((1.0 - pulse * 0.55) * 75.0) as u8;
                                ui.painter().circle_filled(
                                    v_rect.center(),
                                    halo_r,
                                    soft(rec_red, halo_a),
                                );
                                ui.painter().circle_filled(v_rect.center(), 16.0, rec_red);
                                paint_icon(
                                    ui.painter(),
                                    v_rect.shrink(9.5),
                                    Icon::Mic,
                                    Color32::WHITE,
                                );
                                if v_resp
                                    .on_hover_text(
                                        "Listening\u{2026} Click to finish and transcribe (Esc to cancel)",
                                    )
                                    .clicked()
                                {
                                    toggle_voice = true;
                                }

                                // Live waveform & status pill next to the Voice button
                                let bars = self.voice.spectrum_bars();
                                let elapsed = self.voice.elapsed_secs() as u64;
                                let mins = elapsed / 60;
                                let secs = elapsed % 60;
                                egui::Frame::new()
                                    .fill(soft(rec_red, 0x18))
                                    .stroke(Stroke::new(1.0, soft(rec_red, 0x55)))
                                    .corner_radius(CornerRadius::same(16))
                                    .inner_margin(Margin::symmetric(10, 5))
                                    .show(ui, |ui| {
                                        ui.spacing_mut().item_spacing.x = 6.0;
                                        if widgets::icon_button_sized(
                                            ui,
                                            Icon::Close,
                                            "Cancel recording (Esc)",
                                            18.0,
                                            false,
                                        )
                                        .clicked()
                                        {
                                            cancel_voice = true;
                                        }
                                        ui.label(
                                            semi(format!("{mins}:{secs:02}"))
                                                .size(11.5)
                                                .color(rec_red),
                                        );
                                        let (wave_r, _) = ui.allocate_exact_size(
                                            vec2(68.0, 18.0),
                                            Sense::hover(),
                                        );
                                        let n = bars.len();
                                        let step = wave_r.width() / n as f32;
                                        let bar_w = (step * 0.58).max(2.0);
                                        for (idx, &val) in bars.iter().enumerate() {
                                            let idle_wave = ((self.now as f32 * 6.0
                                                + idx as f32 * 0.55)
                                                .sin()
                                                * 0.08
                                                + 0.14)
                                                .clamp(0.08, 0.25);
                                            let h = (val.max(idle_wave) * wave_r.height())
                                                .clamp(3.0, wave_r.height());
                                            let cx =
                                                wave_r.left() + idx as f32 * step + step * 0.5;
                                            let bar_rect = Rect::from_center_size(
                                                pos2(cx, wave_r.center().y),
                                                vec2(bar_w, h),
                                            );
                                            ui.painter().rect_filled(
                                                bar_rect,
                                                CornerRadius::same(2),
                                                rec_red,
                                            );
                                        }
                                        let (dot_r, _) =
                                            ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                                        ui.painter().circle_filled(
                                            dot_r.center(),
                                            3.5,
                                            soft(rec_red, (160.0 + pulse * 95.0) as u8),
                                        );
                                    });
                                ui.ctx().request_repaint();
                            }
                            crate::voice::VoiceState::Processing => {
                                ui.painter()
                                    .circle_filled(v_rect.center(), 16.0, p.accent_soft);
                                ui.painter().circle_stroke(
                                    v_rect.center(),
                                    16.0,
                                    Stroke::new(1.5, soft(p.accent, 0x55)),
                                );
                                let t = (self.now * 4.8) as f32;
                                let arc_pts: Vec<Pos2> = (0..=18)
                                    .map(|i| {
                                        let a = t + (i as f32 / 18.0) * std::f32::consts::PI * 0.9;
                                        pos2(
                                            v_rect.center().x + 16.0 * a.cos(),
                                            v_rect.center().y + 16.0 * a.sin(),
                                        )
                                    })
                                    .collect();
                                ui.painter().add(egui::Shape::line(
                                    arc_pts,
                                    Stroke::new(2.2, p.accent),
                                ));
                                paint_icon(
                                    ui.painter(),
                                    v_rect.shrink(9.5),
                                    Icon::Mic,
                                    p.accent_text,
                                );
                                let _ = v_resp.on_hover_text("Transcribing speech to text\u{2026}");
                                widgets::pill(
                                    ui,
                                    "Transcribing\u{2026}",
                                    p.accent_text,
                                    p.accent_soft,
                                );
                                ui.ctx().request_repaint();
                            }
                            crate::voice::VoiceState::Error(err) => {
                                ui.painter()
                                    .circle_filled(v_rect.center(), 16.0, p.danger_soft);
                                ui.painter().circle_stroke(
                                    v_rect.center(),
                                    16.0,
                                    Stroke::new(1.4, p.danger),
                                );
                                paint_icon(
                                    ui.painter(),
                                    v_rect.shrink(9.5),
                                    Icon::MicOff,
                                    p.danger,
                                );
                                if v_resp
                                    .on_hover_text(format!(
                                        "{} (Click to retry)",
                                        err.message
                                    ))
                                    .clicked()
                                {
                                    toggle_voice = true;
                                }
                            }
                        }
                    });
                });
            });
        self.composer_rect = Some(frame_resp.response.rect);
        if let Some(i) = remove {
            self.chips.remove(i);
        }
        if cancel_branch {
            self.branch_from = None;
            self.input.clear();
        }
        if dismiss_voice_err {
            self.voice.state = crate::voice::VoiceState::Idle;
        }
        if open_mic_settings {
            crate::voice::open_microphone_privacy_settings();
        }
        if cancel_voice {
            self.voice.cancel();
            self.input = self.voice.input_prefix.trim_end().to_string();
            self.blip.set_state(crate::blip::BlipState::Idle);
        }
        if toggle_voice {
            let ctx = ui.ctx().clone();
            self.toggle_voice(&ctx);
        }
        if stop {
            self.cancel();
        }
        if enter && matches!(self.voice.state, crate::voice::VoiceState::Listening) {
            self.voice.finish_listening();
            self.blip.set_state(crate::blip::BlipState::Thinking);
        } else if (send || enter) && !self.busy {
            if self.try_slash_command(&trimmed_cmd) {
                self.input.clear();
            } else {
                self.send();
            }
        }
    }

    fn plus_menu_button(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let btn = widgets::icon_button_sized(
            ui,
            Icon::Plus,
            "Attach files, photos, folders, @mentions, /actions, browser, plugins & MCP connectors",
            30.0,
            false,
        );
        let composer_id = egui::Id::unique("composer-input");
        egui::Popup::menu(&btn).show(|ui| {
            ui.set_min_width(265.0);
            ui.spacing_mut().item_spacing.y = 2.0;

            ui.label(semi("Attach from PC").size(11.0).color(p.faint));
            if popup_row(
                ui,
                &p,
                Icon::File,
                "Files & Office docs (.pdf, .docx, .xlsx, .pptx\u{2026})",
            ) {
                let ctx = ui.ctx().clone();
                self.pick_files(&ctx);
                ui.close();
            }
            if popup_row(ui, &p, Icon::Image, "Photos & images\u{2026}") {
                let ctx = ui.ctx().clone();
                self.pick_photos(&ctx);
                ui.close();
            }
            if popup_row(ui, &p, Icon::Folder, "Folder from PC\u{2026}") {
                let ctx = ui.ctx().clone();
                self.pick_folder(&ctx);
                ui.close();
            }
            if popup_row(ui, &p, Icon::Folder, "Attach workspace folder") {
                let ws = workcraft_core::paths::data_dir().join("workspace");
                let ctx = ui.ctx().clone();
                self.attach_path(&ctx, ws);
                ui.close();
            }

            ui.add_space(4.0);
            ui.separator();
            ui.add_space(2.0);
            ui.label(
                semi("Mentions, Actions & Browser")
                    .size(11.0)
                    .color(p.faint),
            );
            if popup_row(
                ui,
                &p,
                Icon::Users,
                "Mention app, agent, folder or skill (@)",
            ) {
                if !self.input.is_empty() && !self.input.ends_with(char::is_whitespace) {
                    self.input.push(' ');
                }
                self.input.push('@');
                ui.ctx().memory_mut(|m| m.request_focus(composer_id));
                ui.close();
            }
            if popup_row(ui, &p, Icon::Terminal, "Actions & slash commands (/)") {
                if self.input.trim().is_empty() {
                    self.input = "/".to_string();
                } else {
                    if !self.input.ends_with(char::is_whitespace) {
                        self.input.push(' ');
                    }
                    self.input.push('/');
                }
                ui.ctx().memory_mut(|m| m.request_focus(composer_id));
                ui.close();
            }
            if popup_row(
                ui,
                &p,
                Icon::Globe,
                "Browser / Read web page (https://\u{2026})",
            ) {
                if !self.input.is_empty() && !self.input.ends_with(char::is_whitespace) {
                    self.input.push(' ');
                }
                self.input.push_str("Read https://");
                ui.ctx().memory_mut(|m| m.request_focus(composer_id));
                ui.close();
            }

            ui.add_space(4.0);
            ui.separator();
            ui.add_space(2.0);
            ui.label(
                semi("Plugins, Skills & Connectors")
                    .size(11.0)
                    .color(p.faint),
            );
            let connected_apps: Vec<(String, String)> = self
                .craft
                .iter()
                .filter(|c| c["connected"] == true)
                .filter_map(|c| {
                    let id = c["id"].as_str()?.to_string();
                    let name = c["name"].as_str().unwrap_or(&id).to_string();
                    Some((id, name))
                })
                .collect();
            if !connected_apps.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    for (cid, cname) in &connected_apps {
                        let (r, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(r, CornerRadius::same(6), p.hover);
                        }
                        if let Some(tex) = self.assets.app(cid) {
                            ui.painter().image(
                                tex.id(),
                                r.shrink(3.0),
                                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        } else {
                            paint_icon(ui.painter(), r.shrink(5.0), Icon::Plug, p.accent_text);
                        }
                        if resp
                            .on_hover_text(format!("{cname} connected — click to mention @{cid}"))
                            .clicked()
                        {
                            if !self.input.is_empty() && !self.input.ends_with(char::is_whitespace)
                            {
                                self.input.push(' ');
                            }
                            self.input.push('@');
                            self.input.push_str(cid);
                            self.input.push(' ');
                            ui.ctx().memory_mut(|m| m.request_focus(composer_id));
                            ui.close();
                        }
                    }
                });
                ui.add_space(2.0);
            }
            let mcp_count = self
                .mcp_status
                .as_ref()
                .and_then(|s| s["servers"].as_array())
                .map_or(0, Vec::len);
            let mcp_label = if mcp_count > 0 {
                format!("Connectors & MCP ({mcp_count} connected)")
            } else {
                "Connectors & MCP servers\u{2026}".to_string()
            };
            if popup_row(ui, &p, Icon::Plug, &mcp_label) {
                self.open_settings(crate::app::SettingsTab::Tools);
                ui.close();
            }
            let skills_count = self.skills.len();
            let skills_label = if skills_count > 0 {
                format!("Plugins & Skills ({skills_count} available)")
            } else {
                "Plugins & Skills\u{2026}".to_string()
            };
            if popup_row(ui, &p, Icon::Sparkle, &skills_label) {
                self.open_settings(crate::app::SettingsTab::Skills);
                ui.close();
            }
            if popup_row(ui, &p, Icon::Users, "Custom Agents & Team Roles\u{2026}") {
                self.open_settings(crate::app::SettingsTab::Agents);
                ui.close();
            }
        });
    }

    fn try_slash_command(&mut self, cmd: &str) -> bool {
        match cmd {
            "/undo" => {
                self.undo_last();
                true
            }
            "/new" | "/clear" => {
                self.new_chat();
                true
            }
            "/export" | "/export md" => {
                self.export("md", false);
                true
            }
            "/export html" => {
                self.export("html", false);
                true
            }
            "/export json" => {
                self.export("json", false);
                true
            }
            "/export txt" => {
                self.export("txt", false);
                true
            }
            "/copy" => {
                self.export("md", true);
                true
            }
            "/team" => {
                self.set_mode(true);
                true
            }
            "/single" => {
                self.set_mode(false);
                true
            }
            "/auto" => {
                self.set_auto_approve(true);
                true
            }
            "/ask" => {
                self.set_auto_approve(false);
                true
            }
            "/mcp" | "/connectors" => {
                self.open_settings(crate::app::SettingsTab::Tools);
                true
            }
            "/skills" | "/plugins" => {
                self.open_settings(crate::app::SettingsTab::Skills);
                true
            }
            _ => false,
        }
    }
}

fn popup_row(ui: &mut egui::Ui, p: &Palette, icon: Icon, label: &str) -> bool {
    let (r, resp) = widgets::row(ui, false, 28.0);
    paint_icon(
        ui.painter(),
        Rect::from_center_size(pos2(r.left() + 14.0, r.center().y), Vec2::splat(14.0)),
        icon,
        p.muted,
    );
    ui.painter().text(
        pos2(r.left() + 28.0, r.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        p.text,
    );
    resp.clicked()
}

struct SuggestItem {
    icon: Icon,
    app_icon: Option<String>,
    icon_color: Option<Color32>,
    trigger: String,
    desc: String,
    kind: SuggestKind,
}

enum SuggestKind {
    Insert(String),
    Stop,
    Retry,
    SetPreset(&'static str),
    OpenData,
    Rescan,
    Help,
    PickFiles,
    PickPhotos,
    PickFolder,
    UndoLatest,
    NewChat,
    Export(&'static str),
    CopyChat,
    SetTeam(bool),
    SetAuto(bool),
    OpenSettings(crate::app::SettingsTab),
}

/// Scroll mode: a middle click puts a marker on the chat, then moving the pointer above or below it scrolls up or down
/// (the farther, the faster). A click, the middle button again or Esc ends it.
fn autoscroll_tick(state: &std::cell::Cell<Option<egui::Pos2>>, ui: &mut egui::Ui) {
    let view = ui.clip_rect();
    let (middle, other, esc, pos, dt) = ui.input(|i| {
        (
            i.pointer.button_pressed(egui::PointerButton::Middle),
            i.pointer.button_pressed(egui::PointerButton::Primary)
                || i.pointer.button_pressed(egui::PointerButton::Secondary),
            i.key_pressed(egui::Key::Escape),
            i.pointer.hover_pos(),
            i.stable_dt.min(0.05),
        )
    });
    let Some(anchor) = state.get() else {
        if middle
            && let Some(p) = pos
            && view.contains(p)
        {
            state.set(Some(p));
        }
        return;
    };
    if middle || other || esc {
        state.set(None);
        return;
    }
    let p = pal(ui);
    if let Some(pos) = pos {
        let dy = pos.y - anchor.y;
        let dead = 14.0;
        if dy.abs() > dead {
            let speed = ((dy.abs() - dead) * 0.55).min(36.0) * dy.signum() * dt * 60.0;
            ui.scroll_with_delta(vec2(0.0, -speed));
        }
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::AllScroll);
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::unique("autoscroll-marker"),
    ));
    painter.circle_filled(anchor, 15.0, p.raised);
    painter.circle_stroke(anchor, 15.0, Stroke::new(1.5, p.border_strong));
    for s in [-1.0_f32, 1.0] {
        painter.add(egui::Shape::convex_polygon(
            vec![
                pos2(anchor.x - 4.5, anchor.y + s * 5.0),
                pos2(anchor.x + 4.5, anchor.y + s * 5.0),
                pos2(anchor.x, anchor.y + s * 10.5),
            ],
            p.muted,
            Stroke::NONE,
        ));
    }
    painter.circle_filled(anchor, 2.0, p.accent_text);
    ui.ctx().request_repaint();
}

/// Puts a line break at the caret (Shift+Enter). The editor itself never inserts one: Enter and Shift+Enter are ours.
fn insert_newline(ctx: &egui::Context, id: egui::Id, text: &mut String) {
    use egui::text::{CCursor, CCursorRange};
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let total = text.chars().count();
    let (a, b) = state.cursor.char_range().map_or((total, total), |r| {
        (r.primary.index.0.min(total), r.secondary.index.0.min(total))
    });
    let (start, end) = (a.min(b), a.max(b));
    let byte = |n: usize| text.char_indices().nth(n).map_or(text.len(), |(i, _)| i);
    let (bs, be) = (byte(start), byte(end));
    text.replace_range(bs..be, "\n");
    state
        .cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(start + 1))));
    state.store(ctx, id);
}

fn active_token(input: &str) -> Option<(usize, String)> {
    if input.ends_with(char::is_whitespace) || input.is_empty() {
        return None;
    }
    let start = input.rfind(char::is_whitespace).map_or(0, |idx| idx + 1);
    let tok = input.get(start..)?;
    if (tok.starts_with('@') || (tok.starts_with('/') && start == 0)) && tok.len() <= 40 {
        Some((start, tok.to_string()))
    } else {
        None
    }
}

fn replace_token(input: &mut String, start: usize, len: usize, replacement: &str) {
    if let Some(end) = start.checked_add(len)
        && end <= input.len()
        && input.is_char_boundary(start)
        && input.is_char_boundary(end)
    {
        input.replace_range(start..end, replacement);
    }
}

fn badge_color(label: &str) -> Color32 {
    match label {
        "PDF" => Color32::from_rgb(0xd9, 0x38, 0x3a),
        "DOC" => Color32::from_rgb(0x2b, 0x6c, 0xd4),
        "XLS" | "CSV" => Color32::from_rgb(0x1f, 0x8b, 0x4c),
        "PPT" => Color32::from_rgb(0xd9, 0x6b, 0x27),
        "DIR" => Color32::from_rgb(0x8b, 0x5c, 0xf6),
        "IMG" => Color32::from_rgb(0x0d, 0x94, 0x88),
        "HTML" | "JSON" | "XML" | "YAML" | "TOML" => Color32::from_rgb(0xca, 0x8a, 0x04),
        _ => crate::theme::brand(),
    }
}

fn file_badge(ui: &mut egui::Ui, p: &Palette, kind: &str, name: &str, size: &str) {
    let c = badge_color(kind);
    egui::Frame::new()
        .fill(p.bg)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (b_rect, _) = ui.allocate_exact_size(vec2(32.0, 18.0), Sense::hover());
                ui.painter()
                    .rect_filled(b_rect, CornerRadius::same(5), soft(c, 0x26));
                ui.painter().text(
                    b_rect.center(),
                    Align2::CENTER_CENTER,
                    kind,
                    FontId::proportional(10.0),
                    c,
                );
                ui.label(RichText::new(short(name, 28)).size(12.5).color(p.text));
                if !size.is_empty() {
                    ui.label(RichText::new(size).size(11.0).color(p.faint));
                }
            });
        });
}

fn attachment_card(ui: &mut egui::Ui, p: &Palette, c: &crate::attach::Attached) -> bool {
    let mut remove = false;
    let (rect, _) = ui.allocate_exact_size(vec2(224.0, 48.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(12), p.bg);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(12),
        Stroke::new(1.0, p.border),
        StrokeKind::Inside,
    );
    let badge_rect =
        Rect::from_min_size(pos2(rect.left() + 6.0, rect.top() + 6.0), Vec2::splat(36.0));
    if c.pending {
        egui::Spinner::new().size(18.0).color(p.accent).paint_at(
            ui,
            Rect::from_center_size(badge_rect.center(), Vec2::splat(18.0)),
        );
    } else if let Some(tex) = &c.thumb {
        ui.painter().image(
            tex.id(),
            badge_rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        let lbl = crate::attach::type_label(&c.ext);
        let col = badge_color(&lbl);
        ui.painter()
            .rect_filled(badge_rect, CornerRadius::same(8), soft(col, 0x28));
        ui.painter().text(
            badge_rect.center(),
            Align2::CENTER_CENTER,
            &lbl,
            FontId::proportional(11.0),
            col,
        );
    }
    let text_x = badge_rect.right() + 8.0;
    ui.painter().text(
        pos2(text_x, rect.top() + 16.0),
        Align2::LEFT_CENTER,
        short(&c.name, 21),
        FontId::proportional(12.5),
        p.text,
    );
    let sub = if c.pending {
        "Reading file\u{2026}".to_string()
    } else if c.note.is_empty() {
        crate::attach::human_size(c.size)
    } else if c.size > 0 {
        format!("{} \u{00b7} {}", crate::attach::human_size(c.size), c.note)
    } else {
        c.note.clone()
    };
    ui.painter().text(
        pos2(text_x, rect.top() + 33.0),
        Align2::LEFT_CENTER,
        short(&sub, 25),
        FontId::proportional(11.0),
        p.faint,
    );
    let close_r = Rect::from_center_size(
        pos2(rect.right() - 14.0, rect.top() + 14.0),
        Vec2::splat(20.0),
    );
    let close_resp = ui.interact(
        close_r,
        ui.make_persistent_id(("rm-chip", c.id)),
        Sense::click(),
    );
    if close_resp.hovered() {
        ui.painter()
            .rect_filled(close_r, CornerRadius::same(6), p.hover);
    }
    paint_icon(
        ui.painter(),
        close_r.shrink(5.0),
        Icon::Close,
        if close_resp.hovered() {
            p.text
        } else {
            p.faint
        },
    );
    if close_resp.on_hover_text("Remove attachment").clicked() {
        remove = true;
    }
    remove
}

fn turn_checkpoint_bar(
    ui: &mut egui::Ui,
    p: &Palette,
    turn_id: &str,
    cp: &serde_json::Value,
    out: &mut Vec<Action>,
) {
    let files = cp["files"].as_array();
    let n = files.map_or(0, Vec::len);
    if n == 0 {
        return;
    }
    let restored = cp["restored"] == true;
    egui::Frame::new()
        .fill(if p.dark { p.raised } else { p.bg })
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                paint_icon(ui.painter(), r, Icon::Undo, p.muted);
                let first_name = files
                    .and_then(|f| f.first())
                    .and_then(|f| f["path"].as_str())
                    .and_then(|s| std::path::Path::new(s).file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let summary = if n == 1 && !first_name.is_empty() {
                    format!("1 file changed ({first_name})")
                } else {
                    format!("{n} files changed in this turn")
                };
                ui.label(RichText::new(summary).size(12.5).color(p.muted));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if restored {
                        widgets::pill(ui, "Undone", p.faint, p.hover);
                    } else if widgets::button(ui, Some(Icon::Undo), "Undo changes", false)
                        .on_hover_text("Restore all files created or modified in this turn")
                        .clicked()
                    {
                        out.push(Action::Undo(turn_id.to_string()));
                    }
                });
            });
        });
}

fn short_lines(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}\u{2026}", s.chars().take(max).collect::<String>())
    }
}

/// Three dots that breathe while the answer has not started.
fn typing_dots(ui: &mut egui::Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(44.0, 22.0), Sense::hover());
    let t = ui.input(|i| i.time) as f32;
    for i in 0..3 {
        let k = ((t * 4.0 - i as f32 * 0.7).sin() * 0.5 + 0.5).powf(1.5);
        let c = pos2(
            rect.left() + 8.0 + i as f32 * 12.0,
            rect.center().y - 3.0 * k,
        );
        ui.painter()
            .circle_filled(c, 3.2, soft(p.muted, (90.0 + 150.0 * k) as u8));
    }
    ui.ctx().request_repaint();
}

/// The model's reasoning, folded by default. While it streams it stays open and shows the tail.
fn thought_block(ui: &mut egui::Ui, key: &str, text: &str, streaming: bool) {
    let p = pal(ui);
    let id = ui.make_persistent_id(("thought", key));
    let mut cs =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width().min(260.0), 26.0), Sense::click());
    if resp.clicked() {
        cs.toggle(ui);
    }
    let fg = if resp.hovered() { p.text } else { p.muted };
    paint_icon(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 8.0, rect.center().y), Vec2::splat(14.0)),
        Icon::Sparkle,
        fg,
    );
    let label = if streaming { "Thinking" } else { "Thoughts" };
    let g = ui
        .painter()
        .layout_no_wrap(label.to_string(), FontId::proportional(13.0), fg);
    let gw = g.size().x;
    ui.painter().galley(
        pos2(rect.left() + 22.0, rect.center().y - g.size().y / 2.0),
        g,
        fg,
    );
    if streaming {
        // A soft shimmer moves over the word while the model thinks.
        let t = ui.input(|i| i.time) as f32;
        let x = rect.left() + 22.0 + (t * 0.8).fract() * (gw + 20.0) - 10.0;
        ui.painter()
            .with_clip_rect(Rect::from_min_size(
                pos2(rect.left() + 22.0, rect.top()),
                vec2(gw, rect.height()),
            ))
            .rect_filled(
                Rect::from_center_size(pos2(x, rect.center().y), vec2(14.0, 16.0)),
                CornerRadius::same(4),
                soft(p.text, 0x30),
            );
        ui.ctx().request_repaint();
    }
    paint_icon(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.left() + 30.0 + gw, rect.center().y),
            Vec2::splat(11.0),
        ),
        if cs.is_open() {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        },
        p.faint,
    );
    let tail: String = if streaming && !cs.is_open() {
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        lines[lines.len().saturating_sub(2)..].join("\n")
    } else {
        String::new()
    };
    if !tail.is_empty() {
        quote_bar(ui, &p, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(short_lines(&tail, 280))
                        .size(13.0)
                        .italics()
                        .color(p.faint),
                )
                .wrap(),
            );
        });
    }
    cs.show_body_unindented(ui, |ui| {
        quote_bar(ui, &p, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(text.trim())
                        .size(13.0)
                        .color(p.muted)
                        .line_height(Some(20.0)),
                )
                .selectable(true)
                .wrap(),
            );
        });
    });
}

/// Content with a thin bar on its left, as a quote or a thought.
fn quote_bar(ui: &mut egui::Ui, p: &Palette, add: impl FnOnce(&mut egui::Ui)) {
    let r = egui::Frame::new()
        .inner_margin(Margin {
            left: 14,
            right: 0,
            top: 2,
            bottom: 2,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        })
        .response
        .rect;
    ui.painter().vline(
        r.left() + 4.0,
        r.y_range(),
        Stroke::new(2.0, p.border_strong),
    );
}

/// A titled monospace box with a copy button, for tool input and output.
fn mono_block(
    ui: &mut egui::Ui,
    title: &str,
    text: &str,
    key: &str,
    app: &App,
    out: &mut Vec<Action>,
) {
    let p = pal(ui);
    egui::Frame::new()
        .fill(p.code_bg)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(semi(title).size(11.5).color(p.faint));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let copied = app.just_copied(key);
                    if widgets::icon_button_sized(
                        ui,
                        if copied { Icon::Check } else { Icon::Copy },
                        if copied { "Copied" } else { "Copy" },
                        22.0,
                        false,
                    )
                    .clicked()
                    {
                        out.push(Action::Copy {
                            key: key.to_string(),
                            text: text.to_string(),
                        });
                    }
                });
            });
            egui::ScrollArea::vertical()
                .id_salt(key)
                .max_height(220.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(short_lines(text, 6000))
                                .monospace()
                                .size(12.0)
                                .color(p.muted),
                        )
                        .selectable(true)
                        .wrap(),
                    );
                });
        });
}

fn span_text(s: &Span, size: f32, p: &Palette) -> RichText {
    let mut t = RichText::new(&s.text).size(size).color(p.text);
    if s.bold {
        t = semi(t);
    }
    if s.italic {
        t = t.italics();
    }
    if s.code {
        t = RichText::new(format!(" {} ", s.text))
            .monospace()
            .size(size - 1.5)
            .color(if p.dark {
                Color32::from_rgb(0xe6, 0xc0, 0x8a)
            } else {
                Color32::from_rgb(0xb4, 0x3b, 0x2f)
            })
            .background_color(p.surface);
    }
    t
}

fn spans(ui: &mut egui::Ui, list: &[Span], size: f32, p: &Palette) {
    let mut job = egui::text::LayoutJob::default();
    for s in list {
        span_text(s, size, p).append_to(
            &mut job,
            ui.style(),
            egui::FontSelection::Default,
            Align::BOTTOM,
        );
    }
    for sec in &mut job.sections {
        sec.format.line_height = Some((size * 1.65).round());
    }
    ui.add(egui::Label::new(job).selectable(true).wrap());
}

/// Render chat Markdown. `key` makes copy buttons of code blocks unique.
pub(crate) fn markdown(ui: &mut egui::Ui, text: &str, key: &str, app: &App, out: &mut Vec<Action>) {
    let p = pal(ui);
    for (i, b) in parse_blocks(text).into_iter().enumerate() {
        match b {
            Block::Heading(level, list) => {
                let size = match level {
                    1 => 22.0,
                    2 => 19.0,
                    3 => 17.0,
                    _ => 15.5,
                };
                ui.add_space(if level <= 2 { 8.0 } else { 4.0 });
                let mut job = egui::text::LayoutJob::default();
                for s in &list {
                    semi(span_text(s, size, &p)).append_to(
                        &mut job,
                        ui.style(),
                        egui::FontSelection::Default,
                        Align::Center,
                    );
                }
                ui.add(egui::Label::new(job).selectable(true).wrap());
                ui.add_space(2.0);
            }
            Block::Paragraph(list) => {
                spans(ui, &list, 14.5, &p);
                ui.add_space(6.0);
            }
            Block::Quote(list) => {
                quote_bar(ui, &p, |ui| spans(ui, &list, 14.5, &p));
                ui.add_space(4.0);
            }
            Block::Item(level, marker, list) => {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(6.0 + 18.0 * level as f32);
                    let mw = if marker.len() > 1 { 22.0 } else { 16.0 };
                    let (r, _) = ui.allocate_exact_size(vec2(mw, 23.0), Sense::hover());
                    ui.painter().text(
                        pos2(r.left(), r.center().y),
                        Align2::LEFT_CENTER,
                        &marker,
                        FontId::proportional(14.5),
                        p.muted,
                    );
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        spans(ui, &list, 14.5, &p);
                    });
                });
                ui.add_space(2.0);
            }
            Block::Rule => {
                ui.add_space(6.0);
                let (r, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter()
                    .hline(r.x_range(), r.center().y, Stroke::new(1.0, p.border));
                ui.add_space(6.0);
            }
            Block::Code { lang, text } => {
                code_block(ui, &lang, &text, &format!("code-{key}-{i}"), app, out);
                ui.add_space(6.0);
            }
        }
    }
}

fn code_block(
    ui: &mut egui::Ui,
    lang: &str,
    text: &str,
    key: &str,
    app: &App,
    out: &mut Vec<Action>,
) {
    let p = pal(ui);
    ui.add_space(2.0);
    egui::Frame::new()
        .fill(p.code_bg)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::Frame::new()
                .fill(p.code_head)
                .corner_radius(CornerRadius {
                    nw: 12,
                    ne: 12,
                    sw: 0,
                    se: 0,
                })
                .inner_margin(Margin {
                    left: 14,
                    right: 6,
                    top: 3,
                    bottom: 3,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            medium(if lang.is_empty() { "text" } else { lang })
                                .size(12.0)
                                .color(p.muted),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let copied = app.just_copied(key);
                            let (rect, resp) = ui.allocate_exact_size(
                                vec2(if copied { 78.0 } else { 64.0 }, 26.0),
                                Sense::click(),
                            );
                            if resp.hovered() {
                                ui.painter()
                                    .rect_filled(rect, CornerRadius::same(6), p.hover);
                            }
                            let fg = if copied {
                                p.success
                            } else if resp.hovered() {
                                p.text
                            } else {
                                p.muted
                            };
                            paint_icon(
                                ui.painter(),
                                Rect::from_center_size(
                                    pos2(rect.left() + 14.0, rect.center().y),
                                    Vec2::splat(13.0),
                                ),
                                if copied { Icon::Check } else { Icon::Copy },
                                fg,
                            );
                            ui.painter().text(
                                pos2(rect.left() + 26.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                if copied { "Copied" } else { "Copy" },
                                FontId::proportional(12.0),
                                fg,
                            );
                            if resp.clicked() {
                                out.push(Action::Copy {
                                    key: key.to_string(),
                                    text: text.to_string(),
                                });
                            }
                        });
                    });
                });
            egui::Frame::new()
                .inner_margin(Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::horizontal()
                        .id_salt(key)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(text)
                                        .monospace()
                                        .size(13.0)
                                        .color(p.text)
                                        .line_height(Some(19.0)),
                                )
                                .selectable(true)
                                .wrap_mode(TextWrapMode::Extend),
                            );
                        });
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_read_well() {
        assert_eq!(
            split_tool("photocraft__command_run"),
            ("photocraft", "command_run")
        );
        assert_eq!(split_tool("read_file"), ("", "read_file"));
        assert_eq!(
            tool_title("photocraft__command_run"),
            "photocraft \u{00b7} command run"
        );
        assert_eq!(tool_title("read_file"), "read file");
    }
}
