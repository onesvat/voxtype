//! Desktop notifications section.

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
pub struct NotificationsState {
    pub on_recording_start: bool,
    pub on_recording_stop: bool,
    pub on_transcription: bool,
    pub show_engine_icon: bool,
    pub field: Field,
    pub feedback: Option<(FeedbackLevel, String)>,
    pub dirty_since_load: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    OnStart,
    OnStop,
    OnTranscription,
    ShowEngineIcon,
}
impl Field {
    const ALL: &'static [Field] = &[
        Field::OnStart,
        Field::OnStop,
        Field::OnTranscription,
        Field::ShowEngineIcon,
    ];
}

const TABLE: &str = "output.notification";

impl NotificationsState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        Ok(Self {
            on_recording_start: ed.get_bool(TABLE, "on_recording_start").unwrap_or(false),
            on_recording_stop: ed.get_bool(TABLE, "on_recording_stop").unwrap_or(false),
            on_transcription: ed.get_bool(TABLE, "on_transcription").unwrap_or(true),
            show_engine_icon: ed.get_bool(TABLE, "show_engine_icon").unwrap_or(false),
            field: Field::OnStart,
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
        ed.set_bool(TABLE, "on_recording_start", self.on_recording_start);
        ed.set_bool(TABLE, "on_recording_stop", self.on_recording_stop);
        ed.set_bool(TABLE, "on_transcription", self.on_transcription);
        ed.set_bool(TABLE, "show_engine_icon", self.show_engine_icon);
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
    fn cycle(&mut self) {
        match self.field {
            Field::OnStart => self.on_recording_start = !self.on_recording_start,
            Field::OnStop => self.on_recording_stop = !self.on_recording_stop,
            Field::OnTranscription => self.on_transcription = !self.on_transcription,
            Field::ShowEngineIcon => self.show_engine_icon = !self.show_engine_icon,
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title("Notifications");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let state = match &app.notifications {
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
            Constraint::Length(6),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(inner);
    if let Some((lvl, msg)) = &state.feedback {
        common::render_feedback(f, chunks[0], *lvl, msg);
    }
    common::render_section_header(f, chunks[1], "Desktop Notifications", state.dirty_since_load);
    let rows = [
        (Field::OnStart, "On recording start", yesno(state.on_recording_start)),
        (Field::OnStop, "On recording stop", yesno(state.on_recording_stop)),
        (
            Field::OnTranscription,
            "Show transcribed text",
            yesno(state.on_transcription),
        ),
        (
            Field::ShowEngineIcon,
            "Engine icon in title",
            yesno(state.show_engine_icon),
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
            "  • Notifications use libnotify (notify-send under the hood). \
             They respect your DE's notification settings (mako, dunst, KDE, GNOME).",
        ),
        Line::from(
            "  • The transcription notification shows the final transcript text. \
             Useful for confirming what was typed when output went to the wrong window.",
        ),
    ];
    f.render_widget(Paragraph::new(help).wrap(Wrap { trim: true }), chunks[3]);
    common::render_bottom_hint(f, chunks[4], state.dirty_since_load);
}
fn yesno(b: bool) -> String {
    (if b { "yes" } else { "no" }).to_string()
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.notifications.as_mut() {
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
        KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l')
        | KeyCode::Char(' ') => {
            state.cycle();
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
