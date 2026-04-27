//! Waybar status integration: icon theme.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::app::{Action, App};
use super::common::{self, FeedbackLevel, FormRowSpec};
use super::config_editor::{ConfigEditor, EditorError};

#[derive(Debug, Clone)]
pub struct WaybarState {
    pub icon_theme: String,
    pub feedback: Option<(FeedbackLevel, String)>,
    pub dirty_since_load: bool,
}

const THEMES: &[&str] = &[
    "emoji",
    "nerd-font",
    "material",
    "phosphor",
    "codicons",
    "omarchy",
    "minimal",
    "dots",
    "arrows",
    "text",
];

impl WaybarState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        Ok(Self {
            icon_theme: ed
                .get_string("status", "icon_theme")
                .unwrap_or_else(|| "emoji".to_string()),
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
        ed.set_string("status", "icon_theme", &self.icon_theme);
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
            *self = fresh;
            self.feedback = Some((FeedbackLevel::Ok, "Reverted".to_string()));
        }
    }
    fn cycle(&mut self, delta: i32) {
        let idx = THEMES
            .iter()
            .position(|t| *t == self.icon_theme)
            .map(|i| i as i32)
            .unwrap_or(0);
        self.icon_theme = THEMES[((idx + delta).rem_euclid(THEMES.len() as i32)) as usize].to_string();
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let state = match &app.waybar {
        Some(s) => s,
        None => {
            let block = Block::default().borders(Borders::ALL).title("Waybar");
            let inner = block.inner(area);
            f.render_widget(block, area);
            f.render_widget(Paragraph::new("Failed to load config.").wrap(Wrap { trim: true }), inner);
            return;
        }
    };

    let rows = vec![FormRowSpec::new(true, "Icon theme", &state.icon_theme)];

    let feedback_pair = state
        .feedback
        .as_ref()
        .map(|(lvl, msg)| (*lvl, msg.as_str()));

    common::render_form_with_guidance(
        f,
        area,
        "Waybar / Status",
        state.dirty_since_load,
        feedback_pair,
        &rows,
        guidance(),
    );
}

fn heading<'a>(text: &'a str) -> Line<'a> {
    Line::from(Span::styled(
        text,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn guidance<'a>() -> Vec<Line<'a>> {
    vec![
        heading("Icon theme"),
        Line::from(""),
        Line::from(
            "The glyph set `voxtype status --follow` emits to your status \
             bar. Match it to whatever your bar's font supports.",
        ),
        Line::from(""),
        Line::from(Span::styled(
            "Common picks:",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  • emoji — works everywhere, no special font needed."),
        Line::from("  • nerd-font — for users on a Nerd Font."),
        Line::from("  • phosphor — Phosphor icon font."),
        Line::from("  • omarchy — matches Omarchy's stock ricing."),
        Line::from("  • text — plain ASCII, no glyphs at all."),
        Line::from(""),
        Line::from(Span::styled(
            "Per-state icon overrides live under [status.icons] in \
             config.toml (e.g. icons.recording = \"●\"). Inline editing of \
             overrides is on the roadmap.",
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Run `voxtype setup waybar` for ready-to-paste Waybar config.",
            Style::default().fg(Color::Gray),
        )),
    ]
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.waybar.as_mut() {
        Some(s) => s,
        None => return Action::None,
    };
    match key.code {
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
