//! Changes to ottospace's own window that GPUI does not expose.

use std::ffi::c_void;

use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSEvent, NSEventModifierFlags, NSEventType, NSFloatingWindowLevel,
    NSImage, NSView, NSWindowCollectionBehavior,
};
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
}

/// Hands the view a pointer event as AppKit would, by calling its own handler (`mouseDown:` and
/// the like). The event is made here and never posted: the window server, other apps and the
/// real pointer do not see it. The handler runs before this returns.
///
/// # Safety
/// `ns_view` must point to a live `NSView`, and this must run on the main thread, outside any
/// borrow the view's handler takes (for a GPUI view: not inside an `App` update).
pub unsafe fn send_mouse(ns_view: *mut c_void, mouse: Mouse) {
    let view = unsafe { &*(ns_view as *const NSView) };
    let kind = match mouse.kind {
        MouseKind::Down => NSEventType::LeftMouseDown,
        MouseKind::Up => NSEventType::LeftMouseUp,
        MouseKind::Move => NSEventType::MouseMoved,
        MouseKind::Drag => NSEventType::LeftMouseDragged,
    };
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
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind, at, flags, 0.0, number, None, 0, mouse.clicks, 1.0,
    );
    let Some(event) = event else { return };
    match mouse.kind {
        MouseKind::Down => view.mouseDown(&event),
        MouseKind::Up => view.mouseUp(&event),
        MouseKind::Move => view.mouseMoved(&event),
        MouseKind::Drag => view.mouseDragged(&event),
    }
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
