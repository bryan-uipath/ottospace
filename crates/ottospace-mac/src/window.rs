//! Changes to ottospace's own window that GPUI does not expose.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSEvent, NSEventModifierFlags, NSEventType, NSFloatingWindowLevel,
    NSImage, NSScreen, NSView, NSWindowCollectionBehavior,
};
use objc2_core_foundation::CGPoint;
use objc2_core_graphics::{CGEvent, CGEventField, CGEventFlags, CGScrollEventUnit};
use objc2_foundation::NSPoint;

/// One pointer event for `send_mouse`. `x`, `y`: points from the top-left of the view.
#[derive(Clone, Copy, Debug)]
pub struct Mouse {
    pub kind: MouseKind,
    pub x: f64,
    pub y: f64,
    pub clicks: isize,
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MouseKind {
    Down,
    Up,
    /// A move with no button held.
    Move,
    /// A move with the button held.
    Drag,
    /// The scroll wheel, by `dx`, `dy`: points if `precise` (a trackpad), else lines (a mouse
    /// wheel); both in whole numbers, rounded. Positive `dy` scrolls back, as AppKit counts.
    Wheel { dx: f64, dy: f64, precise: bool },
    /// The pointer leaves the window, or comes back into it.
    Exit,
    Enter,
}

/// Hands the view a pointer event as AppKit would, by calling its own handler (`mouseDown:` and
/// the like). The event is made here and never posted: the window server, other apps and the
/// real pointer do not see it. The handler runs before this returns.
///
/// `false`: the event could not be made, and nothing was sent.
///
/// # Safety
/// `ns_view` must point to a live `NSView`, and this must run on the main thread, outside any
/// borrow the view's handler takes (for a GPUI view: not inside an `App` update).
pub unsafe fn send_mouse(ns_view: *mut c_void, mouse: Mouse) -> bool {
    let view = unsafe { &*(ns_view as *const NSView) };
    let mut flags = NSEventModifierFlags::empty();
    for (held, flag) in [
        (mouse.cmd, NSEventModifierFlags::Command),
        (mouse.ctrl, NSEventModifierFlags::Control),
        (mouse.alt, NSEventModifierFlags::Option),
        (mouse.shift, NSEventModifierFlags::Shift),
    ] {
        if held {
            flags |= flag;
        }
    }
    // AppKit measures from the bottom-left.
    let at = NSPoint::new(mouse.x, view.frame().size.height - mouse.y);
    let number = view.window().map_or(0, |window| window.windowNumber());
    let kind = match mouse.kind {
        MouseKind::Down => NSEventType::LeftMouseDown,
        MouseKind::Up => NSEventType::LeftMouseUp,
        MouseKind::Move => NSEventType::MouseMoved,
        MouseKind::Drag => NSEventType::LeftMouseDragged,
        MouseKind::Wheel { dx, dy, precise } => {
            let event = wheel(at, dx, dy, precise, flags);
            if let Some(event) = &event {
                view.scrollWheel(event);
            }
            return event.is_some();
        }
        MouseKind::Exit | MouseKind::Enter => {
            let exit = mouse.kind == MouseKind::Exit;
            let kind = if exit { NSEventType::MouseExited } else { NSEventType::MouseEntered };
            let event = unsafe {
                NSEvent::enterExitEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_trackingNumber_userData(
                    kind, at, flags, 0.0, number, None, 0, 0, std::ptr::null_mut(),
                )
            };
            match &event {
                Some(event) if exit => view.mouseExited(event),
                Some(event) => view.mouseEntered(event),
                None => {}
            }
            return event.is_some();
        }
    };
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind, at, flags, 0.0, number, None, 0, mouse.clicks, 1.0,
    );
    let Some(event) = event else { return false };
    match mouse.kind {
        MouseKind::Down => view.mouseDown(&event),
        MouseKind::Up => view.mouseUp(&event),
        MouseKind::Move => view.mouseMoved(&event),
        _ => view.mouseDragged(&event),
    }
    true
}

/// A scroll-wheel event whose `locationInWindow` is `at`. AppKit makes none itself, so it is
/// Quartz's, converted. It belongs to no window, so AppKit reads its place as a screen point,
/// from the bottom-left of the main screen: Quartz counts from the top-left of that screen.
fn wheel(at: NSPoint, dx: f64, dy: f64, precise: bool, flags: NSEventModifierFlags) -> Option<Retained<NSEvent>> {
    let units = if precise { CGScrollEventUnit::Pixel } else { CGScrollEventUnit::Line };
    let event = CGEvent::new_scroll_wheel_event2(None, units, 2, dy.round() as i32, dx.round() as i32, 0)?;
    let screen = NSScreen::screens(MainThreadMarker::new()?).firstObject()?.frame().size.height;
    CGEvent::set_location(Some(&event), CGPoint::new(at.x, screen - at.y));
    CGEvent::set_flags(Some(&event), CGEventFlags(flags.0 as u64));
    NSEvent::eventWithCGEvent(&event)
}

/// Set while a test's held key is handled: `key_repeating` reads the app's current event,
/// which an event that was never queued is not.
static HELD: AtomicBool = AtomicBool::new(false);

/// Runs `f` as while a key is held down and repeats: `key_repeating` is true in it.
pub fn while_held<R>(f: impl FnOnce() -> R) -> R {
    // Cleared also if `f` unwinds.
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            HELD.store(false, Ordering::Relaxed);
        }
    }
    HELD.store(true, Ordering::Relaxed);
    let _clear = Clear;
    f()
}

/// Hands the view a key that is held down and repeating, as AppKit would, by calling its own
/// `keyDown:`. As `send_mouse`: made here, never posted. `key`: `enter`, `tab`, `space`, or a
/// character the keyboard layout in use types with no modifier on one of the key codes 0 to 50
/// (letters, digits, punctuation: `a`, not `A`). `false`: no such key.
///
/// Not for a key that types nothing (a ⌘ shortcut, Esc, an arrow): the view hands those to the
/// input method first, which takes no event that the system did not queue.
///
/// # Safety
/// As `send_mouse`.
pub unsafe fn send_held_key(ns_view: *mut c_void, key: &str, ctrl: bool, alt: bool, shift: bool) -> bool {
    let view = unsafe { &*(ns_view as *const NSView) };
    let mut flags = CGEventFlags(0);
    for (held, flag) in
        [(ctrl, CGEventFlags::MaskControl), (alt, CGEventFlags::MaskAlternate), (shift, CGEventFlags::MaskShift)]
    {
        if held {
            flags |= flag;
        }
    }
    let made = |code: u16, flags: CGEventFlags, repeat: i64| {
        let event = CGEvent::new_keyboard_event(None, code, true)?;
        CGEvent::set_flags(Some(&event), flags);
        CGEvent::set_integer_value_field(Some(&event), CGEventField::KeyboardEventAutorepeat, repeat);
        NSEvent::eventWithCGEvent(&event)
    };
    // The key's code: fixed for a named key; for a character, the key the layout types it with.
    let types = |code: &u16| {
        let typed = made(*code, CGEventFlags(0), 0).and_then(|event| event.charactersIgnoringModifiers());
        typed.is_some_and(|typed| typed.to_string() == key)
    };
    let code = match key {
        "enter" => Some(36),
        "tab" => Some(48),
        "space" => Some(49),
        _ => (0..=50).find(types),
    };
    let Some(event) = code.and_then(|code| made(code, flags, 1)) else { return false };
    while_held(|| view.keyDown(&event));
    true
}

/// Makes the green button zoom the window instead of entering native fullscreen.
/// Native fullscreen moves the window to its own Space, where other apps' windows cannot follow,
/// so no app window could be hosted there.
///
/// # Safety
/// `ns_view` must point to a live `NSView`, and this must run on the main thread.
pub unsafe fn zoom_instead_of_fullscreen(ns_view: *mut c_void) {
    let view = unsafe { &*(ns_view as *const NSView) };
    if let Some(window) = view.window() {
        let behavior = window.collectionBehavior() & !NSWindowCollectionBehavior::FullScreenPrimary;
        // DisallowsTiling: Split View would also put the window in a fullscreen Space.
        window.setCollectionBehavior(
            behavior | NSWindowCollectionBehavior::FullScreenNone | NSWindowCollectionBehavior::FullScreenDisallowsTiling,
        );
    }
}

/// Keeps a test run's window drawn wherever the user is: above ordinary windows, and on every
/// Space, a fullscreen app's included. GPUI does not draw a window that nobody can see, and a
/// test that waits for layout would then time out. With `click_through`, clicks pass to what is
/// under it, so a stray click cannot bring the test app forward or close its window.
///
/// # Safety
/// `ns_view` must point to a live `NSView`, and this must run on the main thread.
pub unsafe fn keep_on_top(ns_view: *mut c_void, click_through: bool) {
    let view = unsafe { &*(ns_view as *const NSView) };
    if let Some(window) = view.window() {
        window.setLevel(NSFloatingWindowLevel);
        // Auxiliary, not None: only an auxiliary window may sit over a fullscreen Space.
        let behavior = window.collectionBehavior() & !NSWindowCollectionBehavior::FullScreenNone;
        window.setCollectionBehavior(
            behavior | NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        window.setIgnoresMouseEvents(click_through);
    }
}

/// Puts the window in front of other apps' windows without making this app the front app.
/// For when another app is to keep the keyboard, with ottospace visible under its window.
///
/// # Safety
/// `ns_view` must point to a live `NSView`, and this must run on the main thread.
pub unsafe fn order_front(ns_view: *mut c_void) {
    let view = unsafe { &*(ns_view as *const NSView) };
    if let Some(window) = view.window() {
        window.orderFrontRegardless();
    }
}

/// Makes this process a background app: no Dock icon, no place in the app switcher, and it does
/// not come to the front when it starts. For test runs, which must not take the user's keyboard.
pub fn stay_in_background() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    NSApplication::sharedApplication(mtm).setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

/// True while the event being handled is a key held down, repeating. An action bound to a key
/// does not see this, and a held ⌘Q must not answer its own question.
pub fn key_repeating() -> bool {
    if HELD.load(Ordering::Relaxed) {
        return true;
    }
    let Some(mtm) = MainThreadMarker::new() else { return false };
    let event = NSApplication::sharedApplication(mtm).currentEvent();
    // Only a key event has `isARepeat`: asking another kind raises.
    event.is_some_and(|event| event.r#type() == NSEventType::KeyDown && event.isARepeat())
}

/// Shows `icon` (the bytes of an .icns or image file) in the Dock and the app switcher.
/// A bare binary has no bundle to take an icon from, so without this it gets the generic one.
/// Does nothing off the main thread or if the bytes are not an image.
pub fn set_dock_icon(icon: &[u8]) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let data = objc2_foundation::NSData::with_bytes(icon);
    if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
        unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
    }
}
