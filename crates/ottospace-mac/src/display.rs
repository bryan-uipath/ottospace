use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2_core_foundation::{CGPoint, CGRect};
use ottospace_core::ax::Frame;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayCopyDisplayMode(display: u32) -> *mut c_void;
    fn CGDisplayModeGetWidth(mode: *mut c_void) -> usize;
    fn CGDisplayModeGetPixelWidth(mode: *mut c_void) -> usize;
    fn CGDisplayModeRelease(mode: *mut c_void);
    fn CGDisplayRegisterReconfigurationCallback(
        callback: extern "C" fn(u32, u32, *mut c_void),
        user_info: *mut c_void,
    ) -> i32;
}

/// Set when the displays were reconfigured (plugged, unplugged, mirrored, a lid shut), taken by `take_change`.
static CHANGED: AtomicBool = AtomicBool::new(false);

/// `kCGDisplayBeginConfigurationFlag`: the call before a change, which says nothing of the new setup.
const BEGIN: u32 = 1;

extern "C" fn on_reconfigure(_display: u32, flags: u32, _: *mut c_void) {
    if flags & BEGIN == 0 {
        CHANGED.store(true, Ordering::SeqCst);
    }
}

/// From now on, a change of the displays is noted for `take_change`. Once per run.
pub fn watch() {
    unsafe { CGDisplayRegisterReconfigurationCallback(on_reconfigure, std::ptr::null_mut()) };
}

/// Notes a change as `watch` does: for a test, which cannot unplug a display.
pub fn note_change() {
    CHANGED.store(true, Ordering::SeqCst);
}

/// True once after each change of the displays.
pub fn take_change() -> bool {
    CHANGED.swap(false, Ordering::SeqCst)
}

/// Display bounds in global top-left-origin coordinates (same space as AX).
pub fn bounds(display: u32) -> CGRect {
    unsafe { CGDisplayBounds(display) }
}

pub fn all() -> Vec<CGRect> {
    ids().into_iter().map(bounds).collect()
}

fn ids() -> Vec<u32> {
    let mut ids = [0u32; 16];
    let mut count = 0;
    unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
    ids[..count as usize].to_vec()
}

/// Pixels a point on `display`: 2 on a Retina display, 1 on most external ones.
pub fn scale(display: u32) -> f64 {
    let mode = unsafe { CGDisplayCopyDisplayMode(display) };
    if mode.is_null() {
        return 1.0;
    }
    let (points, pixels) = unsafe { (CGDisplayModeGetWidth(mode), CGDisplayModeGetPixelWidth(mode)) };
    unsafe { CGDisplayModeRelease(mode) };
    if points == 0 { 1.0 } else { pixels as f64 / points as f64 }
}

/// The scale of the display a window parked for a pane of `want` is on (`parking_spot`).
pub fn parking_scale(want: f64) -> f64 {
    let spot = parking_spot(want);
    let on = |id: &u32| {
        let b = bounds(*id);
        (b.origin.x..b.origin.x + b.size.width).contains(&spot.x)
            && (b.origin.y..b.origin.y + b.size.height).contains(&spot.y)
    };
    ids().iter().find(|id| on(id)).map_or(1.0, |id| scale(*id))
}

/// Where to park a window so only its top-left pixel is on-screen (AeroSpace-style): a display's
/// bottom-right corner with no other display to its right or below, e.g. the right-hand monitor's
/// in a side-by-side setup; of those, one with `want` pixels a point, the scale its pane is drawn
/// at (`ottospace_core::hosted::parking_corner`). `f64::INFINITY`: the sharpest.
pub fn parking_spot(want: f64) -> CGPoint {
    let displays: Vec<(Frame, f64)> = ids()
        .into_iter()
        .map(|id| {
            let b = bounds(id);
            (Frame::new(b.origin.x, b.origin.y, b.size.width, b.size.height), scale(id))
        })
        .collect();
    match ottospace_core::hosted::parking_corner(&displays, want) {
        Some((x, y)) => CGPoint::new(x, y),
        None => {
            let main = bounds(unsafe { CGMainDisplayID() });
            CGPoint::new(main.origin.x + main.size.width - 1.0, main.origin.y + main.size.height - 1.0)
        }
    }
}
