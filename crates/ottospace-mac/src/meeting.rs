//! What the meeting timeline asks of macOS: time zones' offsets, whether the user is in a call,
//! the reduced-motion setting, the alert sound, and opening a link.

use std::ffi::c_void;

use objc2_app_kit::NSWorkspace;
use objc2_core_foundation::CFString;
use objc2_foundation::{NSString, NSURL};

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFTimeZoneCreateWithName(alloc: *const c_void, name: *const c_void, abbreviations: u8) -> *const c_void;
    fn CFTimeZoneGetSecondsFromGMT(zone: *const c_void, at: f64) -> f64;
    fn CFRelease(object: *const c_void);
}

/// Seconds from 1970 to 2001, CoreFoundation's epoch.
const SINCE_2001: i64 = 978_307_200;
/// Offsets are read this often: every change of offset falls on a quarter hour.
const STEP: i64 = 900;

/// The offsets of a named zone of the system's database (`Europe/Berlin`) from `from` to `to`
/// (UTC seconds), as (from when, seconds east of UTC). `None`: the system does not know it.
pub fn zone_table(name: &str, from: i64, to: i64) -> Option<Vec<(i64, i64)>> {
    let name = CFString::from_str(name);
    let zone = unsafe { CFTimeZoneCreateWithName(std::ptr::null(), (&*name as *const CFString).cast(), 0) };
    if zone.is_null() {
        return None;
    }
    let table = sample(from, to, |at| unsafe { CFTimeZoneGetSecondsFromGMT(zone, (at - SINCE_2001) as f64) } as i64);
    unsafe { CFRelease(zone) };
    Some(table)
}

/// The user's own zone's offsets, as `zone_table`.
pub fn local_table(from: i64, to: i64) -> Vec<(i64, i64)> {
    sample(from, to, crate::time::utc_offset)
}

fn sample(from: i64, to: i64, offset: impl Fn(i64) -> i64) -> Vec<(i64, i64)> {
    let mut table = vec![(i64::MIN, offset(from))];
    let mut at = from;
    while at <= to {
        let now = offset(at);
        if table.last().is_some_and(|(_, last)| *last != now) {
            table.push((at, now));
        }
        at += STEP;
    }
    table
}

/// Whether the user is in a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Call {
    InCall,
    NotInCall,
    /// No microphone, or CoreAudio did not answer.
    Unknown,
}

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectGetPropertyDataSize(
        object: u32,
        address: *const PropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
    ) -> i32;
    fn AudioObjectGetPropertyData(
        object: u32,
        address: *const PropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> i32;
}

/// `AudioObjectPropertyAddress`.
#[repr(C)]
struct PropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

const fn code(name: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*name)
}

/// In a call: some process has a microphone (any input device) running, as CoreAudio says
/// (`kAudioDevicePropertyDeviceIsRunningSomewhere`). Zoom, Meet in a browser and Teams all open
/// one. Nothing of another app is read: no window, title or log, and no permission is asked.
/// A microphone in use for something else (dictation, a recording) counts as a call too.
pub fn call() -> Call {
    let address = |selector: &[u8; 4], scope: &[u8; 4]| PropertyAddress {
        selector: code(selector),
        scope: code(scope),
        element: 0,
    };
    let (system, global, input) = (1, b"glob", b"inpt");
    let devices = address(b"dev#", global);
    let mut size = 0u32;
    if unsafe { AudioObjectGetPropertyDataSize(system, &devices, 0, std::ptr::null(), &mut size) } != 0 {
        return Call::Unknown;
    }
    let mut ids = vec![0u32; size as usize / 4];
    let read = unsafe {
        AudioObjectGetPropertyData(system, &devices, 0, std::ptr::null(), &mut size, ids.as_mut_ptr().cast())
    };
    if read != 0 {
        return Call::Unknown;
    }
    let (mut inputs, mut running) = (0, false);
    for id in ids {
        let streams = address(b"stm#", input);
        let mut streams_size = 0u32;
        let has = unsafe { AudioObjectGetPropertyDataSize(id, &streams, 0, std::ptr::null(), &mut streams_size) };
        if has != 0 || streams_size == 0 {
            continue;
        }
        inputs += 1;
        let (somewhere, mut on, mut on_size) = (address(b"gone", global), 0u32, 4u32);
        let got = unsafe {
            AudioObjectGetPropertyData(id, &somewhere, 0, std::ptr::null(), &mut on_size, (&mut on as *mut u32).cast())
        };
        running |= got == 0 && on != 0;
    }
    match (inputs, running) {
        (0, _) => Call::Unknown,
        (_, true) => Call::InCall,
        (_, false) => Call::NotInCall,
    }
}

/// The user asked for less motion (System Settings, Accessibility, Display).
pub fn reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// The system's alert sound, once.
pub fn beep() {
    objc2_app_kit::NSBeep();
}

/// Opens a link with the app that handles it (Zoom for a Zoom link, else the browser).
pub fn open_url(url: &str) -> bool {
    NSURL::URLWithString(&NSString::from_str(url)).is_some_and(|url| NSWorkspace::sharedWorkspace().openURL(&url))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_named_zone_has_its_summer_and_winter_offsets() {
        // 2026-03-01 to 2026-04-30 UTC: Berlin goes from +1 to +2 on 29 March, 01:00 UTC.
        let table = super::zone_table("Europe/Berlin", 1_772_323_200, 1_777_507_200).expect("a known zone");
        assert_eq!(table, [(i64::MIN, 3_600), (1_774_746_000, 7_200)]);
        assert_eq!(super::zone_table("Not/AZone", 0, 1), None);
    }
}
