//! What a peek of a parked app window reads of the system's input, never watched: where the
//! pointer is, and how long ago the last scroll, click or key came, whichever app it went to.

use std::time::Duration;

use objc2_core_foundation::CGPoint;
use objc2_core_graphics::{CGEvent, CGEventSource, CGEventSourceStateID, CGEventType};

/// Where the pointer is now: global, top-left origin.
pub fn pointer() -> CGPoint {
    CGEvent::new(None).map_or(CGPoint::ZERO, |event| CGEvent::location(Some(&event)))
}

/// How long ago the last of these kinds of event came.
fn since(kinds: &[CGEventType]) -> Option<Duration> {
    let ago = |kind| CGEventSource::seconds_since_last_event_type(CGEventSourceStateID::HIDSystemState, kind);
    let ago = kinds.iter().copied().map(ago).fold(f64::INFINITY, f64::min);
    (ago.is_finite() && ago >= 0.0).then(|| Duration::from_secs_f64(ago))
}

/// How long ago the last click or key press came.
pub fn since_press() -> Option<Duration> {
    since(&[CGEventType::LeftMouseDown, CGEventType::RightMouseDown, CGEventType::OtherMouseDown, CGEventType::KeyDown])
}

/// How long ago the last scroll came.
pub fn since_scroll() -> Option<Duration> {
    since(&[CGEventType::ScrollWheel])
}
