//! The local time zone, which the core has no way to know.

use std::ffi::{c_int, c_long};

/// `struct tm` (`<time.h>`), with macOS's `tm_gmtoff` and `tm_zone`.
#[repr(C)]
#[derive(Default)]
struct Tm {
    fields: [c_int; 9],
    gmtoff: c_long,
    /// `tm_zone`, a pointer: never read.
    zone: usize,
}

unsafe extern "C" {
    fn localtime_r(time: *const i64, out: *mut Tm) -> *mut Tm;
}

/// Seconds east of UTC in the user's time zone at `at` (seconds since 1970): it differs in
/// summer and winter. 0 if it cannot be read.
pub fn utc_offset(at: i64) -> i64 {
    let mut tm = Tm::default();
    let read = unsafe { localtime_r(&at, &mut tm) };
    if read.is_null() { 0 } else { tm.gmtoff as i64 }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_offset_is_whole_minutes_within_a_day() {
        let offset = super::utc_offset(1_794_595_739);
        assert!(offset.abs() <= 14 * 3600 && offset % 60 == 0, "{offset}");
    }
}
