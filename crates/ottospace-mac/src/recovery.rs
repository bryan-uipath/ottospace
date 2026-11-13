//! Puts other apps' windows back on screen: a hosted window at its original frame, read back to
//! be sure, and a window stranded off screen moved onto the main screen. The decisions are the
//! core's (`ottospace_core::restore`); this reads screens and moves windows.

use std::ffi::c_void;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplicationActivationPolicy, NSScreen, NSWorkspace};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use ottospace_core::ax::Frame;
use ottospace_core::restore;

use crate::{ax, display};

pub fn to_frame(r: CGRect) -> Frame {
    Frame::new(r.origin.x, r.origin.y, r.size.width, r.size.height)
}

pub fn to_rect(f: Frame) -> CGRect {
    CGRect { origin: CGPoint::new(f.x, f.y), size: CGSize::new(f.w, f.h) }
}

/// The screens now, in Accessibility's coordinates.
pub struct Screens {
    /// Each screen's visible frame: below the menu bar, beside the Dock.
    pub visible: Vec<Frame>,
    /// Each display's whole bounds.
    pub displays: Vec<Frame>,
    /// The visible frame of the screen with the menu bar.
    pub main: Frame,
    /// Where ottospace parks a window (`display::parking_spot`).
    pub spot: (f64, f64),
}

pub fn screens() -> Screens {
    let displays: Vec<Frame> = display::all().into_iter().map(to_frame).collect();
    // AppKit counts from the main screen's bottom left; Accessibility from its top left.
    let visible: Vec<Frame> = match MainThreadMarker::new() {
        Some(mtm) => {
            let all = NSScreen::screens(mtm);
            let height = all.firstObject().map_or(0.0, |s| s.frame().size.height);
            all.iter()
                .map(|s| {
                    let v = s.visibleFrame();
                    Frame::new(v.origin.x, height - v.origin.y - v.size.height, v.size.width, v.size.height)
                })
                .collect()
        }
        // Off the main thread AppKit cannot be asked: whole displays, less a menu bar's height.
        None => displays.iter().map(|d| Frame::new(d.x, d.y + 25.0, d.w, d.h - 25.0)).collect(),
    };
    let fallback = Frame::new(0.0, 25.0, 1280.0, 775.0);
    let main = visible.first().copied().unwrap_or(fallback);
    let spot = display::parking_spot();
    Screens { visible, displays, main, spot: (spot.x, spot.y) }
}

/// Moves the window to `frame`: position, size, position again. Position first: resized at the
/// parking spot, macOS clamps the window to what fits there. Again after: an app may shift a
/// window as it resizes it.
fn place(window: &ax::Window, frame: Frame) {
    let rect = to_rect(frame);
    let _ = window.set_position(rect.origin);
    let _ = window.set_size(rect.size);
    let _ = window.set_position(rect.origin);
}

/// Puts `window` back at `original`, or centred on the main screen when the user could not reach
/// it there, and reads it back. `None`: it is not where the user can reach it, even after a
/// second try in the middle of the main screen (the app refused, or the window is gone).
pub fn put_back(window: &ax::Window, original: Frame, screens: &Screens) -> Option<Frame> {
    let target = restore::destination(original, &screens.visible, screens.main);
    place(window, target);
    let landed = to_frame(window.frame()?);
    if restore::reachable(landed, &screens.visible) {
        return Some(landed);
    }
    eprintln!("window {:?}: put at {target:?}, it is at {landed:?}; centring it", window.id());
    place(window, restore::centred(target, screens.main));
    window.frame().map(to_frame).filter(|f| restore::reachable(*f, &screens.visible))
}

/// True when the user cannot get at the window: it sits in ottospace's parking spot, or wholly
/// off every display.
pub fn stranded(frame: Frame, screens: &Screens) -> bool {
    restore::parked(frame, screens.spot) || restore::off_every(frame, &screens.displays)
}

/// The app's standard windows that are stranded, with their frames.
pub fn stranded_of(pid: i32, screens: &Screens) -> Vec<(ax::Window, Frame)> {
    ax::Window::standard_of(pid)
        .into_iter()
        .filter_map(|w| {
            let frame = to_frame(w.frame()?);
            stranded(frame, screens).then_some((w, frame))
        })
        .collect()
}

/// Moves every stranded standard window of every running app (of `only`, if named) onto the
/// main screen at its own size, no larger than the screen. Leaves visible windows alone, and
/// those `skip` names (pid, window id): another ottospace's panes. Returns how many it moved.
pub fn rescue(only: Option<i32>, skip: impl Fn(i32, u32) -> bool) -> usize {
    let screens = screens();
    let me = std::process::id() as i32;
    let pids: Vec<i32> = NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter(|app| app.activationPolicy() == NSApplicationActivationPolicy::Regular && !app.isTerminated())
        .map(|app| app.processIdentifier())
        .filter(|pid| *pid != me && only.is_none_or(|only| only == *pid))
        .collect();
    let mut moved = 0;
    for pid in pids {
        for (window, frame) in stranded_of(pid, &screens) {
            if window.id().is_some_and(|id| skip(pid, id)) {
                continue;
            }
            place(&window, restore::centred(frame, screens.main));
            moved += 1;
        }
    }
    moved
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFRelease(object: *const c_void);
}

/// True while the window server has a window with this id, on screen or not. Asks the window
/// server, not the app: a busy app that times out an Accessibility call still has its window.
pub fn window_exists(id: u32) -> bool {
    // kCGWindowListOptionIncludingWindow: just that window.
    let list = unsafe { CGWindowListCopyWindowInfo(1 << 3, id) };
    if list.is_null() {
        return false;
    }
    let count = unsafe { CFArrayGetCount(list) };
    unsafe { CFRelease(list) };
    count > 0
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_window_id_nobody_has_does_not_exist() {
        assert!(!super::window_exists(u32::MAX));
    }
}
