//! Engine section: tunables for the active transcription engine.
//!
//! Currently focuses on Whisper, the default engine. Non-Whisper engines
//! show a placeholder pointing the user at config.toml until each one gets
//! its own form.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::app::{Action, App};
use super::common::{self, FeedbackLevel as CommonFeedback, FormRowSpec};
use super::config_editor::{ConfigEditor, EditorError};

#[derive(Debug, Clone)]
pub struct EngineState {
    pub engine: String,
    pub whisper: WhisperFields,
    pub field: WField,
    pub feedback: Option<Feedback>,
    pub dirty_since_load: bool,
}

#[derive(Debug, Clone)]
pub struct WhisperFields {
    pub mode: String, // local / remote / cli
    pub language: String,
    pub translate: bool,
    pub threads: Option<i64>,
    pub initial_prompt: Option<String>,
    pub flash_attention: bool,
    pub on_demand_loading: bool,
    pub gpu_isolation: bool,
}

#[derive(Debug, Clone)]
pub struct Feedback {
    pub level: FeedbackLevel,
    pub message: String,
}
#[derive(Debug, Clone, Copy)]
pub enum FeedbackLevel {
    Ok,
    Err,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WField {
    Mode,
    Language,
    Translate,
    Threads,
    Prompt,
    FlashAttention,
    OnDemandLoading,
    GpuIsolation,
}

impl WField {
    const ALL: &'static [WField] = &[
        WField::Mode,
        WField::Language,
        WField::Translate,
        WField::Threads,
        WField::Prompt,
        WField::FlashAttention,
        WField::OnDemandLoading,
        WField::GpuIsolation,
    ];
}

const MODE_CHOICES: &[&str] = &["local", "remote", "cli"];
const LANG_CHOICES: &[&str] = &[
    "auto", "en", "fr", "de", "it", "es", "pt", "nl", "pl", "zh", "ja", "ko", "ru", "ar",
];

impl EngineState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        let engine = ed
            .get_string("", "engine")
            .unwrap_or_else(|| "whisper".to_string());
        let whisper = WhisperFields {
            mode: ed
                .get_string("whisper", "mode")
                .unwrap_or_else(|| "local".to_string()),
            language: ed
                .get_string("whisper", "language")
                .unwrap_or_else(|| "auto".to_string()),
            translate: ed.get_bool("whisper", "translate").unwrap_or(false),
            threads: ed.get_int("whisper", "threads"),
            initial_prompt: ed.get_string("whisper", "initial_prompt"),
            flash_attention: ed.get_bool("whisper", "flash_attention").unwrap_or(false),
            on_demand_loading: ed.get_bool("whisper", "on_demand_loading").unwrap_or(false),
            gpu_isolation: ed.get_bool("whisper", "gpu_isolation").unwrap_or(false),
        };
        Ok(Self {
            engine,
            whisper,
            field: WField::Mode,
            feedback: None,
            dirty_since_load: false,
        })
    }

    pub fn save(&mut self) -> Action {
        let mut ed = match ConfigEditor::load() {
            Ok(e) => e,
            Err(e) => {
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Err,
                    message: format!("load: {}", e),
                });
                return Action::None;
            }
        };

        let w = &self.whisper;
        ed.set_string("whisper", "mode", &w.mode);
        ed.set_string("whisper", "language", &w.language);
        ed.set_bool("whisper", "translate", w.translate);
        match w.threads {
            Some(n) => ed.set_int("whisper", "threads", n),
            None => ed.unset("whisper", "threads"),
        }
        match &w.initial_prompt {
            Some(p) if !p.is_empty() => ed.set_string("whisper", "initial_prompt", p),
            _ => ed.unset("whisper", "initial_prompt"),
        }
        ed.set_bool("whisper", "flash_attention", w.flash_attention);
        ed.set_bool("whisper", "on_demand_loading", w.on_demand_loading);
        ed.set_bool("whisper", "gpu_isolation", w.gpu_isolation);

        match ed.save() {
            Ok(()) => {
                self.dirty_since_load = false;
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Ok,
                    message: format!("Saved to {}", ed.path().display()),
                });
            }
            Err(e) => {
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Err,
                    message: format!("save: {}", e),
                });
            }
        }
        Action::None
    }

    pub fn reset(&mut self) {
        match Self::load() {
            Ok(fresh) => {
                let field = self.field;
                *self = fresh;
                self.field = field;
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Ok,
                    message: "Reverted unsaved changes".to_string(),
                });
            }
            Err(e) => {
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Err,
                    message: format!("reload: {}", e),
                });
            }
        }
    }

    fn move_field(&mut self, delta: i32) {
        let len = WField::ALL.len() as i32;
        let cur = WField::ALL.iter().position(|f| *f == self.field).unwrap_or(0) as i32;
        let new = (cur + delta).rem_euclid(len);
        self.field = WField::ALL[new as usize];
    }

    fn cycle(&mut self, delta: i32) {
        let w = &mut self.whisper;
        match self.field {
            WField::Mode => {
                let idx = MODE_CHOICES
                    .iter()
                    .position(|c| *c == w.mode)
                    .map(|i| i as i32)
                    .unwrap_or(0);
                let n = (idx + delta).rem_euclid(MODE_CHOICES.len() as i32);
                w.mode = MODE_CHOICES[n as usize].to_string();
            }
            WField::Language => {
                let idx = LANG_CHOICES
                    .iter()
                    .position(|c| *c == w.language)
                    .map(|i| i as i32)
                    .unwrap_or(0);
                let n = (idx + delta).rem_euclid(LANG_CHOICES.len() as i32);
                w.language = LANG_CHOICES[n as usize].to_string();
            }
            WField::Translate => w.translate = !w.translate,
            WField::Threads => {
                let cur = w.threads.unwrap_or(0);
                let next = cur + delta as i64;
                w.threads = if next <= 0 { None } else { Some(next.min(64)) };
            }
            WField::Prompt => {
                // Toggle between (none) and a sample prompt; in-line editing
                // arrives in a later PR.
                if w.initial_prompt.is_some() {
                    w.initial_prompt = None;
                } else {
                    w.initial_prompt = Some(
                        "Transcribe with proper capitalization and punctuation.".to_string(),
                    );
                }
            }
            WField::FlashAttention => w.flash_attention = !w.flash_attention,
            WField::OnDemandLoading => w.on_demand_loading = !w.on_demand_loading,
            WField::GpuIsolation => w.gpu_isolation = !w.gpu_isolation,
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let state = match &app.engine {
        Some(s) => s,
        None => {
            let block = Block::default().borders(Borders::ALL).title("Engine");
            let inner = block.inner(area);
            f.render_widget(block, area);
            f.render_widget(
                Paragraph::new("Failed to load config; check ~/.config/voxtype/config.toml.")
                    .wrap(Wrap { trim: true }),
                inner,
            );
            return;
        }
    };

    if state.engine != "whisper" {
        let block = Block::default().borders(Borders::ALL).title("Engine");
        let inner = block.inner(area);
        f.render_widget(block, area);
        let lines = vec![
            Line::from(Span::styled(
                format!("Active engine: {}", state.engine),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(format!(
                "Per-engine tuning for {} is not yet exposed in the TUI.",
                state.engine
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Edit ~/.config/voxtype/config.toml directly for now.",
                Style::default().fg(Color::Gray),
            )),
        ];
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
        return;
    }

    let w = &state.whisper;
    let rows = vec![
        FormRowSpec::new(state.field == WField::Mode, "Execution mode", &w.mode),
        FormRowSpec::new(state.field == WField::Language, "Language", &w.language),
        FormRowSpec::new(
            state.field == WField::Translate,
            "Translate to English",
            yesno(w.translate),
        ),
        FormRowSpec::new(
            state.field == WField::Threads,
            "Threads",
            w.threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "auto".to_string()),
        ),
        FormRowSpec::new(
            state.field == WField::Prompt,
            "Initial prompt",
            w.initial_prompt
                .as_deref()
                .map(|s| {
                    if s.len() > 30 {
                        format!("{}…", &s[..30])
                    } else {
                        s.to_string()
                    }
                })
                .unwrap_or_else(|| "(none)".to_string()),
        ),
        FormRowSpec::new(
            state.field == WField::FlashAttention,
            "Flash attention",
            yesno(w.flash_attention),
        ),
        FormRowSpec::new(
            state.field == WField::OnDemandLoading,
            "On-demand model loading",
            yesno(w.on_demand_loading),
        ),
        FormRowSpec::new(
            state.field == WField::GpuIsolation,
            "GPU isolation (subprocess)",
            yesno(w.gpu_isolation),
        ),
    ];

    let feedback_pair = state.feedback.as_ref().map(|fb| {
        (
            match fb.level {
                FeedbackLevel::Ok => CommonFeedback::Ok,
                FeedbackLevel::Err => CommonFeedback::Err,
            },
            fb.message.as_str(),
        )
    });

    common::render_form_with_guidance(
        f,
        area,
        "Whisper engine",
        state.dirty_since_load,
        feedback_pair,
        &rows,
        guidance_for_field(state),
    );
}

fn yesno(b: bool) -> String {
    (if b { "yes" } else { "no" }).to_string()
}

fn heading<'a>(text: &'a str) -> Line<'a> {
    Line::from(Span::styled(
        text,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn guidance_for_field(state: &EngineState) -> Vec<Line<'_>> {
    match state.field {
        WField::Mode => vec![
            heading("Execution mode"),
            Line::from(""),
            Line::from(Span::styled(
                "local: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "Transcribe in-process using whisper-rs. Default; no network.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "remote: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "Send audio to an OpenAI-compatible Whisper endpoint. Set \
                 [whisper] remote_endpoint and remote_api_key first.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "cli: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "Shell out to a `whisper` CLI. Useful for testing custom \
                 builds without rebuilding voxtype.",
            ),
        ],
        WField::Language => vec![
            heading("Language"),
            Line::from(""),
            Line::from(
                "BCP-47-ish language code or `auto`. With `auto`, Whisper \
                 detects the language per recording (slightly slower).",
            ),
            Line::from(""),
            Line::from(
                "Lock to a specific code (en, fr, de, ja, …) when you only \
                 ever dictate in one language — Whisper skips detection and \
                 won't accidentally switch mid-sentence.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Multi-language allowlists (e.g. \"en,fr,de\") can be set in \
                 config.toml as an array.",
                Style::default().fg(Color::Gray),
            )),
        ],
        WField::Translate => vec![
            heading("Translate to English"),
            Line::from(""),
            Line::from(
                "When on, Whisper translates non-English speech to English \
                 in the transcript. Off by default.",
            ),
            Line::from(""),
            Line::from(
                "Useful for capturing meetings where someone speaks another \
                 language and you want a single English transcript.",
            ),
        ],
        WField::Threads => vec![
            heading("CPU threads"),
            Line::from(""),
            Line::from(
                "Number of threads Whisper uses for inference. `auto` lets \
                 voxtype pick (typically physical-core count).",
            ),
            Line::from(""),
            Line::from(
                "Lower it if you want voxtype to leave headroom for other \
                 tasks. Bump it to physical-core count if you're CPU-only \
                 and want max throughput.",
            ),
        ],
        WField::Prompt => vec![
            heading("Initial prompt"),
            Line::from(""),
            Line::from(
                "Hints Whisper about terminology, capitalization, or \
                 formatting. Whisper biases output toward what the prompt \
                 establishes.",
            ),
            Line::from(""),
            Line::from(
                "Useful for proper nouns and technical terms (\"Tern\", \
                 \"Voxtype\", \"Hyprland\") so Whisper doesn't transcribe \
                 them as common words.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "TUI cycles (none) / sample. Edit the body in config.toml \
                 directly for a custom prompt.",
                Style::default().fg(Color::Gray),
            )),
        ],
        WField::FlashAttention => vec![
            heading("Flash attention"),
            Line::from(""),
            Line::from(
                "GPU-only optimization that reduces memory bandwidth in the \
                 attention layers. Faster on Vulkan/CUDA, especially on \
                 large-v3.",
            ),
            Line::from(""),
            Line::from(
                "No effect on CPU runs. Crashes on a few older driver \
                 combinations — leave it off if Whisper hangs.",
            ),
        ],
        WField::OnDemandLoading => vec![
            heading("On-demand model loading"),
            Line::from(""),
            Line::from(
                "Loads the model only when you start recording, then unloads. \
                 Frees ~1-2 GB of RAM at idle.",
            ),
            Line::from(""),
            Line::from(
                "Adds a one-shot delay on the first key press of each \
                 dictation. Worth it if you dictate sporadically; not worth \
                 it if you're constantly hitting the PTT key.",
            ),
        ],
        WField::GpuIsolation => vec![
            heading("GPU isolation"),
            Line::from(""),
            Line::from(
                "Each transcription runs in a short-lived subprocess that \
                 exits afterward, releasing all VRAM.",
            ),
            Line::from(""),
            Line::from(
                "Useful on hybrid-graphics laptops to let the discrete GPU \
                 power down between dictations. Adds ~100-300ms startup per \
                 transcription.",
            ),
        ],
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.engine.as_mut() {
        Some(s) => s,
        None => return Action::None,
    };
    if state.engine != "whisper" {
        return Action::None;
    }
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.move_field(-1);
            Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.move_field(1);
            Action::None
        }
        KeyCode::Left | KeyCode::Char('h') => {
            state.cycle(-1);
            Action::None
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => {
            state.cycle(1);
            Action::None
        }
        KeyCode::Char('s') => state.save(),
        KeyCode::Char('r') => {
            state.reset();
            Action::None
        }
        _ => Action::None,
    }
}
