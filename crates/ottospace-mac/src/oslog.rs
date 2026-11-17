//! The unified log, subsystem `dev.ottospace`, category `events`:
//! `log show --predicate 'subsystem == "dev.ottospace"' --last 1d`.

use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

unsafe extern "C" {
    /// This image's Mach-O header: the unified log finds the format string through it.
    static __dso_handle: u8;
    fn os_log_create(subsystem: *const c_char, category: *const c_char) -> *mut c_void;
    /// What the `os_log` macro calls. `buf`: the arguments, as the compiler would encode them.
    fn _os_log_impl(dso: *const c_void, log: *mut c_void, kind: u8, format: *const c_char, buf: *const u8, size: u32);
}

/// `OS_LOG_TYPE_DEFAULT`: kept on disk, shown by `log show` with no flag.
const DEFAULT: u8 = 0;

/// The log's handle, made once; an address, so it may cross threads (os_log is thread-safe).
fn handle() -> *mut c_void {
    static LOG: OnceLock<usize> = OnceLock::new();
    *LOG.get_or_init(|| unsafe { os_log_create(c"dev.ottospace".as_ptr(), c"events".as_ptr()) } as usize)
        as *mut c_void
}

/// Writes `line` as one public string. A NUL in it ends it there.
pub fn write(line: &str) {
    let text: Vec<u8> = line.bytes().take_while(|b| *b != 0).chain([0]).collect();
    let pointer = (text.as_ptr() as usize as u64).to_ne_bytes();
    // One argument: a summary byte (2: it has a pointer) and the count, then the item: its
    // descriptor (0x22: a string, public), its size and the pointer.
    let mut buf = [2u8, 1, 0x22, 8, 0, 0, 0, 0, 0, 0, 0, 0];
    buf[4..].copy_from_slice(&pointer);
    let dso = unsafe { &__dso_handle as *const u8 as *const c_void };
    unsafe { _os_log_impl(dso, handle(), DEFAULT, c"%{public}s".as_ptr(), buf.as_ptr(), buf.len() as u32) };
}
