//! Other apps' windows via the Accessibility API.
//! Coordinates are global, top-left origin (same space as CGDisplayBounds).

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
use objc2_application_services::{AXError, AXIsProcessTrustedWithOptions, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize};

// Private but long-stable; maps an AX window to its CGWindowID (needed to pick the SCWindow).
unsafe extern "C" {
    fn _AXUIElementGetWindow(element: &AXUIElement, id: *mut u32) -> AXError;
}

/// The bundle id (`us.zoom.xos`) and the name (`zoom.us`) of the app with this pid.
pub fn app_identity(pid: i32) -> (Option<String>, Option<String>) {
    let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else { return (None, None) };
    (app.bundleIdentifier().map(|id| id.to_string()), app.localizedName().map(|name| name.to_string()))
}

/// Whether this process (or its parent terminal) has Accessibility access; optionally prompts.
/// True while the app with this pid is the frontmost app (it has the keyboard).
pub fn is_active(pid: i32) -> bool {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid).is_some_and(|app| app.isActive())
}

/// True if the app with this pid is the front app, asked of the app itself. `is_active` reads a
/// value AppKit keeps for this process, which lags until the run loop has seen the change; this
/// asks now.
pub fn is_frontmost(pid: i32) -> bool {
    let app = unsafe { AXUIElement::new_application(pid) };
    copy_attr(&app, "AXFrontmost").and_then(|v| v.downcast::<CFBoolean>().ok()).is_some_and(|front| front.as_bool())
}

pub fn is_trusted(prompt: bool) -> bool {
    let key = CFString::from_static_str("AXTrustedCheckOptionPrompt");
    let value = CFBoolean::new(prompt);
    let options = CFDictionary::from_slices(&[&*key], &[value]);
    unsafe { AXIsProcessTrustedWithOptions(Some(options.as_opaque())) }
}

pub struct Window {
    element: CFRetained<AXUIElement>,
    pid: i32,
}

impl Window {
    /// The app's standard windows (skips the desktop, panels, popovers).
    pub fn standard_of(pid: i32) -> Vec<Self> {
        let app = unsafe { AXUIElement::new_application(pid) };
        let Some(all) = copy_attr(&app, "AXWindows").and_then(|a| a.downcast::<CFArray>().ok()) else {
            return Vec::new();
        };
        let all = unsafe { CFRetained::cast_unchecked::<CFArray<AXUIElement>>(all) };
        all.iter().map(|element| Self { element, pid }).filter(Self::is_standard).collect()
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

    /// The standard window with this window-server id, if `pid` still has it.
    pub fn find(pid: i32, id: u32) -> Option<Self> {
        Self::standard_of(pid).into_iter().find(|w| w.id() == Some(id))
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
    let attr = CFString::from_str(name);
    let mut out: *const CFType = std::ptr::null();
    let err = unsafe { element.copy_attribute_value(&attr, NonNull::from(&mut out)) };
    if err != AXError::Success {
        return None;
    }
    NonNull::new(out.cast_mut()).map(|ptr| unsafe { CFRetained::from_raw(ptr) })
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
