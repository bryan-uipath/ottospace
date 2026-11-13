//! Presses one element of a hosted window for sneak mode: the user typed its label. The element
//! is found again by its path, checked (`ottospace_core::sneak_apps::check`), and that same
//! element is pressed (`AXPress`) or, a text field, focused. No value is set, no event posted.

use objc2_application_services::AXError;
use objc2_core_foundation::{CFBoolean, CFString};
use ottospace_core::ax::Frame;
use ottospace_core::sneak_apps::{self, Press};

use crate::ax::Window;
use crate::ax_tree;

/// What the status bar says when nothing was pressed.
pub const GONE: &str = "the window is gone: nothing was pressed";
const BUSY: &str = "the control is gone or the app did not answer: nothing was pressed";
const REFUSED: &str = "the app refused the press";

/// Finds the element at `path` in the window `id` of `pid` and presses it, if it is still the one
/// labelled (`role`, `at`). Blocking AX IPC with the tree reader's short timeout: off the UI thread.
pub fn press(pid: i32, id: u32, path: &[usize], role: &str, at: Frame) -> Result<Press, &'static str> {
    let window = Window::find(pid, id).ok_or(GONE)?;
    let f = window.frame().ok_or(GONE)?;
    let frame = Frame { x: f.origin.x, y: f.origin.y, w: f.size.width, h: f.size.height };
    let mut element = window.element();
    for &i in path {
        let (_, children) = ax_tree::node_of(&element).ok_or(BUSY)?;
        // As the reader counted them: a child it could not read was left out.
        let mut readable = children.into_iter().filter(|child| ax_tree::node_of(child).is_some());
        element = readable.nth(i).ok_or(sneak_apps::MOVED)?;
    }
    let (now, _) = ax_tree::node_of(&element).ok_or(BUSY)?;
    let press = sneak_apps::check(role, at, frame, &now)?;
    // The element just checked, not one found again: nothing between the check and the press
    // but the app's own changes in that instant.
    let err = match press {
        Press::Press => unsafe { element.perform_action(&CFString::from_static_str("AXPress")) },
        Press::Focus => unsafe {
            element.set_attribute_value(&CFString::from_static_str("AXFocused"), CFBoolean::new(true))
        },
    };
    match err {
        AXError::Success => Ok(press),
        AXError::CannotComplete => Err(BUSY),
        AXError::InvalidUIElement => Err(sneak_apps::MOVED),
        _ => Err(REFUSED),
    }
}
