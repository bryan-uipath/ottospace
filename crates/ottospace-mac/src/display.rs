use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2_core_foundation::{CGPoint, CGRect};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
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
        crate::wake::post();
    }
}

/// From now on, a change of the displays is noted for `take_change`. Once per run.
pub fn watch() {
    unsafe { CGDisplayRegisterReconfigurationCallback(on_reconfigure, std::ptr::null_mut()) };
}

/// Notes a change as `watch` does: for a test, which cannot unplug a display.
pub fn note_change() {
    CHANGED.store(true, Ordering::SeqCst);
    crate::wake::post();
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
    let mut ids = [0u32; 16];
    let mut count = 0;
    unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
    ids[..count as usize].iter().map(|&id| bounds(id)).collect()
}

/// Where to park a window so only its top-left pixel is on-screen (AeroSpace-style).
/// Picks a display's bottom-right corner with no other display to its right or below,
/// e.g. the right-hand monitor's corner in a side-by-side setup.
pub fn parking_spot() -> CGPoint {
    let displays = all();
    let far = 100_000.0;
    let overlaps = |x: f64, y: f64| {
        displays.iter().any(|d| {
            d.origin.x + d.size.width > x && d.origin.y + d.size.height > y && d.origin.x < x + far && d.origin.y < y + far
        })
    };
    let corner = |d: &CGRect| CGPoint::new(d.origin.x + d.size.width - 1.0, d.origin.y + d.size.height - 1.0);
    displays
        .iter()
        .map(corner)
        .find(|p| !overlaps(p.x + 1.0, p.y) && !overlaps(p.x, p.y + 1.0))
        .unwrap_or_else(|| corner(&bounds(unsafe { CGMainDisplayID() })))
}
