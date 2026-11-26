//! A Metal layer of its own under a GPUI window's content: the code city draws straight to it
//! (`city_live_layer`), and GPUI draws the rest of the window, transparent over its rectangle,
//! above it. A view below GPUI's own view in the window's content view, as GPUI's blurred
//! background is. Main thread only.

use std::ffi::c_void;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSWindowOrderingMode};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CALayer, CAMetalLayer};

/// The view and its layer. Removed from its window when dropped.
pub struct LiveLayer {
    view: Retained<NSView>,
    layer: Retained<CAMetalLayer>,
}

impl LiveLayer {
    /// A view with a Metal layer, below `gpui_view` (GPUI's view, from its window handle) in that
    /// view's superview, hidden until placed. None off the main thread or with no superview.
    ///
    /// # Safety
    /// `gpui_view` is a live `NSView`.
    pub unsafe fn below(gpui_view: *mut c_void) -> Option<LiveLayer> {
        let mtm = MainThreadMarker::new()?;
        // SAFETY: the caller's view, alive while this runs.
        let gpui = unsafe { (gpui_view as *const NSView).as_ref() }?;
        // SAFETY: a view in a window has its content view as superview.
        let parent = unsafe { gpui.superview() }?;
        let rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(1., 1.));
        let view = NSView::initWithFrame(mtm.alloc(), rect);
        let layer = CAMetalLayer::new();
        layer.setOpaque(true);
        // A layer-hosting view: its layer is ours, set before it wants one.
        let as_layer: &CALayer = &layer;
        view.setLayer(Some(as_layer));
        view.setWantsLayer(true);
        view.setHidden(true);
        parent.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, Some(gpui));
        Some(LiveLayer { view, layer })
    }

    /// The `CAMetalLayer`, held, for a GPU surface on another thread.
    pub fn metal_layer(&self) -> MetalLayer {
        MetalLayer(self.layer.clone())
    }

    /// Placed at `x`, `y` (points from the window's top left), `w` × `h`, at `scale` pixels a
    /// point, and shown.
    pub fn place(&self, [x, y, w, h]: [f64; 4], scale: f64) {
        // SAFETY: placed views are in a window: their superview is its content view.
        let Some(parent) = (unsafe { self.view.superview() }) else { return };
        let height = parent.bounds().size.height;
        // The content view is not flipped: y from its bottom.
        let frame = NSRect::new(NSPoint::new(x, height - y - h), NSSize::new(w, h));
        self.view.setFrame(frame);
        self.layer.setContentsScale(scale);
        self.view.setHidden(false);
    }

    pub fn hide(&self) {
        self.view.setHidden(true);
    }
}

/// A held `CAMetalLayer`: alive while any copy is, whatever happens to its view.
#[derive(Clone)]
pub struct MetalLayer(Retained<CAMetalLayer>);

// SAFETY: only retained, released and handed to wgpu off the main thread; a `CAMetalLayer`'s
// drawables may be asked for from any thread. Its view stays the main thread's.
unsafe impl Send for MetalLayer {}
unsafe impl Sync for MetalLayer {}

impl MetalLayer {
    /// Its address: for wgpu's `SurfaceTargetUnsafe::CoreAnimationLayer`, and to tell layers apart.
    pub fn as_ptr(&self) -> *mut c_void {
        Retained::as_ptr(&self.0) as *mut c_void
    }
}

impl Drop for LiveLayer {
    fn drop(&mut self) {
        self.view.removeFromSuperview();
    }
}
