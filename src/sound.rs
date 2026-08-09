//! Best-effort audible alarm.
//!
//! Synthesizes a short two-tone beep WAV on first use, writes it to a temp
//! file, and plays it by spawning whatever system audio player is available.
//! Everything is best-effort: no audio dependencies, and failures are silent.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};

const SAMPLE_RATE: u32 = 44_100;

/// Which alarm sound to play — distinct tone shapes per alert kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Urgent low — descending tones.
    Low,
    /// Urgent high — ascending tones.
    High,
    /// Stale / no data — flat repeated blips.
    Stale,
}

/// Play the alarm sound for `tone` once. Falls back to the terminal bell if no
/// player actually produced sound.
pub fn alarm(tone: Tone) {
    let _ = sound_check(tone);
}

/// What happened when we tried to make a noise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Played {
    /// An audio player produced sound. Which one.
    Player(&'static str),
    /// No player worked, so we rang the terminal bell — which many terminals
    /// render as a silent flash. This is a last resort, not a strategy.
    Bell,
    /// Not even the WAV could be written (read-only or full runtime dir).
    Nothing,
}

/// Play the alarm and report which channel actually carried it.
///
/// `alarm` throws the answer away because the run loop has nothing to do with
/// it; the self-test exists precisely to show it to a human, because "the
/// audible alarm is on" and "this machine can make a sound" are different
/// claims and only one of them was ever checked.
pub fn sound_check(tone: Tone) -> Played {
    let Some(path) = wav_path(tone) else {
        // Silence here would be indistinguishable from "glucose is fine".
        bell();
        return Played::Nothing;
    };
    match play(&path) {
        Some(prog) => Played::Player(prog),
        None => {
            bell();
            Played::Bell
        }
    }
}

/// Path to the generated WAV for a tone, created on first use per tone.
fn wav_path(tone: Tone) -> Option<PathBuf> {
    static PATHS: OnceLock<[Option<PathBuf>; 3]> = OnceLock::new();
    let paths = PATHS.get_or_init(|| {
        let Some(dir) = private_audio_dir() else {
            return [None, None, None];
        };
        [Tone::Low, Tone::High, Tone::Stale].map(|t| {
            let path = dir.join(format!("sugarrush-alarm-{}.wav", t.suffix()));
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&path).ok()?;
            std::io::Write::write_all(&mut file, &alarm_wav(t)).ok()?;
            Some(path)
        })
    });
    paths[tone.index()].clone()
}

fn private_audio_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            #[cfg(unix)]
            {
                let uid = dirs::home_dir()
                    .and_then(|p| std::fs::metadata(p).ok())
                    .map(|m| m.uid())
                    .unwrap_or(0);
                std::env::temp_dir().join(format!("sugarrush-{uid}"))
            }
            #[cfg(not(unix))]
            std::env::temp_dir().join("sugarrush")
        })
        .join("sugarrush-audio");
    std::fs::create_dir_all(&base).ok()?;
    #[cfg(unix)]
    {
        let ours = dirs::home_dir()
            .and_then(|p| std::fs::metadata(p).ok())
            .is_some_and(|home| std::fs::metadata(&base).is_ok_and(|dir| dir.uid() == home.uid()));
        if !ours {
            return None;
        }
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700)).ok()?;
    }
    Some(base)
}

impl Tone {
    fn index(self) -> usize {
        match self {
            Tone::Low => 0,
            Tone::High => 1,
            Tone::Stale => 2,
        }
    }
    fn suffix(self) -> &'static str {
        match self {
            Tone::Low => "low",
            Tone::High => "high",
            Tone::Stale => "stale",
        }
    }
    /// Frequency sequence for the four segments.
    fn freqs(self) -> [f64; 4] {
        match self {
            Tone::Low => [1320.0, 1100.0, 880.0, 660.0],
            Tone::High => [660.0, 880.0, 1100.0, 1320.0],
            Tone::Stale => [880.0, 0.0, 880.0, 0.0],
        }
    }
}

/// Children spawned by [`play`], kept only so they can be reaped.
///
/// A spawned player stays a zombie in the process table until someone waits on
/// it. The alarm re-plays every few seconds for as long as an urgent state
/// lasts, so without this a long overnight low would leave hundreds of dead
/// entries behind — eventually hitting the process limit and taking the alarm
/// (and the rest of the app's subprocesses) with it.
static PLAYERS: Mutex<Vec<Child>> = Mutex::new(Vec::new());

/// Players that spawned and then failed, so a broken one is tried once rather
/// than every three seconds for the length of an overnight low.
static FAILED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
/// The player that last produced sound. Once one is proven, the startup check
/// below is skipped, so the steady-state alarm path adds no delay at all.
static WORKING: Mutex<Option<&'static str>> = Mutex::new(None);

/// How long to watch a newly-tried player before believing it.
const STARTUP_POLL: std::time::Duration = std::time::Duration::from_millis(15);
const STARTUP_CHECKS: usize = 10;

/// Wait on any finished players, keeping the ones still playing.
fn reap(players: &mut Vec<Child>) {
    players.retain_mut(|c| matches!(c.try_wait(), Ok(None)));
}

/// True if the player is still running (or exited cleanly) shortly after
/// launch.
///
/// A successful `spawn` only means the binary exists on `$PATH` — it says
/// nothing about whether the process reached an audio server. `paplay` is
/// installed on essentially every PipeWire/PulseAudio system and exits non-zero
/// within milliseconds when the server is unreachable (a user unit started
/// before the session, SSH, a container). Its stderr is already discarded, so
/// that failure was completely invisible: the alarm counted it as sounded and
/// never reached the bell.
///
/// The sample is ~0.5s long, so a player that is genuinely playing is still
/// alive when this returns.
fn produced_sound(child: &mut Child) -> bool {
    for _ in 0..STARTUP_CHECKS {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => std::thread::sleep(STARTUP_POLL),
            Err(_) => return false,
        }
    }
    true
}

/// Spawn the first working audio player on this platform, detached.
/// Returns which one actually produced sound, if any.
fn play(path: &Path) -> Option<&'static str> {
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
    let known_good = WORKING.lock().ok().and_then(|w| *w);
    for (prog, args) in candidates {
        // Don't keep paying for a player that has already proven it can't
        // reach the audio server here.
        if FAILED.lock().is_ok_and(|f| f.contains(&prog)) {
            continue;
        }
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
        let Ok(mut child) = spawned else { continue };

        // A player we've already seen work is trusted immediately, so the
        // common path stays non-blocking.
        if known_good != Some(prog) && !produced_sound(&mut child) {
            if let Ok(mut failed) = FAILED.lock() {
                failed.push(prog);
            }
            continue;
        }
        if let Ok(mut working) = WORKING.lock() {
            *working = Some(prog);
        }
        if let Ok(mut players) = PLAYERS.lock() {
            reap(&mut players);
            players.push(child);
        }
        return Some(prog);
    }
    // Nothing on this machine can play it — headless boxes, minimal
    // containers, a bare SSH login, or an audio server that isn't answering.
    None
}

/// Ring the terminal bell. Whether it makes a sound is the terminal's call
/// (many map it to a visual flash), so this is a last resort, not a strategy.
fn bell() {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x07");
    let _ = out.flush();
}

/// A ~0.5s alarm: four 110ms segments per the tone's frequency sequence
/// (0 Hz = silence), 16-bit mono PCM.
fn alarm_wav(tone: Tone) -> Vec<u8> {
    let mut samples: Vec<i16> = Vec::new();
    for freq in tone.freqs() {
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
        let wav = alarm_wav(Tone::Low);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        // Declared data length matches the actual sample bytes.
        let declared = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]) as usize;
        assert_eq!(declared, wav.len() - 44);
    }

    #[test]
    fn alarm_files_use_a_private_directory() {
        let dir = private_audio_dir().expect("a private runtime directory");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    /// The C3 failure: a player that exists on PATH, spawns fine, and exits
    /// non-zero milliseconds later because no audio server is reachable.
    #[test]
    fn a_player_that_exits_nonzero_did_not_produce_sound() {
        let mut child = Command::new("sh")
            .args(["-c", "exit 1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("sh should exist");
        assert!(
            !produced_sound(&mut child),
            "a failing player was counted as a sounded alarm"
        );
    }

    #[test]
    fn a_player_still_running_counts_as_sound() {
        // Stands in for a player working through the ~0.5s sample.
        let mut child = Command::new("sh")
            .args(["-c", "sleep 2"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("sh should exist");
        assert!(produced_sound(&mut child));
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn a_player_that_exits_cleanly_counts_as_sound() {
        // Some players return promptly on a very short sample.
        let mut child = Command::new("sh")
            .args(["-c", "exit 0"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("sh should exist");
        assert!(produced_sound(&mut child));
    }
}
