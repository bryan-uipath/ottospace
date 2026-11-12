//! One system-wide hot key, through Carbon's `RegisterEventHotKey`. It fires while any app is
//! frontmost and needs no extra permission. ottospace uses it to take the keyboard back from a
//! hosted app, where its own shortcuts cannot fire.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

static PRESSED: AtomicBool = AtomicBool::new(false);

#[repr(C)]
struct EventTypeSpec {
    class: u32,
    kind: u32,
}

#[repr(C)]
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
        count: u32,
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
}

extern "C" fn on_hot_key(_: *mut c_void, _: *mut c_void, _: *mut c_void) -> i32 {
    PRESSED.store(true, Relaxed);
    0
}

/// Virtual key code for a function-key name, e.g. `"f18"` → 79. Only F13-F20: keys no app uses,
/// so taking one system-wide breaks nothing.
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

/// Registers `key_code` (no modifiers) as the hot key. Call once, on the main thread, after the
/// app has started. Errors if another app, or another ottospace, already holds the key.
pub fn register(key_code: u32) -> Result<(), String> {
    // 'keyb' / kEventHotKeyPressed
    let pressed = EventTypeSpec { class: 0x6b65_7962, kind: 5 };
    unsafe {
        let target = GetApplicationEventTarget();
        let status =
            InstallEventHandler(target, on_hot_key, 1, &pressed, std::ptr::null_mut(), std::ptr::null_mut());
        if status != 0 {
            return Err(format!("could not install the hot key handler (status {status})"));
        }
        let id = EventHotKeyId { signature: 0x6f74_746f, id: 1 }; // 'otto'
        let mut hot_key = std::ptr::null_mut();
        let status = RegisterEventHotKey(key_code, 0, id, target, 0, &mut hot_key);
        if status != 0 {
            return Err(format!("the key is taken by another app (status {status})"));
        }
    }
    Ok(())
}

/// True once per press since the last call.
pub fn take_press() -> bool {
    PRESSED.swap(false, Relaxed)
}

#[cfg(test)]
mod tests {
    use super::key_code;

    #[test]
    fn only_spare_function_keys_are_accepted() {
        assert_eq!(key_code("F18"), Some(79));
        assert_eq!(key_code("f13"), Some(105));
        assert_eq!(key_code("f1"), None);
        assert_eq!(key_code("a"), None);
    }
}
