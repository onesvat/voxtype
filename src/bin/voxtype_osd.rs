//! `voxtype-osd` — on-screen mic visualizer for the Voxtype daemon.
//!
//! This binary connects to the daemon's audio-frame socket
//! (`$XDG_RUNTIME_DIR/voxtype/audio.sock`), decodes the 16-byte frames into a
//! ring buffer, and (in later commits) renders a layer-shell waveform + peak
//! meter on top of all windows during recording.
//!
//! ## Status
//!
//! Skeleton only — no GUI yet. The current binary connects to the socket,
//! pumps frames into a ring buffer, and prints a one-line status to stdout
//! every ~100 ms so the IPC end-to-end can be verified before adding any
//! Wayland code.
//!
//! Run with `RUST_LOG=debug` for verbose logs.

use std::io::ErrorKind;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;
use tokio::time::{sleep, Instant};

use voxtype::audio::levels::{default_socket_path, AudioFrame, FRAME_BYTES, FRAME_HZ};

/// Default ring buffer depth: 3 seconds at 100 Hz.
const RING_DEPTH: usize = 300;

#[derive(Parser, Debug)]
#[command(
    name = "voxtype-osd",
    version,
    about = "Voxtype on-screen mic visualizer"
)]
struct Args {
    /// Path to the audio-frame Unix socket. Defaults to
    /// `$XDG_RUNTIME_DIR/voxtype/audio.sock`.
    #[arg(long, env = "VOXTYPE_OSD_SOCKET")]
    socket: Option<PathBuf>,

    /// How long to wait between reconnect attempts when the daemon is down.
    #[arg(long, default_value = "1.0")]
    reconnect_secs: f32,

    /// Print one debug line per N frames received (0 = quiet).
    #[arg(long, default_value = "100")]
    log_every: u32,
}

/// Fixed-capacity ring buffer of audio frames. New frames overwrite the
/// oldest. Used by the renderer (later commit) to draw the waveform.
struct FrameRing {
    buf: Vec<Option<AudioFrame>>,
    head: usize,
    len: usize,
}

impl FrameRing {
    fn new(capacity: usize) -> Self {
        Self {
            buf: vec![None; capacity],
            head: 0,
            len: 0,
        }
    }

    fn push(&mut self, frame: AudioFrame) {
        let cap = self.buf.len();
        self.buf[self.head] = Some(frame);
        self.head = (self.head + 1) % cap;
        if self.len < cap {
            self.len += 1;
        }
    }

    fn latest(&self) -> Option<AudioFrame> {
        if self.len == 0 {
            return None;
        }
        let cap = self.buf.len();
        let idx = (self.head + cap - 1) % cap;
        self.buf[idx]
    }

    fn len(&self) -> usize {
        self.len
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let socket_path = args.socket.unwrap_or_else(default_socket_path);

    tracing::info!("voxtype-osd starting; socket={:?}", socket_path);

    let mut ring = FrameRing::new(RING_DEPTH);

    loop {
        match UnixStream::connect(&socket_path).await {
            Ok(mut stream) => {
                tracing::info!("Connected to daemon at {:?}", socket_path);
                let mut buf = [0u8; FRAME_BYTES];
                let mut total: u64 = 0;
                let mut last_log = Instant::now();

                loop {
                    match stream.read_exact(&mut buf).await {
                        Ok(_) => {
                            let frame = AudioFrame::from_bytes(&buf);
                            ring.push(frame);
                            total += 1;
                            if args.log_every > 0 && (total % u64::from(args.log_every) == 0) {
                                let elapsed = last_log.elapsed().as_secs_f32();
                                let rate = if elapsed > 0.0 {
                                    args.log_every as f32 / elapsed
                                } else {
                                    0.0
                                };
                                if let Some(latest) = ring.latest() {
                                    tracing::debug!(
                                        target: "osd::frames",
                                        seq = latest.seq,
                                        peak_dbfs = latest.peak_dbfs,
                                        min = latest.min,
                                        max = latest.max,
                                        rate_hz = rate,
                                        ring_len = ring.len(),
                                        "frame batch"
                                    );
                                }
                                last_log = Instant::now();
                            }
                        }
                        Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                            tracing::info!("Daemon closed the socket (EOF)");
                            break;
                        }
                        Err(e) => {
                            tracing::warn!("Read error on audio socket: {}", e);
                            break;
                        }
                    }
                }

                tracing::info!(
                    "Disconnected after {} frames (~{:.1}s of audio)",
                    total,
                    total as f32 / FRAME_HZ as f32
                );
            }
            Err(e) => {
                tracing::debug!(
                    "Cannot connect to {:?}: {} (retrying in {:.1}s)",
                    socket_path,
                    e,
                    args.reconnect_secs
                );
            }
        }

        sleep(Duration::from_secs_f32(args.reconnect_secs)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(seq: u32) -> AudioFrame {
        AudioFrame {
            seq,
            min: -0.1,
            max: 0.1,
            peak_dbfs: -20.0,
        }
    }

    #[test]
    fn ring_keeps_latest_within_capacity() {
        let mut r = FrameRing::new(4);
        for i in 0..10 {
            r.push(frame(i));
        }
        assert_eq!(r.len(), 4);
        let latest = r.latest().unwrap();
        assert_eq!(latest.seq, 9);
    }

    #[test]
    fn ring_latest_none_when_empty() {
        let r = FrameRing::new(8);
        assert!(r.latest().is_none());
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn ring_grows_until_capacity() {
        let mut r = FrameRing::new(8);
        for i in 0..3 {
            r.push(frame(i));
        }
        assert_eq!(r.len(), 3);
        assert_eq!(r.latest().unwrap().seq, 2);
    }
}
