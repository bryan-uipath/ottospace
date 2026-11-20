//! The user's language and region as macOS's settings name them, which the core has no way to know.

use std::ffi::{CStr, c_char, c_void};

unsafe extern "C" {
    fn CFLocaleCopyCurrent() -> *const c_void;
    fn CFLocaleGetIdentifier(locale: *const c_void) -> *const c_void;
    fn CFStringGetCString(string: *const c_void, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
    fn CFRelease(object: *const c_void);
}

/// `kCFStringEncodingUTF8`.
const UTF8: u32 = 0x0800_0100;

/// The current locale's identifier, e.g. `en_US` or `de_CH@rg=chzzzz`; `None` if it cannot be read.
pub fn identifier() -> Option<String> {
    // SAFETY: CFLocaleCopyCurrent returns an owned locale, released below; the identifier is
    // borrowed from it and copied out first.
    unsafe {
        let locale = CFLocaleCopyCurrent();
        if locale.is_null() {
            return None;
        }
        let id = CFLocaleGetIdentifier(locale);
        let mut buffer = [0 as c_char; 128];
        let read = !id.is_null() && CFStringGetCString(id, buffer.as_mut_ptr(), buffer.len() as isize, UTF8) != 0;
        let text = read.then(|| CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned());
        CFRelease(locale);
        text
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_locale_reads_as_language_and_region() {
        let id = super::identifier().expect("a locale");
        assert!(id.chars().next().is_some_and(|c| c.is_ascii_lowercase()), "{id}");
    }
}
