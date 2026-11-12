//! Changes to ottospace's own window that GPUI does not expose.

use std::ffi::c_void;

use objc2_app_kit::{NSFloatingWindowLevel, NSView, NSWindowCollectionBehavior};

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

/// Keeps the window above ordinary windows. Test runs use it: GPUI does not draw a window that
/// is fully covered, and a test that waits for layout would then time out.
///
/// # Safety
/// `ns_view` must point to a live `NSView`, and this must run on the main thread.
pub unsafe fn keep_on_top(ns_view: *mut c_void) {
    let view = unsafe { &*(ns_view as *const NSView) };
    if let Some(window) = view.window() {
        window.setLevel(NSFloatingWindowLevel);
    }
}
