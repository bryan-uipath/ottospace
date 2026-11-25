//! A transcript's real sound: the source pane's app's audio by ScreenCaptureKit
//! (`ottospace_mac::app_audio`), fed to Apple's on-device recogniser (`ottospace_mac::speech`).
//! What the recogniser says and the sound's level wait here, on SCK's and the recogniser's
//! threads, until the shell's next frame takes them (`listen.rs`): lines in its fixed-tick batch,
//! the level for the meter alone.
//!
//! In a test run only the app `OTTOSPACE_HOST_ONLY` names (the canary) is ever heard, and the
//! speech model is never downloaded.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ottospace_core::listen::{Heard, Speaker};
use ottospace_mac::app_audio::{self, AppAudio};
use ottospace_mac::speech::{self, Recogniser, Status};

use crate::{event_log, listen_meter};

/// How often the log counts what the stream gave; no sound in the first, from an app that was
/// playing, is a `listen_quiet`.
const COUNTS: Duration = Duration::from_secs(10);
/// How often the app's helpers are looked for again (Zoom starts `aomhost` with a meeting).
const REFRESH: Duration = Duration::from_secs(5);

/// What the streams gave since the last frame.
#[derive(Default)]
pub(crate) struct Taken {
    pub(crate) lines: Vec<Heard>,
    /// The sound's level (RMS) of the last buffer, if one came.
    pub(crate) level: Option<f32>,
    /// Why the sound stopped coming, if it did.
    pub(crate) failed: Option<String>,
}

/// The streams of one run (a start or a resume): its times count from 0.
pub(crate) struct Live {
    taken: Arc<Mutex<Taken>>,
    /// Dropped first: no more samples, then the recogniser's input ends.
    audio: Arc<AppAudio>,
    _recogniser: Arc<Recogniser>,
    counts: Arc<Counts>,
    /// When `counts` were last logged, and whether they ever were.
    counted: (Instant, bool),
    refreshed: Instant,
}

/// What came since the last `COUNTS`: buffers, those with sound, the recogniser's final lines.
#[derive(Default)]
struct Counts {
    buffers: AtomicUsize,
    loud: AtomicUsize,
    lines: AtomicUsize,
}

impl Drop for Live {
    /// The stream stops here, though a refresh still holds it.
    fn drop(&mut self) {
        self.audio.stop();
    }
}

/// The log's line for a stream that started: the app's bundle id and how many pids it hears.
fn log_start(audio: &AppAudio) {
    let fields = serde_json::json!({
        "bundle": audio.bundle, "pids": audio.heard(), "playing": audio.playing, "unoffered": audio.unoffered,
    });
    event_log::event("listen_audio", fields);
}

/// Why `pid` may not be heard in this run: a test run hears only the app it names.
pub(crate) fn refusal(pid: i32) -> Option<&'static str> {
    let host_only: Option<i32> = std::env::var("OTTOSPACE_HOST_ONLY").ok().and_then(|pid| pid.parse().ok());
    let test_run = std::env::var_os("OTTOSPACE_E2E").is_some();
    (test_run && host_only != Some(pid)).then_some("a test run hears only the app OTTOSPACE_HOST_ONLY names")
}

/// Whether this Mac has the recogniser, asked of the system (it waits: off the main thread).
/// None in a test run that hears no app.
pub(crate) fn available() -> Option<bool> {
    let test_run = std::env::var_os("OTTOSPACE_E2E").is_some();
    if test_run && std::env::var_os("OTTOSPACE_HOST_ONLY").is_none() {
        return None;
    }
    Some(speech::status(None) != Status::Absent)
}

/// The model is ready, or why not: `Ok(false)` when it must be downloaded first. Off the main thread.
pub(crate) fn ready() -> Result<bool, String> {
    match speech::status(None) {
        Status::Installed => Ok(true),
        Status::NotInstalled | Status::Downloading => {
            if std::env::var_os("OTTOSPACE_E2E").is_some() {
                Err("the speech model is not installed (a test run downloads none)".into())
            } else {
                Ok(false)
            }
        }
        Status::Unsupported => Err("the system's language has no speech model".into()),
        Status::Unavailable => Err("this Mac cannot run Apple's speech recogniser".into()),
        Status::Absent => Err(ottospace_core::listen::NO_SOURCE.into()),
    }
}

/// Asks the system to download the model, and waits for it. Off the main thread.
pub(crate) fn install() -> Result<(), String> {
    let (send, receive) = std::sync::mpsc::channel();
    speech::install(None, move |done| {
        let _ = send.send(done);
    });
    receive.recv().map_err(|_| "the download did not answer".to_string())?
}

impl Live {
    /// Hears the app of `pid`: its audio to the recogniser. Off the main thread: both wait on the system.
    pub(crate) fn start(pid: i32) -> Result<Live, String> {
        let taken = Arc::new(Mutex::new(Taken::default()));
        let into = taken.clone();
        let counts = Arc::new(Counts::default());
        let said_counts = counts.clone();
        let heard = move |said: speech::Heard| {
            let Ok(mut taken) = into.lock() else { return };
            match said {
                speech::Heard::Line { text, start_ms, is_final, .. } => {
                    if is_final {
                        said_counts.lines.fetch_add(1, Ordering::Relaxed);
                    }
                    taken.lines.push(Heard { from: Speaker::Them, start_ms, text, is_final })
                }
                speech::Heard::Failed(why) => taken.failed = Some(format!("the recogniser: {why}")),
                speech::Heard::Finished => {}
            }
        };
        let rate = f64::from(app_audio::RATE);
        let recogniser = Recogniser::new(None, rate, heard).map_err(|why| {
            event_log::event("listen_failed", serde_json::json!({ "why": why.code() }));
            format!("the recogniser did not start: {why}")
        })?;
        let recogniser = Arc::new(recogniser);
        let (into, fed) = (taken.clone(), recogniser.clone());
        let counted = counts.clone();
        let samples = move |samples: &[f32], _: Option<f64>| {
            fed.feed(samples);
            let level = listen_meter::rms(samples);
            counted.buffers.fetch_add(1, Ordering::Relaxed);
            if level >= listen_meter::SOUND {
                counted.loud.fetch_add(1, Ordering::Relaxed);
            }
            if let Ok(mut taken) = into.lock() {
                taken.level = Some(level);
            }
        };
        let into = taken.clone();
        let stopped = move |why: String| {
            if let Ok(mut taken) = into.lock() {
                taken.failed = Some(why);
            }
        };
        let audio = AppAudio::start(pid, samples, stopped)?;
        log_start(&audio);
        let (audio, began) = (Arc::new(audio), Instant::now());
        Ok(Live { taken, audio, _recogniser: recogniser, counts, counted: (began, false), refreshed: began })
    }

    /// On the transcript's tick: every `COUNTS`, logs what the stream gave, and after the first
    /// an app that played at the start and gave no sound; every `REFRESH`, the stream to name
    /// the app's helpers again by (`refresh`).
    pub(crate) fn tick(&mut self, now: Instant) -> Option<Arc<AppAudio>> {
        if now.saturating_duration_since(self.counted.0) >= COUNTS {
            let take = |count: &AtomicUsize| count.swap(0, Ordering::Relaxed);
            // `loud` before `buffers`, the samples' order backwards: never more loud than buffers.
            let (loud, buffers, lines) =
                (take(&self.counts.loud), take(&self.counts.buffers), take(&self.counts.lines));
            let bundle = &self.audio.bundle;
            let fields = serde_json::json!({ "bundle": bundle, "buffers": buffers, "loud": loud, "lines": lines });
            event_log::event("listen_counts", fields);
            if !self.counted.1 && self.audio.playing && loud == 0 {
                let fields = serde_json::json!({ "bundle": bundle, "pids": self.audio.heard(), "buffers": buffers });
                event_log::event("listen_quiet", fields);
            }
            self.counted = (now, true);
        }
        (now.saturating_duration_since(self.refreshed) >= REFRESH).then(|| {
            self.refreshed = now;
            self.audio.clone()
        })
    }

    /// What came since the last look.
    pub(crate) fn take(&self) -> Taken {
        self.taken.lock().map(|mut taken| std::mem::take(&mut *taken)).unwrap_or_default()
    }

    /// The lines that came since the last look, the rest left for it.
    pub(crate) fn take_lines(&self) -> Vec<Heard> {
        self.taken.lock().map(|mut taken| std::mem::take(&mut taken.lines)).unwrap_or_default()
    }
}

/// Names `audio`'s app's helpers again, and logs a change. Waits on the system: off the main thread.
pub(crate) fn refresh(audio: &AppAudio) {
    if let Ok(Some(pids)) = audio.refresh() {
        event_log::event("listen_audio_filter", serde_json::json!({ "bundle": audio.bundle, "pids": pids }));
    }
}

/// A test run's ear on the app `OTTOSPACE_HOST_ONLY` names (`audio_probe_start`), with no
/// recogniser: each buffer's presentation time, length and level, to see whether an app's silence
/// comes as buffers or as none.
pub(crate) struct Probe {
    buffers: Arc<Mutex<Vec<serde_json::Value>>>,
    /// Why the system stopped the stream, if it did.
    failed: Arc<Mutex<Option<String>>>,
    _audio: AppAudio,
}

impl Probe {
    pub(crate) fn start() -> Result<Probe, String> {
        let pid = std::env::var("OTTOSPACE_HOST_ONLY").ok().and_then(|pid| pid.parse().ok());
        let pid = pid.ok_or("audio_probe_start hears only the app OTTOSPACE_HOST_ONLY names")?;
        let buffers = Arc::new(Mutex::new(Vec::new()));
        let into = buffers.clone();
        let heard = move |samples: &[f32], at: Option<f64>| {
            if let Ok(mut buffers) = into.lock() {
                let pts_ms = at.map(|at| at * 1_000.);
                let rms = listen_meter::rms(samples);
                buffers.push(serde_json::json!({ "pts_ms": pts_ms, "samples": samples.len(), "rms": rms }));
            }
        };
        let failed = Arc::new(Mutex::new(None));
        let why = failed.clone();
        let stopped = move |reason: String| {
            if let Ok(mut why) = why.lock() {
                *why = Some(reason);
            }
        };
        let audio = AppAudio::start(pid, heard, stopped)?;
        log_start(&audio);
        Ok(Probe { buffers, failed, _audio: audio })
    }

    /// Stops it: the buffers heard, in order, and why the system stopped it, if it did.
    pub(crate) fn stop(self) -> (Vec<serde_json::Value>, Option<String>) {
        let Probe { buffers, failed, _audio } = self;
        drop(_audio);
        let buffers = buffers.lock().map(|mut buffers| std::mem::take(&mut *buffers)).unwrap_or_default();
        (buffers, failed.lock().ok().and_then(|mut failed| failed.take()))
    }
}
