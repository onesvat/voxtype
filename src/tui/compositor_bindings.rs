//! Detect `voxtype record` bindings declared in compositor configs.
//!
//! Useful when the user has the evdev listener disabled and is relying on
//! compositor-level keybindings to call voxtype. The Hotkey section's About
//! pane shows what bindings are wired up so users can verify their config
//! without leaving the TUI.
//!
//! Supports Hyprland, Sway, and Niri. Their config formats are parsed with
//! plain regex — we don't pull in a real KDL/Hyprland parser for what is
//! ultimately advisory output.
//!
//! # Compositors not yet covered
//!
//! - River: shell-script-based init; any function could call voxtype, so a
//!   simple grep would mostly produce false positives.
//! - GNOME / KDE: bindings live in dconf / kglobalshortcuts databases. Worth
//!   a follow-up but a different shape of detection.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Binding {
    pub compositor: &'static str,
    /// Human-readable key combo as written in the config (e.g. "SUPER+HOME").
    pub keys: String,
    /// Voxtype subcommand being bound (`record start`, `record cancel`,
    /// `meeting start`, `meeting stop`, …).
    pub action: String,
    /// Path to the file the binding came from, for reporting.
    pub source: PathBuf,
}

/// Format hint for a [`Suggestion`] — picked from the compositor that owns
/// the most existing bindings, falling back to Hyprland.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compositor {
    Hyprland,
    Sway,
    Niri,
}

impl Compositor {
    pub fn name(self) -> &'static str {
        match self {
            Compositor::Hyprland => "Hyprland",
            Compositor::Sway => "Sway",
            Compositor::Niri => "Niri",
        }
    }
}

/// One missing binding the user might want to add.
#[derive(Debug, Clone)]
pub struct Suggestion {
    pub label: String,
    pub purpose: &'static str,
    pub config_lines: Vec<String>,
}

/// Pick the most likely compositor based on the bindings already detected,
/// or default to Hyprland.
pub fn dominant_compositor(detected: &[Binding]) -> Compositor {
    let mut hypr = 0;
    let mut sway = 0;
    let mut niri = 0;
    for b in detected {
        match b.compositor {
            "Hyprland" => hypr += 1,
            "Sway" => sway += 1,
            "Niri" => niri += 1,
            _ => {}
        }
    }
    if niri > hypr && niri > sway {
        Compositor::Niri
    } else if sway > hypr {
        Compositor::Sway
    } else {
        Compositor::Hyprland
    }
}

/// Look at the actions the user has already bound and suggest config snippets
/// for likely-missing ones (cancel, toggle, meeting start/stop).
pub fn suggest_missing(detected: &[Binding]) -> Vec<Suggestion> {
    let comp = dominant_compositor(detected);
    let actions: std::collections::HashSet<&str> =
        detected.iter().map(|b| b.action.as_str()).collect();

    let mut suggestions = Vec::new();

    let has_start = actions.contains("record start");
    let has_stop = actions.contains("record stop");
    let has_toggle = actions.contains("record toggle");
    let has_cancel = actions.contains("record cancel");
    let has_meeting_start = actions.contains("meeting start");
    let has_meeting_stop = actions.contains("meeting stop");

    // PTT pair: if one half is bound, suggest the other.
    if has_start && !has_stop {
        suggestions.push(Suggestion {
            label: "Stop (release of your PTT key)".to_string(),
            purpose: "Without a stop binding, hold-to-record never finishes — \
                      voxtype will run until max_duration_secs hits.",
            config_lines: render_lines(comp, "record stop", BindShape::PttRelease),
        });
    }
    if has_stop && !has_start {
        suggestions.push(Suggestion {
            label: "Start (press of your PTT key)".to_string(),
            purpose: "You have a stop binding but no start — recording can't \
                      begin from your compositor.",
            config_lines: render_lines(comp, "record start", BindShape::PttPress),
        });
    }

    // No PTT pair at all and no toggle: suggest both flows.
    if !has_start && !has_stop && !has_toggle {
        suggestions.push(Suggestion {
            label: "Push-to-talk (start + stop pair)".to_string(),
            purpose: "Hold the key while you speak; release to transcribe.",
            config_lines: render_lines(comp, "record start/stop", BindShape::PttPair),
        });
        suggestions.push(Suggestion {
            label: "Toggle (single-key alternative)".to_string(),
            purpose: "Press once to start, again to stop. Better for long \
                      dictations.",
            config_lines: render_lines(comp, "record toggle", BindShape::Single),
        });
    } else if !has_toggle && (has_start || has_stop) {
        suggestions.push(Suggestion {
            label: "Toggle (alternative to PTT)".to_string(),
            purpose: "A single-key toggle bound to a different key gives you \
                      a long-dictation flow without competing with the PTT \
                      key.",
            config_lines: render_lines(comp, "record toggle", BindShape::Single),
        });
    }

    if !has_cancel {
        suggestions.push(Suggestion {
            label: "Cancel (abort in-progress recording)".to_string(),
            purpose: "Discards audio without transcribing — useful when you \
                      trip the PTT key by accident or the wrong window has \
                      focus.",
            config_lines: render_lines(comp, "record cancel", BindShape::Single),
        });
    }

    if !has_meeting_start && !has_meeting_stop {
        suggestions.push(Suggestion {
            label: "Meeting mode (start + stop)".to_string(),
            purpose: "Long-form recording with chunked transcription. Bind \
                      separate keys so meeting capture doesn't collide with \
                      regular dictation.",
            config_lines: render_lines(comp, "meeting start/stop", BindShape::MeetingPair),
        });
    } else if has_meeting_start && !has_meeting_stop {
        suggestions.push(Suggestion {
            label: "Meeting stop".to_string(),
            purpose: "You have a meeting-start binding but no stop. Without \
                      it the meeting only ends when you `voxtype meeting stop` \
                      from the CLI.",
            config_lines: render_lines(comp, "meeting stop", BindShape::Single),
        });
    } else if has_meeting_stop && !has_meeting_start {
        suggestions.push(Suggestion {
            label: "Meeting start".to_string(),
            purpose: "You bound meeting stop but not start.",
            config_lines: render_lines(comp, "meeting start", BindShape::Single),
        });
    }

    suggestions
}

#[derive(Debug, Clone, Copy)]
enum BindShape {
    /// Press-only: action fires when the key goes down.
    Single,
    /// Press-only half of a PTT pair (e.g. `record start`).
    PttPress,
    /// Release-only half of a PTT pair (e.g. `record stop`).
    PttRelease,
    /// Both halves of a PTT pair, rendered together.
    PttPair,
    /// Two separate single-press bindings for meeting start + stop.
    MeetingPair,
}

fn render_lines(comp: Compositor, action: &str, shape: BindShape) -> Vec<String> {
    match (comp, shape) {
        (Compositor::Hyprland, BindShape::Single) => vec![format!(
            "bind = SUPER, SPACE, voxtype {}, exec, voxtype {}",
            action.replace(' ', " "),
            action
        )],
        (Compositor::Hyprland, BindShape::PttPress) => vec![format!(
            "bindd  = , F13, Voxtype PTT (start), exec, voxtype record start"
        )],
        (Compositor::Hyprland, BindShape::PttRelease) => vec![format!(
            "bindrd = , F13, Voxtype PTT (stop),  exec, voxtype record stop"
        )],
        (Compositor::Hyprland, BindShape::PttPair) => vec![
            "bindd  = , F13, Voxtype PTT (start), exec, voxtype record start"
                .to_string(),
            "bindrd = , F13, Voxtype PTT (stop),  exec, voxtype record stop"
                .to_string(),
        ],
        (Compositor::Hyprland, BindShape::MeetingPair) => vec![
            "bind = SUPER, M,        Meeting start, exec, voxtype meeting start"
                .to_string(),
            "bind = SUPER SHIFT, M,  Meeting stop,  exec, voxtype meeting stop"
                .to_string(),
        ],

        (Compositor::Sway, BindShape::Single) => vec![format!(
            "bindsym Mod4+space exec voxtype {}",
            action
        )],
        (Compositor::Sway, BindShape::PttPress) => {
            vec!["bindsym F13 exec voxtype record start".to_string()]
        }
        (Compositor::Sway, BindShape::PttRelease) => vec![
            "bindsym --release F13 exec voxtype record stop".to_string(),
        ],
        (Compositor::Sway, BindShape::PttPair) => vec![
            "bindsym F13 exec voxtype record start".to_string(),
            "bindsym --release F13 exec voxtype record stop".to_string(),
        ],
        (Compositor::Sway, BindShape::MeetingPair) => vec![
            "bindsym Mod4+m exec voxtype meeting start".to_string(),
            "bindsym Mod4+Shift+m exec voxtype meeting stop".to_string(),
        ],

        (Compositor::Niri, BindShape::Single) => vec![format!(
            "Mod+Space {{ spawn \"voxtype\" {}; }}",
            quote_words(action)
        )],
        (Compositor::Niri, BindShape::PttPress) => vec![
            "F13 { spawn \"voxtype\" \"record\" \"start\"; }".to_string(),
        ],
        (Compositor::Niri, BindShape::PttRelease) => vec![
            "// Niri does not natively bind on key release; use toggle mode \
             or fall back to a press-only start binding."
                .to_string(),
        ],
        (Compositor::Niri, BindShape::PttPair) => vec![
            "F13 { spawn \"voxtype\" \"record\" \"toggle\"; }".to_string(),
            "// (Niri lacks key-release binds; use toggle in place of PTT.)"
                .to_string(),
        ],
        (Compositor::Niri, BindShape::MeetingPair) => vec![
            "Mod+M { spawn \"voxtype\" \"meeting\" \"start\"; }".to_string(),
            "Mod+Shift+M { spawn \"voxtype\" \"meeting\" \"stop\"; }".to_string(),
        ],
    }
}

fn quote_words(s: &str) -> String {
    s.split_whitespace()
        .map(|w| format!("\"{}\"", w))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn detect() -> Vec<Binding> {
    let mut out = Vec::new();
    let home = match std::env::var("HOME") {
        Ok(h) => PathBuf::from(h),
        Err(_) => return out,
    };

    detect_hyprland(&home, &mut out);
    detect_sway(&home, &mut out);
    detect_niri(&home, &mut out);
    out
}

fn detect_hyprland(home: &Path, out: &mut Vec<Binding>) {
    let dir = home.join(".config/hypr");
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("conf") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            if let Some(b) = parse_hyprland_line(line, &path) {
                out.push(b);
            }
        }
    }
}

/// Hyprland `bindd? = MODS, KEY, NAME, exec, voxtype SUBCMD ACTION` lines
/// (and `bindrd?`, `bindl`, `bindel`, `binde`, `bindle`, …).
fn parse_hyprland_line(line: &str, source: &Path) -> Option<Binding> {
    let trimmed = line.trim();
    if trimmed.starts_with('#') {
        return None;
    }
    let (lhs, rhs) = trimmed.split_once('=')?;
    let lhs = lhs.trim();
    if !lhs.starts_with("bind") {
        return None;
    }
    if !rhs.contains("voxtype") {
        return None;
    }
    // Split by commas; Hyprland tolerates whitespace.
    let parts: Vec<&str> = rhs.split(',').map(str::trim).collect();
    if parts.len() < 4 {
        return None;
    }
    let mods = parts[0];
    let key = parts[1];
    let cmd = parts.last().copied().unwrap_or("");
    let action = action_from_command(cmd)?;
    let keys = if mods.is_empty() {
        key.to_string()
    } else {
        format!("{}+{}", mods, key)
    };
    Some(Binding {
        compositor: "Hyprland",
        keys,
        action,
        source: source.to_path_buf(),
    })
}

fn detect_sway(home: &Path, out: &mut Vec<Binding>) {
    let main = home.join(".config/sway/config");
    if main.exists() {
        if let Ok(text) = fs::read_to_string(&main) {
            for line in text.lines() {
                if let Some(b) = parse_sway_line(line, &main) {
                    out.push(b);
                }
            }
        }
    }
    let conf_d = home.join(".config/sway/config.d");
    if let Ok(entries) = fs::read_dir(&conf_d) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for line in text.lines() {
                if let Some(b) = parse_sway_line(line, &path) {
                    out.push(b);
                }
            }
        }
    }
}

/// Sway `bindsym MOD+KEY exec voxtype SUBCMD ACTION` (or `bindcode`).
fn parse_sway_line(line: &str, source: &Path) -> Option<Binding> {
    let trimmed = line.trim();
    if trimmed.starts_with('#') {
        return None;
    }
    if !trimmed.contains("voxtype") {
        return None;
    }
    let mut parts = trimmed.split_whitespace();
    let head = parts.next()?;
    if head != "bindsym" && head != "bindcode" {
        return None;
    }
    // Skip optional `--release` and similar flags.
    let mut rest: Vec<&str> = parts.collect();
    while let Some(first) = rest.first() {
        if first.starts_with("--") {
            rest.remove(0);
        } else {
            break;
        }
    }
    let keys = rest.first()?.to_string();
    // Find `exec` and look at what comes after `voxtype record`.
    let cmd_start = rest.iter().position(|w| *w == "exec")? + 1;
    let cmd = rest[cmd_start..].join(" ");
    let action = action_from_command(&cmd)?;
    Some(Binding {
        compositor: "Sway",
        keys,
        action,
        source: source.to_path_buf(),
    })
}

fn detect_niri(home: &Path, out: &mut Vec<Binding>) {
    let path = home.join(".config/niri/config.kdl");
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    for line in text.lines() {
        if let Some(b) = parse_niri_line(line, &path) {
            out.push(b);
        }
    }
}

/// Niri's KDL `binds { Mod+Key { spawn "voxtype" "record" "ACTION"; } }`.
/// We only handle single-line bindings, which is the common case.
fn parse_niri_line(line: &str, source: &Path) -> Option<Binding> {
    let trimmed = line.trim();
    if trimmed.starts_with("//") {
        return None;
    }
    if !trimmed.contains("voxtype") || !trimmed.contains("spawn") {
        return None;
    }
    // Form: `Mod+Key { spawn "voxtype" "record" "ACTION"; }`.
    let (keys, rest) = trimmed.split_once('{')?;
    let keys = keys.trim();
    if keys.is_empty() {
        return None;
    }
    // Pull the quoted args after `spawn`.
    let spawn_idx = rest.find("spawn")?;
    let args_part = &rest[spawn_idx + "spawn".len()..];
    let mut quoted: Vec<String> = Vec::new();
    let mut chars = args_part.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            let mut buf = String::new();
            for c in chars.by_ref() {
                if c == '"' {
                    break;
                }
                buf.push(c);
            }
            quoted.push(buf);
        }
    }
    if quoted.first().map(|s| s.as_str()) != Some("voxtype") {
        return None;
    }
    let subcmd = quoted.get(1)?.clone();
    let leaf = quoted.get(2)?.clone();
    let action = format!("{} {}", subcmd, leaf);
    if !is_known_action(&action) {
        return None;
    }
    Some(Binding {
        compositor: "Niri",
        keys: keys.to_string(),
        action,
        source: source.to_path_buf(),
    })
}

fn action_from_command(cmd: &str) -> Option<String> {
    // Look for `voxtype <subcmd> <leaf>` in the command line.
    let lc = cmd.to_lowercase();
    let idx = lc.find("voxtype")?;
    let after = &cmd[idx + "voxtype".len()..];
    let mut iter = after.split_whitespace();
    let subcmd = iter
        .next()?
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string();
    let leaf = iter
        .next()?
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string();
    let action = format!("{} {}", subcmd, leaf);
    if is_known_action(&action) {
        Some(action)
    } else {
        None
    }
}

fn is_known_action(action: &str) -> bool {
    matches!(
        action,
        "record start"
            | "record stop"
            | "record toggle"
            | "record cancel"
            | "meeting start"
            | "meeting stop"
            | "meeting pause"
            | "meeting resume"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn dummy_path() -> &'static Path {
        Path::new("/tmp/dummy.conf")
    }

    #[test]
    fn parses_hyprland_bindd() {
        let line = "bindd  = , HOME, Voxtype PTT (start), exec, voxtype record start";
        let b = parse_hyprland_line(line, dummy_path()).unwrap();
        assert_eq!(b.compositor, "Hyprland");
        assert_eq!(b.keys, "HOME");
        assert_eq!(b.action, "record start");
    }

    #[test]
    fn parses_hyprland_bindrd_with_mod() {
        let line = "bindrd = SUPER, F13, Stop, exec, voxtype record stop";
        let b = parse_hyprland_line(line, dummy_path()).unwrap();
        assert_eq!(b.keys, "SUPER+F13");
        assert_eq!(b.action, "record stop");
    }

    #[test]
    fn parses_hyprland_meeting_start() {
        let line = "bind = SUPER, M, Meeting start, exec, voxtype meeting start";
        let b = parse_hyprland_line(line, dummy_path()).unwrap();
        assert_eq!(b.action, "meeting start");
    }

    #[test]
    fn skips_hyprland_comments_and_unrelated() {
        assert!(parse_hyprland_line("# bind = , HOME, ..., exec, voxtype record start", dummy_path()).is_none());
        assert!(parse_hyprland_line("bind = , HOME, ..., exec, alacritty", dummy_path()).is_none());
    }

    #[test]
    fn parses_sway_bindsym() {
        let line = "bindsym Mod4+Home exec voxtype record toggle";
        let b = parse_sway_line(line, dummy_path()).unwrap();
        assert_eq!(b.compositor, "Sway");
        assert_eq!(b.keys, "Mod4+Home");
        assert_eq!(b.action, "record toggle");
    }

    #[test]
    fn parses_sway_with_release_flag() {
        let line = "bindsym --release Mod4+Home exec voxtype record stop";
        let b = parse_sway_line(line, dummy_path()).unwrap();
        assert_eq!(b.keys, "Mod4+Home");
        assert_eq!(b.action, "record stop");
    }

    #[test]
    fn parses_niri_spawn() {
        let line = r#"    Mod+Home { spawn "voxtype" "record" "start"; }"#;
        let b = parse_niri_line(line, dummy_path()).unwrap();
        assert_eq!(b.compositor, "Niri");
        assert_eq!(b.keys, "Mod+Home");
        assert_eq!(b.action, "record start");
    }

    #[test]
    fn parses_niri_meeting() {
        let line = r#"Mod+M { spawn "voxtype" "meeting" "start"; }"#;
        let b = parse_niri_line(line, dummy_path()).unwrap();
        assert_eq!(b.action, "meeting start");
    }

    #[test]
    fn suggests_cancel_when_only_ptt_bound() {
        let detected = vec![Binding {
            compositor: "Hyprland",
            keys: "HOME".into(),
            action: "record start".into(),
            source: PathBuf::from("/dev/null"),
        }, Binding {
            compositor: "Hyprland",
            keys: "HOME".into(),
            action: "record stop".into(),
            source: PathBuf::from("/dev/null"),
        }];
        let labels: Vec<_> = suggest_missing(&detected)
            .iter()
            .map(|s| s.label.clone())
            .collect();
        assert!(labels.iter().any(|l| l.contains("Cancel")));
        assert!(labels.iter().any(|l| l.contains("Toggle")));
        assert!(labels.iter().any(|l| l.contains("Meeting")));
    }

    #[test]
    fn dominant_compositor_picks_majority() {
        let bindings = vec![
            Binding {
                compositor: "Sway",
                keys: "k".into(),
                action: "record start".into(),
                source: PathBuf::new(),
            },
            Binding {
                compositor: "Sway",
                keys: "k".into(),
                action: "record stop".into(),
                source: PathBuf::new(),
            },
            Binding {
                compositor: "Hyprland",
                keys: "k".into(),
                action: "record toggle".into(),
                source: PathBuf::new(),
            },
        ];
        assert_eq!(dominant_compositor(&bindings), Compositor::Sway);
    }

    #[test]
    fn dominant_compositor_empty_defaults_to_hyprland() {
        assert_eq!(dominant_compositor(&[]), Compositor::Hyprland);
    }

    #[test]
    fn niri_skips_other_spawn_lines() {
        let line = r#"    Mod+T { spawn "alacritty"; }"#;
        assert!(parse_niri_line(line, dummy_path()).is_none());
    }

    #[test]
    fn niri_skips_comments() {
        let line = r#"// Mod+Home { spawn "voxtype" "record" "start"; }"#;
        assert!(parse_niri_line(line, dummy_path()).is_none());
    }

    #[test]
    fn rejects_unknown_action() {
        let line = "bindd = , HOME, ..., exec, voxtype record dance";
        assert!(parse_hyprland_line(line, dummy_path()).is_none());
    }
}
