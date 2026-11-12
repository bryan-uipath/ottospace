//! Changes to ottospace's own window that GPUI does not expose.

use std::ffi::c_void;

use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSFloatingWindowLevel, NSImage, NSView, NSWindowCollectionBehavior,
};

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
