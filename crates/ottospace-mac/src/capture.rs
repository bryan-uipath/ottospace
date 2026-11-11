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
