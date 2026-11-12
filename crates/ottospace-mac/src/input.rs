//! Real (HID-level) clicks. Used to replay the click that swapped a window in, once the real
//! window is under the cursor. (Posting events straight to a background app's process is ignored
//! by Chrome even on-screen, so forwarding input to parked windows isn't viable.)

use std::ffi::c_void;
use std::ptr::null;

use objc2_core_foundation::CGPoint;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreateMouseEvent(source: *const c_void, ty: u32, at: CGPoint, button: u32) -> *mut c_void;
    fn CGEventSetIntegerValueField(event: *mut c_void, field: u32, value: i64);
    fn CGEventPost(tap: u32, event: *mut c_void);
    fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
}

/// Whether the system reads the left mouse button as down now, whichever app its press went to.
/// A test asks, to know that a release it posted was taken.
pub fn left_button_down() -> bool {
    const COMBINED_SESSION: i32 = 0;
    const LEFT: u32 = 0;
    unsafe { CGEventSourceButtonState(COMBINED_SESSION, LEFT) }
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

/// Left click (down + up) at a global top-left-origin point, delivered like real hardware input.
pub fn click(at: CGPoint, clicks: i64) {
    const LEFT_DOWN: u32 = 1;
    const LEFT_UP: u32 = 2;
    const CLICK_STATE: u32 = 1;
    const HID_TAP: u32 = 0;
    for ty in [LEFT_DOWN, LEFT_UP] {
        unsafe {
            let event = CGEventCreateMouseEvent(null(), ty, at, 0);
            if event.is_null() {
                continue;
            }
            CGEventSetIntegerValueField(event, CLICK_STATE, clicks);
            CGEventPost(HID_TAP, event);
            CFRelease(event);
        }
    }
}
