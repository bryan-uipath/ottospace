use objc2_core_foundation::{CGPoint, CGRect};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
}

/// Display bounds in global top-left-origin coordinates (same space as AX).
pub fn bounds(display: u32) -> CGRect {
    unsafe { CGDisplayBounds(display) }
}

pub fn all() -> Vec<CGRect> {
    let mut ids = [0u32; 16];
    let mut count = 0;
    unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
    ids[..count as usize].iter().map(|&id| bounds(id)).collect()
}

/// Where to park a window so only its top-left pixel is on-screen (AeroSpace-style).
/// Picks a display's bottom-right corner with no other display to its right or below,
/// e.g. the right-hand monitor's corner in a side-by-side setup.
pub fn parking_spot() -> CGPoint {
    let displays = all();
    let far = 100_000.0;
    let overlaps = |x: f64, y: f64| {
        displays.iter().any(|d| {
            d.origin.x + d.size.width > x && d.origin.y + d.size.height > y && d.origin.x < x + far && d.origin.y < y + far
        })
    };
    let corner = |d: &CGRect| CGPoint::new(d.origin.x + d.size.width - 1.0, d.origin.y + d.size.height - 1.0);
    displays
        .iter()
        .map(corner)
        .find(|p| !overlaps(p.x + 1.0, p.y) && !overlaps(p.x, p.y + 1.0))
        .unwrap_or_else(|| corner(&bounds(unsafe { CGMainDisplayID() })))
}
