//! Voice input for WorkCraft: microphone capture, live 16-band FFT spectrum visualizer
//! (ported from Handy 0.10.0 `AudioVisualiser`), voice activity detection, live streaming
//! partial transcripts, and speech-to-text powered by:
//!
//! 1. **Handy 0.10.0 + Nemotron Streaming 3.5** (`nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf`
//!    on Vulkan/CPU via `handy.exe --transcribe-file <wav> --json`) as the primary engine.
//! 2. **Windows Speech Recognition (`System.Speech`)** as an instant zero-install offline fallback.
//! 3. **Web Speech API Bridge** (localhost browser `SpeechRecognition` / `webkitSpeechRecognition`
//!    streaming relay) as an optional fallback.
//!
//! Also includes Handy 0.10.0's `apply_custom_words` fuzzy/phonetic (Levenshtein + Soundex)
//! n-gram post-processor and WorkCraft mention/command normalization (e.g.
//! `"at gridcraft read column B"` or `"Grid Kraft read column B"` -> `"@gridcraft read column B"`).

#![cfg_attr(
    not(target_os = "windows"),
    allow(dead_code, unused_variables, unused_imports)
)]

use std::f32::consts::PI;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

pub const SAMPLE_RATE: u32 = 16_000;
pub const VISUALIZER_BUCKETS: usize = 16;
const FFT_WINDOW_SIZE: usize = 512;

/// User-selectable speech recognition engine preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceEnginePref {
    /// Use Handy 0.10.0 (Nemotron Streaming 3.5) when available, falling back to Windows Speech / Web Speech.
    Auto,
    /// Require Handy 0.10.0 (`handy.exe` + `nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf`).
    HandyNemotron,
    /// Use Windows built-in Speech Recognition (`System.Speech.Recognition`).
    WindowsSpeech,
    /// Use Web Speech API via a local browser speech bridge (`webkitSpeechRecognition`).
    WebSpeechApi,
}

impl VoiceEnginePref {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto (Handy + Nemotron 3.5)",
            Self::HandyNemotron => "Handy \u{00b7} Nemotron 3.5",
            Self::WindowsSpeech => "Windows Speech API",
            Self::WebSpeechApi => "Web Speech API",
        }
    }
}

/// Current state of the voice button and recognition pipeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoiceState {
    Idle,
    Listening,
    Processing,
    Error(VoiceError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceError {
    pub message: String,
    pub permission_denied: bool,
}

/// Events emitted by the background recording and transcription threads.
#[derive(Clone, Debug)]
pub enum VoiceEvent {
    /// Live streaming partial transcript updated while the user is still speaking.
    Partial(String),
    /// Recording stopped automatically (e.g. silence after speech) and final transcription started.
    ProcessingStarted,
    /// Final transcription completed.
    Final {
        text: String,
        engine: String,
        duration_ms: u64,
        auto_send: bool,
    },
    /// Recording or transcription failed.
    Error(VoiceError),
}

/// Information about a microphone input device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MicDevice {
    pub id: Option<u32>,
    pub name: String,
}

/// Discovered Handy 0.10.0 runtime and model details on the user's machine.
#[derive(Clone, Debug)]
pub struct HandyRuntimeInfo {
    pub exe_path: Option<PathBuf>,
    pub selected_model: String,
    pub model_display: String,
    pub custom_words: Vec<String>,
    /// File version of `handy.exe` (read from the program file; Handy is never started to ask).
    pub version: Option<String>,
}

/// Manages microphone recording, live visualization, and speech-to-text transcription.
pub struct VoiceController {
    pub state: VoiceState,
    pub engine_pref: VoiceEnginePref,
    pub selected_mic: Option<u32>,
    pub mics: Vec<MicDevice>,
    pub auto_stop_on_silence: bool,
    pub voice_send_command: bool,
    pub handy_info: HandyRuntimeInfo,
    pub started_at: Option<Instant>,
    pub live_partial: String,
    pub last_engine_used: Option<String>,
    pub last_duration_ms: Option<u64>,
    /// Text that was already in the chat input before voice recording began.
    pub input_prefix: String,
    spectrum: Arc<Mutex<[f32; VISUALIZER_BUCKETS]>>,
    stop_flag: Option<Arc<AtomicBool>>,
    cancel_flag: Option<Arc<AtomicBool>>,
    tx: Sender<VoiceEvent>,
    rx: Receiver<VoiceEvent>,
}

impl Default for VoiceController {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceController {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let handy_info = detect_handy_runtime();
        let mics = list_microphones();
        Self {
            state: VoiceState::Idle,
            engine_pref: VoiceEnginePref::Auto,
            selected_mic: None,
            mics,
            auto_stop_on_silence: true,
            voice_send_command: true,
            handy_info,
            started_at: None,
            live_partial: String::new(),
            last_engine_used: None,
            last_duration_ms: None,
            input_prefix: String::new(),
            spectrum: Arc::new(Mutex::new([0.0; VISUALIZER_BUCKETS])),
            stop_flag: None,
            cancel_flag: None,
            tx,
            rx,
        }
    }

    pub fn refresh_devices(&mut self) {
        self.mics = list_microphones();
        self.handy_info = detect_handy_runtime();
    }

    /// Returns a snapshot of the 16 frequency bars (each in `0.0..=1.0`).
    pub fn spectrum_bars(&self) -> [f32; VISUALIZER_BUCKETS] {
        match self.spectrum.lock() {
            Ok(g) => *g,
            Err(p) => *p.into_inner(),
        }
    }

    /// Elapsed recording time in seconds when listening.
    pub fn elapsed_secs(&self) -> f32 {
        self.started_at
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0)
    }

    /// Polls any pending `VoiceEvent`s from the background worker.
    pub fn poll(&mut self) -> Vec<VoiceEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            match &ev {
                VoiceEvent::Partial(text) => {
                    self.live_partial = text.clone();
                }
                VoiceEvent::ProcessingStarted => {
                    self.state = VoiceState::Processing;
                }
                VoiceEvent::Final {
                    engine,
                    duration_ms,
                    ..
                } => {
                    self.state = VoiceState::Idle;
                    self.started_at = None;
                    self.stop_flag = None;
                    self.cancel_flag = None;
                    self.live_partial.clear();
                    self.last_engine_used = Some(engine.clone());
                    self.last_duration_ms = Some(*duration_ms);
                    if let Ok(mut bars) = self.spectrum.lock() {
                        *bars = [0.0; VISUALIZER_BUCKETS];
                    }
                }
                VoiceEvent::Error(err) => {
                    self.state = VoiceState::Error(err.clone());
                    self.started_at = None;
                    self.stop_flag = None;
                    self.cancel_flag = None;
                    self.live_partial.clear();
                    if let Ok(mut bars) = self.spectrum.lock() {
                        *bars = [0.0; VISUALIZER_BUCKETS];
                    }
                }
            }
            out.push(ev);
        }
        out
    }

    /// Starts listening if idle/error, or finishes and transcribes if currently listening.
    pub fn toggle(&mut self, current_input: &str, extra_custom_words: Vec<String>) {
        match self.state {
            VoiceState::Listening => {
                self.finish_listening();
            }
            VoiceState::Processing => {
                // Already processing; ignore duplicate clicks.
            }
            VoiceState::Idle | VoiceState::Error(_) => {
                self.start_listening(current_input, extra_custom_words);
            }
        }
    }

    /// Starts microphone recording (or Web Speech bridge if selected).
    pub fn start_listening(&mut self, current_input: &str, extra_custom_words: Vec<String>) {
        // Check Windows microphone privacy consent before opening the audio device.
        if let Some(denied_reason) = check_windows_mic_permission_denied() {
            self.state = VoiceState::Error(VoiceError {
                message: denied_reason,
                permission_denied: true,
            });
            return;
        }

        let prefix = if current_input.is_empty() || current_input.ends_with(char::is_whitespace) {
            current_input.to_string()
        } else {
            format!("{current_input} ")
        };
        self.input_prefix = prefix;
        self.live_partial.clear();
        if let Ok(mut bars) = self.spectrum.lock() {
            *bars = [0.0; VISUALIZER_BUCKETS];
        }

        let stop_flag = Arc::new(AtomicBool::new(false));
        let cancel_flag = Arc::new(AtomicBool::new(false));
        self.stop_flag = Some(stop_flag.clone());
        self.cancel_flag = Some(cancel_flag.clone());
        self.started_at = Some(Instant::now());
        self.state = VoiceState::Listening;

        let tx = self.tx.clone();
        let spectrum = self.spectrum.clone();
        let mic_id = self.selected_mic;
        let engine_pref = self.engine_pref;
        let auto_stop = self.auto_stop_on_silence;
        let voice_send_cmd = self.voice_send_command;
        let mut handy_info = detect_handy_runtime();
        for w in extra_custom_words {
            if !w.is_empty() && !handy_info.custom_words.contains(&w) {
                handy_info.custom_words.push(w);
            }
        }
        self.handy_info = handy_info.clone();

        std::thread::spawn(move || {
            if engine_pref == VoiceEnginePref::WebSpeechApi {
                run_web_speech_session(
                    tx,
                    spectrum,
                    stop_flag,
                    cancel_flag,
                    handy_info,
                    voice_send_cmd,
                );
            } else {
                run_native_recording_session(
                    tx,
                    spectrum,
                    stop_flag,
                    cancel_flag,
                    mic_id,
                    engine_pref,
                    auto_stop,
                    voice_send_cmd,
                    handy_info,
                );
            }
        });
    }

    /// Signals the recording thread to stop capturing and begin final transcription.
    pub fn finish_listening(&mut self) {
        if let Some(stop) = &self.stop_flag {
            stop.store(true, Ordering::SeqCst);
        }
        if self.state == VoiceState::Listening {
            self.state = VoiceState::Processing;
        }
    }

    /// Cancels an active recording without transcribing.
    pub fn cancel(&mut self) {
        if let Some(cancel) = &self.cancel_flag {
            cancel.store(true, Ordering::SeqCst);
        }
        if let Some(stop) = &self.stop_flag {
            stop.store(true, Ordering::SeqCst);
        }
        self.state = VoiceState::Idle;
        self.started_at = None;
        self.stop_flag = None;
        self.cancel_flag = None;
        self.live_partial.clear();
        if let Ok(mut bars) = self.spectrum.lock() {
            *bars = [0.0; VISUALIZER_BUCKETS];
        }
    }
}

// ============================================================================
// Handy 0.10.0 Runtime Discovery & Settings Reader
// ============================================================================

const DEFAULT_NEMOTRON_MODEL: &str =
    "handy-computer/nemotron-3.5-asr-streaming-0.6b-gguf/nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf";

pub fn detect_handy_runtime() -> HandyRuntimeInfo {
    let mut exe_candidates: Vec<PathBuf> = Vec::new();
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        exe_candidates.push(Path::new(&local_app_data).join("Handy").join("handy.exe"));
    }
    if let Ok(prog_files) = std::env::var("ProgramFiles") {
        exe_candidates.push(Path::new(&prog_files).join("Handy").join("handy.exe"));
    }
    // `HANDY_PATH` points at a Handy build somewhere else (a development build, a portable copy)
    if let Some(p) = std::env::var_os("HANDY_PATH")
        && !p.is_empty()
    {
        exe_candidates.insert(0, PathBuf::from(p));
    }

    let exe_path = exe_candidates.into_iter().find(|p| p.is_file());
    let version = exe_path.as_deref().and_then(handy_file_version);

    let mut selected_model = DEFAULT_NEMOTRON_MODEL.to_string();
    let mut custom_words: Vec<String> = Vec::new();

    if let Ok(appdata) = std::env::var("APPDATA") {
        let store_path = Path::new(&appdata)
            .join("com.pais.handy")
            .join("settings_store.json");
        if let Ok(raw) = std::fs::read_to_string(&store_path)
            && let Ok(val) = serde_json::from_str::<Value>(&raw)
        {
            let settings = val.get("settings").unwrap_or(&val);
            if let Some(m) = settings.get("selected_model").and_then(Value::as_str)
                && !m.trim().is_empty()
            {
                selected_model = m.trim().to_string();
            }
            if let Some(words) = settings.get("custom_words").and_then(Value::as_array) {
                for w in words.iter().filter_map(Value::as_str) {
                    if !w.trim().is_empty() {
                        custom_words.push(w.trim().to_string());
                    }
                }
            }
        }
    }

    let model_display = if selected_model.contains("nemotron-3.5-asr-streaming") {
        "Nemotron Streaming 3.5 (0.6B Q8_0)".to_string()
    } else if selected_model.contains("parakeet") {
        "Parakeet TDT 0.6B".to_string()
    } else if selected_model.contains("whisper") {
        format!("Whisper ({selected_model})")
    } else {
        selected_model
            .rsplit('/')
            .next()
            .unwrap_or(&selected_model)
            .to_string()
    };

    HandyRuntimeInfo {
        exe_path,
        selected_model,
        model_display,
        custom_words,
        version,
    }
}

/// The version WorkCraft is built for. Older Handy versions still work, and Settings says when to update.
pub const HANDY_TARGET_VERSION: &str = "0.10.0";

/// `true` when `version` (`0.10.0`, `0.10.1.0`, `v0.9.8`) is at least `wanted`.
#[must_use]
pub fn version_at_least(version: &str, wanted: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.trim()
            .trim_start_matches(['v', 'V'])
            .split(['.', '-', '+'])
            .map_while(|p| p.parse::<u64>().ok())
            .collect()
    }
    let (a, b) = (parts(version), parts(wanted));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    true
}

/// Reads the file version of `handy.exe` (Windows only; other systems report nothing).
fn handy_file_version(exe: &Path) -> Option<String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let script = format!(
            "(Get-Item -LiteralPath '{}').VersionInfo.FileVersion",
            exe.display().to_string().replace('\'', "''")
        );
        let out = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(0x0800_0000)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let v = text.lines().map(str::trim).find(|l| !l.is_empty())?;
        let token = v.split_whitespace().next()?;
        // 0.10.0.0 prints as 0.10.0
        let trimmed = token
            .strip_suffix(".0")
            .filter(|t| t.matches('.').count() >= 2)
            .unwrap_or(token);
        trimmed
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
            .then(|| trimmed.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = exe;
        None
    }
}

// ============================================================================
// Windows Microphone Permission Check & Privacy Settings Link
// (Ported from Handy-0.10.0/src-tauri/src/commands/audio.rs)
// ============================================================================

/// Returns `Some(friendly_error)` ONLY when Windows registry explicitly states that
/// desktop microphone access is `"Deny"`.
pub fn check_windows_mic_permission_denied() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        if is_reg_consent_denied(
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone\NonPackaged",
        ) || is_reg_consent_denied(
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone",
        ) || is_reg_consent_denied(
            r"HKLM\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone",
        ) {
            return Some(
                "Microphone access is turned off in Windows Privacy Settings. Enable 'Let desktop apps access your microphone' to use voice input."
                    .to_string(),
            );
        }
        None
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

#[cfg(target_os = "windows")]
fn is_reg_consent_denied(key_path: &str) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(out) = Command::new("reg")
        .args(["query", key_path, "/v", "Value"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .any(|line| line.contains("Value") && line.to_ascii_lowercase().contains("deny"))
}

/// Opens the system microphone privacy settings page (ported from Handy 0.10.0).
pub fn open_microphone_privacy_settings() {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = Command::new("cmd")
            .args(["/C", "start", "", "ms-settings:privacy-microphone"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
}

// ============================================================================
// Handy 0.10.0 AudioVisualiser (16-Band Perceptual FFT Spectrum Analyzer)
// Ported from Handy-0.10.0/src-tauri/src/audio_toolkit/audio/visualizer.rs
// ============================================================================

const DB_MIN: f32 = -68.0;
const DB_MAX: f32 = -30.0;
const GAIN: f32 = 1.3;
const CURVE_POWER: f32 = 0.7;

pub struct AudioVisualiser {
    window: Vec<f32>,
    bucket_ranges: Vec<(usize, usize)>,
    fft_real: Vec<f32>,
    fft_imag: Vec<f32>,
    noise_floor: Vec<f32>,
    buffer: Vec<f32>,
    window_size: usize,
    buckets: usize,
}

impl AudioVisualiser {
    pub fn new(
        sample_rate: u32,
        window_size: usize,
        buckets: usize,
        freq_min: f32,
        freq_max: f32,
    ) -> Self {
        let window: Vec<f32> = (0..window_size)
            .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / window_size as f32).cos()))
            .collect();

        let nyquist = sample_rate as f32 / 2.0;
        let freq_min = freq_min.min(nyquist);
        let freq_max = freq_max.min(nyquist);

        let mut bucket_ranges = Vec::with_capacity(buckets);
        for b in 0..buckets {
            let log_start = (b as f32 / buckets as f32).powi(2);
            let log_end = ((b + 1) as f32 / buckets as f32).powi(2);

            let start_hz = freq_min + (freq_max - freq_min) * log_start;
            let end_hz = freq_min + (freq_max - freq_min) * log_end;

            let start_bin = ((start_hz * window_size as f32) / sample_rate as f32) as usize;
            let mut end_bin = ((end_hz * window_size as f32) / sample_rate as f32) as usize;

            if end_bin <= start_bin {
                end_bin = start_bin + 1;
            }

            let start_bin = start_bin.min(window_size / 2);
            let end_bin = end_bin.min(window_size / 2);
            bucket_ranges.push((start_bin, end_bin));
        }

        Self {
            window,
            bucket_ranges,
            fft_real: vec![0.0; window_size],
            fft_imag: vec![0.0; window_size],
            noise_floor: vec![-40.0; buckets],
            buffer: Vec::with_capacity(window_size * 2),
            window_size,
            buckets,
        }
    }

    pub fn feed(&mut self, samples: &[f32]) -> Option<[f32; VISUALIZER_BUCKETS]> {
        self.buffer.extend_from_slice(samples);
        if self.buffer.len() < self.window_size {
            return None;
        }

        let window_samples = &self.buffer[..self.window_size];
        let mean = window_samples.iter().sum::<f32>() / self.window_size as f32;

        for (i, &sample) in window_samples.iter().enumerate() {
            self.fft_real[i] = (sample - mean) * self.window[i];
            self.fft_imag[i] = 0.0;
        }

        fft_in_place(&mut self.fft_real, &mut self.fft_imag);

        let mut buckets = [0.0f32; VISUALIZER_BUCKETS];
        let count = self.buckets.min(VISUALIZER_BUCKETS);

        for (bucket_idx, bucket_slot) in buckets.iter_mut().enumerate().take(count) {
            let (start_bin, end_bin) = self.bucket_ranges[bucket_idx];
            if start_bin >= end_bin || end_bin > self.window_size / 2 {
                continue;
            }

            let mut power_sum = 0.0f32;
            for bin_idx in start_bin..end_bin {
                let re = self.fft_real[bin_idx];
                let im = self.fft_imag[bin_idx];
                power_sum += re * re + im * im;
            }

            let avg_power = power_sum / (end_bin - start_bin) as f32;
            let db = if avg_power > 1e-12 {
                20.0 * (avg_power.sqrt() / self.window_size as f32).log10()
            } else {
                -80.0
            };

            if db < self.noise_floor[bucket_idx] + 10.0 {
                const NOISE_ALPHA: f32 = 0.001;
                self.noise_floor[bucket_idx] =
                    NOISE_ALPHA * db + (1.0 - NOISE_ALPHA) * self.noise_floor[bucket_idx];
            }

            let normalized = ((db - DB_MIN) / (DB_MAX - DB_MIN)).clamp(0.0, 1.0);
            *bucket_slot = (normalized * GAIN).powf(CURVE_POWER).clamp(0.0, 1.0);
        }

        let copy = buckets;
        for i in 1..count.saturating_sub(1) {
            buckets[i] = copy[i] * 0.7 + copy[i - 1] * 0.15 + copy[i + 1] * 0.15;
        }

        self.buffer.clear();
        Some(buckets)
    }
}

/// Radix-2 Cooley-Tukey in-place forward FFT (`real.len()` must be a power of two).
fn fft_in_place(real: &mut [f32], imag: &mut [f32]) {
    let n = real.len();
    if n <= 1 || !n.is_power_of_two() || imag.len() != n {
        return;
    }

    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            real.swap(i, j);
            imag.swap(i, j);
        }
    }

    let mut len = 2usize;
    while len <= n {
        let half = len / 2;
        let angle = -2.0 * PI / len as f32;
        let wlen_re = angle.cos();
        let wlen_im = angle.sin();
        let mut i = 0;
        while i < n {
            let mut w_re = 1.0f32;
            let mut w_im = 0.0f32;
            for k in 0..half {
                let u_re = real[i + k];
                let u_im = imag[i + k];
                let v_re = real[i + k + half] * w_re - imag[i + k + half] * w_im;
                let v_im = real[i + k + half] * w_im + imag[i + k + half] * w_re;
                real[i + k] = u_re + v_re;
                imag[i + k] = u_im + v_im;
                real[i + k + half] = u_re - v_re;
                imag[i + k + half] = u_im - v_im;
                let next_w_re = w_re * wlen_re - w_im * wlen_im;
                let next_w_im = w_re * wlen_im + w_im * wlen_re;
                w_re = next_w_re;
                w_im = next_w_im;
            }
            i += len;
        }
        len <<= 1;
    }
}

// ============================================================================
// Pure-Rust 16 kHz Mono 16-bit PCM WAV Writer
// ============================================================================

/// Largest absolute sample of a recording, 0.0 to 1.0.
pub fn peak_level(samples: &[i16]) -> f32 {
    samples
        .iter()
        .map(|s| f32::from(s.unsigned_abs()) / 32768.0)
        .fold(0.0_f32, f32::max)
}

/// What goes to the speech model: the recording without a DC offset, brought up to a healthy level when the
/// microphone is quiet (at most 16 times, about 24 dB; a loud recording is left alone), and half a second of silence
/// before and after. Handy 0.10 chunks long recordings with voice activity detection, which drops quiet speech and
/// the first syllable of a recording that starts at once ("No speech detected").
pub fn prepare_for_speech(samples: &[i16]) -> Vec<i16> {
    const PAD: usize = SAMPLE_RATE as usize / 2;
    let n = samples.len();
    let mean = if n == 0 {
        0.0
    } else {
        samples.iter().map(|&s| f64::from(s)).sum::<f64>() / n as f64
    };
    let peak = samples
        .iter()
        .map(|&s| (f64::from(s) - mean).abs())
        .fold(0.0_f64, f64::max)
        / 32768.0;
    let gain = if peak > 0.0005 && peak < 0.5 {
        (0.85 / peak).min(16.0)
    } else {
        1.0
    };
    let mut out = vec![0_i16; n + PAD * 2];
    for (i, &s) in samples.iter().enumerate() {
        let v = (f64::from(s) - mean) * gain;
        out[PAD + i] = v.clamp(-32768.0, 32767.0) as i16;
    }
    out
}

pub fn write_wav_16k_mono(path: &Path, samples: &[i16]) -> Result<(), String> {
    let mut file = std::fs::File::create(path).map_err(|e| format!("Failed to create WAV: {e}"))?;
    let data_bytes = (samples.len() * 2) as u32;
    let riff_size = 36u32.saturating_add(data_bytes);
    let byte_rate = SAMPLE_RATE * 2;
    let block_align: u16 = 2;
    let bits_per_sample: u16 = 16;

    let mut header = Vec::with_capacity(44);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&riff_size.to_le_bytes());
    header.extend_from_slice(b"WAVE");
    header.extend_from_slice(b"fmt ");
    header.extend_from_slice(&16u32.to_le_bytes()); // PCM chunk size
    header.extend_from_slice(&1u16.to_le_bytes()); // WAVE_FORMAT_PCM = 1
    header.extend_from_slice(&1u16.to_le_bytes()); // 1 channel (mono)
    header.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    header.extend_from_slice(&byte_rate.to_le_bytes());
    header.extend_from_slice(&block_align.to_le_bytes());
    header.extend_from_slice(&bits_per_sample.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&data_bytes.to_le_bytes());

    file.write_all(&header)
        .map_err(|e| format!("Failed to write WAV header: {e}"))?;

    let mut pcm_bytes = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        pcm_bytes.extend_from_slice(&s.to_le_bytes());
    }
    file.write_all(&pcm_bytes)
        .map_err(|e| format!("Failed to write WAV samples: {e}"))?;
    file.flush()
        .map_err(|e| format!("Failed to flush WAV: {e}"))?;
    Ok(())
}

// ============================================================================
// Windows Multimedia Audio Capture (`winmm.dll` waveIn at 16 kHz Mono PCM)
// ============================================================================

#[cfg(target_os = "windows")]
#[allow(unsafe_code, clippy::upper_case_acronyms)]
mod winmm {
    use super::MicDevice;

    pub const WAVE_MAPPER: u32 = 0xFFFF_FFFF;
    pub const WAVE_FORMAT_PCM: u16 = 1;
    pub const CALLBACK_NULL: u32 = 0;
    pub const MMSYSERR_NOERROR: u32 = 0;
    pub const WHDR_DONE: u32 = 0x0000_0001;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct WAVEFORMATEX {
        pub w_format_tag: u16,
        pub n_channels: u16,
        pub n_samples_per_sec: u32,
        pub n_avg_bytes_per_sec: u32,
        pub n_block_align: u16,
        pub w_bits_per_sample: u16,
        pub cb_size: u16,
    }

    #[repr(C)]
    pub struct WAVEHDR {
        pub lp_data: *mut u8,
        pub dw_buffer_length: u32,
        pub dw_bytes_recorded: u32,
        pub dw_user: usize,
        pub dw_flags: u32,
        pub dw_loops: u32,
        pub lp_next: *mut WAVEHDR,
        pub reserved: usize,
    }

    #[repr(C)]
    pub struct WAVEINCAPSW {
        pub w_mid: u16,
        pub w_pid: u16,
        pub v_driver_version: u32,
        pub sz_pname: [u16; 32],
        pub dw_formats: u32,
        pub w_channels: u16,
        pub w_reserved1: u16,
    }

    #[link(name = "winmm")]
    unsafe extern "system" {
        pub fn waveInGetNumDevs() -> u32;
        pub fn waveInGetDevCapsW(u_device_id: usize, pwic: *mut WAVEINCAPSW, cbwic: u32) -> u32;
        pub fn waveInOpen(
            phwi: *mut usize,
            u_device_id: u32,
            pwfx: *const WAVEFORMATEX,
            dw_callback: usize,
            dw_instance: usize,
            fdw_open: u32,
        ) -> u32;
        pub fn waveInPrepareHeader(hwi: usize, pwh: *mut WAVEHDR, cbwh: u32) -> u32;
        pub fn waveInUnprepareHeader(hwi: usize, pwh: *mut WAVEHDR, cbwh: u32) -> u32;
        pub fn waveInAddBuffer(hwi: usize, pwh: *mut WAVEHDR, cbwh: u32) -> u32;
        pub fn waveInStart(hwi: usize) -> u32;
        pub fn waveInStop(hwi: usize) -> u32;
        pub fn waveInReset(hwi: usize) -> u32;
        pub fn waveInClose(hwi: usize) -> u32;
    }

    pub fn enumerate_mics() -> Vec<MicDevice> {
        let mut out = vec![MicDevice {
            id: None,
            name: "System Default Microphone".to_string(),
        }];
        let count = unsafe { waveInGetNumDevs() };
        for id in 0..count {
            let mut caps = WAVEINCAPSW {
                w_mid: 0,
                w_pid: 0,
                v_driver_version: 0,
                sz_pname: [0u16; 32],
                dw_formats: 0,
                w_channels: 0,
                w_reserved1: 0,
            };
            let rc = unsafe {
                waveInGetDevCapsW(
                    id as usize,
                    &mut caps as *mut WAVEINCAPSW,
                    std::mem::size_of::<WAVEINCAPSW>() as u32,
                )
            };
            if rc == MMSYSERR_NOERROR {
                let end = caps
                    .sz_pname
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(caps.sz_pname.len());
                let name = String::from_utf16_lossy(&caps.sz_pname[..end])
                    .trim()
                    .to_string();
                if !name.is_empty() {
                    out.push(MicDevice { id: Some(id), name });
                }
            }
        }
        out
    }
}

pub fn list_microphones() -> Vec<MicDevice> {
    #[cfg(target_os = "windows")]
    {
        winmm::enumerate_mics()
    }
    #[cfg(not(target_os = "windows"))]
    {
        vec![MicDevice {
            id: None,
            name: "Default Microphone".to_string(),
        }]
    }
}

#[allow(clippy::too_many_arguments, unsafe_code)]
fn run_native_recording_session(
    tx: Sender<VoiceEvent>,
    spectrum: Arc<Mutex<[f32; VISUALIZER_BUCKETS]>>,
    stop_flag: Arc<AtomicBool>,
    cancel_flag: Arc<AtomicBool>,
    mic_id: Option<u32>,
    engine_pref: VoiceEnginePref,
    auto_stop_on_silence: bool,
    voice_send_cmd: bool,
    handy_info: HandyRuntimeInfo,
) {
    #[cfg(target_os = "windows")]
    {
        use winmm::*;

        let num_devs = unsafe { waveInGetNumDevs() };
        if num_devs == 0 {
            let _ = tx.send(VoiceEvent::Error(VoiceError {
                message: "No microphone input device detected. Connect a microphone or headset and try again.".to_string(),
                permission_denied: false,
            }));
            return;
        }

        let fmt = WAVEFORMATEX {
            w_format_tag: WAVE_FORMAT_PCM,
            n_channels: 1,
            n_samples_per_sec: SAMPLE_RATE,
            n_avg_bytes_per_sec: SAMPLE_RATE * 2,
            n_block_align: 2,
            w_bits_per_sample: 16,
            cb_size: 0,
        };

        let mut hwi: usize = 0;
        let dev = mic_id.unwrap_or(WAVE_MAPPER);
        let rc = unsafe {
            waveInOpen(
                &mut hwi as *mut usize,
                dev,
                &fmt as *const WAVEFORMATEX,
                0,
                0,
                CALLBACK_NULL,
            )
        };
        if rc != MMSYSERR_NOERROR || hwi == 0 {
            let denied = check_windows_mic_permission_denied().is_some();
            let msg = if denied {
                "Microphone access was denied by Windows Privacy Settings. Enable desktop microphone access and try again.".to_string()
            } else {
                format!(
                    "Could not open microphone device (winmm code {rc}). Check that your microphone is connected and not locked by another app."
                )
            };
            let _ = tx.send(VoiceEvent::Error(VoiceError {
                message: msg,
                permission_denied: denied,
            }));
            return;
        }

        const NUM_BUFFERS: usize = 8;
        const CHUNK_SAMPLES: usize = FFT_WINDOW_SIZE; // 512 samples = 32 ms @ 16 kHz
        const CHUNK_BYTES: usize = CHUNK_SAMPLES * 2;

        let mut backing: Vec<Vec<u8>> = (0..NUM_BUFFERS).map(|_| vec![0u8; CHUNK_BYTES]).collect();
        let mut headers: Vec<WAVEHDR> = backing
            .iter_mut()
            .map(|buf| WAVEHDR {
                lp_data: buf.as_mut_ptr(),
                dw_buffer_length: CHUNK_BYTES as u32,
                dw_bytes_recorded: 0,
                dw_user: 0,
                dw_flags: 0,
                dw_loops: 0,
                lp_next: std::ptr::null_mut(),
                reserved: 0,
            })
            .collect();

        let hdr_size = std::mem::size_of::<WAVEHDR>() as u32;
        for hdr in &mut headers {
            let _ = unsafe { waveInPrepareHeader(hwi, hdr as *mut WAVEHDR, hdr_size) };
            let _ = unsafe { waveInAddBuffer(hwi, hdr as *mut WAVEHDR, hdr_size) };
        }

        let start_rc = unsafe { waveInStart(hwi) };
        if start_rc != MMSYSERR_NOERROR {
            for hdr in &mut headers {
                let _ = unsafe { waveInUnprepareHeader(hwi, hdr as *mut WAVEHDR, hdr_size) };
            }
            let _ = unsafe { waveInClose(hwi) };
            let _ = tx.send(VoiceEvent::Error(VoiceError {
                message: format!("Failed to start microphone stream (code {start_rc})."),
                permission_denied: false,
            }));
            return;
        }

        let mut visualiser = AudioVisualiser::new(
            SAMPLE_RATE,
            FFT_WINDOW_SIZE,
            VISUALIZER_BUCKETS,
            80.0,
            4000.0,
        );
        let all_samples: Arc<Mutex<Vec<i16>>> =
            Arc::new(Mutex::new(Vec::with_capacity(SAMPLE_RATE as usize * 30)));
        let float_chunk = &mut [0.0f32; CHUNK_SAMPLES];

        // Track speech activity for silence auto-stop and live partial streaming
        let mut speech_frames: usize = 0;
        let mut last_speech_at: Option<Instant> = None;
        let mut last_partial_samples: usize = 0;
        let mut last_partial_at = Instant::now();
        let partial_in_flight = Arc::new(AtomicBool::new(false));
        let latest_partial_seq = Arc::new(AtomicUsize::new(0));
        let mut next_partial_seq: usize = 1;

        while !stop_flag.load(Ordering::Relaxed) && !cancel_flag.load(Ordering::Relaxed) {
            let mut any_done = false;
            for (idx, hdr) in headers.iter_mut().enumerate() {
                if (hdr.dw_flags & WHDR_DONE) != 0 {
                    any_done = true;
                    let recorded_bytes = (hdr.dw_bytes_recorded as usize).min(CHUNK_BYTES);
                    let sample_count = recorded_bytes / 2;
                    if sample_count > 0 {
                        let raw = &backing[idx][..sample_count * 2];
                        let mut sum_sq = 0.0f32;
                        let mut chunk_i16 = Vec::with_capacity(sample_count);
                        for (i, &pair) in raw.as_chunks::<2>().0.iter().enumerate() {
                            let s = i16::from_le_bytes(pair);
                            chunk_i16.push(s);
                            let f = s as f32 / 32768.0;
                            if i < CHUNK_SAMPLES {
                                float_chunk[i] = f;
                            }
                            sum_sq += f * f;
                        }

                        if let Ok(mut guard) = all_samples.lock() {
                            guard.extend_from_slice(&chunk_i16);
                        }

                        let rms = (sum_sq / sample_count as f32).sqrt();
                        // Speech activity threshold (~ -42 dBFS)
                        if rms > 0.0085 {
                            speech_frames += 1;
                            last_speech_at = Some(Instant::now());
                        }

                        if let Some(bars) = visualiser.feed(&float_chunk[..sample_count])
                            && let Ok(mut g) = spectrum.lock()
                        {
                            *g = bars;
                        }
                    }

                    hdr.dw_bytes_recorded = 0;
                    hdr.dw_flags &= !WHDR_DONE;
                    let _ = unsafe { waveInAddBuffer(hwi, hdr as *mut WAVEHDR, hdr_size) };
                }
            }

            // Check if we should launch a live streaming partial transcription snapshot
            let total_len = all_samples.lock().map(|g| g.len()).unwrap_or(0);
            if speech_frames >= 6
                && total_len >= last_partial_samples + (SAMPLE_RATE as usize * 9 / 5) // ~1.8s of new audio
                && last_partial_at.elapsed() >= Duration::from_millis(1600)
                && !partial_in_flight.load(Ordering::Relaxed)
                && !stop_flag.load(Ordering::Relaxed)
                && let Ok(guard) = all_samples.lock()
            {
                let snapshot: Vec<i16> = guard.clone();
                drop(guard);
                last_partial_samples = total_len;
                last_partial_at = Instant::now();
                partial_in_flight.store(true, Ordering::SeqCst);

                let p_tx = tx.clone();
                let p_in_flight = partial_in_flight.clone();
                let p_stop = stop_flag.clone();
                let p_handy = handy_info.clone();
                let p_seq = next_partial_seq;
                next_partial_seq += 1;
                let p_latest_seq = latest_partial_seq.clone();

                std::thread::spawn(move || {
                    if let Ok((raw_text, _)) =
                        transcribe_samples(&snapshot, engine_pref, &p_handy, true)
                    {
                        let (cleaned, _) =
                            normalize_voice_transcript(&raw_text, &p_handy.custom_words);
                        if !cleaned.is_empty() && !p_stop.load(Ordering::Relaxed) {
                            let prev = p_latest_seq.fetch_max(p_seq, Ordering::SeqCst);
                            if p_seq >= prev {
                                let _ = p_tx.send(VoiceEvent::Partial(cleaned));
                            }
                        }
                    }
                    p_in_flight.store(false, Ordering::SeqCst);
                });
            }

            // Auto-stop when the user has spoken (>= ~320ms of active speech) and then stops speaking for 2.3s
            if auto_stop_on_silence
                && speech_frames >= 10
                && let Some(last_sp) = last_speech_at
                && last_sp.elapsed() >= Duration::from_millis(2300)
            {
                stop_flag.store(true, Ordering::SeqCst);
                let _ = tx.send(VoiceEvent::ProcessingStarted);
                break;
            }

            if !any_done {
                std::thread::sleep(Duration::from_millis(12));
            }
        }

        // Stop and clean up waveIn device
        let _ = unsafe { waveInStop(hwi) };
        let _ = unsafe { waveInReset(hwi) };
        for hdr in &mut headers {
            let _ = unsafe { waveInUnprepareHeader(hwi, hdr as *mut WAVEHDR, hdr_size) };
        }
        let _ = unsafe { waveInClose(hwi) };

        if let Ok(mut g) = spectrum.lock() {
            *g = [0.0; VISUALIZER_BUCKETS];
        }

        if cancel_flag.load(Ordering::Relaxed) {
            return;
        }

        let final_samples = match all_samples.lock() {
            Ok(g) => g.clone(),
            Err(p) => p.into_inner().clone(),
        };

        // Ensure at least 0.25s of audio was captured
        if final_samples.len() < (SAMPLE_RATE as usize / 4) {
            let _ = tx.send(VoiceEvent::Error(VoiceError {
                message: "Recording was too short. Click the microphone and speak a command."
                    .to_string(),
                permission_denied: false,
            }));
            return;
        }

        let t0 = Instant::now();
        match transcribe_samples(&final_samples, engine_pref, &handy_info, false) {
            Ok((raw_text, engine_used)) => {
                let (normalized, spoken_send) =
                    normalize_voice_transcript(&raw_text, &handy_info.custom_words);
                if normalized.trim().is_empty() {
                    let _ = tx.send(VoiceEvent::Error(VoiceError {
                        message: if peak_level(&final_samples) < 0.03 {
                            "The microphone picked up almost nothing. Check that the right one is selected and not muted, or raise its input level."
                        } else {
                            "The recording was heard but no words came out. Speak a little slower and closer, or try a larger speech model in Handy."
                        }
                        .to_string(),
                        permission_denied: false,
                    }));
                    return;
                }
                let _ = tx.send(VoiceEvent::Final {
                    text: normalized,
                    engine: engine_used,
                    duration_ms: t0.elapsed().as_millis() as u64,
                    auto_send: voice_send_cmd && spoken_send,
                });
            }
            Err(err_msg) => {
                let _ = tx.send(VoiceEvent::Error(VoiceError {
                    message: err_msg,
                    permission_denied: false,
                }));
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (
            spectrum,
            stop_flag,
            cancel_flag,
            mic_id,
            engine_pref,
            auto_stop_on_silence,
            voice_send_cmd,
            handy_info,
        );
        let _ = tx.send(VoiceEvent::Error(VoiceError {
            message: "Native microphone capture is only enabled on Windows in this build."
                .to_string(),
            permission_denied: false,
        }));
    }
}

// ============================================================================
// Multi-Engine Speech-to-Text Transcription
// 1. Primary: Handy 0.10.0 (`handy.exe --transcribe-file <wav> --json`)
//    with Nemotron Streaming 3.5 (`nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf`)
// 2. Fallback: Windows Speech Recognition (`System.Speech.Recognition`)
// ============================================================================

fn transcribe_samples(
    samples: &[i16],
    pref: VoiceEnginePref,
    handy_info: &HandyRuntimeInfo,
    is_partial: bool,
) -> Result<(String, String), String> {
    let tag = if is_partial { "partial" } else { "final" };
    let wav_path =
        std::env::temp_dir().join(format!("workcraft-voice-{tag}-{}.wav", std::process::id()));
    write_wav_16k_mono(&wav_path, &prepare_for_speech(samples))?;

    let res = match pref {
        VoiceEnginePref::Auto => {
            if let Some(exe) = &handy_info.exe_path {
                match transcribe_with_handy(exe, &wav_path, &handy_info.selected_model) {
                    Ok(out) => Ok(out),
                    Err(handy_err) => {
                        if is_partial {
                            Err(handy_err)
                        } else {
                            transcribe_with_windows_speech(&wav_path).map_err(|win_err| {
                                format!("{handy_err} (Fallback failed: {win_err})")
                            })
                        }
                    }
                }
            } else {
                transcribe_with_windows_speech(&wav_path)
            }
        }
        VoiceEnginePref::HandyNemotron => {
            let Some(exe) = &handy_info.exe_path else {
                let _ = std::fs::remove_file(&wav_path);
                return Err(
                    "Handy (handy.exe) was not found in %LOCALAPPDATA%\\Handy. Switch Voice Engine to Auto or install Handy 0.10.0 or newer."
                        .to_string(),
                );
            };
            transcribe_with_handy(exe, &wav_path, &handy_info.selected_model)
        }
        VoiceEnginePref::WindowsSpeech | VoiceEnginePref::WebSpeechApi => {
            transcribe_with_windows_speech(&wav_path)
        }
    };

    let _ = std::fs::remove_file(&wav_path);
    res
}

fn transcribe_with_handy(
    handy_exe: &Path,
    wav_path: &Path,
    selected_model: &str,
) -> Result<(String, String), String> {
    let mut cmd = Command::new(handy_exe);
    cmd.arg("--transcribe-file")
        .arg(wav_path)
        .arg("--json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if !selected_model.trim().is_empty() {
        cmd.arg("--model").arg(selected_model);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let out = cmd
        .output()
        .map_err(|e| format!("Failed to start Handy ({}): {e}", handy_exe.display()))?;

    let stdout_str = String::from_utf8_lossy(&out.stdout);
    let stderr_str = String::from_utf8_lossy(&out.stderr);

    // Find JSON line in stdout
    for line in stdout_str.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with('{')
            && trimmed.ends_with('}')
            && let Ok(val) = serde_json::from_str::<Value>(trimmed)
        {
            if let Some(err) = val.get("error").and_then(Value::as_str) {
                return Err(format!("Handy transcription error: {err}"));
            }
            let text = val
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let backend = val
                .get("bound_backend")
                .and_then(Value::as_str)
                .unwrap_or("CPU")
                .to_string();
            let model = val
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(selected_model);
            let engine_label = if model.contains("nemotron-3.5") {
                format!("Handy \u{00b7} Nemotron 3.5 ({backend})")
            } else {
                format!("Handy ({backend})")
            };
            return Ok((text, engine_label));
        }
    }

    if !out.status.success() {
        let detail = if !stderr_str.trim().is_empty() {
            stderr_str.trim().to_string()
        } else {
            format!("exit status {}", out.status)
        };
        return Err(format!("Handy failed: {detail}"));
    }

    Ok((stdout_str.trim().to_string(), "Handy".to_string()))
}

/// Windows Speech, English only (WorkCraft AI speaks English).
fn transcribe_with_windows_speech(wav_path: &Path) -> Result<(String, String), String> {
    let wav_escaped = wav_path.display().to_string().replace('\'', "''");
    let preferred_culture = "en-US";
    let ps_script = format!(
        r#"[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Add-Type -AssemblyName System.Speech
$installed = [System.Speech.Recognition.SpeechRecognitionEngine]::InstalledRecognizers()
if ($installed.Count -eq 0) {{ Write-Error 'No Windows Speech recognizer installed'; exit 2 }}
$recInfo = $installed | Where-Object {{ $_.Culture.Name -eq '{preferred_culture}' }} | Select-Object -First 1
if (-not $recInfo) {{ $recInfo = $installed | Select-Object -First 1 }}
$engine = New-Object System.Speech.Recognition.SpeechRecognitionEngine($recInfo.Culture)
$engine.LoadGrammar((New-Object System.Speech.Recognition.DictationGrammar))
$engine.SetInputToWaveFile('{wav_escaped}')
$sb = New-Object System.Text.StringBuilder
while ($true) {{
    $r = $engine.Recognize()
    if (-not $r) {{ break }}
    if ($r.Text) {{ [void]$sb.Append($r.Text).Append(' ') }}
}}
$engine.Dispose()
Write-Output ($sb.ToString().Trim())"#
    );

    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        &ps_script,
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let out = cmd
        .output()
        .map_err(|e| format!("Failed to invoke Windows Speech recognizer: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(format!("Windows Speech Recognition failed: {err}"));
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok((text, format!("Windows Speech API ({preferred_culture})")))
}

// ============================================================================
// Web Speech API Fallback Bridge (Localhost HTTP + Browser SpeechRecognition)
// ============================================================================

fn run_web_speech_session(
    tx: Sender<VoiceEvent>,
    spectrum: Arc<Mutex<[f32; VISUALIZER_BUCKETS]>>,
    stop_flag: Arc<AtomicBool>,
    cancel_flag: Arc<AtomicBool>,
    handy_info: HandyRuntimeInfo,
    voice_send_cmd: bool,
) {
    use std::io::Read;
    use std::net::TcpListener;

    let Ok(listener) = TcpListener::bind("127.0.0.1:0") else {
        let _ = tx.send(VoiceEvent::Error(VoiceError {
            message: "Failed to bind local Web Speech API bridge port.".to_string(),
            permission_denied: false,
        }));
        return;
    };
    let _ = listener.set_nonblocking(true);
    let Ok(addr) = listener.local_addr() else {
        return;
    };
    let url = format!("http://127.0.0.1:{}/", addr.port());
    open_url_in_browser(&url);

    let t0 = Instant::now();
    let mut final_text = String::new();
    let mut phase = 0.0f32;

    while !stop_flag.load(Ordering::Relaxed) && !cancel_flag.load(Ordering::Relaxed) {
        phase += 0.22;
        if let Ok(mut bars) = spectrum.lock() {
            for (i, b) in bars.iter_mut().enumerate() {
                *b = ((phase + i as f32 * 0.45).sin() * 0.35 + 0.45).clamp(0.08, 0.92);
            }
        }

        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
            let mut buf = [0u8; 8192];
            if let Ok(n) = stream.read(&mut buf)
                && n > 0
            {
                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or("");
                if first_line.starts_with("GET ") {
                    let html = web_speech_bridge_html(addr.port());
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        html.len(),
                        html
                    );
                    let _ = stream.write_all(resp.as_bytes());
                } else if first_line.starts_with("POST /partial") {
                    let body = req.split("\r\n\r\n").nth(1).unwrap_or("").trim();
                    if !body.is_empty() {
                        let (cleaned, _) =
                            normalize_voice_transcript(body, &handy_info.custom_words);
                        final_text = cleaned.clone();
                        let _ = tx.send(VoiceEvent::Partial(cleaned));
                    }
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 2\r\n\r\nok",
                    );
                } else if first_line.starts_with("POST /final") {
                    let body = req.split("\r\n\r\n").nth(1).unwrap_or("").trim();
                    if !body.is_empty() {
                        final_text = body.to_string();
                    }
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 2\r\n\r\nok",
                    );
                    stop_flag.store(true, Ordering::SeqCst);
                    break;
                }
            }
        }

        if t0.elapsed() > Duration::from_secs(60) {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }

    if cancel_flag.load(Ordering::Relaxed) {
        return;
    }

    let (normalized, spoken_send) =
        normalize_voice_transcript(&final_text, &handy_info.custom_words);
    if normalized.is_empty() {
        let _ = tx.send(VoiceEvent::Error(VoiceError {
            message: "Web Speech API session closed without speech.".to_string(),
            permission_denied: false,
        }));
    } else {
        let _ = tx.send(VoiceEvent::Final {
            text: normalized,
            engine: "Web Speech API".to_string(),
            duration_ms: t0.elapsed().as_millis() as u64,
            auto_send: voice_send_cmd && spoken_send,
        });
    }
}

fn open_url_in_browser(url: &str) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("xdg-open").arg(url).spawn();
    }
}

fn web_speech_bridge_html(port: u16) -> String {
    format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>WorkCraft Web Speech Input</title>
<style>
body {{ font-family: system-ui, sans-serif; background: #0f141c; color: #e5e7eb; display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }}
.card {{ background: #18202c; border: 1px solid #2e3b4e; border-radius: 16px; padding: 28px 32px; max-width: 460px; text-align: center; }}
h2 {{ margin: 0 0 8px; font-size: 18px; color: #60a5fa; }}
p {{ margin: 8px 0; font-size: 14px; color: #9ca3af; }}
#transcript {{ color: #f9fafb; font-size: 16px; min-height: 28px; margin: 16px 0; }}
button {{ background: #2563eb; color: #fff; border: none; border-radius: 999px; padding: 10px 22px; font-size: 14px; cursor: pointer; }}
</style></head>
<body><div class="card">
<h2>WorkCraft Web Speech API</h2>
<p id="status">Listening... Speak your command now.</p>
<div id="transcript"></div>
<button onclick="finish()">Done</button>
</div>
<script>
const SR = window.SpeechRecognition || window.webkitSpeechRecognition;
let full = "";
if (!SR) {{
  document.getElementById('status').innerText = 'Web Speech API is not supported in this browser.';
}} else {{
  const rec = new SR();
  rec.continuous = true;
  rec.interimResults = true;
  rec.onresult = (e) => {{
    let text = "";
    for (let i = 0; i < e.results.length; i++) text += e.results[i][0].transcript;
    full = text;
    document.getElementById('transcript').innerText = text;
    fetch('http://127.0.0.1:{port}/partial', {{ method: 'POST', body: text }});
  }};
  rec.onend = () => finish();
  rec.start();
}}
function finish() {{
  fetch('http://127.0.0.1:{port}/final', {{ method: 'POST', body: full }}).then(() => window.close());
}}
</script></body></html>"#
    )
}

// ============================================================================
// Handy 0.10.0 Custom Word Fuzzy/Phonetic Matcher + WorkCraft Voice Normalizer
// Ported from Handy-0.10.0/src-tauri/src/audio_toolkit/text.rs
// ============================================================================

fn match_craft_app_id(s: &str) -> Option<&'static str> {
    let clean = s.trim();
    if clean.is_empty() {
        return None;
    }
    workcraft_agents::craft::specs()
        .iter()
        .find(|spec| {
            spec.id.eq_ignore_ascii_case(clean)
                || spec
                    .id
                    .replace('-', "")
                    .eq_ignore_ascii_case(&clean.replace('-', ""))
        })
        .map(|spec| spec.id)
}

/// Normalizes a raw speech transcript for WorkCraft:
/// 1. Applies Handy 0.10.0's fuzzy + Soundex n-gram custom word replacement for Craft apps
///    and user vocabulary (so `"Grid Kraft"` / `"Word Kraft"` become `"gridcraft"` / `"wordcraft"`).
/// 2. Converts spoken `@` mentions (`"at gridcraft"`, or a leading Craft app name like
///    `"gridcraft read column B"`) into `"@gridcraft read column B"`.
/// 3. Converts spoken `/` slash commands (`"slash docx"`, `"slash team"`, `"slash undo"`) into `"/docx"`, etc.
/// 4. Detects optional trailing voice-send phrases (`"send message"`, `"send this message"`),
///    returning `(normalized_text, should_auto_send)`.
pub fn normalize_voice_transcript(raw: &str, extra_custom_words: &[String]) -> (String, bool) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return (String::new(), false);
    }

    // Build combined custom vocabulary (Craft apps + user custom words)
    let mut vocab: Vec<String> = workcraft_agents::craft::specs()
        .iter()
        .map(|s| s.id.to_string())
        .collect();
    vocab.push("WorkCraft".to_string());
    for w in extra_custom_words {
        if !w.trim().is_empty() && !vocab.iter().any(|v| v.eq_ignore_ascii_case(w.trim())) {
            vocab.push(w.trim().to_string());
        }
    }

    // Step 1: Apply Handy 0.10.0 fuzzy + Soundex n-gram custom word correction
    let corrected = apply_custom_words(trimmed, &vocab, 0.24);

    // Step 2: Normalize spoken `@` mentions and `/` slash commands
    let normalized = normalize_mentions_and_commands(&corrected);

    // Step 3: Check for explicit trailing voice send command
    let (without_send, should_send) = strip_trailing_send_command(&normalized);

    (without_send, should_send)
}

fn normalize_mentions_and_commands(text: &str) -> String {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.is_empty() {
        return String::new();
    }

    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut i = 0;

    while i < tokens.len() {
        let tok = tokens[i];
        let lower_clean = tok
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '@' && c != '/')
            .to_ascii_lowercase();

        // Handle spoken "at <craftapp>" or "@ <craftapp>"
        if matches!(lower_clean.as_str(), "at" | "@" | "et")
            && let Some(&next_tok) = tokens.get(i + 1)
        {
            let (_, next_core, next_suffix) = split_punct(next_tok);
            let next_lower = next_core.to_ascii_lowercase();
            if let Some(app) = match_craft_app_id(&next_lower) {
                let tail = next_suffix.trim_matches(|c: char| c == ',' || c == ':');
                out.push(format!("@{app}{tail}"));
                i += 2;
                continue;
            }
        }

        // Handle spoken "slash <cmd>" at the start of the message
        if i == 0
            && lower_clean == "slash"
            && let Some(&next_tok) = tokens.get(i + 1)
        {
            let (_, next_core, _) = split_punct(next_tok);
            let cmd_lower = next_core.to_ascii_lowercase();
            if matches!(
                cmd_lower.as_str(),
                "undo"
                    | "new"
                    | "copy"
                    | "docx"
                    | "xlsx"
                    | "pptx"
                    | "web"
                    | "team"
                    | "single"
                    | "auto"
                    | "ask"
                    | "mcp"
                    | "skills"
                    | "export"
            ) {
                out.push(format!("/{cmd_lower}"));
                i += 2;
                continue;
            }
        }

        // Handle leading Craft app name at token 0 when addressing an app directly,
        // e.g. "Gridcraft read column B" or "Gridcraft, read column B" -> "@gridcraft read column B"
        if i == 0 && tokens.len() >= 2 {
            let (prefix, core, suffix) = split_punct(tok);
            let core_lower = core.to_ascii_lowercase();
            if let Some(app) = match_craft_app_id(&core_lower) {
                let clean_suffix = suffix.trim_matches(|c: char| c == ',' || c == ':');
                let clean_prefix = prefix.trim_start_matches('@');
                out.push(format!("{clean_prefix}@{app}{clean_suffix}"));
                i += 1;
                continue;
            }
        }

        // Also ensure existing `@GridCraft` is lowercase `@gridcraft`
        if let Some(stripped) = tok.strip_prefix('@') {
            let (_, core, suffix) = split_punct(stripped);
            let core_lower = core.to_ascii_lowercase();
            if let Some(app) = match_craft_app_id(&core_lower) {
                let clean_suffix = suffix.trim_matches(|c: char| c == ',' || c == ':');
                out.push(format!("@{app}{clean_suffix}"));
                i += 1;
                continue;
            }
        }

        out.push(tok.to_string());
        i += 1;
    }

    let joined = out.join(" ");
    // Strip trailing period on short single-clause @app commands like "@gridcraft read column B."
    if joined.starts_with('@') && joined.ends_with('.') && !joined[..joined.len() - 1].contains('.')
    {
        joined[..joined.len() - 1].to_string()
    } else {
        joined
    }
}

fn strip_trailing_send_command(text: &str) -> (String, bool) {
    let trimmed = text.trim_end_matches(|c: char| c.is_whitespace() || c == '.' || c == '!');
    let lower = trimmed.to_ascii_lowercase();
    const SEND_SUFFIXES: [&str; 2] = [" send message", " send this message"];
    for suf in SEND_SUFFIXES {
        if lower.ends_with(suf) && trimmed.len() > suf.len() {
            let head = trimmed[..trimmed.len() - suf.len()]
                .trim_end_matches(|c: char| c.is_whitespace() || c == ',' || c == '.' || c == ';')
                .to_string();
            if !head.is_empty() {
                return (head, true);
            }
        }
    }
    (text.to_string(), false)
}

fn split_punct(word: &str) -> (&str, &str, &str) {
    let prefix_end = word
        .char_indices()
        .find(|(_, c)| c.is_alphanumeric())
        .map(|(i, _)| i)
        .unwrap_or(word.len());
    let suffix_start = word
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_alphanumeric())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    if prefix_end <= suffix_start {
        (
            &word[..prefix_end],
            &word[prefix_end..suffix_start],
            &word[suffix_start..],
        )
    } else {
        (word, "", "")
    }
}

/// Applies custom word corrections to transcribed text using Levenshtein + Soundex n-gram matching
/// (ported from `Handy-0.10.0/src-tauri/src/audio_toolkit/text.rs`).
pub fn apply_custom_words(text: &str, custom_words: &[String], threshold: f64) -> String {
    if custom_words.is_empty() {
        return text.to_string();
    }

    let custom_keys: Vec<(usize, String)> = custom_words
        .iter()
        .enumerate()
        .map(|(idx, w)| (idx, build_match_key(w)))
        .filter(|(_, k)| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric()))
        .collect();

    let words: Vec<&str> = text.split_whitespace().collect();
    let mut result = Vec::with_capacity(words.len());
    let mut i = 0;

    while i < words.len() {
        let mut best_match: Option<(usize, &String, f64)> = None;

        for n in (1..=3).rev() {
            if i + n > words.len() {
                continue;
            }
            let ngram_words = &words[i..i + n];
            if ngram_words[..n.saturating_sub(1)]
                .iter()
                .any(|w| !split_punct(w).2.is_empty())
            {
                continue;
            }
            let candidate: String = ngram_words.iter().map(|w| build_match_key(w)).collect();
            if candidate.is_empty() || !candidate.chars().all(|c| c.is_ascii_alphanumeric()) {
                continue;
            }

            for (word_idx, key) in &custom_keys {
                let cand_len = candidate.len();
                let key_len = key.len();
                let len_diff = cand_len.abs_diff(key_len) as f64;
                let max_len = cand_len.max(key_len) as f64;
                if len_diff > (max_len * 0.25).max(2.0) {
                    continue;
                }
                let dist = levenshtein(&candidate, key);
                if n > 1 {
                    let without_first: String = ngram_words[1..]
                        .iter()
                        .map(|w| build_match_key(w))
                        .collect();
                    let without_last: String = ngram_words[..n - 1]
                        .iter()
                        .map(|w| build_match_key(w))
                        .collect();
                    if levenshtein(&without_first, key) <= dist
                        || levenshtein(&without_last, key) <= dist
                    {
                        continue;
                    }
                }
                let lev_score = if max_len > 0.0 {
                    dist as f64 / max_len
                } else {
                    1.0
                };
                let phonetic = candidate.chars().all(|c| c.is_ascii_alphabetic())
                    && key.chars().all(|c| c.is_ascii_alphabetic())
                    && soundex_equal(&candidate, key);
                let score = if phonetic { lev_score * 0.3 } else { lev_score };
                if score < threshold
                    && best_match
                        .as_ref()
                        .is_none_or(|(_, _, best_s)| score < *best_s)
                {
                    best_match = Some((n, &custom_words[*word_idx], score));
                }
            }
        }

        if let Some((n, replacement, _)) = best_match {
            let ngram_words = &words[i..i + n];
            let (prefix, _, _) = split_punct(ngram_words[0]);
            let (_, _, suffix) = split_punct(ngram_words[n - 1]);
            result.push(format!("{prefix}{replacement}{suffix}"));
            i += n;
        } else {
            result.push(words[i].to_string());
            i += 1;
        }
    }

    result.join(" ")
}

fn build_match_key(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b_chars: Vec<char> = b.chars().collect();
    let m = b_chars.len();
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr = vec![0usize; m + 1];

    for (i, ca) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b_chars.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

fn soundex_code(s: &str) -> [u8; 4] {
    let mut out = [b'0'; 4];
    let mut chars = s.chars().filter(|c| c.is_ascii_alphabetic());
    let Some(first) = chars.next() else {
        return out;
    };
    out[0] = first.to_ascii_uppercase() as u8;
    let map = |c: char| -> u8 {
        match c.to_ascii_uppercase() {
            'B' | 'F' | 'P' | 'V' => b'1',
            'C' | 'G' | 'J' | 'K' | 'Q' | 'S' | 'X' | 'Z' => b'2',
            'D' | 'T' => b'3',
            'L' => b'4',
            'M' | 'N' => b'5',
            'R' => b'6',
            _ => b'0',
        }
    };
    let mut prev = map(first);
    let mut idx = 1usize;
    for c in chars {
        let code = map(c);
        if code != b'0' && code != prev {
            out[idx] = code;
            idx += 1;
            if idx == 4 {
                break;
            }
        }
        prev = code;
    }
    out
}

fn soundex_equal(a: &str, b: &str) -> bool {
    soundex_code(a) == soundex_code(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handy_versions_compare_numerically() {
        assert!(version_at_least("0.10.0", HANDY_TARGET_VERSION));
        assert!(version_at_least("0.10.1", "0.10.0"));
        assert!(version_at_least("1.0.0", "0.10.0"));
        assert!(version_at_least("v0.10.0", "0.10.0"));
        assert!(version_at_least("0.10.0.0", "0.10.0"));
        assert!(
            !version_at_least("0.9.8", "0.10.0"),
            "0.9.8 is older than 0.10.0, not newer"
        );
        assert!(!version_at_least("0.9", "0.10.0"));
    }

    #[test]
    fn spoken_commands_are_english() {
        let (out, send) =
            normalize_voice_transcript("at gridcraft read column B send message", &[]);
        assert_eq!(out, "@gridcraft read column B");
        assert!(send);
        // only the English words trigger anything: other text stays exactly as spoken
        let (out, send) = normalize_voice_transcript("read column B and sum it", &[]);
        assert_eq!(out, "read column B and sum it");
        assert!(!send);
    }

    #[test]
    fn normalizes_spoken_gridcraft_command() {
        let (out, send) = normalize_voice_transcript("at Grid Kraft read column B.", &[]);
        assert_eq!(out, "@gridcraft read column B");
        assert!(!send);

        let (out2, send2) = normalize_voice_transcript("Gridcraft, read column B", &[]);
        assert_eq!(out2, "@gridcraft read column B");
        assert!(!send2);
    }

    #[test]
    fn normalizes_spoken_slash_and_team_commands() {
        let (out, send) = normalize_voice_transcript(
            "Switch to team mode and create a color grading specialist",
            &[],
        );
        assert_eq!(
            out,
            "Switch to team mode and create a color grading specialist"
        );
        assert!(!send);

        let (slash, _) = normalize_voice_transcript("slash docx quarterly report", &[]);
        assert_eq!(slash, "/docx quarterly report");
    }

    #[test]
    fn detects_explicit_trailing_voice_send_command() {
        let (out, send) =
            normalize_voice_transcript("Summarize the current document, send message.", &[]);
        assert_eq!(out, "Summarize the current document");
        assert!(send);
    }

    #[test]
    fn visualiser_produces_16_bounded_frequency_bars() {
        let mut vis = AudioVisualiser::new(SAMPLE_RATE, 512, VISUALIZER_BUCKETS, 80.0, 4000.0);
        let samples: Vec<f32> = (0..512)
            .map(|i| (2.0 * PI * 440.0 * i as f32 / SAMPLE_RATE as f32).sin() * 0.4)
            .collect();
        let bars = vis.feed(&samples);
        assert!(bars.is_some());
        if let Some(b) = bars {
            assert_eq!(b.len(), VISUALIZER_BUCKETS);
            assert!(b.iter().any(|&v| v > 0.05));
            assert!(b.iter().all(|&v| (0.0..=1.0).contains(&v)));
        }
    }

    #[test]
    fn wav_writer_creates_valid_riff_wave_header() {
        let path = std::env::temp_dir().join("workcraft-test-voice-header.wav");
        let samples: Vec<i16> = vec![0, 1200, -1200, 3200, -3200];
        assert!(write_wav_16k_mono(&path, &samples).is_ok());
        let bytes = std::fs::read(&path).unwrap_or_default();
        let _ = std::fs::remove_file(&path);
        assert_eq!(bytes.len(), 44 + samples.len() * 2);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(&bytes[36..40], b"data");
    }

    #[test]
    fn quiet_recordings_are_raised_and_padded_with_silence() {
        // A quiet tone (peak about 0.01): comes out near 0.85 and with half a second of silence on each side
        let quiet: Vec<i16> = (0..16_000)
            .map(|i| ((2.0 * PI * 220.0 * i as f32 / SAMPLE_RATE as f32).sin() * 330.0) as i16)
            .collect();
        let out = prepare_for_speech(&quiet);
        assert_eq!(out.len(), quiet.len() + SAMPLE_RATE as usize);
        assert!(out[..8_000].iter().all(|&s| s == 0));
        assert!(out[out.len() - 8_000..].iter().all(|&s| s == 0));
        assert!(peak_level(&out) > 0.15, "raised to {}", peak_level(&out));
        // A loud recording keeps its level
        let loud: Vec<i16> = quiet.iter().map(|&s| s.saturating_mul(80)).collect();
        assert!((peak_level(&prepare_for_speech(&loud)) - peak_level(&loud)).abs() < 0.02);
        // Silence stays silence and does not turn into noise
        assert_eq!(peak_level(&prepare_for_speech(&vec![0; 4_000])), 0.0);
    }
}
