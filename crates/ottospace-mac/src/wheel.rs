//! A scroll that reached ottospace, sent again as real (HID-level) input once the app's window
//! it was for is up under the pointer. Posting to the app's process is ignored by Chrome.

use objc2_core_foundation::CGPoint;
use objc2_core_graphics::{CGEvent, CGEventTapLocation, CGEventType, CGMouseButton, CGScrollEventUnit};

/// Where the pointer is now: global, top-left origin.
pub fn pointer() -> CGPoint {
    CGEvent::new(None).map_or(CGPoint::ZERO, |event| CGEvent::location(Some(&event)))
}

/// A scroll of `dx, dy` points at `at`, to the window under it. A move there first: without it the
/// window server sent a scroll again to ottospace's window, under the app's, on CI.
pub fn scroll(at: CGPoint, dx: i32, dy: i32) {
    let moved = CGEvent::new_mouse_event(None, CGEventType::MouseMoved, at, CGMouseButton::Left);
    CGEvent::post(CGEventTapLocation::HIDEventTap, moved.as_deref());
    let event = CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Pixel, 2, dy, dx, 0);
    CGEvent::set_location(event.as_deref(), at);
    CGEvent::post(CGEventTapLocation::HIDEventTap, event.as_deref());
}
