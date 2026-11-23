//! The modifier keys' changes in other apps, for ⌘⌘ (`ottospace_core::double_cmd`). A global
//! NSEvent monitor of `flagsChanged` only: it cannot change or stop an event, sees no key code or
//! character, and gets nothing while ottospace is in front. It needs Accessibility.

use std::ptr::NonNull;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use block2::RcBlock;
use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags};
use ottospace_core::double_cmd::Flags;

static CHANGES: Mutex<Vec<Flags>> = Mutex::new(Vec::new());
/// Caps Lock's state at the last change: a toggle between taps is a key pressed.
static CAPS: AtomicBool = AtomicBool::new(false);

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state: i32, kind: u32) -> f64;
    fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
}

unsafe extern "C" {
    fn clock_gettime_nsec_np(clock: u32) -> u64;
}

/// Starts listening. Call once, on the main thread. Errors when AppKit gives no monitor.
pub fn listen() -> Result<(), String> {
    let block = RcBlock::new(|event: NonNull<NSEvent>| {
        let event = unsafe { event.as_ref() };
        on_flags(event.modifierFlags(), event.timestamp());
    });
    let monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(NSEventMask::FlagsChanged, &block);
    // Kept for the app's life: the monitor is never removed.
    std::mem::forget(monitor.ok_or("AppKit gave no event monitor")?);
    Ok(())
}

/// The changes seen since the last call, oldest first.
pub fn take() -> Vec<Flags> {
    CHANGES.lock().map(|mut c| std::mem::take(&mut *c)).unwrap_or_default()
}

/// `stamp`: the event's own time (seconds since boot), so a late delivery keeps its timing.
fn on_flags(flags: NSEventModifierFlags, stamp: f64) {
    const HID: i32 = 1;
    const UPTIME_RAW: u32 = 8; // NSEvent's clock
    let now = unsafe { clock_gettime_nsec_np(UPTIME_RAW) } / 1_000_000;
    // When a key or mouse button last went down, anywhere: a time, never which.
    // keyDown 10; left, right, other mouse down 1, 3, 25.
    let ago = [10, 1, 3, 25]
        .map(|kind| unsafe { CGEventSourceSecondsSinceLastEventType(HID, kind) })
        .into_iter()
        .fold(f64::INFINITY, f64::min);
    // A button held through the taps (a drag) counts as pressed.
    let held = (0..3).any(|button| unsafe { CGEventSourceButtonState(HID, button) });
    use NSEventModifierFlags as M;
    let caps = flags.contains(M::CapsLock);
    let toggled = CAPS.swap(caps, Relaxed) != caps;
    let change = Flags {
        at: (stamp * 1000.0) as u64,
        cmd: flags.contains(M::Command),
        others: held || toggled || flags.intersects(M::Shift | M::Control | M::Option | M::Function),
        input_at: ago.is_finite().then(|| now.saturating_sub((ago * 1000.0) as u64)),
    };
    if let Ok(mut changes) = CHANGES.lock() {
        changes.push(change);
    }
    crate::wake::post();
}
