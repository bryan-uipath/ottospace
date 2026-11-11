//! Step 1: does a window parked on a 1px sliver keep rendering, as seen by ScreenCaptureKit?
//! Usage: probe <AppName> [seconds-per-phase]
//! Runs three phases (on-screen → sliver → restored) and reports frames/sec by status.
//! Point the app at animating content (a video, a CSS animation) so "complete" frames are expected.

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

use ottospace_mac::{ax, capture, display};
use screencapturekit::cm::{CMSampleBufferSCExt, SCFrameStatus};
use screencapturekit::prelude::*;

#[derive(Default)]
struct Counts {
    complete: AtomicU64,
    idle: AtomicU64,
    other: AtomicU64,
}

impl Counts {
    fn take(&self) -> [u64; 3] {
        [&self.complete, &self.idle, &self.other].map(|c| c.swap(0, Relaxed))
    }
}

struct Handler(Arc<Counts>);

impl SCStreamOutputTrait for Handler {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, _: SCStreamOutputType) {
        let counter = match sample.frame_status() {
            Some(SCFrameStatus::Complete) => &self.0.complete,
            Some(SCFrameStatus::Idle) => &self.0.idle,
            _ => &self.0.other,
        };
        counter.fetch_add(1, Relaxed);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let app_name = args.next().ok_or("usage: probe <AppName> [seconds-per-phase]")?;
    let secs: u64 = args.next().map(|s| s.parse()).transpose()?.unwrap_or(5);

    capture::init();
    if !ax::is_trusted(true) {
        return Err("grant Accessibility access to your terminal, then rerun".into());
    }
    let capture::Target { window, sc_window, id: window_id } = capture::find(&app_name)?;
    let original = window.frame().ok_or("could not read window frame")?;

    let counts = Arc::new(Counts::default());
    let filter = SCContentFilter::create().with_window(&sc_window).build()?;
    let config = SCStreamConfiguration::new()
        .with_width(original.size.width as u32)
        .with_height(original.size.height as u32)
        .with_fps(60);
    let mut stream = SCStream::new(&filter, &config)?;
    stream.add_output_handler(Handler(counts.clone()), SCStreamOutputType::Screen)?;
    stream.start_capture()?;

    // Ctrl-C ends the current phase early; the window is always restored below.
    let abort = Arc::new(AtomicBool::new(false));
    let flag = abort.clone();
    ctrlc::set_handler(move || flag.store(true, Relaxed))?;

    let sliver = display::parking_spot();
    let phases = [("on-screen", None), ("sliver", Some(sliver)), ("restored", Some(original.origin))];

    println!("{app_name} window {window_id}, {}s per phase", secs);
    println!("{:<10} {:>14} {:>10} {:>10} {:>10}", "phase", "position", "complete/s", "idle/s", "other/s");
    for (name, target) in phases {
        if abort.load(Relaxed) {
            break;
        }
        let at = match target {
            Some(p) => window.set_position(p).map_err(|e| format!("move failed: {e:?}"))?,
            None => original.origin,
        };
        counts.take();
        let elapsed = wait(Duration::from_secs(secs), &abort).as_secs_f64();
        let [complete, idle, other] = counts.take().map(|n| n as f64 / elapsed);
        let pos = format!("({:.0},{:.0})", at.x, at.y);
        println!("{name:<10} {pos:>14} {complete:>10.1} {idle:>10.1} {other:>10.1}");
    }

    stream.stop_capture()?;
    window.set_position(original.origin).map_err(|e| format!("restore failed: {e:?}"))?;
    Ok(())
}

/// Sleeps up to `dur`, returning early if `abort` is set. Returns time actually waited.
fn wait(dur: Duration, abort: &AtomicBool) -> Duration {
    let start = Instant::now();
    while start.elapsed() < dur && !abort.load(Relaxed) {
        std::thread::sleep(Duration::from_millis(50));
    }
    start.elapsed()
}
