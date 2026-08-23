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
    // Player discovery can probe several binaries for ~1s on first use. Never
    // spend that time on the async alarm/event loop: it must keep classifying,
    // fetching and accepting snooze input while audio starts.
    std::thread::spawn(move || {
        let _ = sound_check(tone);
    });
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
    started_playing(|| {
        child
            .try_wait()
            .map(|done| done.map(|status| status.success()))
    })
}

/// The startup watch itself, over anything that reports an exit.
///
/// Split from the process it usually watches so it can be tested without
/// spawning one. Testing it through a real child meant `sh -c "exit 1"`, and a
/// cold shell on a loaded Windows runner takes longer to start than the whole
/// budget here — the child had not exited yet, so the watch correctly said
/// "still playing" and the test failed for a reason that had nothing to do
/// with the behaviour it was pinning.
///
/// `poll` reports `Some(true)` for a clean exit, `Some(false)` for a failure,
/// and `None` while the player is still running.
fn started_playing(mut poll: impl FnMut() -> std::io::Result<Option<bool>>) -> bool {
    for _ in 0..STARTUP_CHECKS {
        match poll() {
            Ok(Some(success)) => return success,
            Ok(None) => std::thread::sleep(STARTUP_POLL),
            Err(_) => return false,
        }
    }
    true
}

/// A player table: (program, args-before-file). The file path is appended
/// last.
type Candidates = &'static [(&'static str, &'static [&'static str])];

/// The players to try, in order.
#[cfg(not(test))]
const CANDIDATES: Candidates = &[
    ("paplay", &[]),
    ("pw-play", &[]),
    ("aplay", &["-q"]),
    ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
    ("canberra-gtk-play", &["-f"]), // canberra wants --file=; handled below
    ("afplay", &[]),                // macOS
    ("cvlc", &["--play-and-exit", "--intf", "dummy"]),
];

/// Stand-ins for the real players in a test build, because `alarm` is
/// fire-and-forget: it spawns a detached thread, the test process exits, and
/// the orphaned player goes on playing. `cargo test` on a desktop machine
/// sounded a real alarm through the speakers, from a run that asserts nothing
/// about whether sound was produced.
///
/// Swapping the table rather than short-circuiting `play` keeps the seam
/// closed for every test, present and future, instead of asking each one to
/// remember to opt in. It also keeps discovery honest: `WORKS` outlives
/// `STARTUP_CHECKS`, so `play` takes its full ~150ms, and a `play` that
/// stopped being backgrounded fails the timing test rather than passing it
/// trivially.
///
/// Both outcomes stay reachable. A stand-in that could only succeed would
/// make `Played::Bell` and `Played::Nothing` unreachable under test, which
/// fakes the two real consumers of that answer — `App::run_alarm_test` and
/// the alarm self-test — and would let a test claim "sounded via …" on a
/// machine with no audio at all.
///
/// The choice is thread-local because cargo runs tests as parallel threads in
/// one process: a global would have one test's failing player decide another
/// test's outcome.
#[cfg(all(test, unix))]
pub(crate) mod stand_in {
    use std::cell::Cell;

    pub const WORKS: super::Candidates = &[("sh", &["-c", "sleep 0.4"])];
    pub const FAILS: super::Candidates = &[("sh", &["-c", "exit 1"])];

    thread_local! {
        static TABLE: Cell<super::Candidates> = const { Cell::new(WORKS) };
    }

    pub fn table() -> super::Candidates {
        TABLE.with(|t| t.get())
    }

    /// Choose what the next `play` on this thread finds.
    pub fn set(next: super::Candidates) {
        TABLE.with(|t| t.set(next));
    }
}

/// Spawn the first working audio player on this platform, detached.
/// Returns which one actually produced sound, if any.
fn play(path: &Path) -> Option<&'static str> {
    #[cfg(not(test))]
    let candidates = CANDIDATES;
    #[cfg(all(test, unix))]
    let candidates = stand_in::table();
    // Nothing to stand in with off unix, so a test build finds no player.
    #[cfg(all(test, not(unix)))]
    let candidates: Candidates = &[];

    // The two caches are process-global and the stand-ins share one program
    // name, so under test they would carry one test's verdict into another
    // thread's run. Skipping them costs a test build nothing: there is one
    // candidate, and it is spawned per call either way.
    let known_good = if cfg!(test) {
        None
    } else {
        WORKING.lock().ok().and_then(|w| *w)
    };
    for &(prog, args) in candidates {
        // Don't keep paying for a player that has already proven it can't
        // reach the audio server here.
        if !cfg!(test) && FAILED.lock().is_ok_and(|f| f.contains(&prog)) {
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
            if !cfg!(test) {
                if let Ok(mut failed) = FAILED.lock() {
                    failed.push(prog);
                }
            }
            continue;
        }
        if !cfg!(test) {
            if let Ok(mut working) = WORKING.lock() {
                *working = Some(prog);
            }
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
    // Never from a test build. `alarm` rings this on a detached thread, and
    // cargo's output capture is per-thread and not inherited by threads a test
    // spawns, so the BEL would reach the real terminal — the CI runner's
    // included. The answer `Played::Bell` still travels; only the noise stops.
    if cfg!(test) {
        return;
    }
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

    #[test]
    fn alarm_player_discovery_does_not_block_the_caller() {
        let started = std::time::Instant::now();
        alarm(Tone::Stale);
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
    }

    /// The stand-in exists so a test build makes no noise, but a seam that can
    /// only succeed would fake the two callers that read this answer —
    /// `App::run_alarm_test` and the alarm self-test both print it to a human.
    #[test]
    #[cfg(unix)]
    fn a_working_player_is_reported_by_name() {
        stand_in::set(stand_in::WORKS);
        assert_eq!(sound_check(Tone::Low), Played::Player("sh"));
    }

    /// The headless case: every player spawns and dies without reaching an
    /// audio server, so the bell is all that is left. Unreachable under test
    /// until the stand-in could fail.
    #[test]
    #[cfg(unix)]
    fn a_machine_with_no_working_player_falls_back_to_the_bell() {
        stand_in::set(stand_in::FAILS);
        let played = sound_check(Tone::Low);
        stand_in::set(stand_in::WORKS);
        assert_eq!(played, Played::Bell);
    }

    /// The C3 failure: a player that exists on PATH, spawns fine, and exits
    /// non-zero milliseconds later because no audio server is reachable.
    #[test]
    fn a_player_that_exits_nonzero_did_not_produce_sound() {
        assert!(
            !started_playing(|| Ok(Some(false))),
            "a failing player was counted as a sounded alarm"
        );
    }

    /// The same failure, from a player that takes a few polls to give up
    /// rather than dying on the first one.
    #[test]
    fn a_player_that_fails_a_little_later_is_still_caught() {
        let mut polls = 0;
        let played = started_playing(|| {
            polls += 1;
            Ok(if polls < 3 { None } else { Some(false) })
        });
        assert!(
            !played,
            "a player that failed on the third poll was believed"
        );
        assert_eq!(polls, 3);
    }

    #[test]
    fn a_player_still_running_counts_as_sound() {
        // Stands in for a player working through the ~0.5s sample: it never
        // exits inside the budget, and the watch runs out of checks.
        let mut polls = 0;
        let played = started_playing(|| {
            polls += 1;
            Ok(None)
        });
        assert!(played);
        assert_eq!(polls, STARTUP_CHECKS, "the watch gave up early");
    }

    #[test]
    fn a_player_that_exits_cleanly_counts_as_sound() {
        // Some players return promptly on a very short sample.
        assert!(started_playing(|| Ok(Some(true))));
    }

    /// A `try_wait` that errors says nothing about the audio, and the bell is
    /// the safe answer for an alarm.
    #[test]
    fn a_player_that_cannot_be_watched_is_not_believed() {
        assert!(!started_playing(|| Err(std::io::Error::other(
            "no such process"
        ))));
    }

    /// The watch above is only worth anything if it is wired to a real child,
    /// which needs a program that exits when told. Unix only: `sh` is the one
    /// spelling every platform sugarrush's alarm runs on agrees about.
    #[test]
    #[cfg(unix)]
    fn the_watch_is_wired_to_the_player_it_launches() {
        let spawn = |script: &str| {
            Command::new("sh")
                .args(["-c", script])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("sh should exist")
        };

        let mut failing = spawn("exit 1");
        assert!(
            !produced_sound(&mut failing),
            "a failing player was believed"
        );

        let mut clean = spawn("exit 0");
        assert!(produced_sound(&mut clean));

        let mut playing = spawn("sleep 2");
        assert!(produced_sound(&mut playing));
        let _ = playing.kill();
        let _ = playing.wait();
    }
}
