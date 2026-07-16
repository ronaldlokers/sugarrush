//! Best-effort audible alarm.
//!
//! Synthesizes a short two-tone beep WAV on first use, writes it to a temp
//! file, and plays it by spawning whatever system audio player is available.
//! Everything is best-effort: no audio dependencies, and failures are silent.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

const SAMPLE_RATE: u32 = 44_100;

/// Play the alarm sound once, if a player is available. Non-blocking.
pub fn alarm() {
    let Some(path) = wav_path() else { return };
    play(path);
}

/// Path to the generated WAV, created on first call.
fn wav_path() -> Option<&'static Path> {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = dir.join("sugarrush-alarm.wav");
        std::fs::write(&path, alarm_wav()).ok().map(|_| path)
    })
    .as_deref()
}

/// Spawn the first available audio player on this platform, detached.
fn play(path: &Path) {
    // (program, args-before-file). The file path is appended last.
    let candidates: [(&str, &[&str]); 7] = [
        ("paplay", &[]),
        ("pw-play", &[]),
        ("aplay", &["-q"]),
        ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
        ("canberra-gtk-play", &["-f"]), // canberra wants --file=; handled below
        ("afplay", &[]),                // macOS
        ("cvlc", &["--play-and-exit", "--intf", "dummy"]),
    ];
    for (prog, args) in candidates {
        let mut cmd = Command::new(prog);
        if prog == "canberra-gtk-play" {
            cmd.arg(format!("--file={}", path.display()));
        } else {
            cmd.args(args).arg(path);
        }
        let spawned = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if spawned.is_ok() {
            return;
        }
    }
}

/// A ~0.5s alarm: alternating 880 Hz / 1320 Hz tones, 16-bit mono PCM.
fn alarm_wav() -> Vec<u8> {
    let mut samples: Vec<i16> = Vec::new();
    // Four 110ms segments alternating pitch, with a short fade to avoid clicks.
    for seg in 0..4 {
        let freq = if seg % 2 == 0 { 880.0 } else { 1320.0 };
        let n = SAMPLE_RATE as usize * 110 / 1000;
        for i in 0..n {
            let t = i as f64 / SAMPLE_RATE as f64;
            // Simple linear fade in/out over 4ms.
            let fade_len = (SAMPLE_RATE as f64 * 0.004) as usize;
            let amp = if i < fade_len {
                i as f64 / fade_len as f64
            } else if i > n - fade_len {
                (n - i) as f64 / fade_len as f64
            } else {
                1.0
            };
            let s = (t * freq * std::f64::consts::TAU).sin() * amp * 0.5;
            samples.push((s * i16::MAX as f64) as i16);
        }
    }
    encode_wav(&samples)
}

/// Minimal 16-bit mono PCM WAV container.
fn encode_wav(samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    let byte_rate = SAMPLE_RATE * 2;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_has_valid_header() {
        let wav = alarm_wav();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        // Declared data length matches the actual sample bytes.
        let declared = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]) as usize;
        assert_eq!(declared, wav.len() - 44);
    }
}
