//! The modifier keys' changes in other apps, for ⌘⌘ (`ottospace_core::double_cmd`). A global
//! NSEvent monitor of `flagsChanged` only: it cannot change or stop an event, sees no key code or
//! character, and gets nothing while ottospace is in front. It needs Accessibility.

use std::ptr::NonNull;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use block2::RcBlock;
use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags};
use ottospace_core::double_cmd::Flags;

static CHANGES: Mutex<Vec<Flags>> = Mutex::new(Vec::new());
static START: OnceLock<Instant> = OnceLock::new();

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state: i32, kind: u32) -> f64;
}

/// Starts listening. Call once, on the main thread. Errors when AppKit gives no monitor.
pub fn listen() -> Result<(), String> {
    START.get_or_init(Instant::now);
    let block = RcBlock::new(|event: NonNull<NSEvent>| on_flags(unsafe { event.as_ref() }.modifierFlags()));
    let monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(NSEventMask::FlagsChanged, &block);
    // Kept for the app's life: the monitor is never removed.
    std::mem::forget(monitor.ok_or("AppKit gave no event monitor")?);
    Ok(())
}

/// The changes seen since the last call, oldest first.
pub fn take() -> Vec<Flags> {
    CHANGES.lock().map(|mut c| std::mem::take(&mut *c)).unwrap_or_default()
}

fn on_flags(flags: NSEventModifierFlags) {
    let now = START.get_or_init(Instant::now).elapsed().as_millis() as u64;
    // When a key or mouse button last went down, anywhere (HID state): a time, never which.
    // keyDown 10; left, right, other mouse down 1, 3, 25.
    let ago = [10, 1, 3, 25]
        .map(|kind| unsafe { CGEventSourceSecondsSinceLastEventType(1, kind) })
        .into_iter()
        .fold(f64::INFINITY, f64::min);
    use NSEventModifierFlags as M;
    let change = Flags {
        at: now,
        cmd: flags.contains(M::Command),
        others: flags.intersects(M::Shift | M::Control | M::Option | M::Function),
        input_at: ago.is_finite().then(|| now.saturating_sub((ago * 1000.0) as u64)),
    };
    if let Ok(mut changes) = CHANGES.lock() {
        changes.push(change);
    }
    crate::wake::post();
}
