//! Other apps' windows via the Accessibility API.
//! Coordinates are global, top-left origin (same space as CGDisplayBounds).

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2_app_kit::{NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace};
use objc2_application_services::{AXError, AXIsProcessTrustedWithOptions, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{
    CFArray, CFBoolean, CFDictionary, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize,
};

// Private but long-stable; maps an AX window to its CGWindowID (needed to pick the SCWindow).
unsafe extern "C" {
    fn _AXUIElementGetWindow(element: &AXUIElement, id: *mut u32) -> AXError;
}

/// The bundle id (`us.zoom.xos`) and the name (`zoom.us`) of the app with this pid.
pub fn app_identity(pid: i32) -> (Option<String>, Option<String>) {
    let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else { return (None, None) };
    (app.bundleIdentifier().map(|id| id.to_string()), app.localizedName().map(|name| name.to_string()))
}

/// Every running app with a Dock icon but this process, windows or none: pid, bundle id, name.
pub fn running() -> Vec<(i32, Option<String>, String)> {
    let me = std::process::id() as i32;
    let apps = NSWorkspace::sharedWorkspace().runningApplications();
    let regular = |app: &Retained<NSRunningApplication>| {
        app.activationPolicy() == NSApplicationActivationPolicy::Regular && !app.isTerminated()
    };
    let identity = |app: Retained<NSRunningApplication>| {
        let name = app.localizedName()?.to_string();
        Some((app.processIdentifier(), app.bundleIdentifier().map(|id| id.to_string()), name))
    };
    apps.into_iter().filter(regular).filter_map(identity).filter(|(pid, ..)| *pid != me).collect()
}

/// True if the app with this pid is the front app, asked of the app itself. `isActive` reads a
/// value AppKit keeps for this process, which lags until the run loop has seen the change; this
/// asks now.
pub fn is_frontmost(pid: i32) -> bool {
    let app = unsafe { AXUIElement::new_application(pid) };
    copy_attr(&app, "AXFrontmost").and_then(|v| v.downcast::<CFBoolean>().ok()).is_some_and(|front| front.as_bool())
}

/// As `is_frontmost`, asked of AppKit, not of the app: a hung app costs no wait.
pub fn is_active(pid: i32) -> bool {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid).is_some_and(|app| app.isActive())
}

/// Whether this process (or its parent terminal) has Accessibility access; optionally prompts.
pub fn is_trusted(prompt: bool) -> bool {
    let key = CFString::from_static_str("AXTrustedCheckOptionPrompt");
    let value = CFBoolean::new(prompt);
    let options = CFDictionary::from_slices(&[&*key], &[value]);
    unsafe { AXIsProcessTrustedWithOptions(Some(options.as_opaque())) }
}

/// An element's `AXChildren`.
fn children(element: &AXUIElement) -> Vec<CFRetained<AXUIElement>> {
    let Some(all) = copy_attr(element, "AXChildren").and_then(|a| a.downcast::<CFArray>().ok()) else {
        return Vec::new();
    };
    let all = unsafe { CFRetained::cast_unchecked::<CFArray<AXUIElement>>(all) };
    all.iter().collect()
}

/// The enabled item of the app's menus titled exactly `title`, in a submenu too (File ▸ New ▸
/// Window). Asked of the app over Accessibility: it need not be in front.
fn menu_item(pid: i32, title: &str) -> Option<CFRetained<AXUIElement>> {
    let app = unsafe { AXUIElement::new_application(pid) };
    let bar = copy_attr(&app, "AXMenuBar").and_then(|b| b.downcast::<AXUIElement>().ok())?;
    let named = |item: &AXUIElement| {
        let text = copy_attr(item, "AXTitle").and_then(|t| t.downcast::<CFString>().ok());
        let enabled = copy_attr(item, "AXEnabled").and_then(|e| e.downcast::<CFBoolean>().ok());
        text.is_some_and(|t| t.to_string() == title) && enabled.is_some_and(|e| e.as_bool())
    };
    // Each menu of the bar, then its items, and those of its submenus.
    fn items(element: &AXUIElement, depth: usize, out: &mut Vec<CFRetained<AXUIElement>>) {
        let level = children(element);
        out.extend(level.iter().cloned());
        if depth > 0 {
            for child in &level {
                items(child, depth - 1, out);
            }
        }
    }
    let mut all = Vec::new();
    items(&bar, 5, &mut all);
    all.into_iter().find(|item| named(item))
}

/// Whether the app's menus have an enabled item titled exactly `title`.
pub fn has_menu_item(pid: i32, title: &str) -> bool {
    menu_item(pid, title).is_some()
}

/// Presses the app's menu item titled exactly `title`.
pub fn press_menu_item(pid: i32, title: &str) -> Result<(), String> {
    let item = menu_item(pid, title).ok_or_else(|| format!("has no menu item {title}"))?;
    match unsafe { item.perform_action(&CFString::from_static_str("AXPress")) } {
        AXError::Success => Ok(()),
        _ => Err(format!("did not take its {title} menu item")),
    }
}

pub struct Window {
    element: CFRetained<AXUIElement>,
    pid: i32,
}

/// `Window::quick`'s wait for each Accessibility call, in seconds.
const QUICK: f32 = 0.25;

impl Window {
    /// The app's standard windows (skips the desktop, panels, popovers).
    pub fn standard_of(pid: i32) -> Vec<Self> {
        Self::all_of(pid).into_iter().filter(Self::is_standard).collect()
    }

    /// Every window the app lists: dialogs and panels too.
    /// Whether the app answers Accessibility at all: an app with no window still names its role.
    pub fn answers(pid: i32) -> bool {
        let app = unsafe { AXUIElement::new_application(pid) };
        copy_attr(&app, "AXRole").is_some()
    }

    pub fn all_of(pid: i32) -> Vec<Self> {
        let app = unsafe { AXUIElement::new_application(pid) };
        let Some(all) = copy_attr(&app, "AXWindows").and_then(|a| a.downcast::<CFArray>().ok()) else {
            return Vec::new();
        };
        let all = unsafe { CFRetained::cast_unchecked::<CFArray<AXUIElement>>(all) };
        all.iter().map(|element| Self { element, pid }).collect()
    }

    /// The app's main window if it's a standard window, else its first standard window.
    /// (Finder's main window is often the desktop, which isn't standard.)
    pub fn main_of(pid: i32) -> Option<Self> {
        Self::main_where(pid, |_| true)
    }

    /// The window-server id of the window the app says is its main one, whatever kind it is.
    /// `None` if it has none. No stand-in: this is "which window does the app show now".
    pub fn main_id(pid: i32) -> Option<u32> {
        let app = unsafe { AXUIElement::new_application(pid) };
        let element = copy_attr(&app, "AXMainWindow")?.downcast::<AXUIElement>().ok()?;
        Self { element, pid }.id()
    }

    /// As `main_of`, among the standard windows that `keep` accepts.
    pub fn main_where(pid: i32, keep: impl Fn(&Self) -> bool) -> Option<Self> {
        let app = unsafe { AXUIElement::new_application(pid) };
        copy_attr(&app, "AXMainWindow")
            .and_then(|w| w.downcast::<AXUIElement>().ok())
            .map(|element| Self { element, pid })
            .filter(|window| window.is_standard() && keep(window))
            .or_else(|| Self::standard_of(pid).into_iter().find(|window| keep(window)))
    }

    /// The standard window or dialog (`rules::judged`: one a `split` rule may put in a pane) with
    /// this window-server id, if `pid` still has it. Never a toast or popover.
    pub fn find(pid: i32, id: u32) -> Option<Self> {
        Self::all_of(pid).into_iter().find(|w| w.id() == Some(id) && w.judged())
    }

    /// A kind of window `rules::place` decides on: standard, dialog or panel.
    pub fn judged(&self) -> bool {
        ottospace_core::rules::judged(&self.subrole().unwrap_or_default())
    }

    /// Runs `ask` with a short wait for the app (a quarter of a second a call, not AX's 6), then
    /// puts the default back: for a question whose answer has a fallback, asked on the main
    /// thread. An app that hangs then costs the user a blink, not seconds.
    pub fn quick<R>(&self, ask: impl FnOnce(&Self) -> R) -> R {
        unsafe { self.element.set_messaging_timeout(QUICK) };
        let answer = ask(self);
        // 0: the default again.
        unsafe { self.element.set_messaging_timeout(0.0) };
        answer
    }

    pub(crate) fn element(&self) -> CFRetained<AXUIElement> {
        self.element.clone()
    }

    fn is_standard(&self) -> bool {
        self.subrole().as_deref() == Some("AXStandardWindow")
    }

    pub fn title(&self) -> Option<String> {
        Some(copy_attr(&self.element, "AXTitle")?.downcast::<CFString>().ok()?.to_string())
    }

    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// True if the window is in the Dock, minimised.
    pub fn minimized(&self) -> bool {
        let value = copy_attr(&self.element, "AXMinimized").and_then(|v| v.downcast::<CFBoolean>().ok());
        value.is_some_and(|minimized| minimized.as_bool())
    }

    pub fn set_size(&self, mut size: CGSize) -> Result<(), AXError> {
        write_value(&self.element, "AXSize", AXValueType::CGSize, &mut size)
    }

    /// Brings just this window to the front and activates its app for keyboard input.
    /// Avoids AXFrontmost, which raises every window of the app (e.g. all Chrome windows).
    pub fn raise(&self) {
        unsafe { self.element.perform_action(&CFString::from_static_str("AXRaise")) };
        if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(self.pid) {
            app.activateWithOptions(NSApplicationActivationOptions::empty());
        }
    }

    pub fn id(&self) -> Option<u32> {
        let mut id = 0;
        let err = unsafe { _AXUIElementGetWindow(&self.element, &mut id) };
        (err == AXError::Success).then_some(id)
    }

    pub fn subrole(&self) -> Option<String> {
        Some(copy_attr(&self.element, "AXSubrole")?.downcast::<CFString>().ok()?.to_string())
    }

    /// `AXModal`: the window blocks its app until answered.
    pub fn modal(&self) -> bool {
        let value = copy_attr(&self.element, "AXModal").and_then(|v| v.downcast::<CFBoolean>().ok());
        value.is_some_and(|modal| modal.as_bool())
    }

    /// A sheet (`AXSheet`), or a window whose `AXParent` is a window: macOS keeps it on that one.
    pub fn sheet(&self) -> bool {
        let role = |element: &AXUIElement| Some(copy_attr(element, "AXRole")?.downcast::<CFString>().ok()?.to_string());
        let parent = copy_attr(&self.element, "AXParent").and_then(|p| p.downcast::<AXUIElement>().ok());
        // `quick`'s short wait for the parent too: a reference of its own, the wait goes with it.
        if let Some(parent) = &parent {
            unsafe { parent.set_messaging_timeout(QUICK) };
        }
        role(&self.element).as_deref() == Some("AXSheet")
            || parent.is_some_and(|parent| matches!(role(&parent).as_deref(), Some("AXWindow" | "AXSheet")))
    }

    /// Asks for this window in front, without activating its app: the keyboard stays where it is.
    pub fn raise_only(&self) {
        unsafe { self.element.perform_action(&CFString::from_static_str("AXRaise")) };
    }

    /// The app's own name for this window (`AXIdentifier`): Zoom's `zm.meeting.window.main`.
    /// `None` when it gives none.
    pub fn identifier(&self) -> Option<String> {
        let id = copy_attr(&self.element, "AXIdentifier")?.downcast::<CFString>().ok()?.to_string();
        (!id.is_empty()).then_some(id)
    }

    /// `AXIdentifier` as read now: `Some(None)` when the window has none (no value, unsupported or
    /// empty), `None` when the app gave no answer (a timeout, an error).
    pub fn identifier_read(&self) -> Option<Option<String>> {
        match copy_attr_or(&self.element, "AXIdentifier") {
            Ok(value) => {
                let id = value.and_then(|value| value.downcast::<CFString>().ok()).map(|id| id.to_string());
                Some(id.filter(|id| !id.is_empty()))
            }
            Err(AXError::NoValue | AXError::AttributeUnsupported) => Some(None),
            Err(_) => None,
        }
    }

    pub fn frame(&self) -> Option<CGRect> {
        frame_of(&self.element)
    }

    /// Frame of the element the attribute `name` points to, e.g. "AXCloseButton", "AXTitleUIElement".
    pub fn part(&self, name: &str) -> Option<CGRect> {
        let element = copy_attr(&self.element, name)?.downcast::<AXUIElement>().ok()?;
        frame_of(&element)
    }

    /// Whether a direct child has the role `role` (e.g. "AXToolbar"). None: a child or role unreadable.
    pub fn has_child(&self, role: &str) -> Option<bool> {
        let children = copy_attr(&self.element, "AXChildren")?.downcast::<CFArray>().ok()?;
        let children = unsafe { CFRetained::cast_unchecked::<CFArray<AXUIElement>>(children) };
        let roles: Option<Vec<String>> = children
            .iter()
            .map(|child| Some(copy_attr(&child, "AXRole")?.downcast::<CFString>().ok()?.to_string()))
            .collect();
        Some(roles?.iter().any(|r| r == role))
    }

    /// Whether its app says it has unsaved changes (`AXEdited`: on the window, or on its close
    /// button, which AppKit draws with a dot then). `None`: the window did not answer (gone,
    /// busy); an app that keeps no such flag answers no.
    pub fn edited(&self) -> Option<bool> {
        let flag = |element: &AXUIElement| {
            copy_attr(element, "AXEdited").and_then(|e| e.downcast::<CFBoolean>().ok()).map(|e| e.as_bool())
        };
        let on = flag(&self.element) == Some(true) || self.close_button().and_then(|b| flag(&b)) == Some(true);
        // Its role answers unless the window is gone or busy.
        (on || copy_attr(&self.element, "AXRole").is_some()).then_some(on)
    }

    fn close_button(&self) -> Option<CFRetained<AXUIElement>> {
        copy_attr(&self.element, "AXCloseButton").and_then(|b| b.downcast::<AXUIElement>().ok())
    }

    /// Presses its close button, as a click on it would: an app with unsaved changes asks.
    pub fn close(&self) -> Result<(), &'static str> {
        let button = self.close_button().ok_or("has no close button")?;
        match unsafe { button.perform_action(&CFString::from_static_str("AXPress")) } {
            AXError::Success => Ok(()),
            _ => Err("did not close"),
        }
    }

    /// Moves the window; returns where it actually landed (macOS may clamp).
    pub fn set_position(&self, mut point: CGPoint) -> Result<CGPoint, AXError> {
        write_value(&self.element, "AXPosition", AXValueType::CGPoint, &mut point)?;
        Ok(self.frame().map_or(point, |f| f.origin))
    }
}

fn frame_of(element: &AXUIElement) -> Option<CGRect> {
    let mut origin = CGPoint::ZERO;
    let mut size = CGSize::ZERO;
    read_value(element, "AXPosition", AXValueType::CGPoint, &mut origin)?;
    read_value(element, "AXSize", AXValueType::CGSize, &mut size)?;
    Some(CGRect { origin, size })
}

fn copy_attr(element: &AXUIElement, name: &str) -> Option<CFRetained<CFType>> {
    copy_attr_or(element, name).ok().flatten()
}

/// `copy_attr`, with the app's error when it has no answer (`NoValue`: the attribute is empty).
fn copy_attr_or(element: &AXUIElement, name: &str) -> Result<Option<CFRetained<CFType>>, AXError> {
    let attr = CFString::from_str(name);
    let mut out: *const CFType = std::ptr::null();
    let err = unsafe { element.copy_attribute_value(&attr, NonNull::from(&mut out)) };
    if err != AXError::Success {
        return Err(err);
    }
    Ok(NonNull::new(out.cast_mut()).map(|ptr| unsafe { CFRetained::from_raw(ptr) }))
}

fn read_value<T>(element: &AXUIElement, name: &str, ty: AXValueType, out: &mut T) -> Option<()> {
    let value = copy_attr(element, name)?.downcast::<AXValue>().ok()?;
    unsafe { value.value(ty, NonNull::from(out).cast::<c_void>()) }.then_some(())
}

fn write_value<T>(element: &AXUIElement, name: &str, ty: AXValueType, value: &mut T) -> Result<(), AXError> {
    let value = unsafe { AXValue::new(ty, NonNull::from(value).cast::<c_void>()) }.ok_or(AXError::Failure)?;
    let attr = CFString::from_str(name);
    match unsafe { element.set_attribute_value(&attr, &value) } {
        AXError::Success => Ok(()),
        err => Err(err),
    }
}
