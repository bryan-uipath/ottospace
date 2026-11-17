//! A transcript's real sound: the source pane's app's audio by ScreenCaptureKit
//! (`ottospace_mac::app_audio`), fed to Apple's on-device recogniser (`ottospace_mac::speech`).
//! What the recogniser says and the sound's level wait here, on SCK's and the recogniser's
//! threads, until the shell's next frame takes them (`listen.rs`): lines in its fixed-tick batch,
//! the level for the meter alone.
//!
//! In a test run only the app `OTTOSPACE_HOST_ONLY` names (the canary) is ever heard, and the
//! speech model is never downloaded.

use std::sync::{Arc, Mutex};

use ottospace_core::listen::{Heard, Speaker};
use ottospace_mac::app_audio::{self, AppAudio};
use ottospace_mac::speech::{self, Recogniser, Status};

use crate::listen_meter;

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
    _audio: AppAudio,
    _recogniser: Arc<Recogniser>,
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
        let heard = move |said: speech::Heard| {
            let Ok(mut taken) = into.lock() else { return };
            match said {
                speech::Heard::Line { text, start_ms, is_final, .. } => {
                    taken.lines.push(Heard { from: Speaker::Them, start_ms, text, is_final })
                }
                speech::Heard::Failed(why) => taken.failed = Some(format!("the recogniser: {why}")),
                speech::Heard::Finished => {}
            }
        };
        let rate = f64::from(app_audio::RATE);
        let recogniser = Arc::new(Recogniser::new(None, rate, heard).ok_or("the recogniser did not start")?);
        let (into, fed) = (taken.clone(), recogniser.clone());
        let samples = move |samples: &[f32], _: Option<f64>| {
            fed.feed(samples);
            if let Ok(mut taken) = into.lock() {
                taken.level = Some(listen_meter::rms(samples));
            }
        };
        let into = taken.clone();
        let stopped = move |why: String| {
            if let Ok(mut taken) = into.lock() {
                taken.failed = Some(why);
            }
        };
        let audio = AppAudio::start(pid, samples, stopped)?;
        Ok(Live { taken, _audio: audio, _recogniser: recogniser })
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

/// A test run's ear on the app `OTTOSPACE_HOST_ONLY` names (`audio_probe_start`), with no
/// recogniser: each buffer's presentation time, length and level, to see whether an app's silence
/// comes as buffers or as none.
pub(crate) struct Probe {
    buffers: Arc<Mutex<Vec<serde_json::Value>>>,
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
        let audio = AppAudio::start(pid, heard, |_| {})?;
        Ok(Probe { buffers, _audio: audio })
    }

    /// Stops it: the buffers heard, in order.
    pub(crate) fn stop(self) -> Vec<serde_json::Value> {
        let Probe { buffers, _audio } = self;
        drop(_audio);
        buffers.lock().map(|mut buffers| std::mem::take(&mut *buffers)).unwrap_or_default()
    }
}
