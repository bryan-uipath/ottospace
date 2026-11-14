//! The user's scroll that brought a parked app window up over its pane (a peek). Once the
//! window is under the pointer, that one scroll is handed on: posted again where the user's
//! hardware put it, so macOS gives it to the window now there. Only a scroll the system queued
//! is handed on, never one made here.

use std::time::Duration;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSEvent, NSEventType};
use objc2_core_foundation::{CFRetained, CGPoint};
use objc2_core_graphics::{CGEvent, CGEventField, CGEventSource, CGEventSourceStateID, CGEventTapLocation, CGEventType};

/// Marks a scroll this process handed on, so it is not taken for the user's next one.
const HANDED_ON: i64 = 0x6f74_746f;

/// One scroll as the device sent it: its phases, momentum and deltas.
pub struct Scroll(CFRetained<CGEvent>);

/// The scroll AppKit is handling now, if it is one.
fn current_scroll() -> Option<Retained<NSEvent>> {
    let event = NSApplication::sharedApplication(MainThreadMarker::new()?).currentEvent()?;
    // The type first: asking another kind of event for its deltas raises.
    (event.r#type() == NSEventType::ScrollWheel).then_some(event)
}

impl Scroll {
    /// The scroll AppKit is handling now, copied whole, if its deltas are `delta` (`[dx, dy]` as
    /// the view was handed them). `None` for another event: a test's, handed to a view and never
    /// queued, leaves AppKit's current event as it was.
    pub fn current(delta: [f64; 2]) -> Option<Self> {
        let event = current_scroll()?;
        let same = |a: f64, b: f64| (a - b).abs() < 0.01;
        if !same(event.scrollingDeltaX(), delta[0]) || !same(event.scrollingDeltaY(), delta[1]) {
            return None;
        }
        CGEvent::new_copy(Some(&*event.CGEvent()?)).map(Self)
    }

    /// Posts it again at the system's input level, where it was (the pointer is there): the
    /// window under the pointer gets it, as it gets the user's next scroll.
    pub fn hand_on(&self) {
        let event = Some(&*self.0);
        CGEvent::set_integer_value_field(event, CGEventField::EventSourceUserData, HANDED_ON);
        // It still names ottospace's window as the one under the pointer: the system finds it anew.
        for field in [
            CGEventField::MouseEventWindowUnderMousePointer,
            CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent,
        ] {
            CGEvent::set_integer_value_field(event, field, 0);
        }
        CGEvent::post(CGEventTapLocation::HIDEventTap, event);
    }
}

/// True if the scroll AppKit is handling now is one this process handed on, come back.
pub fn handed_on() -> bool {
    let read = |event: &CGEvent| CGEvent::integer_value_field(Some(event), CGEventField::EventSourceUserData);
    current_scroll().and_then(|event| event.CGEvent()).is_some_and(|event| read(&event) == HANDED_ON)
}

/// Where the pointer is now: global, top-left origin.
pub fn pointer() -> CGPoint {
    CGEvent::new(None).map_or(CGPoint::ZERO, |event| CGEvent::location(Some(&event)))
}

/// How long ago the last click or key press came, whichever app it went to. Read, never watched.
pub fn since_press() -> Option<Duration> {
    let ago = |kind| CGEventSource::seconds_since_last_event_type(CGEventSourceStateID::HIDSystemState, kind);
    let kinds = [CGEventType::LeftMouseDown, CGEventType::RightMouseDown, CGEventType::KeyDown];
    let ago = kinds.into_iter().map(ago).fold(f64::INFINITY, f64::min);
    (ago.is_finite() && ago >= 0.0).then(|| Duration::from_secs_f64(ago))
}

/// How long ago the last scroll came, whichever window it went to. Read, never watched.
pub fn since_scroll() -> Option<Duration> {
    let ago = CGEventSource::seconds_since_last_event_type(CGEventSourceStateID::HIDSystemState, CGEventType::ScrollWheel);
    (ago.is_finite() && ago >= 0.0).then(|| Duration::from_secs_f64(ago))
}
