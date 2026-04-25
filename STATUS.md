# Streaming transcription — implementation log

Branch: `feature/streaming-transcription`. Tracking commits, design decisions,
and what is left.

## Commits landed

### Commit 1 — state machine + streaming trait scaffolding (in progress)

Adds the trait, event types, and `State::Streaming` variant. No backends yet,
no daemon.rs wiring yet. All existing `Transcriber` impls untouched (default
`as_streaming()` returns `None`). All 544 existing lib tests still pass; 7 new
tests added (5 in `state::tests`, 2 in `transcribe::streaming::tests`).

Changed files:
- `src/state.rs` — `State::Streaming { started_at, model_override, partial_buffer, finalized_text, typed_chars }`. `is_recording()` now also returns `true` for `Streaming`. New `is_streaming()` helper. `Display` impl extended.
- `src/transcribe/mod.rs` — `pub mod streaming;` and a default `as_streaming(&self) -> Option<&dyn StreamingTranscriber>` method on `Transcriber`.
- `src/transcribe/streaming.rs` — new file. `StreamingEvent { Partial, Final, Ended, Error }`, `StreamHandle { events: mpsc::Receiver, cancel: oneshot::Sender, task: JoinHandle }`, `StreamingTranscriber` trait taking `mpsc::Receiver<Vec<f32>>` (matches `AudioCapture`'s output type).

Design notes / divergences from the v2 proposal in the prior STATUS:
- The v2 proposal had `cancel: Box<dyn FnOnce() + Send>`. Replaced with `oneshot::Sender<()>` — `FnOnce` requires consuming `self` to call, awkward to share alongside the events Receiver. A oneshot is cleaner and idiomatic in tokio.
- Added `task: JoinHandle<Result<(), TranscribeError>>` so the daemon can `await` the backend's drive task on shutdown / error reporting.
- `StreamingEvent` is **not** `Clone` (because `TranscribeError` isn't). Documented in code; events are consumed once from the channel which is the only realistic path.

## What's next

- **Commit 2** — output-layer changes. Default policy: incremental typing of *Final* segments only, with `typed_chars` tracking on the daemon-side state. Partials update an in-memory status string, never hit the keyboard. Post-process hook: per-final-segment with `VOXTYPE_CONTEXT = finalized_text_so_far` (mirrors the existing eager-mode pattern in `output/post_process.rs`).
- **Commit 3** — Gemini Live backend via `tokio-tungstenite`. Add dependency, implement `StreamingTranscriber`. Daemon wiring — only when `[transcribe] streaming = true`.
- **Commit 4** — one local streaming backend. First investigate `whisper-rs` streaming; fall back to chunked-VAD over the existing `WhisperTranscriber` if it doesn't expose stream APIs.
- **Commit 5** — docs (USER_MANUAL.md, CONFIGURATION.md, TROUBLESHOOTING.md).

## Permission status

Resolved. `git -C <wt> ...` and `cd <wt> && ...` forms both work. `cargo check`, `cargo test`, and explicit-file `rustfmt` invocations work. Note: `cargo fmt -- path/to/file.rs` in workspace mode reformats the whole crate, not just the named files — pre-existing fmt drift in unmodified files leaks in. Workaround: format the new files in isolation, then `git checkout` any unrelated file the workspace fmt touched.
