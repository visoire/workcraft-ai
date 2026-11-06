//! The activity panel: who is working on what (team roster with live status), the plan as steps that
//! run in parallel, and every tool call of the turn.

use std::collections::HashMap;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, Rect, RichText, Sense,
    Stroke, Vec2, pos2, vec2,
};
use serde_json::json;
use workcraft_core::TaskStatus;

use crate::app::{AgentInfo, AgentStatus, App, TaskRow};
use crate::theme::{Palette, agent_color, pal, semi, soft};
use crate::view_chat::{split_tool, tool_title};
use crate::view_side::fmt_secs;
use crate::widgets::{self, Icon, icon_button, paint_icon, short};

pub(crate) fn status_look(s: AgentStatus, p: &Palette) -> (&'static str, Color32, bool) {
    match s {
        AgentStatus::Idle => ("Idle", p.faint, false),
        AgentStatus::Thinking => ("Thinking", p.accent_text, true),
        AgentStatus::Writing => ("Writing", p.accent_text, true),
        AgentStatus::Tool => ("Using a tool", p.accent_text, true),
        AgentStatus::Waiting => ("Needs you", p.warning, true),
        AgentStatus::Reviewing => ("Reviewing", p.accent_text, true),
        AgentStatus::Done => ("Finished", p.success, false),
        AgentStatus::Failed => ("Failed", p.danger, false),
    }
}

/// Longest chain of dependencies before each task: tasks with the same depth can run at the same time.
pub(crate) fn depths(tasks: &[TaskRow]) -> HashMap<String, usize> {
    let by_id: HashMap<&str, &TaskRow> = tasks.iter().map(|t| (t.id.as_str(), t)).collect();
    fn depth<'a>(
        id: &'a str,
        by_id: &HashMap<&'a str, &'a TaskRow>,
        memo: &mut HashMap<&'a str, usize>,
        guard: usize,
    ) -> usize {
        if let Some(d) = memo.get(id) {
            return *d;
        }
        // A cycle or a very deep chain stops here instead of looping.
        if guard > 64 {
            return 0;
        }
        let d = by_id.get(id).map_or(0, |t| {
            t.depends
                .iter()
                .filter(|d| by_id.contains_key(d.as_str()))
                .map(|d| depth(d, by_id, memo, guard + 1) + 1)
                .max()
                .unwrap_or(0)
        });
        memo.insert(id, d);
        d
    }
    let mut memo = HashMap::new();
    for t in tasks {
        depth(&t.id, &by_id, &mut memo, 0);
    }
    memo.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

impl App {
    pub(crate) fn activity(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        ui.horizontal(|ui| {
            let title = if self.mode_team { "Team" } else { "Activity" };
            ui.label(semi(title).size(15.0).color(p.text));
            if self.busy {
                widgets::dot(
                    ui,
                    pos2(ui.cursor().left() + 6.0, ui.cursor().center().y),
                    p.accent,
                    true,
                );
                ui.add_space(14.0);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, Icon::Close, "Hide activity").clicked() {
                    self.show_activity = false;
                    self.save_prefs();
                }
            });
        });
        ui.add_space(4.0);
        // Two views of the same side panel: what is happening, and what has been made.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let n = self.artifacts.1.len();
            for (i, label) in [
                (0u8, "Activity".to_string()),
                (
                    1u8,
                    if n > 0 {
                        format!("Artifacts {n}")
                    } else {
                        "Artifacts".to_string()
                    },
                ),
            ] {
                if widgets::chip(
                    ui,
                    if i == 0 { Icon::Bolt } else { Icon::Image },
                    &label,
                    self.activity_tab == i,
                    "",
                )
                .clicked()
                {
                    self.activity_tab = i;
                }
            }
        });
        ui.add_space(6.0);
        if self.activity_tab == 1 {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| self.artifacts_tab(ui));
            return;
        }
        let mut answer = None;
        let mut undo_turn = None;
        // An order for the running plan: (task id, "skip" or "redo", note).
        let mut steer: Option<(String, String, String)> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if !self.approvals.is_empty() {
                    for a in &self.approvals {
                        let (bg, border, fg) = if a.destructive {
                            (p.danger_soft, soft(p.danger, 0x80), p.danger)
                        } else {
                            (p.warning_soft, soft(p.warning, 0x70), p.warning)
                        };
                        egui::Frame::new()
                            .fill(bg)
                            .stroke(Stroke::new(1.0, border))
                            .corner_radius(CornerRadius::same(12))
                            .inner_margin(Margin::same(10))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    let (r, _) =
                                        ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                                    paint_icon(ui.painter(), r, Icon::Shield, fg);
                                    ui.label(semi("Waiting for you").size(13.0).color(p.text));
                                    if a.destructive {
                                        widgets::pill(
                                            ui,
                                            "Destructive",
                                            p.danger,
                                            soft(p.danger, 0x24),
                                        );
                                    }
                                });
                                ui.label(
                                    RichText::new(format!(
                                        "{} wants to run {}",
                                        self.agent_name(a.agent.as_deref()),
                                        tool_title(&a.tool)
                                    ))
                                    .size(12.5)
                                    .color(p.muted),
                                );
                                if let Some(reason) = &a.reason {
                                    ui.label(RichText::new(reason).size(11.5).color(fg));
                                }
                                ui.horizontal(|ui| {
                                    if widgets::button(ui, None, "Allow", true).clicked() {
                                        answer = Some((a.id.clone(), true));
                                    }
                                    if widgets::button(ui, None, "Deny", false).clicked() {
                                        answer = Some((a.id.clone(), false));
                                    }
                                });
                            });
                        ui.add_space(8.0);
                    }
                }
                if self.mode_team || !self.tasks.is_empty() {
                    self.roster_view(ui);
                    ui.add_space(10.0);
                    self.plan_view(ui, &mut steer);
                    ui.add_space(10.0);
                }
                if !self.checkpoints.is_empty() {
                    self.checkpoints_view(ui, &mut undo_turn);
                    ui.add_space(10.0);
                }
                self.tools_view(ui);
            });
        if let Some((id, allow)) = answer {
            self.respond(&id, allow);
        }
        if let Some((task, action, note)) = steer
            && let Some(session) = self.current.clone()
        {
            let msg = if action == "skip" {
                format!("Skipping {task}.")
            } else {
                format!("Running {task} again with your note.")
            };
            self.bridge.run(
                "steer",
                "team.steer",
                json!({ "session": session, "task": task, "action": action, "note": note }),
            );
            self.notify(msg);
        }
        if let Some(turn) = undo_turn {
            self.undo_turn(&turn);
        }
    }

    fn roster_view(&self, ui: &mut egui::Ui) {
        let p = pal(ui);
        widgets::caption(ui, "Agents");
        ui.add_space(2.0);
        let roster: Vec<AgentInfo> = if self.roster.is_empty() {
            // Before team.list answers, show whoever is active.
            self.live
                .keys()
                .map(|k| AgentInfo {
                    id: k.clone(),
                    name: self.agent_name(Some(k)),
                    scope: String::new(),
                    prompt: String::new(),
                    skills: Vec::new(),
                    tools: Vec::new(),
                    model: None,
                    approval: None,
                })
                .collect()
        } else {
            self.roster.clone()
        };
        for a in &roster {
            let live = self.live.get(&a.id);
            let status = live.map_or(AgentStatus::Idle, |l| l.status);
            let (label, color, pulsing) = status_look(status, &p);
            let tint = agent_color(&a.id, p.dark);
            let active = pulsing || status == AgentStatus::Waiting;
            egui::Frame::new()
                .fill(if active {
                    p.surface
                } else {
                    Color32::TRANSPARENT
                })
                .stroke(Stroke::new(
                    1.0,
                    if active { soft(tint, 0x70) } else { p.border },
                ))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_top(|ui| {
                        widgets::avatar(
                            ui,
                            32.0,
                            self.assets.agent(&a.id, &a.skills),
                            &a.name,
                            tint,
                        );
                        ui.add_space(2.0);
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.horizontal(|ui| {
                                ui.label(semi(&a.name).size(13.5).color(p.text));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(
                                        RichText::new(label)
                                            .size(11.5)
                                            .color(color)
                                            .variation("wght", 550.0),
                                    );
                                    let c = ui.cursor();
                                    widgets::dot(
                                        ui,
                                        pos2(c.right() - 4.0, c.center().y),
                                        color,
                                        pulsing,
                                    );
                                    ui.add_space(12.0);
                                });
                            });
                            let detail = live.map(|l| l.detail.as_str()).unwrap_or("");
                            let line = match status {
                                AgentStatus::Tool | AgentStatus::Waiting if !detail.is_empty() => {
                                    tool_title(detail)
                                }
                                _ if !detail.is_empty() => detail.to_string(),
                                _ => a.scope.clone(),
                            };
                            if !line.is_empty() {
                                ui.label(RichText::new(short(&line, 70)).size(12.0).color(p.muted));
                            }
                            if let Some(l) = live.filter(|l| {
                                status == AgentStatus::Writing && !l.text.trim().is_empty()
                            }) {
                                let tail: String =
                                    l.text.split_whitespace().collect::<Vec<_>>().join(" ");
                                let n = tail.chars().count();
                                let tail: String =
                                    tail.chars().skip(n.saturating_sub(90)).collect();
                                ui.label(
                                    RichText::new(format!("\u{2026}{tail}"))
                                        .size(11.5)
                                        .italics()
                                        .color(p.faint),
                                );
                            }
                        });
                    });
                });
            ui.add_space(6.0);
        }
    }

    fn plan_view(&self, ui: &mut egui::Ui, steer: &mut Option<(String, String, String)>) {
        let p = pal(ui);
        let total = self.tasks.len();
        let done = self
            .tasks
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Done | TaskStatus::Skipped))
            .count();
        ui.horizontal(|ui| {
            widgets::caption(ui, "Plan");
            if total > 0 {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{done} of {total} done"))
                            .size(11.5)
                            .color(p.faint),
                    );
                });
            }
        });
        if total == 0 {
            let msg = if self.busy {
                "The leader is writing the plan..."
            } else {
                "When you send a goal, the leader splits it into tasks for the specialists. The plan shows here, step by step."
            };
            ui.label(RichText::new(msg).size(12.5).color(p.faint));
            return;
        }
        // Progress bar.
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
        ui.painter()
            .rect_filled(bar, CornerRadius::same(2), p.pressed);
        let k = ui.ctx().animate_value_with_time(
            egui::Id::unique("plan-progress"),
            done as f32 / total as f32,
            0.3,
        );
        ui.painter().rect_filled(
            Rect::from_min_size(bar.min, vec2(bar.width() * k, bar.height())),
            CornerRadius::same(2),
            if done == total { p.success } else { p.accent },
        );
        ui.add_space(8.0);

        let depth = depths(&self.tasks);
        let steps = depth.values().copied().max().unwrap_or(0) + 1;
        let titles: HashMap<&str, &str> = self
            .tasks
            .iter()
            .map(|t| {
                (
                    t.id.as_str(),
                    if t.title.is_empty() {
                        t.id.as_str()
                    } else {
                        t.title.as_str()
                    },
                )
            })
            .collect();
        for s in 0..steps {
            let in_step: Vec<&TaskRow> = self
                .tasks
                .iter()
                .filter(|t| depth.get(&t.id).copied().unwrap_or(0) == s)
                .collect();
            if in_step.is_empty() {
                continue;
            }
            ui.horizontal(|ui| {
                ui.label(semi(format!("Step {}", s + 1)).size(11.5).color(p.muted));
                if in_step.len() > 1 {
                    ui.label(
                        RichText::new(format!("{} in parallel", in_step.len()))
                            .size(11.0)
                            .color(p.faint),
                    );
                }
            });
            for t in in_step {
                self.task_card(ui, t, &titles, &p, steer);
            }
            ui.add_space(4.0);
        }
    }

    fn task_card(
        &self,
        ui: &mut egui::Ui,
        t: &TaskRow,
        titles: &HashMap<&str, &str>,
        p: &Palette,
        steer: &mut Option<(String, String, String)>,
    ) {
        let tint = agent_color(&t.agent, p.dark);
        let running = matches!(t.status, TaskStatus::Running | TaskStatus::Reviewing);
        let r = egui::Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(
                1.0,
                if running { soft(tint, 0x80) } else { p.border },
            ))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin {
                left: 14,
                right: 10,
                top: 8,
                bottom: 8,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                    task_status_icon(ui, r, t.status, p);
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 3.0;
                        let title = if t.title.is_empty() { &t.id } else { &t.title };
                        ui.add(
                            egui::Label::new(semi(title.as_str()).size(13.0).color(p.text)).wrap(),
                        );
                        ui.horizontal_wrapped(|ui| {
                            if !t.agent.is_empty() {
                                let (r, _) =
                                    ui.allocate_exact_size(vec2(8.0, 14.0), Sense::hover());
                                ui.painter().circle_filled(r.center(), 3.5, tint);
                                ui.label(
                                    RichText::new(self.agent_name(Some(&t.agent)))
                                        .size(11.5)
                                        .color(p.muted),
                                );
                            }
                            let (label, color) = match t.status {
                                TaskStatus::Pending => ("waiting", p.faint),
                                TaskStatus::Running => ("working", p.accent_text),
                                TaskStatus::Reviewing => ("in review", p.accent_text),
                                TaskStatus::Done => ("done", p.success),
                                TaskStatus::Failed => ("failed", p.danger),
                                TaskStatus::Skipped => ("skipped", p.faint),
                            };
                            ui.label(RichText::new(label).size(11.5).color(color));
                        });
                        if !t.depends.is_empty() {
                            let after: Vec<String> = t
                                .depends
                                .iter()
                                .map(|d| short(titles.get(d.as_str()).copied().unwrap_or(d), 24))
                                .collect();
                            ui.label(
                                RichText::new(format!("after {}", after.join(", ")))
                                    .size(11.0)
                                    .color(p.faint),
                            );
                        }
                        if let Some(n) = t.note.as_deref().filter(|n| !n.trim().is_empty()) {
                            ui.label(RichText::new(short(n, 160)).size(11.5).color(p.muted))
                                .on_hover_text(n);
                        }
                        // Steering: while the plan runs, a task that waits, works or failed can be left out or redone.
                        let steerable = self.busy
                            && matches!(
                                t.status,
                                TaskStatus::Pending | TaskStatus::Running | TaskStatus::Failed
                            );
                        if steerable {
                            let open_id = ui.make_persistent_id(("steer-open", &t.id));
                            let note_id = ui.make_persistent_id(("steer-note", &t.id));
                            let open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                if widgets::chip(
                                    ui,
                                    Icon::Close,
                                    "Skip",
                                    false,
                                    "Leave this task out (tasks that need it are skipped too)",
                                )
                                .clicked()
                                {
                                    *steer = Some((t.id.clone(), "skip".into(), String::new()));
                                }
                                if widgets::chip(
                                    ui,
                                    Icon::Refresh,
                                    "Redo",
                                    open,
                                    "Run this task again now, with an extra instruction",
                                )
                                .clicked()
                                {
                                    ui.data_mut(|d| d.insert_temp(open_id, !open));
                                }
                            });
                            if open {
                                let mut note = ui
                                    .data(|d| d.get_temp::<String>(note_id))
                                    .unwrap_or_default();
                                ui.add(
                                    egui::TextEdit::multiline(&mut note)
                                        .desired_rows(2)
                                        .desired_width(f32::INFINITY)
                                        .hint_text("What should it do differently?"),
                                );
                                ui.data_mut(|d| d.insert_temp(note_id, note.clone()));
                                if widgets::button(ui, None, "Run again", true).clicked() {
                                    *steer = Some((t.id.clone(), "redo".into(), note));
                                    ui.data_mut(|d| {
                                        d.insert_temp(open_id, false);
                                        d.insert_temp(note_id, String::new());
                                    });
                                }
                            }
                        }
                    });
                });
            })
            .response
            .rect;
        // The agent colour as a strip on the left edge.
        ui.painter().rect_filled(
            Rect::from_min_max(
                pos2(r.left() + 1.0, r.top() + 8.0),
                pos2(r.left() + 4.0, r.bottom() - 8.0),
            ),
            CornerRadius::same(2),
            tint,
        );
        ui.add_space(6.0);
    }

    fn tools_view(&self, ui: &mut egui::Ui) {
        let p = pal(ui);
        ui.horizontal(|ui| {
            widgets::caption(ui, "Tool calls");
            if !self.tools.is_empty() {
                ui.label(
                    RichText::new(self.tools.len().to_string())
                        .size(11.5)
                        .color(p.faint),
                );
            }
        });
        if self.tools.is_empty() {
            ui.label(
                RichText::new("Tools the agents run during a reply show here: Craft app commands, files, MCP servers.")
                    .size(12.5)
                    .color(p.faint),
            );
            return;
        }
        for t in self.tools.iter().rev().take(60) {
            let id = ui.make_persistent_id(("tool-log", &t.id));
            let mut cs = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                false,
            );
            let (rect, resp) = widgets::row(ui, cs.is_open(), 30.0);
            if resp.clicked() {
                cs.toggle(ui);
            }
            let mut x = rect.left() + 6.0;
            let ir = Rect::from_min_size(pos2(x, rect.center().y - 8.0), Vec2::splat(16.0));
            match self
                .assets
                .app(&t.name)
                .filter(|_| !split_tool(&t.name).0.is_empty())
            {
                Some(tex) => {
                    ui.painter().image(
                        tex.id(),
                        ir,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                None => paint_icon(ui.painter(), ir, Icon::Terminal, p.muted),
            }
            x += 22.0;
            if let Some(a) = &t.agent {
                ui.painter().circle_filled(
                    pos2(x + 3.0, rect.center().y),
                    3.5,
                    agent_color(a, p.dark),
                );
                x += 12.0;
            }
            let right = 56.0;
            let g = ui.painter().layout_no_wrap(
                tool_title(&t.name),
                FontId::proportional(12.5),
                p.text,
            );
            ui.painter()
                .with_clip_rect(Rect::from_min_max(
                    pos2(x, rect.top()),
                    pos2(rect.right() - right, rect.bottom()),
                ))
                .galley(pos2(x, rect.center().y - g.size().y / 2.0), g, p.text);
            let sx = rect.right() - 12.0;
            match &t.result {
                None => {
                    egui::Spinner::new().size(12.0).color(p.accent).paint_at(
                        ui,
                        Rect::from_center_size(pos2(sx, rect.center().y), Vec2::splat(12.0)),
                    );
                }
                Some((ok, _)) => {
                    let ic = if *ok { Icon::Check } else { Icon::Close };
                    paint_icon(
                        ui.painter(),
                        Rect::from_center_size(pos2(sx, rect.center().y), Vec2::splat(12.0)),
                        ic,
                        if *ok { p.success } else { p.danger },
                    );
                    if let Some(f) = t.finished {
                        ui.painter().text(
                            pos2(sx - 12.0, rect.center().y),
                            Align2::RIGHT_CENTER,
                            fmt_secs(f - t.started),
                            FontId::proportional(11.0),
                            p.faint,
                        );
                    }
                }
            }
            cs.show_body_unindented(ui, |ui| {
                egui::Frame::new()
                    .fill(p.code_bg)
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if let Some(a) = &t.agent {
                            ui.label(
                                RichText::new(self.agent_name(Some(a)))
                                    .size(11.5)
                                    .color(p.faint),
                            );
                        }
                        ui.add(
                            egui::Label::new(
                                RichText::new(short_text(&t.args, 800))
                                    .monospace()
                                    .size(11.5)
                                    .color(p.muted),
                            )
                            .wrap(),
                        );
                        if let Some((_, out)) = &t.result {
                            ui.separator();
                            ui.add(
                                egui::Label::new(
                                    RichText::new(short_text(out, 1200))
                                        .monospace()
                                        .size(11.5)
                                        .color(p.muted),
                                )
                                .wrap(),
                            );
                        }
                    });
                ui.add_space(4.0);
            });
        }
    }

    fn checkpoints_view(&self, ui: &mut egui::Ui, undo_turn: &mut Option<String>) {
        let p = pal(ui);
        widgets::caption(ui, "Changed files & undo");
        ui.add_space(4.0);
        let mut list: Vec<(&String, &serde_json::Value)> = self.checkpoints.iter().collect();
        list.sort_by_key(|(_, v)| std::cmp::Reverse(v["ts"].as_i64().unwrap_or(0)));
        for (turn, ckpt) in list {
            let restored = ckpt["restored"] == true;
            let files = ckpt["files"].as_array();
            let count = files.map_or(0, Vec::len);
            egui::Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            semi(if count == 1 {
                                "1 file changed".to_string()
                            } else {
                                format!("{count} files changed")
                            })
                            .size(12.5)
                            .color(p.text),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if restored {
                                widgets::pill(ui, "Undone", p.faint, p.pressed);
                            } else if widgets::chip(
                                ui,
                                Icon::Undo,
                                "Undo",
                                false,
                                "Restore these files to how they were before this reply",
                            )
                            .clicked()
                            {
                                *undo_turn = Some(turn.clone());
                            }
                        });
                    });
                    for f in files.into_iter().flatten().take(6) {
                        let path = f["path"].as_str().unwrap_or_default();
                        let name = std::path::Path::new(path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.to_string());
                        let created = f["created"] == true;
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(if created { "+" } else { "~" })
                                    .monospace()
                                    .size(11.5)
                                    .color(if created { p.success } else { p.accent_text }),
                            );
                            ui.label(RichText::new(short(&name, 36)).size(11.5).color(p.muted))
                                .on_hover_text(path);
                        });
                    }
                });
            ui.add_space(6.0);
        }
    }
}

fn short_text(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}\u{2026}", s.chars().take(max).collect::<String>())
    }
}

fn task_status_icon(ui: &egui::Ui, r: Rect, s: TaskStatus, p: &Palette) {
    let c = r.center();
    let painter = ui.painter();
    match s {
        TaskStatus::Pending => {
            painter.circle_stroke(c, 7.0, Stroke::new(1.5, p.border_strong));
        }
        TaskStatus::Running | TaskStatus::Reviewing => {
            egui::Spinner::new()
                .size(15.0)
                .color(p.accent)
                .paint_at(ui, r);
        }
        TaskStatus::Done => {
            painter.circle_filled(c, 8.0, p.success);
            paint_icon(
                painter,
                r.shrink(4.0),
                Icon::Check,
                if p.dark {
                    Color32::from_gray(0x14)
                } else {
                    Color32::WHITE
                },
            );
        }
        TaskStatus::Failed => {
            painter.circle_filled(c, 8.0, p.danger);
            paint_icon(painter, r.shrink(5.0), Icon::Close, Color32::WHITE);
        }
        TaskStatus::Skipped => {
            painter.circle_stroke(c, 7.0, Stroke::new(1.5, p.faint));
            painter.hline((c.x - 3.5)..=(c.x + 3.5), c.y, Stroke::new(1.5, p.faint));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str, deps: &[&str]) -> TaskRow {
        TaskRow {
            id: id.into(),
            title: id.into(),
            agent: "image".into(),
            status: TaskStatus::Pending,
            note: None,
            depends: deps.iter().map(|d| (*d).to_string()).collect(),
        }
    }

    #[test]
    fn plan_steps_follow_dependencies() {
        let tasks = vec![
            t("a", &[]),
            t("b", &[]),
            t("c", &["a", "b"]),
            t("d", &["c"]),
            t("e", &["ghost"]),
        ];
        let d = depths(&tasks);
        assert_eq!((d["a"], d["b"], d["c"], d["d"], d["e"]), (0, 0, 1, 2, 0));
    }

    #[test]
    fn a_dependency_cycle_does_not_hang() {
        let tasks = vec![t("a", &["b"]), t("b", &["a"])];
        let d = depths(&tasks);
        assert_eq!(d.len(), 2);
    }
}
