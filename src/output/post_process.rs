//! Post-processing command execution
//!
//! Pipes transcribed text through an external command for cleanup/formatting.
//! Commonly used with local LLMs (Ollama, llama.cpp) or text processing tools.
//!
//! # Example Configuration
//!
//! ```toml
//! [output.post_process]
//! command = "ollama run llama3.2:1b 'Clean up this dictation:'"
//! timeout_ms = 30000
//! ```
//!
//! The command receives the transcribed text on stdin and should output
//! the processed text on stdout. On any failure, the original text is used.

use crate::config::PostProcessConfig;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

const MAX_BACKSPACES: u32 = 4096;

/// Parsed action headers from post-process output
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OutputActions {
    pub backspaces: u32,
    pub enter: bool,
}

/// Result of post-processing with optional actions
#[derive(Debug, Clone)]
pub struct ProcessedOutput {
    pub text: String,
    pub actions: OutputActions,
}

/// Post-processor that runs an external command on transcribed text
pub struct PostProcessor {
    command: String,
    timeout: Duration,
    trim: bool,
    fallback_on_empty: bool,
    actions_enabled: bool,
}

impl PostProcessor {
    /// Create a new post-processor from configuration
    pub fn new(config: &PostProcessConfig) -> Self {
        Self {
            command: config.command.clone(),
            timeout: Duration::from_millis(config.timeout_ms),
            trim: config.trim,
            fallback_on_empty: config.fallback_on_empty,
            actions_enabled: config.enable_control_codes,
        }
    }

    /// Parse action headers from the beginning of text
    ///
    /// Supported leading headers:
    /// - `<<<VOXTYPE:BACKSPACE=N>>>` - delete N characters
    /// - `<<<VOXTYPE:ENTER>>>` - press Enter after text
    ///
    /// Multiple headers are allowed. Malformed headers are treated as literal text.
    pub fn parse_actions(text: &str, actions_enabled: bool) -> ProcessedOutput {
        if !actions_enabled {
            return ProcessedOutput {
                text: text.to_string(),
                actions: OutputActions::default(),
            };
        }

        let mut remaining = text;
        let mut actions = OutputActions::default();

        loop {
            if let Some(rest) = remaining.strip_prefix("<<<VOXTYPE:BACKSPACE=") {
                if let Some(end) = rest.find(">>>") {
                    let num_str = &rest[..end];
                    if let Ok(n) = num_str.parse::<u32>() {
                        actions.backspaces =
                            actions.backspaces.saturating_add(n).min(MAX_BACKSPACES);
                        remaining = &rest[end + 3..];
                        continue;
                    }
                }
            }

            if let Some(rest) = remaining.strip_prefix("<<<VOXTYPE:ENTER>>>") {
                actions.enter = true;
                remaining = rest;
                continue;
            }

            break;
        }

        ProcessedOutput {
            text: remaining.to_string(),
            actions,
        }
    }

    /// Process text with optional context from a previous chunk
    ///
    /// When context is provided, it is passed via the VOXTYPE_CONTEXT environment
    /// variable so the post-processing command can use it for continuity.
    /// Stdin always contains only the current text, keeping existing scripts compatible.
    /// Returns the processed output with any parsed actions.
    pub async fn process_with_context(&self, text: &str, context: Option<&str>) -> ProcessedOutput {
        match self.execute_command_with_env(text, context).await {
            Ok(processed) => {
                if processed.is_empty() && self.fallback_on_empty {
                    tracing::warn!(
                        "Post-process command returned empty output, using original text"
                    );
                    Self::parse_actions(text, self.actions_enabled)
                } else if processed.is_empty() {
                    tracing::debug!("Post-process command returned empty output");
                    ProcessedOutput {
                        text: String::new(),
                        actions: OutputActions::default(),
                    }
                } else {
                    tracing::debug!(
                        "Post-processed ({} -> {} chars)",
                        text.len(),
                        processed.len()
                    );
                    Self::parse_actions(&processed, self.actions_enabled)
                }
            }
            Err(e) => {
                tracing::warn!("Post-process command failed: {}, using original text", e);
                ProcessedOutput {
                    text: text.to_string(),
                    actions: OutputActions::default(),
                }
            }
        }
    }

    /// Process text through the external command
    ///
    /// Returns the processed output with any parsed actions.
    pub async fn process(&self, text: &str) -> ProcessedOutput {
        self.process_with_context(text, None).await
    }

    async fn execute_command_with_env(
        &self,
        text: &str,
        context: Option<&str>,
    ) -> Result<String, PostProcessError> {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", &self.command])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Always clear to prevent inheriting stale context from parent environment
        cmd.env_remove("VOXTYPE_CONTEXT");
        if let Some(ctx) = context {
            cmd.env("VOXTYPE_CONTEXT", ctx);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| PostProcessError::SpawnFailed(e.to_string()))?;

        // Write text to stdin
        if let Some(mut stdin) = child.stdin.take() {
            // Ignore write errors: the command may not read stdin or may exit
            // before we finish writing (e.g., `echo` or `head -1`). The command's
            // exit code and stdout output determine success, not whether it
            // consumed all of stdin.
            let _ = stdin.write_all(text.as_bytes()).await;
            drop(stdin);
        }

        // Wait for completion with timeout
        let output = timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| PostProcessError::Timeout(self.timeout.as_secs()))?
            .map_err(|e| PostProcessError::WaitFailed(e.to_string()))?;

        // Check exit status
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PostProcessError::NonZeroExit {
                code: output.status.code(),
                stderr: stderr.trim().to_string(),
            });
        }

        // Parse stdout as UTF-8
        let processed = String::from_utf8(output.stdout)
            .map_err(|e| PostProcessError::InvalidUtf8(e.to_string()))?;

        if self.trim {
            Ok(processed.trim().to_string())
        } else {
            // Only strip trailing newlines (artifact of shell output), preserve other whitespace
            Ok(processed.trim_end_matches('\n').to_string())
        }
    }
}

/// Errors that can occur during post-processing
#[derive(Debug)]
pub enum PostProcessError {
    /// Failed to spawn the command process
    SpawnFailed(String),
    /// Failed to write text to stdin
    WriteFailed(String),
    /// Command timed out
    Timeout(u64),
    /// Failed to wait for command completion
    WaitFailed(String),
    /// Command exited with non-zero status
    NonZeroExit { code: Option<i32>, stderr: String },
    /// Command output was not valid UTF-8
    InvalidUtf8(String),
}

impl std::fmt::Display for PostProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SpawnFailed(e) => write!(f, "failed to spawn command: {}", e),
            Self::WriteFailed(e) => write!(f, "failed to write to stdin: {}", e),
            Self::Timeout(secs) => write!(f, "command timed out after {}s", secs),
            Self::WaitFailed(e) => write!(f, "failed to wait for command: {}", e),
            Self::NonZeroExit { code, stderr } => {
                if stderr.is_empty() {
                    write!(f, "command exited with code {:?}", code)
                } else {
                    write!(f, "command exited with code {:?}: {}", code, stderr)
                }
            }
            Self::InvalidUtf8(e) => write!(f, "output is not valid UTF-8: {}", e),
        }
    }
}

impl std::error::Error for PostProcessError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(command: &str, timeout_ms: u64) -> PostProcessConfig {
        PostProcessConfig {
            command: command.to_string(),
            timeout_ms,
            trim: true,
            fallback_on_empty: true,
            enable_control_codes: false,
        }
    }

    #[tokio::test]
    async fn test_simple_passthrough() {
        let config = make_config("cat", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("hello world").await;
        assert_eq!(result.text, "hello world");
    }

    #[tokio::test]
    async fn test_sed_transformation() {
        let config = make_config("sed 's/foo/bar/g'", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("foo bar foo").await;
        assert_eq!(result.text, "bar bar bar");
    }

    #[tokio::test]
    async fn test_tr_uppercase() {
        let config = make_config("tr '[:lower:]' '[:upper:]'", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("hello world").await;
        assert_eq!(result.text, "HELLO WORLD");
    }

    #[tokio::test]
    async fn test_timeout_fallback() {
        let config = make_config("sleep 10", 100);
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result.text, "original text");
    }

    #[tokio::test]
    async fn test_command_failure_fallback() {
        let config = make_config("exit 1", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result.text, "original text");
    }

    #[tokio::test]
    async fn test_empty_output_fallback() {
        let config = make_config("printf ''", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result.text, "original text");
    }

    #[tokio::test]
    async fn test_command_not_found_fallback() {
        let config = make_config("nonexistent_command_xyz_12345", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result.text, "original text");
    }

    #[tokio::test]
    async fn test_multiline_input() {
        let config = make_config("cat", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("line one\nline two\nline three").await;
        assert_eq!(result.text, "line one\nline two\nline three");
    }

    #[tokio::test]
    async fn test_unicode_handling() {
        let config = make_config("cat", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("Hello 世界! 🎉").await;
        assert_eq!(result.text, "Hello 世界! 🎉");
    }

    #[tokio::test]
    async fn test_whitespace_trimming() {
        let config = make_config("printf 'hello\\n'", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("ignored").await;
        assert_eq!(result.text, "hello");
    }

    #[tokio::test]
    async fn test_complex_shell_command() {
        let config = make_config("echo 'prefix:' && cat", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("test input").await;
        assert_eq!(result.text, "prefix:\ntest input");
    }

    #[tokio::test]
    async fn test_no_trim_preserves_trailing_space() {
        let config = PostProcessConfig {
            command: "printf '%s ' \"$( cat )\"".to_string(),
            timeout_ms: 5000,
            trim: false,
            fallback_on_empty: true,
            enable_control_codes: false,
        };
        let processor = PostProcessor::new(&config);
        let result = processor.process("hello world.").await;
        assert_eq!(result.text, "hello world. ");
    }

    #[tokio::test]
    async fn test_no_trim_still_strips_trailing_newlines() {
        let config = PostProcessConfig {
            command: "echo 'hello'".to_string(),
            timeout_ms: 5000,
            trim: false,
            fallback_on_empty: true,
            enable_control_codes: false,
        };
        let processor = PostProcessor::new(&config);
        let result = processor.process("ignored").await;
        assert_eq!(result.text, "hello");
    }

    #[tokio::test]
    async fn test_no_fallback_on_empty_returns_empty() {
        let config = PostProcessConfig {
            command: "printf ''".to_string(),
            timeout_ms: 5000,
            trim: true,
            fallback_on_empty: false,
            enable_control_codes: false,
        };
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result.text, "");
    }

    #[tokio::test]
    async fn test_fallback_on_empty_default_returns_original() {
        let config = make_config("printf ''", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result.text, "original text");
    }

    #[tokio::test]
    async fn test_no_trim_no_fallback_combination() {
        let config = PostProcessConfig {
            command: "printf ''".to_string(),
            timeout_ms: 5000,
            trim: false,
            fallback_on_empty: false,
            enable_control_codes: false,
        };
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result.text, "");
    }

    #[tokio::test]
    async fn test_trim_then_empty_triggers_fallback() {
        let config = PostProcessConfig {
            command: "printf '   \\n  '".to_string(),
            timeout_ms: 5000,
            trim: true,
            fallback_on_empty: true,
            enable_control_codes: false,
        };
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result.text, "original text");
    }

    #[tokio::test]
    async fn test_trim_then_empty_no_fallback_returns_empty() {
        let config = PostProcessConfig {
            command: "printf '   \\n  '".to_string(),
            timeout_ms: 5000,
            trim: true,
            fallback_on_empty: false,
            enable_control_codes: false,
        };
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result.text, "");
    }

    #[tokio::test]
    async fn test_context_passed_via_env_var() {
        let config = make_config("echo \"context:$VOXTYPE_CONTEXT stdin:$(cat)\"", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor
            .process_with_context("current text", Some("previous text"))
            .await;
        assert_eq!(result.text, "context:previous text stdin:current text");
    }

    #[tokio::test]
    async fn test_no_context_env_var_when_none() {
        let config = make_config(
            "echo \"context:${VOXTYPE_CONTEXT:-unset} stdin:$(cat)\"",
            5000,
        );
        let processor = PostProcessor::new(&config);
        let result = processor.process_with_context("current text", None).await;
        assert_eq!(result.text, "context:unset stdin:current text");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_context_env_not_inherited_from_parent() {
        std::env::set_var("VOXTYPE_CONTEXT", "stale parent context");
        let config = make_config("echo \"${VOXTYPE_CONTEXT:-unset}\"", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process_with_context("text", None).await;
        std::env::remove_var("VOXTYPE_CONTEXT");
        assert_eq!(result.text, "unset");
    }

    // Action parsing tests

    #[test]
    fn test_parse_actions_disabled() {
        let output = PostProcessor::parse_actions("<<<VOXTYPE:BACKSPACE=3>>>hello", false);
        assert_eq!(output.text, "<<<VOXTYPE:BACKSPACE=3>>>hello");
        assert_eq!(output.actions, OutputActions::default());
    }

    #[test]
    fn test_parse_actions_backspace() {
        let output = PostProcessor::parse_actions("<<<VOXTYPE:BACKSPACE=3>>>hello", true);
        assert_eq!(output.text, "hello");
        assert_eq!(output.actions.backspaces, 3);
        assert!(!output.actions.enter);
    }

    #[test]
    fn test_parse_actions_enter() {
        let output = PostProcessor::parse_actions("<<<VOXTYPE:ENTER>>>hello", true);
        assert_eq!(output.text, "hello");
        assert_eq!(output.actions.backspaces, 0);
        assert!(output.actions.enter);
    }

    #[test]
    fn test_parse_actions_multiple_headers() {
        let output =
            PostProcessor::parse_actions("<<<VOXTYPE:BACKSPACE=2>>><<<VOXTYPE:ENTER>>>hello", true);
        assert_eq!(output.text, "hello");
        assert_eq!(output.actions.backspaces, 2);
        assert!(output.actions.enter);
    }

    #[test]
    fn test_parse_actions_malformed_header() {
        let output = PostProcessor::parse_actions("<<<VOXTYPE:BACKSPACE=abc>>>hello", true);
        assert_eq!(output.text, "<<<VOXTYPE:BACKSPACE=abc>>>hello");
        assert_eq!(output.actions, OutputActions::default());
    }

    #[test]
    fn test_parse_actions_clamp() {
        let output = PostProcessor::parse_actions("<<<VOXTYPE:BACKSPACE=99999>>>hello", true);
        assert_eq!(output.text, "hello");
        assert_eq!(output.actions.backspaces, MAX_BACKSPACES);
    }

    #[test]
    fn test_parse_actions_header_only() {
        let output = PostProcessor::parse_actions("<<<VOXTYPE:ENTER>>>", true);
        assert_eq!(output.text, "");
        assert!(output.actions.enter);
        assert_eq!(output.actions.backspaces, 0);
    }

    #[test]
    fn test_parse_actions_no_actions() {
        let output = PostProcessor::parse_actions("just text", true);
        assert_eq!(output.text, "just text");
        assert_eq!(output.actions, OutputActions::default());
    }
}
