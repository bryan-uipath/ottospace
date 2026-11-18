//! One app's audio output, by ScreenCaptureKit: a display filter with only that app and the
//! helpers it started, of its bundle id, that SCK offers as apps, audio on, ottospace's own sound left out, 16 kHz
//! mono Float32. SCK needs a display stream, so the video is 2×2 at one frame a second and
//! dropped unread. Audio is filtered per app (its pid), not per window. Screen Recording covers
//! it ("Screen & System Audio Recording").

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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

/// How far up a process's parents an app is looked for.
const DEPTH: usize = 8;

/// `pid` and its parents, nearest first, up to `DEPTH`; never launchd (1).
fn lineage(pid: i32, parent: impl Fn(i32) -> Option<i32>) -> impl Iterator<Item = i32> {
    std::iter::successors(Some(pid), move |p| parent(*p).filter(|q| *q > 1 && q != p)).take(DEPTH)
}

/// Whether `helper`'s bundle id is `app`'s own or under it (`us.zoom.xos.aomhost` of
/// `us.zoom.xos`). Neither empty: a browser a terminal's shell started is not the terminal's.
fn same_maker(helper: &str, app: &str) -> bool {
    !app.is_empty() && helper.strip_prefix(app).is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

/// The pids a stream of `chosen` hears: it, then each `offered` (pid, bundle id) it started,
/// directly or not, of its own bundle id or one under it, in order. Zoom (89087) with
/// `us.zoom.xos.aomhost` (its child) offered: `[89087, aomhost's pid]`.
pub(crate) fn included(offered: &[(i32, &str)], parent: impl Fn(i32) -> Option<i32> + Copy, chosen: i32) -> Vec<i32> {
    let bundle = offered.iter().find(|(pid, _)| *pid == chosen).map_or("", |(_, bundle)| bundle);
    let mut helpers: Vec<i32> = offered
        .iter()
        .filter(|(pid, helper)| *pid != chosen && same_maker(helper, bundle) && lineage(*pid, parent).any(|p| p == chosen))
        .map(|(pid, _)| *pid)
        .collect();
    // SCK's list has no order: the same helpers are the same filter.
    helpers.sort_unstable();
    std::iter::once(chosen).chain(helpers).collect()
}

/// The stream of one app's sound. Dropped: it stops.
pub struct AppAudio {
    stream: SCStream,
    pid: i32,
    /// The pids its filter names now, `pid` first. Held through a refresh: one at a time.
    pids: Mutex<Vec<i32>>,
    /// How many: read with no wait on a refresh.
    count: AtomicUsize,
    stopped: AtomicBool,
    /// The app's bundle id, empty if it has none.
    pub bundle: String,
    /// It or a process it started played sound when the stream started.
    pub playing: bool,
    /// Processes it started that played then but SCK does not offer: their sound is not heard.
    pub unoffered: usize,
}

/// What SCK offers now: a display and its apps.
fn offered() -> Result<(SCDisplay, Vec<SCRunningApplication>), String> {
    let content = SCShareableContent::get().map_err(|e| format!("{e}"))?;
    let display = content.displays().into_iter().next().ok_or("no display")?;
    Ok((display, content.applications()))
}

/// The pids a stream of `pid` hears among `apps`.
fn included_of(apps: &[SCRunningApplication], pid: i32) -> Vec<i32> {
    let bundles: Vec<(i32, String)> = apps.iter().map(|app| (app.process_id(), app.bundle_identifier())).collect();
    let all: Vec<(i32, &str)> = bundles.iter().map(|(pid, bundle)| (*pid, bundle.as_str())).collect();
    included(&all, crate::process::parent, pid)
}

/// The filter that hears `pids` of `apps`.
fn filter_of(display: &SCDisplay, apps: &[SCRunningApplication], pids: &[i32]) -> Result<SCContentFilter, String> {
    let heard: Vec<&SCRunningApplication> = pids.iter().filter_map(|p| apps.iter().find(|app| app.process_id() == *p)).collect();
    SCContentFilter::create().with_display(display).with_including_applications(&heard, &[]).build().map_err(|e| format!("{e}"))
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
        let (display, apps) = offered()?;
        let app = apps.iter().find(|app| app.process_id() == pid).ok_or("the app is not offered for capture")?;
        let bundle = app.bundle_identifier();
        let pids = included_of(&apps, pid);
        let filter = filter_of(&display, &apps, &pids)?;
        // Played now by it or its children, and of those not offered: whose sound goes unheard.
        let parent = crate::process::parent;
        let played: Vec<i32> =
            crate::meeting::playing_pids().into_iter().filter(|p| lineage(*p, parent).any(|q| q == pid)).collect();
        let unoffered = played.iter().filter(|p| !pids.contains(p)).count();
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
        let (count, stopped) = (AtomicUsize::new(pids.len()), AtomicBool::new(false));
        let playing = !played.is_empty();
        Ok(AppAudio { stream, pid, pids: Mutex::new(pids), count, stopped, bundle, playing, unoffered })
    }

    /// How many pids its filter names.
    pub fn heard(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    /// Stops it now, though a refresh still holds it.
    pub fn stop(&self) {
        if !self.stopped.swap(true, Ordering::Relaxed) {
            let _ = self.stream.stop_capture();
        }
    }

    /// Names the app's helpers again (Zoom starts `aomhost` when a meeting starts): `Some(count)`
    /// when they changed and the filter now names `count` pids. Waits on the system: off the main thread.
    pub fn refresh(&self) -> Result<Option<usize>, String> {
        let mut pids = self.pids.lock().map_err(|_| "poisoned")?;
        let (display, apps) = offered()?;
        let now = included_of(&apps, self.pid);
        // The app itself gone: its stream stops by itself. Stopped meanwhile: left stopped.
        let gone = !apps.iter().any(|app| app.process_id() == self.pid);
        if gone || now == *pids || self.stopped.load(Ordering::Relaxed) {
            return Ok(None);
        }
        self.stream.update_content_filter(&filter_of(&display, &apps, &now)?).map_err(|e| format!("{e}"))?;
        *pids = now;
        self.count.store(pids.len(), Ordering::Relaxed);
        Ok(Some(pids.len()))
    }
}

impl Drop for AppAudio {
    fn drop(&mut self) {
        self.stop();
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
        .filter_map(|pid| lineage(pid, crate::process::parent).find(|p| pids.contains(p)))
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

/// Plays a sound file as an app ScreenCaptureKit offers (one with a window, never shown), and
/// returns when it ends: for the canary's child, a helper like those Teams or Zoom start.
pub fn play_as_app(path: &str) -> bool {
    use objc2::AnyThread;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSBackingStoreType, NSSound, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    let Some(main) = MainThreadMarker::new() else { return false };
    let app = NSApplication::sharedApplication(main);
    app.finishLaunching();
    // A window the server knows, never shown: SCK offers no process without one.
    let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(10., 10.));
    let style = NSWindowStyleMask::Borderless;
    let _window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(main.alloc(), frame, style, NSBackingStoreType::Buffered, false)
    };
    let sound = NSSound::initWithContentsOfFile_byReference(NSSound::alloc(), &NSString::from_str(path), false);
    let Some(sound) = sound.filter(|sound| sound.play()) else { return false };
    std::thread::sleep(std::time::Duration::from_secs_f64(sound.duration() + 0.5));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn parents(pairs: &[(i32, i32)]) -> HashMap<i32, i32> {
        pairs.iter().copied().collect()
    }

    const ZOOM: &str = "us.zoom.xos";

    #[test]
    fn a_zoom_like_tree_names_its_offered_children() {
        // zoom.us 89087; aomhost ×2, caphost and a Cef helper its children; another app beside it.
        let map = parents(&[(89087, 1), (96341, 89087), (97986, 89087), (90952, 89087), (28884, 89087), (500, 1)]);
        let aom = "us.zoom.xos.aomhost";
        let offered = [(500, "com.other"), (89087, ZOOM), (96341, aom), (97986, aom), (90952, "us.zoom.xos.caphost")];
        assert_eq!(included(&offered, |p| map.get(&p).copied(), 89087), [89087, 90952, 96341, 97986]);
    }

    #[test]
    fn grandchildren_are_named() {
        let map = parents(&[(10, 1), (11, 10), (12, 11), (13, 12)]);
        let offered = [(10, ZOOM), (13, "us.zoom.xos.b"), (12, "us.zoom.xos.a")];
        assert_eq!(included(&offered, |p| map.get(&p).copied(), 10), [10, 12, 13]);
    }

    #[test]
    fn an_app_with_no_children_is_named_alone() {
        let map = parents(&[(10, 1), (20, 1)]);
        assert_eq!(included(&[(10, ZOOM), (20, "us.zoom.xos.x")], |p| map.get(&p).copied(), 10), [10]);
        // Not offered at all: still named, the caller finds it.
        assert_eq!(included(&[], |p| map.get(&p).copied(), 10), [10]);
    }

    #[test]
    fn an_unrelated_app_of_the_same_name_is_not_named() {
        // Two `zoom.us`: 30 and 40, each launched by launchd; 41 is 40's child.
        let map = parents(&[(30, 1), (40, 1), (41, 40)]);
        let offered = [(30, ZOOM), (40, ZOOM), (41, "us.zoom.xos.aomhost")];
        assert_eq!(included(&offered, |p| map.get(&p).copied(), 30), [30]);
    }

    #[test]
    fn a_child_of_another_maker_or_of_none_is_not_named() {
        // iTerm 10; Chrome its shell's child; a helper of no bundle id; a look-alike prefix; a
        // second iTerm it started.
        let map = parents(&[(10, 1), (11, 10), (12, 11), (13, 10), (14, 10), (15, 10)]);
        let iterm = "com.googlecode.iterm2";
        let offered = [(10, iterm), (12, "com.google.Chrome"), (13, ""), (14, "com.googlecode.iterm2x"), (15, iterm)];
        assert_eq!(included(&offered, |p| map.get(&p).copied(), 10), [10, 15]);
        // An app of no bundle id names no helper.
        assert_eq!(included(&[(10, ""), (13, "")], |p| map.get(&p).copied(), 10), [10]);
    }

    #[test]
    fn helpers_in_another_order_are_the_same() {
        let map = parents(&[(10, 1), (11, 10), (12, 10)]);
        let of = |offered: &[(i32, &str)]| included(offered, |p| map.get(&p).copied(), 10);
        let (a, b) = ((11, "us.zoom.xos.a"), (12, "us.zoom.xos.b"));
        assert_eq!(of(&[b, (10, ZOOM), a]), of(&[a, b, (10, ZOOM)]));
    }

    #[test]
    fn a_parent_loop_ends() {
        // A pid reused while read: 50 and 51 each other's parent.
        let map = parents(&[(50, 51), (51, 50), (60, 1)]);
        assert_eq!(included(&[(50, ZOOM), (51, ZOOM)], |p| map.get(&p).copied(), 60), [60]);
    }
}
