//! System-wide hot keys, through Carbon's `RegisterEventHotKey`. They fire while any app is
//! frontmost and need no extra permission. ottospace uses one to take the keyboard back from a
//! hosted app, where its own shortcuts cannot fire, and one to capture the pane in use.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

/// One flag a key, by its id less 1: the return key (`RETURN`), the capture key (`CAPTURE`).
static PRESSED: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];
static INSTALLED: AtomicBool = AtomicBool::new(false);

const RETURN: u32 = 1;
const CAPTURE: u32 = 2;

#[repr(C)]
struct EventTypeSpec {
    class: u32,
    kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct EventHotKeyId {
    signature: u32,
    id: u32,
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
        count: usize, // ItemCount is `unsigned long`
        types: *const EventTypeSpec,
        user_data: *mut c_void,
        out_handler: *mut *mut c_void,
    ) -> i32;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        id: EventHotKeyId,
        target: *mut c_void,
        options: u32,
        out_hot_key: *mut *mut c_void,
    ) -> i32;
    fn GetEventParameter(
        event: *mut c_void,
        name: u32,
        kind: u32,
        out_kind: *mut u32,
        size: usize,
        out_size: *mut usize,
        out: *mut c_void,
    ) -> i32;
}

extern "C" fn on_hot_key(_: *mut c_void, event: *mut c_void, _: *mut c_void) -> i32 {
    // Which key: kEventParamDirectObject ('----') as typeEventHotKeyID ('hkid').
    let mut id = EventHotKeyId::default();
    let (name, kind, size) = (0x2d2d_2d2d, 0x686b_6964, std::mem::size_of::<EventHotKeyId>());
    let out = (&raw mut id).cast::<c_void>();
    let status = unsafe { GetEventParameter(event, name, kind, std::ptr::null_mut(), size, std::ptr::null_mut(), out) };
    let flag = (status == 0).then(|| PRESSED.get(id.id.wrapping_sub(1) as usize)).flatten();
    if let Some(flag) = flag {
        flag.store(true, Relaxed);
    }
    0
}

/// A key with its modifiers, e.g. `ctrl-alt-cmd-space`: its virtual key code and Carbon's
/// modifier bits. `None`: no such key, or a plain key with no modifier, which would take that
/// key from every app.
pub fn combo(name: &str) -> Option<(u32, u32)> {
    let name = name.to_ascii_lowercase();
    let mut parts: Vec<&str> = name.split('-').collect();
    let key = parts.pop()?;
    let mut modifiers = 0;
    for part in parts {
        modifiers |= match part {
            "cmd" => 0x0100,
            "shift" => 0x0200,
            "alt" | "option" => 0x0800,
            "ctrl" | "control" => 0x1000,
            _ => return None,
        };
    }
    // The ANSI layout's codes: a key is where it is on a US keyboard.
    const LETTERS: &str = "asdfhgzxcv\0bqweryt123465=97-80]ou[ip\0lj'k;\\,/nm.";
    let code = match key {
        "space" => 49,
        "f1" => 122,
        "f2" => 120,
        "f3" => 99,
        "f4" => 118,
        "f5" => 96,
        "f6" => 97,
        "f7" => 98,
        "f8" => 100,
        "f9" => 101,
        "f10" => 109,
        "f11" => 103,
        "f12" => 111,
        key if key.len() == 1 && key != "\0" => LETTERS.chars().position(|c| key.starts_with(c) && c != '\0')? as u32,
        key => key_code(key)?,
    };
    // Only ⇧ is no modifier either: ⇧A is a capital letter.
    let spare = key_code(key).is_some();
    (spare || modifiers & !0x0200 != 0).then_some((code, modifiers))
}

/// Virtual key code for a function-key name, e.g. `"f18"` → 79. Only F13-F20: keys that most
/// keyboards lack and apps rarely bind. (Key remappers such as Karabiner do use them.)
pub fn key_code(name: &str) -> Option<u32> {
    Some(match name.to_ascii_lowercase().as_str() {
        "f13" => 105,
        "f14" => 107,
        "f15" => 113,
        "f16" => 106,
        "f17" => 64,
        "f18" => 79,
        "f19" => 80,
        "f20" => 90,
        _ => return None,
    })
}

/// Registers `key_code` (no modifiers) as the hot key, exclusively. Call once, on the main
/// thread, after the app has started. Errors if any other process has registered the key:
/// without "exclusive", two ottospace instances would both answer one press.
pub fn register(key_code: u32) -> Result<(), String> {
    register_as(RETURN, key_code, 0)
}

/// Registers the capture key: a key with modifiers, as `combo` reads it. As `register`.
pub fn register_capture(key_code: u32, modifiers: u32) -> Result<(), String> {
    register_as(CAPTURE, key_code, modifiers)
}

fn register_as(id: u32, key_code: u32, modifiers: u32) -> Result<(), String> {
    // 'keyb' / kEventHotKeyPressed
    let pressed = EventTypeSpec { class: 0x6b65_7962, kind: 5 };
    unsafe {
        let target = GetApplicationEventTarget();
        // One handler for every key: it reads which one was pressed.
        if !INSTALLED.swap(true, Relaxed) {
            let status =
                InstallEventHandler(target, on_hot_key, 1, &pressed, std::ptr::null_mut(), std::ptr::null_mut());
            if status != 0 {
                INSTALLED.store(false, Relaxed);
                return Err(format!("could not install the hot key handler (status {status})"));
            }
        }
        let id = EventHotKeyId { signature: 0x6f74_746f, id }; // 'otto'
        let mut hot_key = std::ptr::null_mut();
        const EXCLUSIVE: u32 = 1; // kEventHotKeyExclusive
        let status = RegisterEventHotKey(key_code, modifiers, id, target, EXCLUSIVE, &mut hot_key);
        if status != 0 {
            return Err(format!("the key is taken by another app (status {status})"));
        }
    }
    Ok(())
}

/// True once per press of the return key since the last call.
pub fn take_press() -> bool {
    PRESSED[RETURN as usize - 1].swap(false, Relaxed)
}

/// True once per press of the capture key since the last call.
pub fn take_capture() -> bool {
    PRESSED[CAPTURE as usize - 1].swap(false, Relaxed)
}

#[cfg(test)]
mod tests {
    use super::{combo, key_code};

    #[test]
    fn a_capture_key_is_a_key_with_modifiers() {
        // ⌃⌥⌘Space: space is 49; control 0x1000, option 0x800, command 0x100.
        assert_eq!(combo("ctrl-alt-cmd-space"), Some((49, 0x1900)));
        assert_eq!(combo("Cmd-Shift-2"), Some((19, 0x0300)));
        assert_eq!((combo("ctrl-a"), combo("ctrl-s"), combo("alt-m"), combo("ctrl-/")), (Some((0, 0x1000)), Some((1, 0x1000)), Some((46, 0x0800)), Some((44, 0x1000))));
        assert_eq!((combo("f19"), combo("ctrl-f1")), (Some((80, 0)), Some((122, 0x1000))));
        // A plain key, or one with ⇧ alone, would be taken from every app.
        assert_eq!((combo("space"), combo("shift-a"), combo("a"), combo("f1")), (None, None, None, None));
        assert_eq!((combo("hyper-space"), combo("ctrl-nope"), combo(""), combo("ctrl-")), (None, None, None, None));
    }

    #[test]
    fn only_spare_function_keys_are_accepted() {
        assert_eq!(key_code("F18"), Some(79));
        assert_eq!(key_code("f13"), Some(105));
        assert_eq!(key_code("f1"), None);
        assert_eq!(key_code("a"), None);
    }
}
