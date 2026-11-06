//! Blue Blip, the interactive 18x15 pixel-art mascot of WorkCraft.
//!
//! Thirteen states: `Idle`, `Thinking`, `Working` (a tool runs), `Waiting` (needs your OK), `Listening` (voice),
//! `Success`, `Error`, and the short reactions `Happy`, `Startled`, `Wave`, `Walk`, `Spawn`, `Sleep`. Eight accessories,
//! ten colour themes, three animation levels (Full, Calm, Off), speech bubbles that can be switched off and three sizes.
//! Every pixel sits on a whole physical pixel of the screen (no shimmering edges, no eyes of changing width), and
//! the animation costs about nothing: 30 frames a second while something happens, 10 while idle, none when Off.

use eframe::egui::{self, Color32, CornerRadius, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlipState {
    Idle,
    Walk,
    /// A model is thinking: eyes drift up and to the side, a thought bubble grows over the head.
    Thinking,
    /// A tool is running: narrowed eyes, busy hands, a turning gear.
    Working,
    Happy,
    Startled,
    Spawn,
    Wave,
    Sleep,
    /// The microphone is open: wide eyes, a headset and sound arcs that pulse outwards.
    Listening,
    /// Something failed: crossed eyes, a shake, drooping antennae and a drop of sweat.
    Error,
    /// A job is done: a hop with both arms up, a check mark and sparkles.
    Success,
    /// A call waits for your answer: a raised hand and a question mark.
    Waiting,
}

impl BlipState {
    pub const ALL: [Self; 13] = [
        Self::Idle,
        Self::Thinking,
        Self::Working,
        Self::Waiting,
        Self::Listening,
        Self::Success,
        Self::Error,
        Self::Happy,
        Self::Startled,
        Self::Wave,
        Self::Walk,
        Self::Spawn,
        Self::Sleep,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Walk => "Walk",
            Self::Thinking => "Thinking",
            Self::Working => "Working",
            Self::Happy => "Happy",
            Self::Startled => "Startled",
            Self::Spawn => "Spawn",
            Self::Wave => "Wave",
            Self::Sleep => "Sleep",
            Self::Listening => "Listening",
            Self::Error => "Error",
            Self::Success => "Success",
            Self::Waiting => "Waiting",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlipAccessory {
    None,
    Glasses,
    Headset,
    Crown,
    Coffee,
    Hardhat,
    Cap,
    Bowtie,
}

impl BlipAccessory {
    pub const ALL: [Self; 8] = [
        Self::None,
        Self::Glasses,
        Self::Headset,
        Self::Crown,
        Self::Coffee,
        Self::Hardhat,
        Self::Cap,
        Self::Bowtie,
    ];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Glasses => "glasses",
            Self::Headset => "headset",
            Self::Crown => "crown",
            Self::Coffee => "coffee",
            Self::Hardhat => "hardhat",
            Self::Cap => "cap",
            Self::Bowtie => "bowtie",
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Glasses => "Glasses",
            Self::Headset => "Headset",
            Self::Crown => "Crown",
            Self::Coffee => "Coffee",
            Self::Hardhat => "Hardhat",
            Self::Cap => "Cap",
            Self::Bowtie => "Bow tie",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" => Self::None,
            "glasses" => Self::Glasses,
            "crown" => Self::Crown,
            "coffee" => Self::Coffee,
            "hardhat" => Self::Hardhat,
            "cap" => Self::Cap,
            "bowtie" => Self::Bowtie,
            _ => Self::Headset,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlipTheme {
    Blue,
    Indigo,
    Ice,
    Teal,
    Amber,
    Coral,
    Emerald,
    Mono,
    Violet,
    Rose,
}

impl BlipTheme {
    /// The body colour, which the whole app uses as its accent.
    #[must_use]
    pub fn accent(self) -> Color32 {
        palette(self).body
    }

    pub const ALL: [Self; 10] = [
        Self::Blue,
        Self::Indigo,
        Self::Ice,
        Self::Teal,
        Self::Amber,
        Self::Coral,
        Self::Emerald,
        Self::Mono,
        Self::Violet,
        Self::Rose,
    ];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Blue => "blue",
            Self::Indigo => "indigo",
            Self::Ice => "ice",
            Self::Teal => "teal",
            Self::Amber => "amber",
            Self::Coral => "coral",
            Self::Emerald => "emerald",
            Self::Mono => "mono",
            Self::Violet => "violet",
            Self::Rose => "rose",
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Blue => "Blue",
            Self::Indigo => "Indigo",
            Self::Ice => "Ice",
            Self::Teal => "Teal",
            Self::Amber => "Amber",
            Self::Coral => "Coral",
            Self::Emerald => "Emerald",
            Self::Mono => "Mono",
            Self::Violet => "Violet",
            Self::Rose => "Rose",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "indigo" => Self::Indigo,
            "ice" => Self::Ice,
            "teal" => Self::Teal,
            "amber" => Self::Amber,
            "coral" => Self::Coral,
            "emerald" => Self::Emerald,
            "mono" => Self::Mono,
            "violet" => Self::Violet,
            "rose" => Self::Rose,
            _ => Self::Blue,
        }
    }
}

#[derive(Clone, Copy)]
struct BlipPalette {
    body: Color32,
    top: Color32,
    shade: Color32,
    foot: Color32,
    ink: Color32,
    shine: Color32,
    tip: Color32,
    blush: Color32,
    alert: Color32,
}

fn palette(theme: BlipTheme) -> BlipPalette {
    match theme {
        BlipTheme::Blue => BlipPalette {
            body: Color32::from_rgb(0x2C, 0x84, 0xFC),
            top: Color32::from_rgb(0x2C, 0x84, 0xFC),
            shade: Color32::from_rgb(0x1E, 0x64, 0xD0),
            foot: Color32::from_rgb(0x16, 0x4D, 0xA8),
            ink: Color32::from_rgb(0x08, 0x1C, 0x38),
            shine: Color32::WHITE,
            tip: Color32::from_rgb(0xFF, 0xBF, 0x35),
            blush: Color32::from_rgb(0xFE, 0x94, 0x7F),
            alert: Color32::from_rgb(0xFF, 0x64, 0x58),
        },
        BlipTheme::Indigo => BlipPalette {
            body: Color32::from_rgb(0x5E, 0x5C, 0xE6),
            top: Color32::from_rgb(0x5E, 0x5C, 0xE6),
            shade: Color32::from_rgb(0x46, 0x43, 0xC4),
            foot: Color32::from_rgb(0x34, 0x31, 0x9C),
            ink: Color32::from_rgb(0x13, 0x11, 0x3A),
            shine: Color32::WHITE,
            tip: Color32::from_rgb(0xFF, 0xCC, 0x3D),
            blush: Color32::from_rgb(0xFF, 0x93, 0xB8),
            alert: Color32::from_rgb(0xFF, 0x64, 0x58),
        },
        BlipTheme::Ice => BlipPalette {
            body: Color32::from_rgb(0x2A, 0xA8, 0xD8),
            top: Color32::from_rgb(0x2A, 0xA8, 0xD8),
            shade: Color32::from_rgb(0x1D, 0x86, 0xB2),
            foot: Color32::from_rgb(0x15, 0x67, 0x8C),
            ink: Color32::from_rgb(0x07, 0x26, 0x36),
            shine: Color32::WHITE,
            tip: Color32::from_rgb(0xFF, 0xBF, 0x35),
            blush: Color32::from_rgb(0xFF, 0x9A, 0x8F),
            alert: Color32::from_rgb(0xFF, 0x64, 0x58),
        },
        BlipTheme::Teal => BlipPalette {
            body: Color32::from_rgb(0x10, 0xA3, 0x7F),
            top: Color32::from_rgb(0x10, 0xA3, 0x7F),
            shade: Color32::from_rgb(0x0C, 0x80, 0x63),
            foot: Color32::from_rgb(0x09, 0x62, 0x4C),
            ink: Color32::from_rgb(0x06, 0x28, 0x20),
            shine: Color32::WHITE,
            tip: Color32::from_rgb(0xFF, 0xBF, 0x35),
            blush: Color32::from_rgb(0xFF, 0x9A, 0x8B),
            alert: Color32::from_rgb(0xFF, 0x64, 0x58),
        },
        BlipTheme::Amber => BlipPalette {
            body: Color32::from_rgb(0xE6, 0x86, 0x19),
            top: Color32::from_rgb(0xE6, 0x86, 0x19),
            shade: Color32::from_rgb(0xC2, 0x6B, 0x0F),
            foot: Color32::from_rgb(0x9A, 0x52, 0x0A),
            ink: Color32::from_rgb(0x2D, 0x18, 0x04),
            shine: Color32::WHITE,
            tip: Color32::from_rgb(0x2C, 0x84, 0xFC),
            blush: Color32::from_rgb(0xFF, 0x6B, 0x5B),
            alert: Color32::from_rgb(0xE5, 0x48, 0x4D),
        },
        BlipTheme::Coral => BlipPalette {
            body: Color32::from_rgb(0xE5, 0x48, 0x4D),
            top: Color32::from_rgb(0xE5, 0x48, 0x4D),
            shade: Color32::from_rgb(0xC2, 0x34, 0x39),
            foot: Color32::from_rgb(0x9B, 0x25, 0x2A),
            ink: Color32::from_rgb(0x2C, 0x0B, 0x0E),
            shine: Color32::WHITE,
            tip: Color32::from_rgb(0xFF, 0xCC, 0x3D),
            blush: Color32::from_rgb(0xFF, 0xB3, 0xA7),
            alert: Color32::from_rgb(0xFF, 0xBF, 0x35),
        },
        BlipTheme::Emerald => BlipPalette {
            body: Color32::from_rgb(0x1F, 0x9D, 0x55),
            top: Color32::from_rgb(0x1F, 0x9D, 0x55),
            shade: Color32::from_rgb(0x16, 0x7B, 0x42),
            foot: Color32::from_rgb(0x10, 0x5E, 0x32),
            ink: Color32::from_rgb(0x07, 0x26, 0x15),
            shine: Color32::WHITE,
            tip: Color32::from_rgb(0xFF, 0xCC, 0x3D),
            blush: Color32::from_rgb(0xFF, 0x9A, 0x8B),
            alert: Color32::from_rgb(0xFF, 0x64, 0x58),
        },
        BlipTheme::Violet => BlipPalette {
            body: Color32::from_rgb(0x8B, 0x5C, 0xF6),
            top: Color32::from_rgb(0x8B, 0x5C, 0xF6),
            shade: Color32::from_rgb(0x6D, 0x3F, 0xD8),
            foot: Color32::from_rgb(0x52, 0x2D, 0xB0),
            ink: Color32::from_rgb(0x1B, 0x0C, 0x3A),
            shine: Color32::WHITE,
            tip: Color32::from_rgb(0xFF, 0xCC, 0x3D),
            blush: Color32::from_rgb(0xFF, 0x9A, 0xC8),
            alert: Color32::from_rgb(0xFF, 0x64, 0x58),
        },
        BlipTheme::Rose => BlipPalette {
            body: Color32::from_rgb(0xEC, 0x48, 0x99),
            top: Color32::from_rgb(0xEC, 0x48, 0x99),
            shade: Color32::from_rgb(0xC8, 0x2F, 0x7B),
            foot: Color32::from_rgb(0x9D, 0x1F, 0x5E),
            ink: Color32::from_rgb(0x34, 0x08, 0x20),
            shine: Color32::WHITE,
            tip: Color32::from_rgb(0xFF, 0xD1, 0x4D),
            blush: Color32::from_rgb(0xFF, 0xB3, 0xCC),
            alert: Color32::from_rgb(0xFF, 0x64, 0x58),
        },
        BlipTheme::Mono => BlipPalette {
            body: Color32::from_rgb(0x4B, 0x55, 0x63),
            top: Color32::from_rgb(0x4B, 0x55, 0x63),
            shade: Color32::from_rgb(0x37, 0x41, 0x51),
            foot: Color32::from_rgb(0x1F, 0x29, 0x37),
            ink: Color32::from_rgb(0x09, 0x0D, 0x14),
            shine: Color32::WHITE,
            tip: Color32::from_rgb(0x60, 0xA5, 0xFA),
            blush: Color32::from_rgb(0x9C, 0xA3, 0xAF),
            alert: Color32::from_rgb(0xF8, 0x71, 0x71),
        },
    }
}

/// How much Blip moves. Full: 30 frames a second while something happens. Calm: 15, and no hopping. Off: a still picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlipAnim {
    Full,
    Calm,
    Off,
}

impl BlipAnim {
    pub const ALL: [Self; 3] = [Self::Full, Self::Calm, Self::Off];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Calm => "calm",
            Self::Off => "off",
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "Full",
            Self::Calm => "Calm",
            Self::Off => "Off",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "calm" => Self::Calm,
            "off" => Self::Off,
            _ => Self::Full,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlipSize {
    Small,
    Medium,
    Large,
}

impl BlipSize {
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Medium => "Medium",
            Self::Large => "Large",
        }
    }

    #[must_use]
    pub fn factor(self) -> f32 {
        match self {
            Self::Small => 0.8,
            Self::Medium => 1.0,
            Self::Large => 1.25,
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "small" => Self::Small,
            "large" => Self::Large,
            _ => Self::Medium,
        }
    }
}

/// Interactive Blip controller held on `App`.
pub struct Blip {
    pub base_state: BlipState,
    /// Temporary triggered state `(state, started_at, until_time)`.
    pub temp_state: Option<(BlipState, f64, f64)>,
    /// Temporary context accessory `(accessory, until_time)`.
    pub temp_accessory: Option<(BlipAccessory, f64)>,
    /// Speech bubble `(text, until_time, is_error)`.
    pub bubble: Option<(String, f64, bool)>,
    pub accessory: BlipAccessory,
    pub theme: BlipTheme,
    pub anim: BlipAnim,
    /// Speech bubbles on or off.
    pub bubbles: bool,
    pub size: BlipSize,
    pub last_activity: f64,
    /// When Blip last said something about a long wait, so the nudges are 25 s apart.
    last_nudge: f64,
    click_idx: usize,
}

impl Default for Blip {
    fn default() -> Self {
        Self {
            base_state: BlipState::Idle,
            temp_state: Some((BlipState::Spawn, 0.0, 1.5)),
            temp_accessory: None,
            bubble: Some(("Ta-da! Ready to help!".into(), 2.8, false)),
            accessory: BlipAccessory::Headset,
            theme: BlipTheme::Blue,
            anim: BlipAnim::Full,
            bubbles: true,
            size: BlipSize::Medium,
            last_activity: 0.0,
            last_nudge: 0.0,
            click_idx: 0,
        }
    }
}

impl Blip {
    /// Record user/agent activity so Blip wakes up if sleeping.
    pub fn touch(&mut self, now: f64) {
        if self.base_state == BlipState::Sleep {
            self.base_state = BlipState::Idle;
        }
        self.last_activity = now;
    }

    /// Set the persistent lifecycle state (`Idle`, `Thinking`, `Working`, `Listening`, `Sleep`).
    pub fn set_state(&mut self, state: BlipState) {
        self.base_state = state;
    }

    /// Trigger a temporary reaction (`Happy`, `Startled`, `Wave`, `Walk`, `Spawn`) for `duration_secs`.
    pub fn trigger(&mut self, state: BlipState, duration_secs: f64, now: f64) {
        self.touch(now);
        self.temp_state = Some((state, now, now + duration_secs));
    }

    /// Temporarily equip an accessory (e.g. `Glasses` while reading, `Hardhat` while building).
    pub fn equip_temp(&mut self, acc: BlipAccessory, duration_secs: f64, now: f64) {
        self.temp_accessory = Some((acc, now + duration_secs));
    }

    /// Show a speech bubble above Blip for `duration_secs`.
    pub fn say(&mut self, text: impl Into<String>, duration_secs: f64, now: f64) {
        self.say_kind(text, duration_secs, now, false);
    }

    /// Show an error or normal speech bubble above Blip.
    pub fn say_kind(
        &mut self,
        text: impl Into<String>,
        duration_secs: f64,
        now: f64,
        is_error: bool,
    ) {
        let t = text.into();
        if t.trim().is_empty() {
            self.bubble = None;
        } else {
            self.bubble = Some((t, now + duration_secs, is_error));
        }
    }

    /// React to a tool starting: picks an animation, accessory, and speech bubble tailored to the tool.
    pub fn on_tool_start(&mut self, tool_name: &str, now: f64) {
        self.touch(now);
        self.base_state = BlipState::Working;

        if tool_name == "open_craft_app" {
            self.trigger(BlipState::Walk, 1.5, now);
            self.equip_temp(BlipAccessory::Hardhat, 6.0, now);
            self.say("Opening Craft desktop app...", 2.8, now);
            return;
        }

        if let Some((srv, sub)) = tool_name.split_once("__") {
            let app_label = workcraft_agents::craft::find_spec(srv).map_or(srv, |s| s.name);
            if matches!(
                sub,
                "open_document"
                    | "new_document"
                    | "open_workbook"
                    | "new_workbook"
                    | "open_presentation"
                    | "new_presentation"
                    | "start_app"
                    | "open_app"
                    | "launch"
            ) {
                self.trigger(BlipState::Walk, 1.4, now);
                self.equip_temp(BlipAccessory::Hardhat, 8.0, now);
                self.say(format!("Opening {app_label}..."), 2.6, now);
            } else if sub.contains("inspect") || sub.contains("read") || sub.contains("list") {
                self.equip_temp(BlipAccessory::Glasses, 5.0, now);
                self.say(format!("Inspecting {app_label}..."), 2.4, now);
            } else {
                self.equip_temp(BlipAccessory::Hardhat, 6.0, now);
                self.say(format!("Working in {app_label}..."), 2.5, now);
            }
            return;
        }

        match tool_name {
            "read_file" | "file_info" => {
                self.equip_temp(BlipAccessory::Glasses, 4.5, now);
                self.say("Reading file...", 2.0, now);
            }
            "list_dir" | "search_files" => {
                self.equip_temp(BlipAccessory::Glasses, 4.5, now);
                self.say("Searching files...", 2.0, now);
            }
            "read_skill" | "list_skills" => {
                self.equip_temp(BlipAccessory::Glasses, 4.5, now);
                self.say("Checking skill guide...", 2.0, now);
            }
            "fetch_url" => {
                self.equip_temp(BlipAccessory::Glasses, 5.0, now);
                self.say("Reading web page...", 2.4, now);
            }
            "create_document" => {
                self.equip_temp(BlipAccessory::Hardhat, 6.0, now);
                self.say("Building Word document...", 2.5, now);
            }
            "create_spreadsheet" => {
                self.equip_temp(BlipAccessory::Hardhat, 6.0, now);
                self.say("Building spreadsheet...", 2.5, now);
            }
            "create_presentation" => {
                self.equip_temp(BlipAccessory::Hardhat, 6.0, now);
                self.say("Building presentation...", 2.5, now);
            }
            "write_file" | "edit_file" | "copy_file" => {
                self.equip_temp(BlipAccessory::Hardhat, 5.0, now);
                self.say("Updating file...", 2.2, now);
            }
            _ => {
                self.say(format!("Running {tool_name}..."), 2.0, now);
            }
        }
    }

    /// React to a tool finishing (success or failure).
    pub fn on_tool_finished(&mut self, tool_name: &str, ok: bool, output: &str, now: f64) {
        self.touch(now);
        self.base_state = BlipState::Thinking;
        if !ok {
            self.trigger(BlipState::Error, 3.0, now);
            let short_tool = tool_name.split("__").last().unwrap_or(tool_name);
            let snippet: String = output
                .lines()
                .next()
                .unwrap_or("Tool failed")
                .chars()
                .take(38)
                .collect();
            self.say_kind(format!("Oops ({short_tool}): {snippet}"), 3.2, now, true);
        }
    }

    /// A model call is being retried (rate limit, busy server, no connection): look worried, say why.
    pub fn on_notice(&mut self, message: &str, now: f64) {
        self.touch(now);
        self.trigger(BlipState::Startled, 1.6, now);
        self.equip_temp(BlipAccessory::Glasses, 6.0, now);
        let line: String = message.chars().take(70).collect();
        self.say_kind(line, 5.0, now, false);
    }

    /// A tool returned a picture (render, preview, screenshot): celebrate it.
    pub fn on_render(&mut self, tool_name: &str, now: f64) {
        self.touch(now);
        self.trigger(BlipState::Happy, 2.2, now);
        let app = tool_name
            .split_once("__")
            .and_then(|(srv, _)| workcraft_agents::craft::find_spec(srv))
            .map_or("the app", |s| s.name);
        self.say(format!("A picture from {app}! Click it to open."), 3.4, now);
    }

    /// Called every frame while a turn runs: after a long silence say it is still going, every 25 s.
    pub fn on_slow(&mut self, elapsed_secs: f64, now: f64) {
        if elapsed_secs < 20.0 || now - self.last_nudge < 25.0 {
            return;
        }
        self.last_nudge = now;
        let line = if elapsed_secs < 60.0 {
            format!("Still on it ({elapsed_secs:.0} s). Big jobs take a moment.")
        } else {
            format!(
                "{:.0} min and counting. You can stop it any time.",
                elapsed_secs / 60.0
            )
        };
        self.say(line, 4.0, now);
    }

    /// The turn is over: the closing line names what it cost when that is known.
    pub fn on_turn_done(&mut self, cost: Option<String>, now: f64) {
        self.trigger(BlipState::Success, 2.6, now);
        match cost {
            Some(c) => self.say(format!("Done! That reply cost {c}."), 3.0, now),
            None => self.say("All tasks complete!", 2.6, now),
        }
    }

    /// React when a tool call awaits manual user approval.
    pub fn on_approval_requested(&mut self, tool_name: &str, now: f64) {
        self.touch(now);
        self.base_state = BlipState::Waiting;
        let short_tool = tool_name.split("__").last().unwrap_or(tool_name);
        self.say(format!("Waiting for your OK on {short_tool}!"), 3.5, now);
    }

    /// React when a turn error occurs.
    pub fn on_error(&mut self, message: &str, now: f64) {
        self.touch(now);
        self.base_state = BlipState::Idle;
        self.trigger(BlipState::Error, 3.4, now);
        let snippet: String = message
            .lines()
            .next()
            .unwrap_or("Something went wrong!")
            .chars()
            .take(44)
            .collect();
        self.say_kind(format!("Whoa! {snippet}"), 3.5, now, true);
    }

    /// Current effective state at timestamp `now` (including automatic sleep after 45s of inactivity).
    #[must_use]
    pub fn active_state(&self, now: f64) -> BlipState {
        if let Some((st, _, until)) = self.temp_state
            && now < until
        {
            return st;
        }
        if self.base_state == BlipState::Idle
            && self.last_activity > 0.0
            && (now - self.last_activity) > 45.0
        {
            return BlipState::Sleep;
        }
        self.base_state
    }

    /// Current effective accessory at timestamp `now`.
    #[must_use]
    pub fn active_accessory(&self, now: f64) -> BlipAccessory {
        if let Some((acc, until)) = self.temp_accessory
            && now < until
        {
            return acc;
        }
        if self.active_state(now) == BlipState::Sleep && self.accessory == BlipAccessory::None {
            return BlipAccessory::Coffee;
        }
        // Ears on while the microphone is open
        if self.active_state(now) == BlipState::Listening && self.accessory == BlipAccessory::None {
            return BlipAccessory::Headset;
        }
        self.accessory
    }

    /// Phase time (in seconds) within the current state so triggered animations start cleanly at t=0.
    #[must_use]
    pub fn state_elapsed(&self, now: f64) -> f64 {
        if let Some((_, start, until)) = self.temp_state
            && now < until
        {
            return (now - start).max(0.0);
        }
        now
    }

    /// Active speech bubble `(text, is_error)` at timestamp `now`.
    #[must_use]
    pub fn active_bubble(&self, now: f64) -> Option<(&str, bool)> {
        if !self.bubbles {
            return None;
        }
        if let Some((ref text, until, is_err)) = self.bubble
            && now < until
        {
            return Some((text.as_str(), is_err));
        }
        None
    }

    /// Whether Blip is currently animating and wants continuous frames.
    #[must_use]
    pub fn is_animating(&self, now: f64) -> bool {
        self.anim != BlipAnim::Off
            && (!matches!(self.active_state(now), BlipState::Idle | BlipState::Sleep)
                || self.active_bubble(now).is_some())
    }

    /// Cycle reaction when the user clicks Blip.
    pub fn on_click(&mut self, now: f64) {
        let reactions = [
            (BlipState::Happy, 2.4, "Ready to help!"),
            (
                BlipState::Wave,
                2.2,
                "Drop a file or ask me to open WordCraft!",
            ),
            (BlipState::Walk, 2.0, "On the move across your Craft apps!"),
            (BlipState::Startled, 1.9, "Whoa! That tickles!"),
            (BlipState::Spawn, 1.5, "Ta-da! Pixel power!"),
        ];
        let (st, dur, msg) = reactions[self.click_idx % reactions.len()];
        self.click_idx = self.click_idx.wrapping_add(1);
        self.trigger(st, dur, now);
        self.say(msg, dur + 0.3, now);
    }
}

/// Paint the full interactive Blue Blip mascot (with optional speech bubble above). Returns `true` if clicked.
pub fn paint_mascot(
    ui: &mut Ui,
    blip: &Blip,
    size: f32,
    accessory_override: Option<BlipAccessory>,
    now: f64,
) -> bool {
    let state = blip.active_state(now);
    // Off: a still picture (the pose of the state, no movement).
    let t = if blip.anim == BlipAnim::Off {
        0.4
    } else {
        blip.state_elapsed(now)
    };
    let acc = accessory_override.unwrap_or_else(|| blip.active_accessory(now));
    let bubble = blip.active_bubble(now);

    let size = size * blip.size.factor();
    let (rect, resp) = ui.allocate_exact_size(vec2(size * 1.4, size * 1.08), Sense::click());
    let hovered = resp.hovered();
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let effective_state = if hovered
        && blip.anim != BlipAnim::Off
        && matches!(state, BlipState::Idle | BlipState::Sleep)
    {
        BlipState::Wave
    } else {
        state
    };

    let amp = if blip.anim == BlipAnim::Calm {
        0.5
    } else {
        1.0
    };
    paint_mascot_amp(ui, rect, blip.theme, effective_state, acc, t, true, amp);

    if let Some((text, is_err)) = bubble {
        paint_speech_bubble(ui, rect, blip.theme, text, is_err);
    }

    // Cheap by design: 30 frames a second only while something happens, 10 while idle (blinking), nothing when Off.
    let wait_ms = match blip.anim {
        BlipAnim::Off => 0,
        BlipAnim::Calm => {
            if blip.is_animating(now) || hovered {
                66
            } else {
                200
            }
        }
        BlipAnim::Full => {
            if blip.is_animating(now) || hovered || resp.clicked() {
                33
            } else {
                100
            }
        }
    };
    if wait_ms > 0 {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(wait_ms));
    }

    resp.on_hover_text("Click Blip! (Customize in Settings > General)")
        .clicked()
}

/// Paint a compact Blue Blip avatar (for assistant message headers and the live streaming indicator).
pub fn paint_avatar(ui: &mut Ui, size: f32, state: BlipState, now: f64) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    paint_mascot_in_rect(
        ui,
        rect,
        BlipTheme::Blue,
        state,
        BlipAccessory::Headset,
        now,
        false,
    );
    if state != BlipState::Idle {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(45));
    }
    rect
}

fn paint_speech_bubble(ui: &Ui, blip_rect: Rect, theme: BlipTheme, text: &str, is_error: bool) {
    let p = crate::theme::pal(ui);
    let pal = palette(theme);
    let border_col = if is_error {
        pal.alert
    } else {
        pal.body.gamma_multiply(0.65)
    };
    let font = FontId::proportional(11.5);
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font, p.text));
    let pad = vec2(10.0, 5.0);
    let bsize = galley.size() + pad * 2.0;
    let center_x = blip_rect.center().x;
    let bottom_y = blip_rect.top() - 2.0;
    let brect = Rect::from_min_size(
        pos2(
            (center_x - bsize.x * 0.5).round(),
            (bottom_y - bsize.y).round(),
        ),
        bsize,
    );

    let painter = ui.painter();
    painter.rect_filled(
        brect.translate(vec2(0.0, 2.0)),
        CornerRadius::same(8),
        Color32::from_black_alpha(28),
    );
    painter.rect(
        brect,
        CornerRadius::same(8),
        p.surface,
        Stroke::new(1.5, border_col),
        egui::StrokeKind::Inside,
    );
    let tmp_x = center_x.round();
    painter.rect_filled(
        Rect::from_min_size(pos2(tmp_x - 3.0, brect.bottom()), vec2(6.0, 2.0)),
        CornerRadius::ZERO,
        p.surface,
    );
    painter.rect_filled(
        Rect::from_min_size(pos2(tmp_x - 1.5, brect.bottom() + 2.0), vec2(3.0, 2.0)),
        CornerRadius::ZERO,
        border_col,
    );
    painter.galley(brect.min + pad, galley, p.text);
}

fn ease_out_back(x: f32) -> f32 {
    let c1 = 1.70158_f32;
    let c3 = c1 + 1.0;
    1.0 + c3 * (x - 1.0).powi(3) + c1 * (x - 1.0).powi(2)
}

/// One blink every 4.6 s; every third time it is a double blink. Shared by every state with open eyes, so
/// the eyes never snap shut at odd moments.
fn blinking(t: f64) -> bool {
    let c = t % 4.6;
    let n = (t / 4.6) as u64;
    c < 0.13 || (n % 3 == 1 && (0.30..0.43).contains(&c))
}

const SWEAT: Color32 = Color32::from_rgb(0x7D, 0xD3, 0xFC);
const OK_GREEN: Color32 = Color32::from_rgb(0x34, 0xD3, 0x99);

/// Paint Blip at full animation strength. See [`paint_mascot_amp`].
pub fn paint_mascot_in_rect(
    ui: &Ui,
    rect: Rect,
    theme: BlipTheme,
    state: BlipState,
    acc: BlipAccessory,
    t: f64,
    with_shadow_and_fx: bool,
) {
    paint_mascot_amp(ui, rect, theme, state, acc, t, with_shadow_and_fx, 1.0);
}

/// Paint Blip. `amp` scales every body movement (1.0 = Full, 0.5 = Calm).
///
/// Crisp by construction: the pixel scale is a whole number of physical screen pixels, every offset snaps to a
/// half grid unit (a whole unit on small sizes) and every edge to a physical pixel, so eyes keep their width and
/// nothing shimmers while Blip moves.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub fn paint_mascot_amp(
    ui: &Ui,
    rect: Rect,
    theme: BlipTheme,
    state: BlipState,
    acc: BlipAccessory,
    t: f64,
    with_shadow_and_fx: bool,
    amp: f32,
) {
    let pal = palette(theme);
    let painter = ui.painter();
    let ppp = ui.ctx().pixels_per_point().max(1.0);
    let snap = |v: f32| (v * ppp).round() / ppp;

    let (vb_w, vb_h, ox_grid, oy_grid) = if with_shadow_and_fx {
        (26.0_f32, 21.0_f32, 4.0_f32, 4.0_f32)
    } else {
        (20.0_f32, 17.0_f32, 1.0_f32, 1.0_f32)
    };
    let raw = (rect.width() / vb_w).min(rect.height() / vb_h) * ppp;
    let phys = if raw >= 4.0 {
        (raw / 2.0).floor() * 2.0
    } else {
        raw.floor().max(1.0)
    };
    let scale = phys / ppp;
    let quantum = if phys >= 4.0 { 0.5_f32 } else { 1.0_f32 };
    let q = |v: f32| (v / quantum).round() * quantum;
    let total_w = vb_w * scale;
    let total_h = vb_h * scale;
    let base_x = snap(rect.center().x - total_w * 0.5 + ox_grid * scale);
    let base_y = snap(rect.center().y - total_h * 0.5 + oy_grid * scale);
    let s = |hz: f64| (t * hz).sin() as f32;

    // One drawing primitive: grid coordinates in, whole physical pixels out.
    let put = |gx: f32, gy: f32, gw: f32, gh: f32, col: Color32, bx: f32, by: f32, spin: f32| {
        let (x0g, x1g) = if (spin - 1.0).abs() < 1e-3 {
            (q(gx) + bx, q(gx) + gw + bx)
        } else {
            let a = q(9.0 + (gx - 9.0) * spin) + bx;
            let b = q(9.0 + (gx + gw - 9.0) * spin) + bx;
            let lo = a.min(b);
            (lo, a.max(b).max(lo + quantum))
        };
        let y0g = q(gy) + by;
        let x0 = snap(base_x + x0g * scale);
        let x1 = snap(base_x + x1g * scale).max(x0 + 1.0 / ppp);
        let y0 = snap(base_y + y0g * scale);
        let y1 = snap(base_y + (y0g + gh) * scale).max(y0 + 1.0 / ppp);
        painter.rect_filled(
            Rect::from_min_max(pos2(x0, y0), pos2(x1, y1)),
            CornerRadius::ZERO,
            col,
        );
    };

    // Spawn cloud assembly: for the first 0.85 s of Spawn, loose pixel blocks converge into Blip.
    if state == BlipState::Spawn && t < 0.85 {
        let p = ((t as f32) / 0.85).clamp(0.0, 1.0);
        let scatter = 1.0 - ease_out_back(p);
        let pieces: [(f32, f32, f32, f32, Color32, f32, f32); 20] = [
            (3.0, 0.0, 2.0, 1.0, pal.tip, -8.0, -7.0),
            (13.0, 0.0, 2.0, 1.0, pal.tip, 9.0, -7.0),
            (4.0, 1.0, 1.0, 2.0, pal.shade, -6.0, -5.0),
            (13.0, 1.0, 1.0, 2.0, pal.shade, 6.0, -5.0),
            (4.0, 3.0, 10.0, 1.0, pal.top, 0.0, -8.0),
            (3.0, 4.0, 12.0, 1.0, pal.top, -5.0, -6.0),
            (2.0, 5.0, 7.0, 4.0, pal.body, -9.0, -2.0),
            (9.0, 5.0, 7.0, 4.0, pal.body, 9.0, -2.0),
            (2.0, 9.0, 7.0, 3.0, pal.body, -8.0, 4.0),
            (9.0, 9.0, 7.0, 3.0, pal.body, 8.0, 4.0),
            (3.0, 12.0, 12.0, 1.0, pal.shade, 0.0, 7.0),
            (0.0, 7.0, 2.0, 3.0, pal.body, -11.0, 1.0),
            (16.0, 7.0, 2.0, 3.0, pal.body, 11.0, 1.0),
            (2.0, 13.0, 4.0, 2.0, pal.foot, -6.0, 8.0),
            (12.0, 13.0, 4.0, 2.0, pal.foot, 6.0, 8.0),
            (4.0, 6.0, 3.0, 3.0, pal.ink, -4.0, -3.0),
            (11.0, 6.0, 3.0, 3.0, pal.ink, 4.0, -3.0),
            (4.0, 6.0, 2.0, 2.0, pal.shine, -5.0, -4.0),
            (11.0, 6.0, 2.0, 2.0, pal.shine, 5.0, -4.0),
            (8.0, 10.0, 2.0, 1.0, pal.ink, 0.0, 5.0),
        ];
        for (gx, gy, gw, gh, col, dx, dy) in pieces {
            put(
                gx + dx * scatter,
                gy + dy * scatter,
                gw,
                gh,
                col.gamma_multiply(p.clamp(0.25, 1.0)),
                0.0,
                0.0,
                1.0,
            );
        }
        return;
    }

    // Body movement of the state: (shift x, shift y, horizontal squash for the spin, back view).
    let (bx, by, spin_x, show_back): (f32, f32, f32, bool) = match state {
        BlipState::Idle => (0.0, s(2.2) * 0.5, 1.0, false),
        BlipState::Sleep => (0.0, s(1.2) * 0.5 + 0.5, 1.0, false),
        BlipState::Walk => (s(6.5), -s(13.0).abs(), 1.0, false),
        BlipState::Thinking => (0.0, s(2.6) * 0.5, 1.0, false),
        BlipState::Working => (0.0, if s(12.0) > 0.0 { -0.5 } else { 0.0 }, 1.0, false),
        BlipState::Waiting | BlipState::Listening => (0.0, s(3.0) * 0.5, 1.0, false),
        BlipState::Error => {
            // A short shake that dies down, then a slump.
            let shake = if t < 0.7 {
                s(46.0) * (1.0 - t as f32 / 0.7) * 1.5
            } else {
                0.0
            };
            (shake, 0.5, 1.0, false)
        }
        BlipState::Success => {
            let ph = ((t % 0.8) / 0.8) as f32;
            let decay = 1.0 - (t as f32 / 2.6).min(1.0) * 0.5;
            (
                0.0,
                -(ph * std::f32::consts::PI).sin() * 2.0 * decay,
                1.0,
                false,
            )
        }
        BlipState::Wave | BlipState::Spawn => (0.0, s(4.5) * 0.5 - 0.2, 1.0, false),
        BlipState::Startled => {
            // Phase 1 (0..0.40 s): sharp shock jump; phase 2: dizzy sideways stumble.
            let phase = (t % 1.8) as f32;
            if phase < 0.40 {
                let hop = -(phase / 0.40 * std::f32::consts::PI).sin() * 2.0;
                (0.0, hop, 1.0, false)
            } else {
                (s(14.0) * 0.75, (t * 22.0).cos() as f32 * 0.25, 1.0, false)
            }
        }
        BlipState::Happy => {
            let cycle = (t % 1.2) as f32 / 1.2;
            let hop = -(cycle * std::f32::consts::PI * 2.0).sin().abs() * 1.4;
            let (sx, back) = if (0.18..0.58).contains(&cycle) {
                let spin_t = (cycle - 0.18) / 0.40;
                let cos_v = (spin_t * std::f32::consts::TAU).cos();
                (cos_v.abs().max(0.28), cos_v < 0.0)
            } else {
                (1.0, false)
            };
            (0.0, hop, sx, back)
        }
    };
    let bob_x = q(bx * amp);
    let bob_y = q(by * amp);
    let px = |gx: f32, gy: f32, gw: f32, gh: f32, col: Color32| {
        put(gx, gy, gw, gh, col, bob_x, bob_y, spin_x);
    };
    // Effects stay where they are while the body hops.
    let fxp = |gx: f32, gy: f32, gw: f32, gh: f32, col: Color32| {
        put(gx, gy, gw, gh, col, 0.0, 0.0, 1.0);
    };

    // Ground shadow
    if with_shadow_and_fx {
        let sh_w = (12.0 - bob_y.abs() * 1.4).clamp(8.0, 13.0) * scale;
        let sh_h = (2.0 * scale).max(2.0 / ppp);
        let sh_cx = base_x + (9.0 + bob_x) * scale;
        let sh_cy = base_y + 15.6 * scale;
        painter.rect_filled(
            Rect::from_center_size(pos2(snap(sh_cx), snap(sh_cy)), vec2(snap(sh_w), snap(sh_h))),
            CornerRadius::same((sh_h * 0.5) as u8),
            pal.ink.gamma_multiply(0.18),
        );
    }

    // Feet
    let (lf_dx, lf_dy, rf_dx, rf_dy) = match state {
        BlipState::Walk => {
            let w = s(13.0);
            (w, -(-w).max(0.0), -w, -w.max(0.0))
        }
        BlipState::Working => (
            0.0,
            if s(10.0) > 0.0 { -0.5 } else { 0.0 },
            0.0,
            if s(10.0) <= 0.0 { -0.5 } else { 0.0 },
        ),
        BlipState::Happy => (
            0.0,
            if s(8.0) > 0.0 { -0.5 } else { 0.0 },
            0.0,
            if s(8.0) <= 0.0 { -0.5 } else { 0.0 },
        ),
        BlipState::Waiting => (0.0, 0.0, 0.0, if s(6.0) > 0.4 { -0.5 } else { 0.0 }),
        BlipState::Success => (-0.5, 0.0, 0.5, 0.0),
        _ => (0.0, 0.0, 0.0, 0.0),
    };
    px(3.0 + lf_dx, 13.0 + lf_dy, 3.0, 1.0, pal.foot);
    px(2.0 + lf_dx, 14.0 + lf_dy, 4.0, 1.0, pal.foot);
    px(12.0 + rf_dx, 13.0 + rf_dy, 3.0, 1.0, pal.foot);
    px(12.0 + rf_dx, 14.0 + rf_dy, 4.0, 1.0, pal.foot);

    // Body
    px(4.0, 3.0, 10.0, 1.0, pal.body);
    px(3.0, 4.0, 12.0, 1.0, pal.body);
    px(2.0, 5.0, 14.0, 7.0, pal.body);
    px(3.0, 12.0, 12.0, 1.0, pal.body);

    // Shade
    px(15.0, 5.0, 1.0, 7.0, pal.shade);
    px(14.0, 12.0, 1.0, 1.0, pal.shade);
    px(3.0, 12.0, 11.0, 1.0, pal.shade);

    // Top highlight
    px(4.0, 3.0, 10.0, 1.0, pal.top);
    px(3.0, 4.0, 11.0, 1.0, pal.top);

    // Antennae: they sway, light up with the work and droop when something failed.
    if state == BlipState::Error {
        px(4.0, 2.0, 1.0, 1.0, pal.shade);
        px(3.0, 2.0, 1.0, 1.0, pal.shade);
        px(2.0, 3.0, 1.0, 1.0, pal.shade);
        px(13.0, 2.0, 1.0, 1.0, pal.shade);
        px(14.0, 2.0, 1.0, 1.0, pal.shade);
        px(15.0, 3.0, 1.0, 1.0, pal.shade);
    } else {
        let ant_dy = ((t * 3.6 + 0.8).sin() as f32) * 0.5;
        let flip = s(5.0) > 0.0;
        let (tl, tr) = match state {
            BlipState::Thinking | BlipState::Spawn => {
                let c = if s(6.0) > 0.0 { pal.shine } else { pal.tip };
                (c, c)
            }
            BlipState::Working | BlipState::Waiting | BlipState::Listening => {
                if flip {
                    (pal.shine, pal.tip)
                } else {
                    (pal.tip, pal.shine)
                }
            }
            _ => (pal.tip, pal.tip),
        };
        px(4.0, 1.0 + ant_dy, 1.0, 2.0, pal.shade);
        px(13.0, 1.0 + ant_dy, 1.0, 2.0, pal.shade);
        px(3.0, ant_dy, 2.0, 1.0, tl);
        px(13.0, ant_dy, 2.0, 1.0, tr);
    }

    // Arms
    let toggle = ((t * 9.0) as u64).is_multiple_of(2);
    let (l_up, r_up) = match state {
        BlipState::Success => (true, true),
        BlipState::Happy | BlipState::Wave | BlipState::Spawn | BlipState::Waiting => (false, true),
        _ => (false, false),
    };
    if l_up {
        let w = s(10.0) * 0.5;
        px(0.0, 5.0 + w, 2.0, 2.0, pal.body);
        px(-1.0, 3.0 + w, 2.0, 2.0, pal.body);
        px(0.0, 7.0 + w, 2.0, 1.0, pal.shade);
    } else {
        let dy = match state {
            BlipState::Working => {
                if toggle {
                    -1.0
                } else {
                    0.0
                }
            }
            BlipState::Walk => s(13.0) * 0.7,
            BlipState::Happy | BlipState::Startled => -1.5,
            BlipState::Error => 1.0,
            _ => 0.0,
        };
        px(0.0, 7.0 + dy, 2.0, 2.0, pal.body);
        px(0.0, 9.0 + dy, 2.0, 1.0, pal.shade);
    }
    if r_up {
        let w = if state == BlipState::Waiting {
            s(5.0) * 0.5
        } else {
            s(12.0) * 0.6
        };
        px(16.0, 5.0 + w, 2.0, 2.0, pal.body);
        px(17.0, 3.0 + w, 2.0, 2.0, pal.body);
        px(16.0, 7.0 + w, 2.0, 1.0, pal.shade);
    } else {
        let dy = match state {
            BlipState::Working => {
                if toggle {
                    0.0
                } else {
                    -1.0
                }
            }
            BlipState::Walk => -s(13.0) * 0.7,
            BlipState::Startled => -1.5,
            BlipState::Thinking => -1.0,
            BlipState::Error => 1.0,
            _ => 0.0,
        };
        px(16.0, 7.0 + dy, 2.0, 2.0, pal.body);
        px(16.0, 9.0 + dy, 2.0, 1.0, pal.shade);
    }

    // Face. Eyes only ever move by whole steps, so they keep their shape.
    let cheeks = || {
        px(3.0, 9.0, 1.0, 1.0, pal.blush);
        px(14.0, 9.0, 1.0, 1.0, pal.blush);
    };
    let eyes_open = |gx: f32, gy: f32| {
        px(4.0 + gx, 6.0 + gy, 3.0, 3.0, pal.ink);
        px(11.0 + gx, 6.0 + gy, 3.0, 3.0, pal.ink);
        px(4.0 + gx, 6.0 + gy, 2.0, 2.0, pal.shine);
        px(11.0 + gx, 6.0 + gy, 2.0, 2.0, pal.shine);
        px(6.0 + gx, 8.0 + gy, 1.0, 1.0, pal.shine);
        px(13.0 + gx, 8.0 + gy, 1.0, 1.0, pal.shine);
    };
    let eyes_shut = || {
        px(4.0, 8.0, 3.0, 1.0, pal.ink);
        px(11.0, 8.0, 3.0, 1.0, pal.ink);
    };

    if show_back {
        px(8.0, 4.0, 2.0, 8.0, pal.shade);
        px(8.0, 5.0, 2.0, 1.0, pal.tip);
        px(8.0, 8.0, 2.0, 1.0, pal.tip);
    } else {
        match state {
            BlipState::Sleep => {
                eyes_shut();
                px(8.0, 10.0, 2.0, 1.0, pal.ink);
                cheeks();
            }
            BlipState::Idle
            | BlipState::Thinking
            | BlipState::Walk
            | BlipState::Spawn
            | BlipState::Waiting => {
                if blinking(t) {
                    eyes_shut();
                } else {
                    let (gx, gy) = match state {
                        // Looks up and around while it thinks.
                        BlipState::Thinking => {
                            [(1.0, -1.0), (0.0, -1.0), (-1.0, -1.0), (0.0, -1.0)]
                                [((t * 0.7) as usize) % 4]
                        }
                        BlipState::Walk => (if s(6.5) > 0.3 { 1.0 } else { 0.0 }, 0.0),
                        // Looks at you and waits.
                        BlipState::Waiting => (0.0, if s(1.3) > 0.7 { -1.0 } else { 0.0 }),
                        _ => {
                            let look = s(0.7);
                            (
                                if look > 0.85 {
                                    1.0
                                } else if look < -0.85 {
                                    -1.0
                                } else {
                                    0.0
                                },
                                0.0,
                            )
                        }
                    };
                    eyes_open(gx, gy);
                }
                if state == BlipState::Thinking {
                    px(9.0, 10.0, 2.0, 1.0, pal.ink);
                } else if state == BlipState::Waiting {
                    px(8.0, 10.0, 2.0, 1.0, pal.ink);
                    px(8.0, 11.0, 2.0, 1.0, pal.ink);
                } else {
                    px(8.0, 10.0, 2.0, 1.0, pal.ink);
                }
                cheeks();
            }
            BlipState::Listening => {
                // Wide open eyes and a small round mouth: all ears.
                px(5.0, 5.0, 2.0, 4.0, pal.ink);
                px(11.0, 5.0, 2.0, 4.0, pal.ink);
                px(5.0, 5.0, 1.0, 1.0, pal.shine);
                px(11.0, 5.0, 1.0, 1.0, pal.shine);
                px(8.0, 10.0, 2.0, 1.0, pal.ink);
                cheeks();
            }
            BlipState::Working => {
                // Narrowed, focused eyes that read from side to side.
                let gx = if ((t * 2.2) as u64).is_multiple_of(2) {
                    0.0
                } else {
                    1.0
                };
                px(4.0 + gx, 7.0, 3.0, 2.0, pal.ink);
                px(11.0 + gx, 7.0, 3.0, 2.0, pal.ink);
                px(4.0 + gx, 7.0, 1.0, 1.0, pal.shine);
                px(11.0 + gx, 7.0, 1.0, 1.0, pal.shine);
                px(7.0, 10.0, 4.0, 1.0, pal.ink);
            }
            BlipState::Happy | BlipState::Wave | BlipState::Success => {
                px(4.0, 7.0, 1.0, 1.0, pal.ink);
                px(5.0, 6.0, 1.0, 1.0, pal.ink);
                px(6.0, 7.0, 1.0, 1.0, pal.ink);
                px(11.0, 7.0, 1.0, 1.0, pal.ink);
                px(12.0, 6.0, 1.0, 1.0, pal.ink);
                px(13.0, 7.0, 1.0, 1.0, pal.ink);
                px(7.0, 9.0, 4.0, 2.0, pal.ink);
                px(8.0, 10.0, 2.0, 1.0, pal.blush);
                px(3.0, 8.0, 2.0, 2.0, pal.blush);
                px(13.0, 8.0, 2.0, 2.0, pal.blush);
            }
            BlipState::Error => {
                // Crossed-out eyes and a bent mouth.
                for ex in [4.0_f32, 11.0] {
                    px(ex, 6.0, 1.0, 1.0, pal.ink);
                    px(ex + 2.0, 6.0, 1.0, 1.0, pal.ink);
                    px(ex + 1.0, 7.0, 1.0, 1.0, pal.ink);
                    px(ex, 8.0, 1.0, 1.0, pal.ink);
                    px(ex + 2.0, 8.0, 1.0, 1.0, pal.ink);
                }
                px(8.0, 10.0, 2.0, 1.0, pal.ink);
                px(7.0, 11.0, 1.0, 1.0, pal.ink);
                px(10.0, 11.0, 1.0, 1.0, pal.ink);
                px(3.0, 9.0, 1.0, 1.0, pal.alert.gamma_multiply(0.8));
                px(14.0, 9.0, 1.0, 1.0, pal.alert.gamma_multiply(0.8));
            }
            BlipState::Startled => {
                if (t % 1.8) < 0.42 {
                    // Shock: wide white eyes with tiny pupils.
                    px(4.0, 6.0, 3.0, 3.0, pal.shine);
                    px(11.0, 6.0, 3.0, 3.0, pal.shine);
                    px(5.0, 7.0, 1.0, 1.0, pal.ink);
                    px(12.0, 7.0, 1.0, 1.0, pal.ink);
                } else {
                    // Dizzy diamond eyes.
                    for ex in [5.0_f32, 12.0] {
                        px(ex, 6.0, 1.0, 1.0, pal.ink);
                        px(ex - 1.0, 7.0, 1.0, 1.0, pal.ink);
                        px(ex + 1.0, 7.0, 1.0, 1.0, pal.ink);
                        px(ex, 8.0, 1.0, 1.0, pal.ink);
                        px(ex, 7.0, 1.0, 1.0, pal.shine);
                    }
                }
                px(6.0, 10.0, 2.0, 1.0, pal.ink);
                px(8.0, 9.0, 1.0, 1.0, pal.ink);
                px(9.0, 10.0, 2.0, 1.0, pal.ink);
                px(11.0, 9.0, 1.0, 1.0, pal.ink);
            }
        }
    }

    // Accessories
    match acc {
        BlipAccessory::None => {}
        BlipAccessory::Glasses => {
            if !show_back {
                px(3.0, 5.0, 5.0, 1.0, pal.ink);
                px(10.0, 5.0, 5.0, 1.0, pal.ink);
                px(3.0, 9.0, 5.0, 1.0, pal.ink);
                px(10.0, 9.0, 5.0, 1.0, pal.ink);
                px(3.0, 6.0, 1.0, 3.0, pal.ink);
                px(7.0, 6.0, 1.0, 3.0, pal.ink);
                px(10.0, 6.0, 1.0, 3.0, pal.ink);
                px(14.0, 6.0, 1.0, 3.0, pal.ink);
                px(8.0, 6.0, 2.0, 1.0, pal.ink);
                px(2.0, 6.0, 1.0, 1.0, pal.ink);
                px(15.0, 6.0, 1.0, 1.0, pal.ink);
                px(4.0, 6.0, 1.0, 1.0, pal.shine);
                px(11.0, 6.0, 1.0, 1.0, pal.shine);
            }
        }
        BlipAccessory::Headset => {
            px(3.0, 2.0, 12.0, 1.0, pal.ink);
            px(1.0, 6.0, 1.0, 4.0, pal.ink);
            px(16.0, 6.0, 1.0, 4.0, pal.ink);
            px(2.0, 7.0, 1.0, 2.0, pal.shade);
            px(15.0, 7.0, 1.0, 2.0, pal.shade);
            if !show_back {
                px(1.0, 10.0, 2.0, 1.0, pal.ink);
                px(3.0, 10.0, 1.0, 1.0, pal.tip);
            }
        }
        BlipAccessory::Crown => {
            px(5.0, 2.0, 8.0, 1.0, pal.tip);
            px(5.0, 1.0, 1.0, 1.0, pal.tip);
            px(7.0, 1.0, 1.0, 1.0, pal.tip);
            px(10.0, 1.0, 1.0, 1.0, pal.tip);
            px(12.0, 1.0, 1.0, 1.0, pal.tip);
            px(6.0, 0.0, 1.0, 1.0, pal.tip);
            px(8.0, 0.0, 2.0, 1.0, pal.tip);
            px(11.0, 0.0, 1.0, 1.0, pal.tip);
            px(6.0, 2.0, 1.0, 1.0, pal.alert);
            px(11.0, 2.0, 1.0, 1.0, pal.alert);
            px(8.0, 1.0, 2.0, 1.0, pal.shine);
        }
        BlipAccessory::Coffee => {
            px(17.0, 8.0, 3.0, 4.0, pal.shine);
            px(17.0, 9.0, 3.0, 2.0, pal.tip);
            px(20.0, 9.0, 1.0, 2.0, pal.shine);
            px(17.0, 11.0, 3.0, 1.0, pal.ink);
            let steam_dy = s(4.0) * 0.5;
            px(18.0, 6.0 + steam_dy, 1.0, 1.0, pal.shine);
            px(19.0, 4.5 - steam_dy, 1.0, 1.0, pal.shine);
        }
        BlipAccessory::Hardhat => {
            px(4.0, 1.0, 10.0, 2.0, pal.tip);
            px(5.0, 0.0, 8.0, 1.0, pal.tip);
            px(2.0, 3.0, 14.0, 1.0, pal.tip);
            px(8.0, 0.0, 2.0, 3.0, pal.shine);
            px(3.0, 3.0, 12.0, 1.0, pal.alert);
        }
        BlipAccessory::Cap => {
            px(5.0, 1.0, 8.0, 2.0, pal.alert);
            px(8.0, 0.0, 2.0, 1.0, pal.alert);
            px(5.0, 2.0, 8.0, 1.0, pal.alert.gamma_multiply(0.82));
            px(11.0, 3.0, 6.0, 1.0, pal.alert.gamma_multiply(0.65));
            px(7.0, 1.0, 1.0, 1.0, pal.shine);
        }
        BlipAccessory::Bowtie => {
            if !show_back {
                px(6.0, 12.0, 2.0, 2.0, pal.alert);
                px(10.0, 12.0, 2.0, 2.0, pal.alert);
                px(8.0, 12.0, 2.0, 2.0, pal.alert.gamma_multiply(0.7));
            }
        }
    }

    // Foreground FX overlays (they stand still while Blip hops)
    if !with_shadow_and_fx {
        return;
    }
    match state {
        BlipState::Thinking => {
            let step = ((t * 3.0) as usize) % 4;
            if step >= 1 {
                fxp(16.0, 2.0, 1.0, 1.0, pal.body);
            }
            if step >= 2 {
                fxp(18.0, 0.0, 2.0, 2.0, pal.body);
            }
            if step >= 3 {
                fxp(20.0, -3.0, 3.0, 3.0, pal.body);
                fxp(21.0, -2.0, 1.0, 1.0, pal.shine);
            }
        }
        BlipState::Working => {
            // A gear that turns between "+" and "x", plus sparks at the hands.
            if ((t * 6.0) as u64).is_multiple_of(2) {
                fxp(20.0, 0.0, 1.0, 3.0, pal.tip);
                fxp(19.0, 1.0, 3.0, 1.0, pal.tip);
            } else {
                for (gx, gy) in [
                    (19.0, 0.0),
                    (21.0, 0.0),
                    (20.0, 1.0),
                    (19.0, 2.0),
                    (21.0, 2.0),
                ] {
                    fxp(gx, gy, 1.0, 1.0, pal.tip);
                }
            }
            let phase = ((t * 10.0) as usize) % 3;
            let (a, b) = [(6.0, 8.0), (8.0, 5.0), (5.0, 9.0)][phase];
            fxp(-2.0, a, 1.0, 1.0, pal.tip);
            fxp(19.0, b + 3.0, 1.0, 1.0, pal.body);
        }
        BlipState::Waiting => {
            let bounce = s(5.0) * 0.5;
            fxp(19.0, -3.0 + bounce, 3.0, 1.0, pal.tip);
            fxp(21.0, -2.0 + bounce, 1.0, 1.0, pal.tip);
            fxp(20.0, -1.0 + bounce, 1.0, 1.0, pal.tip);
            fxp(20.0, 1.0 + bounce, 1.0, 1.0, pal.tip);
        }
        BlipState::Error => {
            let fall = (t * 1.2).fract() as f32;
            let a = 1.0 - fall * 0.6;
            fxp(16.0, 3.0 + fall * 5.0, 1.0, 2.0, SWEAT.gamma_multiply(a));
            if (t % 0.8) < 0.4 {
                fxp(-2.0, 2.0, 1.0, 1.0, pal.alert);
                fxp(20.0, 3.0, 1.0, 1.0, pal.alert);
            }
        }
        BlipState::Success => {
            let pop = ((t as f32) / 0.2).clamp(0.0, 1.0);
            let c = OK_GREEN.gamma_multiply(pop);
            for (gx, gy) in [
                (17.0, -1.0),
                (18.0, 0.0),
                (19.0, -1.0),
                (20.0, -2.0),
                (21.0, -3.0),
            ] {
                fxp(gx, gy, 1.0, 1.0, c);
            }
            let tw = (t * 6.0) as usize % 2;
            let fy = if tw == 0 { 0.0 } else { 1.0 };
            // Above the raised left arm, not in front of it.
            fxp(-4.0, -1.0 + fy, 3.0, 1.0, pal.tip);
            fxp(-3.0, -2.0 + fy, 1.0, 3.0, pal.tip);
            fxp(-2.0, 11.0 - fy, 2.0, 1.0, pal.blush);
            fxp(-1.0, 10.0 - fy, 1.0, 3.0, pal.blush);
        }
        BlipState::Happy | BlipState::Wave | BlipState::Spawn => {
            let fy = s(5.0) * 0.6;
            fxp(-3.0, 2.0 + fy, 3.0, 1.0, pal.tip);
            fxp(-2.0, 1.0 + fy, 1.0, 3.0, pal.tip);
            fxp(19.0, 3.0 - fy, 3.0, 1.0, pal.tip);
            fxp(20.0, 2.0 - fy, 1.0, 3.0, pal.tip);
            fxp(-2.0, 11.0 - fy * 0.5, 2.0, 1.0, pal.blush);
            fxp(-1.0, 10.0 - fy * 0.5, 1.0, 3.0, pal.blush);
        }
        BlipState::Startled => {
            let ay = -(s(10.0).abs()) * 0.8;
            fxp(8.0, -3.0 + ay, 2.0, 2.0, pal.alert);
            fxp(8.0, 0.0 + ay, 2.0, 1.0, pal.alert);
        }
        BlipState::Sleep => {
            let z_cycle = (t * 0.9) as f32;
            for z in [z_cycle.fract(), (z_cycle + 0.5).fract()] {
                let zx = 16.0 + z * 4.5;
                let zy = 2.0 - z * 5.5;
                let col = pal.body.gamma_multiply(1.0 - z * 0.7);
                fxp(zx, zy, 3.0, 1.0, col);
                fxp(zx + 1.0, zy + 1.0, 1.0, 1.0, col);
                fxp(zx, zy + 2.0, 3.0, 1.0, col);
            }
        }
        BlipState::Listening => {
            // Sound ripples travel outwards on both sides.
            let (a, b) =
                [(1.0_f32, 0.2_f32), (0.6, 0.6), (0.2, 1.0), (0.1, 0.1)][((t * 4.0) as usize) % 4];
            fxp(19.0, 7.0, 1.0, 3.0, pal.tip.gamma_multiply(a));
            fxp(21.0, 6.0, 1.0, 5.0, pal.tip.gamma_multiply(b));
            fxp(-2.0, 7.0, 1.0, 3.0, pal.tip.gamma_multiply(a));
            fxp(-4.0, 6.0, 1.0, 5.0, pal.tip.gamma_multiply(b));
        }
        BlipState::Idle | BlipState::Walk => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blink_is_short_and_regular() {
        let mut shut = 0;
        for i in 0..4600 {
            if blinking(f64::from(i) / 100.0) {
                shut += 1;
            }
        }
        // 46 s: about 10 blinks of 0.13 s plus a few double blinks, far from "eyes closed half the time".
        assert!((100..260).contains(&shut), "closed frames: {shut}");
    }

    #[test]
    fn states_accessories_and_themes_round_trip() {
        assert_eq!(BlipState::ALL.len(), 13);
        for a in BlipAccessory::ALL {
            assert_eq!(BlipAccessory::parse(a.id()), a);
        }
        for th in BlipTheme::ALL {
            assert_eq!(BlipTheme::parse(th.id()), th);
        }
        for an in BlipAnim::ALL {
            assert_eq!(BlipAnim::parse(an.id()), an);
        }
        for sz in BlipSize::ALL {
            assert_eq!(BlipSize::parse(sz.id()), sz);
        }
    }

    #[test]
    fn reactions_use_the_new_states() {
        let mut b = Blip::default();
        // (the intro Spawn animation of a fresh Blip is over after 1.5 s)
        b.on_tool_start("read_file", 10.0);
        assert_eq!(b.active_state(10.1), BlipState::Working);
        b.on_error("boom", 20.0);
        assert_eq!(b.active_state(20.1), BlipState::Error);
        b.on_turn_done(None, 30.0);
        assert_eq!(b.active_state(30.1), BlipState::Success);
        b.on_approval_requested("write_file", 40.0);
        assert_eq!(b.active_state(40.1), BlipState::Waiting);
    }

    #[test]
    fn off_and_no_bubbles_are_respected() {
        let mut b = Blip {
            anim: BlipAnim::Off,
            ..Blip::default()
        };
        b.on_turn_done(None, 10.0);
        assert!(!b.is_animating(10.1));
        b.anim = BlipAnim::Full;
        b.bubbles = false;
        b.say("hi", 5.0, 10.0);
        assert!(b.active_bubble(10.1).is_none());
    }
}
