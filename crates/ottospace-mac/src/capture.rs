//! ScreenCaptureKit helpers: locate an app's window for both AX control and capture.

use screencapturekit::prelude::*;

use crate::ax;

/// Must run before any SCK call in a process without AppKit set up (else CGS_REQUIRE_INIT aborts).
pub fn init() {
    unsafe extern "C" {
        fn sc_initialize_core_graphics();
    }
    unsafe { sc_initialize_core_graphics() };
}

/// Names of running apps that have a sizeable normal-layer on-screen window, sorted. Excludes this process.
pub fn apps() -> Vec<String> {
    let Some(snapshot) = SCShareableContent::get().ok().and_then(|c| c.snapshot()) else {
        return Vec::new();
    };
    let me = std::process::id() as i32;
    let mut names: Vec<String> = snapshot
        .windows
        .iter()
        .filter(|w| w.window_layer == 0 && w.is_on_screen && w.frame.size.width > 200.0 && w.frame.size.height > 200.0)
        .filter_map(|w| snapshot.applications.get(w.owning_app_index?))
        .filter(|a| a.process_id != me && !a.application_name.is_empty())
        .map(|a| a.application_name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

pub struct Target {
    pub window: ax::Window,
    pub sc_window: SCWindow,
    pub id: u32,
}

/// Finds `app_name`'s main window via AX, plus its matching SCK window. Prompts for Screen Recording on first use.
pub fn find(app_name: &str) -> Result<Target, String> {
    let content = SCShareableContent::get().map_err(|e| format!("{e:?}"))?;
    let pid = content
        .applications()
        .iter()
        .find(|a| a.application_name() == app_name)
        .map(|a| a.process_id())
        .ok_or_else(|| format!("{app_name} is not running"))?;
    target(ax::Window::main_of(pid).ok_or("no standard main window")?)
}

/// Pairs an AX window with its SCK window (matched by CGWindowID).
pub fn target(window: ax::Window) -> Result<Target, String> {
    let id = window.id().ok_or("could not resolve CGWindowID")?;
    let content = SCShareableContent::get().map_err(|e| format!("{e:?}"))?;
    let sc_window = content
        .windows()
        .into_iter()
        .find(|w| w.window_id() == id)
        .ok_or("window not shareable")?;
    Ok(Target { window, sc_window, id })
}

/// The window's corner radius in points, measured from one alpha screenshot.
/// Along the top pixel row the corner is transparent for `r - √r` pixels (circle of radius r,
/// row centre at y≈½), so solving the measured run for r gives the radius.
pub fn corner_radius(window: &SCWindow, scale: f64) -> Option<f64> {
    use screencapturekit::cm::CMSampleBufferExt;
    use screencapturekit::screenshot_manager::SCScreenshotManager;

    let filter = SCContentFilter::create().with_window(window).build().ok()?;
    let frame = window.frame();
    let config = SCStreamConfiguration::new()
        .with_width((frame.size.width * scale) as u32)
        .with_height((frame.size.height * scale) as u32)
        .with_pixel_format(PixelFormat::BGRA);
    let sample = SCScreenshotManager::capture_sample_buffer(&filter, &config).ok()?;
    let buffer = sample.pixel_buffer()?;
    let guard = buffer.lock_read_only().ok()?;
    let base = guard.base_address();
    let width = (frame.size.width * scale) as usize;
    if base.is_null() || width == 0 {
        return None;
    }
    // Only the pixels of the top row: `bytes_per_row` also counts padding.
    let row = unsafe { std::slice::from_raw_parts(base, (width * 4).min(guard.bytes_per_row())) };
    let run = row.chunks_exact(4).take_while(|bgra| bgra[3] < 128).count();
    // A run over a quarter of the row is not a corner (e.g. a fully transparent top row).
    // Masking with it would cover the pane, so report no corner.
    if run * 4 > width {
        return Some(0.0);
    }
    let run = run as f64;
    let sqrt_r = (1.0 + (1.0 + 4.0 * run).sqrt()) / 2.0;
    Some(if run == 0.0 { 0.0 } else { sqrt_r * sqrt_r / scale })
}
