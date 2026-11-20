//! Changes to ottospace's own window that GPUI does not expose.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use objc2::ffi::class_addMethod;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{AnyThread, MainThreadMarker, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationPresentationOptions, NSEvent, NSEventModifierFlags,
    NSEventType, NSFloatingWindowLevel, NSImage, NSScreen, NSView, NSWindowCollectionBehavior, NSWindowOcclusionState,
};
use objc2_core_foundation::CGPoint;
use objc2_core_graphics::{CGEvent, CGEventField, CGEventFlags, CGEventType, CGMouseButton, CGScrollEventUnit};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use ottospace_core::ax::Frame;

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
    /// The right button: a press, a release.
    RightDown,
    RightUp,
    /// The scroll wheel, by `dx`, `dy`: points if `precise` (a trackpad), else lines (a mouse
    /// wheel); both in whole numbers, rounded. Positive `dy` scrolls back, as AppKit counts.
    Wheel { dx: f64, dy: f64, precise: bool },
    /// The pointer leaves the window, or comes back into it.
    Exit,
    Enter,
}

/// The window server's number for the view's window (a `CGWindowID`), or 0 if it has none.
///
/// # Safety
/// `ns_view` must point to a live `NSView`, on the main thread.
pub unsafe fn number(ns_view: *mut c_void) -> isize {
    let view = unsafe { &*(ns_view as *const NSView) };
    view.window().map_or(0, |window| window.windowNumber())
}

/// True while some of the view's window shows on a screen: GPUI draws it only then.
///
/// # Safety
/// `ns_view` must point to a live `NSView`, on the main thread.
pub unsafe fn visible(ns_view: *mut c_void) -> bool {
    let view = unsafe { &*(ns_view as *const NSView) };
    view.window().is_some_and(|window| window.occlusionState().contains(NSWindowOcclusionState::Visible))
}

/// Makes GPUI draw its window again when frames stopped coming, as after a display went away.
/// GPUI 0.2 draws on a CVDisplayLink of the window's display, made again only by its own
/// `windowDidChangeScreen:`, occlusion and activation handlers; made while the displays change,
/// it may be for a display that is gone, or fail and be none. Two of GPUI's own paths, as AppKit
/// would take them: its delegate's `windowDidChangeScreen:` (the window is its own delegate)
/// makes a new link for the screen the window is on now; the layer marked for display has
/// GPUI's `displayLayer:` draw a frame at the next commit, link or none, and make a link again.
/// False: the window lacks one of them (not GPUI 0.2's), and what it has was still done.
///
/// # Safety
/// `ns_view` must point to a live GPUI `NSView`, on the main thread, outside GPUI's window lock:
/// not inside one of its platform callbacks (an `App` update from a task or timer is fine).
pub unsafe fn redraw(ns_view: *mut c_void) -> bool {
    let view = unsafe { &*(ns_view as *const NSView) };
    let Some(window) = view.window() else { return false };
    let delegate: *mut AnyObject = unsafe { msg_send![&*window, delegate] };
    let relinks = !delegate.is_null()
        && unsafe { msg_send![delegate, respondsToSelector: sel!(windowDidChangeScreen:)] };
    if relinks {
        let _: () = unsafe { msg_send![delegate, windowDidChangeScreen: std::ptr::null_mut::<AnyObject>()] };
    }
    let layer: *mut AnyObject = unsafe { msg_send![view, layer] };
    let draws = !layer.is_null() && unsafe { msg_send![view, respondsToSelector: sel!(displayLayer:)] };
    if draws {
        let _: () = unsafe { msg_send![layer, setNeedsDisplay] };
    }
    relinks && draws
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
        // AppKit's own constructor leaves the button number 0, which GPUI reads as the left one.
        MouseKind::RightDown | MouseKind::RightUp => {
            let down = mouse.kind == MouseKind::RightDown;
            let event = right(at, down, flags);
            match &event {
                Some(event) if down => view.rightMouseDown(event),
                Some(event) => view.rightMouseUp(event),
                None => {}
            }
            return event.is_some();
        }
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

/// A right-button press or release whose `locationInWindow` is `at`, its button number 1: made
/// by Quartz as `wheel`'s is, and placed the same way.
fn right(at: NSPoint, down: bool, flags: NSEventModifierFlags) -> Option<Retained<NSEvent>> {
    let kind = if down { CGEventType::RightMouseDown } else { CGEventType::RightMouseUp };
    let screen = NSScreen::screens(MainThreadMarker::new()?).firstObject()?.frame().size.height;
    let event = CGEvent::new_mouse_event(None, kind, CGPoint::new(at.x, screen - at.y), CGMouseButton::Right)?;
    CGEvent::set_integer_value_field(Some(&event), CGEventField::MouseEventClickState, 1);
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

/// Set by a zoom of the window (the green button, Window ▸ Zoom, a title bar double-click), taken by `take_zoom`.
static ZOOM_ASKED: AtomicBool = AtomicBool::new(false);

/// `zoom:`: no zoom; the shell fills the display instead.
extern "C-unwind" fn zoom_instead(_: &AnyObject, _: Sel, _: *mut AnyObject) {
    ZOOM_ASKED.store(true, Ordering::SeqCst);
    crate::wake::post();
}

/// Makes a zoom of the window ask the shell (`take_zoom`) instead of resizing it: `zoom:` is
/// added to the window's class (GPUI's `GPUIWindow`, which inherits NSWindow's), not calling
/// NSWindow's. Every zoom goes through it: the button's `performZoom:`, a menu item's, GPUI's
/// `zoom_window` and title bar double-click. Not the delegate's `windowShouldZoom:toFrame:`:
/// on the CI runner `zoom:` resized the window without asking it. False: the class has a
/// `zoom:` of its own, left alone, and zooms as before.
///
/// # Safety
/// `ns_view` must point to a live `NSView`, and this must run on the main thread.
pub unsafe fn zoom_asks(ns_view: *mut c_void) -> bool {
    let view = unsafe { &*(ns_view as *const NSView) };
    let Some(window) = view.window() else { return false };
    let class = AnyObject::class(&window) as *const AnyClass as *mut AnyClass;
    // SAFETY: `zoom_instead` takes the receiver, the selector and the sender, and returns nothing.
    let imp: Imp = unsafe { std::mem::transmute(zoom_instead as extern "C-unwind" fn(_, _, _)) };
    // Adds only: NO if the class defines `zoom:` itself.
    unsafe { class_addMethod(class, sel!(zoom:), imp, c"v@:@".as_ptr()) }.as_bool()
}

/// True once after each zoom `zoom_asks` turned away.
pub fn take_zoom() -> bool {
    ZOOM_ASKED.swap(false, Ordering::SeqCst)
}

/// The part of the title bar the shell's own drags take (`x, y, w, h`, points from the view's
/// top-left): a press there never moves the window.
static HELD_TITLEBAR: std::sync::Mutex<[f64; 4]> = std::sync::Mutex::new([0.; 4]);
/// The main window's view, the one `HELD_TITLEBAR` is of: a tab out's window (the same class) holds none.
static HELD_VIEW: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// `_opaqueRectForWindowMoveWhenInTitlebar`: `HELD_TITLEBAR`, in AppKit's bottom-left points.
extern "C-unwind" fn held_titlebar(this: &AnyObject, _: Sel) -> NSRect {
    let main = std::ptr::eq(this as *const AnyObject as *const c_void, HELD_VIEW.load(Ordering::SeqCst));
    let [x, y, w, h] = HELD_TITLEBAR.lock().ok().filter(|_| main).map_or([0.; 4], |held| *held);
    let view = unsafe { &*(this as *const AnyObject as *const NSView) };
    NSRect::new(NSPoint::new(x, view.bounds().size.height - y - h), NSSize::new(w, h))
}

/// Makes a press in `set_held_titlebar`'s rect the view's, never a move of the window: AppKit
/// drags a transparent title bar's window unless the view under the press claims that part of it
/// (the private `_opaqueRectForWindowMoveWhenInTitlebar`, as Zed's GPUI does). False: the class
/// has its own, left alone.
///
/// # Safety
/// `ns_view` must point to a live `NSView`, and this must run on the main thread.
pub unsafe fn titlebar_holds(ns_view: *mut c_void) -> bool {
    let view = unsafe { &*(ns_view as *const NSView) };
    HELD_VIEW.store(ns_view, Ordering::SeqCst);
    let class = AnyObject::class(view) as *const AnyClass as *mut AnyClass;
    // SAFETY: `held_titlebar` takes the receiver and the selector, and returns an NSRect.
    let imp: Imp = unsafe { std::mem::transmute(held_titlebar as extern "C-unwind" fn(_, _) -> NSRect) };
    let types = c"{CGRect={CGPoint=dd}{CGSize=dd}}@:";
    unsafe { class_addMethod(class, sel!(_opaqueRectForWindowMoveWhenInTitlebar), imp, types.as_ptr()) }.as_bool()
}

/// Where the title bar's own controls are now: `[x, y, w, h]`, points from the view's top-left.
pub fn set_held_titlebar(rect: [f64; 4]) {
    if let Ok(mut held) = HELD_TITLEBAR.lock() {
        *held = rect;
    }
}

/// Presses the window's zoom as its green button does (`performZoom:`). True if `zoom_asks`
/// took it, as `take_zoom` will say.
///
/// # Safety
/// As `zoom_asks`.
pub unsafe fn press_zoom(ns_view: *mut c_void) -> bool {
    let view = unsafe { &*(ns_view as *const NSView) };
    if let Some(window) = view.window() {
        window.performZoom(None);
    }
    ZOOM_ASKED.load(Ordering::SeqCst)
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

/// `at` (x, y, width, height in points from the view's top left) as a frame in `view`: AppKit
/// measures from the bottom left, unless the view says it is flipped (GPUI's is).
fn frame_in(view: &NSView, at: [f64; 4]) -> objc2_foundation::NSRect {
    let y = if view.isFlipped() { at[1] } else { view.frame().size.height - at[1] - at[3] };
    objc2_foundation::NSRect::new(NSPoint::new(at[0], y), objc2_foundation::NSSize::new(at[2], at[3]))
}

/// Adds a native push button titled `title` to the view, `at` points from its top left: a
/// control with a label in the window's accessibility tree, which a GPUI view has none of.
/// For the canary, so a test can circle something an app's tree names.
///
/// # Safety
/// `ns_view` must point to a live `NSView`, and this must run on the main thread.
pub unsafe fn add_button(ns_view: *mut c_void, title: &str, at: [f64; 4]) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let view = unsafe { &*(ns_view as *const NSView) };
    let button = unsafe {
        objc2_app_kit::NSButton::buttonWithTitle_target_action(&objc2_foundation::NSString::from_str(title), None, None, mtm)
    };
    button.setFrame(frame_in(view, at));
    view.addSubview(&button);
}

/// Gives the view's window the accessibility subrole `subrole` (`AXDialog`): what another app
/// reads of it. For the canary, whose GPUI windows are all standard.
///
/// # Safety
/// `ns_view` must point to a live `NSView`, and this must run on the main thread.
pub unsafe fn set_subrole(ns_view: *mut c_void, subrole: &str) {
    let view = unsafe { &*(ns_view as *const NSView) };
    if let Some(window) = view.window() {
        let subrole = objc2_foundation::NSString::from_str(subrole);
        let _: () = unsafe { msg_send![&*window, setAccessibilitySubrole: &*subrole] };
    }
}

/// Moves the view's window so its top left is at `x`, `y`, in Accessibility's coordinates
/// (global, from the main screen's top left). Unconstrained: off screen if asked. For the canary.
///
/// # Safety
/// `ns_view` must point to a live `NSView`, and this must run on the main thread.
pub unsafe fn move_to(ns_view: *mut c_void, x: f64, y: f64) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let view = unsafe { &*(ns_view as *const NSView) };
    let height = NSScreen::screens(mtm).firstObject().map_or(0.0, |s| s.frame().size.height);
    if let Some(window) = view.window() {
        window.setFrameTopLeftPoint(NSPoint::new(x, height - y));
    }
}

/// The view's window frame `[x, y, w, h]` in Accessibility's coordinates. For the canary.
///
/// # Safety
/// `ns_view` must point to a live `NSView`, and this must run on the main thread.
pub unsafe fn frame(ns_view: *mut c_void) -> Option<[f64; 4]> {
    let mtm = MainThreadMarker::new()?;
    let view = unsafe { &*(ns_view as *const NSView) };
    let height = NSScreen::screens(mtm).firstObject().map_or(0.0, |s| s.frame().size.height);
    let f = view.window()?.frame();
    Some([f.origin.x, height - f.origin.y - f.size.height, f.size.width, f.size.height])
}

/// An AppKit rect (from the main screen's bottom left) in Accessibility's coordinates.
fn flipped(r: objc2_foundation::NSRect, mtm: MainThreadMarker) -> Frame {
    let height = NSScreen::screens(mtm).firstObject().map_or(0.0, |s| s.frame().size.height);
    Frame::new(r.origin.x, height - r.origin.y - r.size.height, r.size.width, r.size.height)
}

/// Where the view's window is, in Accessibility's coordinates.
pub struct Placement {
    pub window: Frame,
    /// The whole of the screen the window is on.
    pub screen: Frame,
    /// What the menu bar and the Dock leave of it, as they are now.
    pub visible: Frame,
    /// How tall the strip is that the camera housing cuts into, at the screen's top. 0: none.
    pub safe_top: f64,
}

/// # Safety
/// `ns_view` must point to a live `NSView`, and this must run on the main thread.
pub unsafe fn placement(ns_view: *mut c_void) -> Option<Placement> {
    let mtm = MainThreadMarker::new()?;
    let view = unsafe { &*(ns_view as *const NSView) };
    let window = view.window()?;
    let screen = window.screen()?;
    Some(Placement {
        window: flipped(window.frame(), mtm),
        screen: flipped(screen.frame(), mtm),
        visible: flipped(screen.visibleFrame(), mtm),
        safe_top: screen.safeAreaInsets().top,
    })
}

/// Moves and sizes the view's window to `frame` (Accessibility's coordinates). Unconstrained:
/// over the menu bar if asked.
///
/// # Safety
/// As `placement`.
pub unsafe fn set_frame(ns_view: *mut c_void, frame: Frame) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let view = unsafe { &*(ns_view as *const NSView) };
    if let Some(window) = view.window() {
        // AppKit measures from the main screen's bottom left.
        let height = NSScreen::screens(mtm).firstObject().map_or(0.0, |s| s.frame().size.height);
        let origin = NSPoint::new(frame.x, height - frame.y - frame.h);
        window.setFrame_display(objc2_foundation::NSRect::new(origin, objc2_foundation::NSSize::new(frame.w, frame.h)), true);
    }
}

/// While this app is the one in front, the menu bar and the Dock stay away until the pointer
/// reaches their edge. `false`: as the system has them. Ends with the process.
pub fn hide_bars(on: bool) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let hidden = NSApplicationPresentationOptions::AutoHideMenuBar | NSApplicationPresentationOptions::AutoHideDock;
    let options = if on { hidden } else { NSApplicationPresentationOptions::Default };
    NSApplication::sharedApplication(mtm).setPresentationOptions(options);
}

/// True while the app in front keeps the menu bar or the Dock away: the system's state, the
/// same from any process. For the canary.
pub fn bars_hidden() -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    NSApplication::sharedApplication(mtm).currentSystemPresentationOptions() != NSApplicationPresentationOptions::Default
}

/// A native checkbox of the canary's: a press (a click or `AXPress`) turns it on or off.
pub struct Checkbox(Retained<objc2_app_kit::NSButton>);

impl Checkbox {
    pub fn on(&self) -> bool {
        self.0.state() != 0
    }
}

/// Adds a native checkbox titled `title` to the view, `at` points from its top left, enabled or
/// not: a control a test can press through sneak mode and then read. For the canary.
///
/// # Safety
/// As `add_button`.
pub unsafe fn add_checkbox(ns_view: *mut c_void, title: &str, at: [f64; 4], enabled: bool) -> Option<Checkbox> {
    let mtm = MainThreadMarker::new()?;
    let view = unsafe { &*(ns_view as *const NSView) };
    let title = objc2_foundation::NSString::from_str(title);
    let checkbox = unsafe { objc2_app_kit::NSButton::checkboxWithTitle_target_action(&title, None, None, mtm) };
    checkbox.setFrame(frame_in(view, at));
    checkbox.setEnabled(enabled);
    view.addSubview(&checkbox);
    Some(Checkbox(checkbox))
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

/// Shows the standard About panel: "Version 0.1.0 (a4a2582c0ffe, 2026-11-12T08:09:00Z)". A bare binary has no
/// Info.plist to read them from. Does nothing off the main thread.
pub fn show_about(version: &str, build: &str) {
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSAboutPanelOptionApplicationVersion, NSAboutPanelOptionVersion};
    use objc2_foundation::{NSDictionary, NSString};
    let Some(mtm) = MainThreadMarker::new() else { return };
    let (version, build) = (NSString::from_str(version), NSString::from_str(build));
    let values: [&AnyObject; 2] = [version.as_ref(), build.as_ref()];
    // SAFETY: AppKit's own keys, on the main thread.
    let keys = unsafe { [NSAboutPanelOptionApplicationVersion, NSAboutPanelOptionVersion] };
    let options = NSDictionary::from_slices(&keys, &values);
    let app = NSApplication::sharedApplication(mtm);
    unsafe { app.orderFrontStandardAboutPanelWithOptions(&options) };
}
