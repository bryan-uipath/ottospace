//! One app's audio output, by ScreenCaptureKit: a display filter with only that app, audio on,
//! ottospace's own sound left out, 16 kHz mono Float32. SCK needs a display stream, so the video
//! is 2×2 at one frame a second and dropped unread. Audio is filtered per app (its pid), not per
//! window. Screen Recording covers it ("Screen & System Audio Recording").

use std::sync::Arc;

use screencapturekit::cm::CMTime;
use screencapturekit::prelude::*;
use screencapturekit::stream::delegate_trait::SCStreamDelegateTrait;

/// The rate the recogniser is fed at.
pub const RATE: i32 = 16_000;

type Samples = Arc<dyn Fn(&[f32], Option<f64>) + Send + Sync>;
type Stopped = Arc<dyn Fn(String) + Send + Sync>;

struct Delegate(Stopped);

impl SCStreamDelegateTrait for Delegate {
    fn did_stop_with_error(&self, error: SCError) {
        (self.0)(format!("{error}"));
    }
}

/// The stream of one app's sound. Dropped: it stops.
pub struct AppAudio {
    stream: SCStream,
}

impl AppAudio {
    /// Starts hearing the app of `pid`: its samples to `samples`, on SCK's queue, with the
    /// buffer's presentation time in seconds (a host clock's, if it has one); a stop by the
    /// system (permission revoked, the app gone) to `stopped`, with why.
    pub fn start(
        pid: i32,
        samples: impl Fn(&[f32], Option<f64>) + Send + Sync + 'static,
        stopped: impl Fn(String) + Send + Sync + 'static,
    ) -> Result<AppAudio, String> {
        let content = SCShareableContent::get().map_err(|e| format!("{e}"))?;
        let display = content.displays().into_iter().next().ok_or("no display")?;
        let apps = content.applications();
        let app = apps.iter().find(|app| app.process_id() == pid).ok_or("the app is not offered for capture")?;
        let filter = SCContentFilter::create()
            .with_display(&display)
            .with_including_applications(&[app], &[])
            .build()
            .map_err(|e| format!("{e}"))?;
        let config = SCStreamConfiguration::new()
            .with_width(2)
            .with_height(2)
            .with_minimum_frame_interval(&CMTime::new(1, 1))
            .with_captures_audio(true)
            .with_excludes_current_process_audio(true)
            .with_sample_rate(RATE)
            .with_channel_count(1);
        let mut stream = SCStream::new_with_delegate(&filter, &config, Delegate(Arc::new(stopped)))
            .map_err(|e| format!("{e}"))?;
        let samples: Samples = Arc::new(samples);
        let heard = move |sample: CMSampleBuffer, kind: SCStreamOutputType| {
            if kind != SCStreamOutputType::Audio {
                return;
            }
            let at = sample.presentation_timestamp().as_seconds();
            let Ok(list) = sample.audio_buffer_list() else { return };
            for buffer in &list {
                let floats: Vec<f32> =
                    buffer.data().chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
                if !floats.is_empty() {
                    samples(&floats, at);
                }
            }
        };
        stream.add_output_handler(heard, SCStreamOutputType::Audio).map_err(|e| format!("{e}"))?;
        // The frames: taken and dropped, or SCK logs each one it has nowhere to put.
        stream.add_output_handler(|_: CMSampleBuffer, _: SCStreamOutputType| {}, SCStreamOutputType::Screen).map_err(|e| format!("{e}"))?;
        stream.start_capture().map_err(|e| format!("{e}"))?;
        Ok(AppAudio { stream })
    }
}

impl Drop for AppAudio {
    fn drop(&mut self) {
        let _ = self.stream.stop_capture();
    }
}

/// The apps that can be heard, for "Transcribe a window…": those ScreenCaptureKit offers with a
/// window on screen (each its titles, for the user's eyes), or playing sound now. A helper
/// process that plays (a browser's tab) counts for the app it was started by. Waits on the
/// system: off the main thread.
pub fn sound_apps() -> Vec<ottospace_core::listen::SoundApp> {
    let Ok(content) = SCShareableContent::get() else { return Vec::new() };
    let apps = content.applications();
    let pids: Vec<i32> = apps.iter().map(|app| app.process_id()).collect();
    // A playing pid, or the nearest of its parents that is an offered app.
    let playing: Vec<i32> = crate::meeting::playing_pids()
        .into_iter()
        .filter_map(|pid| std::iter::successors(Some(pid), |p| crate::process::parent(*p).filter(|q| *q > 1)).take(8).find(|p| pids.contains(p)))
        .collect();
    let own = std::process::id() as i32;
    let windows = content.windows();
    let mut found = Vec::new();
    for app in &apps {
        let pid = app.process_id();
        let titles: Vec<String> = windows
            .iter()
            .filter(|w| w.is_on_screen() && w.window_layer() == 0)
            .filter(|w| w.owning_application().is_some_and(|owner| owner.process_id() == pid))
            .filter_map(|w| w.title().filter(|t| !t.is_empty()))
            .collect();
        let plays = playing.contains(&pid);
        let shown = windows.iter().any(|w| {
            w.is_on_screen() && w.window_layer() == 0 && w.owning_application().is_some_and(|o| o.process_id() == pid)
        });
        if pid == own || !(plays || shown) {
            continue;
        }
        let bundle = Some(app.bundle_identifier()).filter(|b| !b.is_empty());
        found.push(ottospace_core::listen::SoundApp { pid, name: app.application_name(), bundle, windows: titles, playing: plays });
    }
    found
}

/// Plays a sound file in this process (`NSSound`): for the canary, whose own audio a test hears.
pub fn play(path: &str) -> bool {
    use objc2::AnyThread;
    use objc2_app_kit::NSSound;
    use objc2_foundation::NSString;
    let path = NSString::from_str(path);
    let sound = NSSound::initWithContentsOfFile_byReference(NSSound::alloc(), &path, false);
    let Some(sound) = sound else { return false };
    let played = sound.play();
    // Kept for as long as it plays: the canary lives the test.
    std::mem::forget(sound);
    played
}
