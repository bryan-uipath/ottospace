//! The user's scroll over a parked app pane, handed to that app's window: the window is neither
//! raised nor activated, and the real pointer does not move. The event is posted to the app's
//! process alone (`CGEventPostToPid`), never to the system's event stream.

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSEventType};
use objc2_core_foundation::{CFRetained, CGPoint};
use objc2_core_graphics::{CGEvent, CGEventField, CGScrollEventUnit};

/// One scroll as the device sent it: its phases (a trackpad's began, changed, ended, and the
/// momentum after) and its deltas.
pub struct Scroll(CFRetained<CGEvent>);

/// A scroll event's deltas, `[vertical, horizontal]`: whole lines, fixed-point lines, points.
const LINES: [CGEventField; 2] = [CGEventField::ScrollWheelEventDeltaAxis1, CGEventField::ScrollWheelEventDeltaAxis2];
const FIXED: [CGEventField; 2] =
    [CGEventField::ScrollWheelEventFixedPtDeltaAxis1, CGEventField::ScrollWheelEventFixedPtDeltaAxis2];
const POINTS: [CGEventField; 2] =
    [CGEventField::ScrollWheelEventPointDeltaAxis1, CGEventField::ScrollWheelEventPointDeltaAxis2];

impl Scroll {
    /// The scroll AppKit is handling now, copied whole, if its deltas are `delta` (`[dx, dy]` as
    /// the view was handed them). `None` for another event: a test's, handed to a view and never
    /// queued, leaves AppKit's current event as it was.
    pub fn current(delta: [f64; 2]) -> Option<Self> {
        let event = NSApplication::sharedApplication(MainThreadMarker::new()?).currentEvent()?;
        // The type first: asking another kind of event for its deltas raises.
        if event.r#type() != NSEventType::ScrollWheel {
            return None;
        }
        let same = |a: f64, b: f64| (a - b).abs() < 0.01;
        if !same(event.scrollingDeltaX(), delta[0]) || !same(event.scrollingDeltaY(), delta[1]) {
            return None;
        }
        CGEvent::new_copy(Some(&*event.CGEvent()?)).map(Self)
    }

    /// A scroll of `dx`, `dy` points (a trackpad) or lines, made here. Positive `dy` scrolls
    /// back, as AppKit counts.
    pub fn new(dx: f64, dy: f64, precise: bool) -> Option<Self> {
        let units = if precise { CGScrollEventUnit::Pixel } else { CGScrollEventUnit::Line };
        CGEvent::new_scroll_wheel_event2(None, units, 2, dy.round() as i32, dx.round() as i32, 0).map(Self)
    }

    fn get(&self, field: CGEventField) -> f64 {
        match FIXED.contains(&field) {
            true => CGEvent::double_value_field(Some(&self.0), field),
            false => CGEvent::integer_value_field(Some(&self.0), field) as f64,
        }
    }

    fn set(&self, field: CGEventField, value: f64) {
        match FIXED.contains(&field) {
            true => CGEvent::set_double_value_field(Some(&self.0), field, value),
            false => CGEvent::set_integer_value_field(Some(&self.0), field, value.round() as i64),
        }
    }

    /// `[dx, dy]` in points. An event with lines alone counts 10 points a line, as AppKit scrolls one.
    pub fn points(&self) -> [f64; 2] {
        match [self.get(POINTS[1]), self.get(POINTS[0])] {
            [0.0, 0.0] => [self.get(FIXED[1]) * 10.0, self.get(FIXED[0]) * 10.0],
            points => points,
        }
    }

    /// Its scroll phase and momentum phase: scrolls with the same ones may be added up.
    pub fn phases(&self) -> [i64; 2] {
        let read = |field| CGEvent::integer_value_field(Some(&self.0), field);
        [read(CGEventField::ScrollWheelEventScrollPhase), read(CGEventField::ScrollWheelEventMomentumPhase)]
    }

    /// Adds `other`'s deltas to this one's: one event a frame.
    pub fn add(&mut self, other: &Scroll) {
        for field in LINES.into_iter().chain(FIXED).chain(POINTS) {
            self.set(field, self.get(field) + other.get(field));
        }
    }

    /// Scales each axis so its points are `to` (`[dx, dy]`, the core's cap): lines with them.
    pub fn limit(&mut self, to: [f64; 2]) {
        let now = self.points();
        for (axis, fields) in [(1, [LINES[0], FIXED[0], POINTS[0]]), (0, [LINES[1], FIXED[1], POINTS[1]])] {
            if now[axis] != 0.0 && now[axis] != to[axis] {
                let by = to[axis] / now[axis];
                fields.into_iter().for_each(|field| self.set(field, self.get(field) * by));
            }
        }
    }

    /// Posts it to process `pid` alone, at the global point `at`, as over its window `window`:
    /// AppKit hands it to that window's view under `at`, whether or not the window is in front.
    pub fn post(&self, pid: i32, window: u32, at: CGPoint) {
        let event = Some(&*self.0);
        CGEvent::set_location(event, at);
        for field in [
            CGEventField::MouseEventWindowUnderMousePointer,
            CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent,
        ] {
            CGEvent::set_integer_value_field(event, field, window.into());
        }
        CGEvent::post_to_pid(pid, event);
    }
}
