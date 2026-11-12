//! App icons, as PNG bytes the shell can draw.

use objc2::AnyThread;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSRunningApplication};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize};

/// The icon of the running app `pid`, drawn `points` wide and high: that many pixels, or twice
/// as many when the main display is Retina. `None` if the process is gone or has no icon.
pub fn png(pid: i32, points: f64) -> Option<Vec<u8>> {
    let icon = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?.icon()?;
    let mut rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(points, points));
    // AppKit picks the icon's best size for the rect. No context: the main display's scale.
    let image = unsafe { icon.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
    let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &image);
    let data = unsafe { bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
    Some(data.to_vec())
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    #[test]
    fn a_running_app_has_a_png_icon() {
        assert!(super::png(-1, 32.).is_none(), "no such process");
        // Finder runs in every login session. Without one (a bare SSH session) there is no app to ask.
        let finder = Command::new("pgrep").args(["-x", "Finder"]).output().expect("pgrep");
        let Some(pid) = String::from_utf8_lossy(&finder.stdout).lines().next().and_then(|l| l.parse().ok()) else {
            return;
        };
        let png = super::png(pid, 32.).expect("Finder's icon");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        // Width and height from the PNG header: square, the size asked for or its Retina double.
        let side = |bytes: &[u8]| bytes.iter().fold(0u32, |n, b| n << 8 | u32::from(*b));
        let (width, height) = (side(&png[16..20]), side(&png[20..24]));
        assert!(width == height && [32, 64].contains(&width), "{width}x{height}");
    }
}
