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
    let (mut before, mut at) = (from, from.div_euclid(STEP) * STEP + STEP);
    while at <= to + STEP {
        let now = offset(at);
        if let Some(&(_, last)) = table.last()
            && last != now
        {
            // The second it changed, between the two samples.
            let (mut low, mut high) = (before, at);
            while high - low > 1 {
                let mid = low + (high - low) / 2;
                if offset(mid) == last { low = mid } else { high = mid }
            }
            table.push((high, now));
        }
        (before, at) = (at, at + STEP);
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

/// In a call: some process records from a microphone, as CoreAudio says of each audio process
/// (`kAudioProcessPropertyIsRunningInput`, macOS 14.2; before it, any input device running,
/// `kAudioDevicePropertyDeviceIsRunningSomewhere`). Zoom, Meet in a browser and Teams all record.
/// Nothing of another app is read: no name, window, title or log, and no permission is asked.
/// A microphone in use for something else (dictation, a recording) counts as a call too. A read
/// that fails makes the answer `Unknown`, unless another one already says `InCall`.
pub fn call() -> Call {
    // No process list (before 14.2): the input devices. None of those: no microphone to tell by.
    let (list, flag, unknown) = match objects(b"prs#") {
        Some(list) => (list, b"piri", false),
        None => {
            let inputs: Vec<u32> =
                objects(b"dev#").unwrap_or_default().into_iter().filter(|id| has_input(*id)).collect();
            let none = inputs.is_empty();
            (inputs, b"gone", none)
        }
    };
    let (mut running, mut failed) = (false, unknown);
    for id in list {
        match read_u32(id, flag) {
            Some(on) => running |= on != 0,
            None => failed = true,
        }
    }
    match (running, failed) {
        (true, _) => Call::InCall,
        (false, true) => Call::Unknown,
        (false, false) => Call::NotInCall,
    }
}

fn address(selector: &[u8; 4], scope: &[u8; 4]) -> PropertyAddress {
    PropertyAddress { selector: code(selector), scope: code(scope), element: 0 }
}

/// A list of audio object ids the system object has (`prs#` processes, `dev#` devices).
fn objects(selector: &[u8; 4]) -> Option<Vec<u32>> {
    let (system, what) = (1, address(selector, b"glob"));
    let mut size = 0u32;
    if unsafe { AudioObjectGetPropertyDataSize(system, &what, 0, std::ptr::null(), &mut size) } != 0 {
        return None;
    }
    let mut ids = vec![0u32; size as usize / 4];
    let read =
        unsafe { AudioObjectGetPropertyData(system, &what, 0, std::ptr::null(), &mut size, ids.as_mut_ptr().cast()) };
    (read == 0).then(|| ids.into_iter().take(size as usize / 4).collect())
}

fn has_input(device: u32) -> bool {
    let mut size = 0u32;
    let streams = address(b"stm#", b"inpt");
    unsafe { AudioObjectGetPropertyDataSize(device, &streams, 0, std::ptr::null(), &mut size) == 0 && size > 0 }
}

fn read_u32(object: u32, selector: &[u8; 4]) -> Option<u32> {
    let (what, mut value, mut size) = (address(selector, b"glob"), 0u32, 4u32);
    let read = unsafe {
        AudioObjectGetPropertyData(object, &what, 0, std::ptr::null(), &mut size, (&mut value as *mut u32).cast())
    };
    (read == 0).then_some(value)
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
        // Read from a time off the quarter hour: the change still falls on its own second.
        let off = super::zone_table("Europe/Berlin", 1_772_323_637, 1_777_507_200).expect("a known zone");
        assert_eq!(off, table);
        assert_eq!(super::zone_table("Not/AZone", 0, 1), None);
    }
}
