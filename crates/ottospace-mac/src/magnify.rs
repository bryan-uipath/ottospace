//! Trackpad pinches over ottospace's own windows: GPUI 0.2.2 delivers none. A local NSEvent
//! monitor of `magnify` events only, each passed on unchanged; it sees nothing of other apps.

use std::ptr::NonNull;
use std::sync::Mutex;

use block2::RcBlock;
use objc2_app_kit::{NSEvent, NSEventMask};

/// One pinch step: the window's number, where in it (points from its content's top left), and
/// how much nearer (0.02: 2% nearer; negative: farther).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pinch {
    pub window: isize,
    pub x: f64,
    pub y: f64,
    pub magnification: f64,
}

static PINCHES: Mutex<Vec<Pinch>> = Mutex::new(Vec::new());

/// Starts listening. Call once, on the main thread. Errors when AppKit gives no monitor.
pub fn listen() -> Result<(), String> {
    let block = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
        let seen = unsafe { event.as_ref() };
        // A local monitor's handler runs on the main thread.
        if let Some(window) = objc2::MainThreadMarker::new().and_then(|mtm| seen.window(mtm)) {
            // AppKit's origin is the bottom left: from the content's top.
            let height = window.contentView().map_or(0., |view| view.frame().size.height);
            let at = seen.locationInWindow();
            let pinch = Pinch { window: window.windowNumber(), x: at.x, y: height - at.y, magnification: seen.magnification() };
            if let Ok(mut pinches) = PINCHES.lock() {
                pinches.push(pinch);
            }
            crate::wake::post();
        }
        event.as_ptr()
    });
    let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::Magnify, &block) };
    // Kept for the app's life: the monitor is never removed.
    std::mem::forget(monitor.ok_or("AppKit gave no event monitor")?);
    Ok(())
}

/// The pinch steps seen since the last call, oldest first.
pub fn take() -> Vec<Pinch> {
    PINCHES.lock().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default()
}
