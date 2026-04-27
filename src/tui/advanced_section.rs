//! Advanced settings: less-common knobs the TUI surfaces in one place.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::app::{Action, App};
use super::common::{self, FeedbackLevel};
use super::config_editor::{ConfigEditor, EditorError};

#[derive(Debug, Clone)]
pub struct AdvancedState {
    pub gpu_isolation: bool,
    pub on_demand_loading: bool,
    pub flash_attention: bool,
    pub eager_processing: bool,
    pub gpu_device: Option<i64>,
    pub field: Field,
    pub feedback: Option<(FeedbackLevel, String)>,
    pub dirty_since_load: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    GpuIsolation,
    OnDemand,
    FlashAttention,
    Eager,
    GpuDevice,
}
impl Field {
    const ALL: &'static [Field] = &[
        Field::GpuIsolation,
        Field::OnDemand,
        Field::FlashAttention,
        Field::Eager,
        Field::GpuDevice,
    ];
}

impl AdvancedState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        Ok(Self {
            gpu_isolation: ed.get_bool("whisper", "gpu_isolation").unwrap_or(false),
            on_demand_loading: ed
                .get_bool("whisper", "on_demand_loading")
                .unwrap_or(false),
            flash_attention: ed.get_bool("whisper", "flash_attention").unwrap_or(false),
            eager_processing: ed
                .get_bool("whisper", "eager_processing")
                .unwrap_or(false),
            gpu_device: ed.get_int("whisper", "gpu_device"),
            field: Field::GpuIsolation,
            feedback: None,
            dirty_since_load: false,
        })
    }
    pub fn save(&mut self) -> Action {
        let mut ed = match ConfigEditor::load() {
            Ok(e) => e,
            Err(e) => {
                self.feedback = Some((FeedbackLevel::Err, format!("load: {}", e)));
                return Action::None;
            }
        };
        ed.set_bool("whisper", "gpu_isolation", self.gpu_isolation);
        ed.set_bool("whisper", "on_demand_loading", self.on_demand_loading);
        ed.set_bool("whisper", "flash_attention", self.flash_attention);
        ed.set_bool("whisper", "eager_processing", self.eager_processing);
        match self.gpu_device {
            Some(n) if n >= 0 => ed.set_int("whisper", "gpu_device", n),
            _ => ed.unset("whisper", "gpu_device"),
        }
        match ed.save() {
            Ok(()) => {
                self.dirty_since_load = false;
                self.feedback = Some((
                    FeedbackLevel::Ok,
                    format!("Saved to {}", ed.path().display()),
                ));
            }
            Err(e) => self.feedback = Some((FeedbackLevel::Err, format!("save: {}", e))),
        }
        Action::None
    }
    pub fn reset(&mut self) {
        if let Ok(fresh) = Self::load() {
            let field = self.field;
            *self = fresh;
            self.field = field;
            self.feedback = Some((FeedbackLevel::Ok, "Reverted".to_string()));
        }
    }
    fn move_field(&mut self, delta: i32) {
        let len = Field::ALL.len() as i32;
        let cur = Field::ALL.iter().position(|f| *f == self.field).unwrap_or(0) as i32;
        self.field = Field::ALL[((cur + delta).rem_euclid(len)) as usize];
    }
    fn cycle(&mut self, delta: i32) {
        match self.field {
            Field::GpuIsolation => self.gpu_isolation = !self.gpu_isolation,
            Field::OnDemand => self.on_demand_loading = !self.on_demand_loading,
            Field::FlashAttention => self.flash_attention = !self.flash_attention,
            Field::Eager => self.eager_processing = !self.eager_processing,
            Field::GpuDevice => {
                let cur = self.gpu_device.unwrap_or(-1);
                let next = cur + delta as i64;
                self.gpu_device = if next < 0 { None } else { Some(next.min(7)) };
            }
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title("Advanced");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let state = match &app.advanced {
        Some(s) => s,
        None => {
            f.render_widget(Paragraph::new("Failed to load config."), inner);
            return;
        }
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if state.feedback.is_some() { 2 } else { 0 }),
            Constraint::Length(2),
            Constraint::Length(7),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(inner);
    if let Some((lvl, msg)) = &state.feedback {
        common::render_feedback(f, chunks[0], *lvl, msg);
    }
    common::render_section_header(f, chunks[1], "Advanced", state.dirty_since_load);

    let rows = [
        (
            Field::GpuIsolation,
            "GPU isolation (subprocess)",
            yesno(state.gpu_isolation),
        ),
        (
            Field::OnDemand,
            "On-demand model loading",
            yesno(state.on_demand_loading),
        ),
        (
            Field::FlashAttention,
            "Flash attention",
            yesno(state.flash_attention),
        ),
        (
            Field::Eager,
            "Eager input processing",
            yesno(state.eager_processing),
        ),
        (
            Field::GpuDevice,
            "GPU device index",
            state
                .gpu_device
                .map(|n| n.to_string())
                .unwrap_or_else(|| "auto".to_string()),
        ),
    ];
    let lines: Vec<Line> = rows
        .iter()
        .map(|(f, l, v)| common::form_row(*f == state.field, l, v))
        .collect();
    f.render_widget(Paragraph::new(lines), chunks[2]);

    let help = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Tips",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(
            "  • These settings are also surfaced under Engine; they're \
             collected here for users who want one screen with all the \
             advanced knobs.",
        ),
        Line::from(
            "  • GPU device index targets the discrete GPU on hybrid systems. \
             Leave at auto unless transcription is slower than you'd expect \
             (the integrated GPU often gets picked by accident).",
        ),
        Line::from(
            "  • Eager processing transcribes audio while you're still \
             recording. Reduces latency at the cost of slightly more CPU.",
        ),
    ];
    f.render_widget(Paragraph::new(help).wrap(Wrap { trim: true }), chunks[3]);
    common::render_bottom_hint(f, chunks[4], state.dirty_since_load);
}
fn yesno(b: bool) -> String {
    (if b { "yes" } else { "no" }).to_string()
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.advanced.as_mut() {
        Some(s) => s,
        None => return Action::None,
    };
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
