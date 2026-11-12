//! App icons, as PNG bytes the shell can draw.

use objc2::AnyThread;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSRunningApplication};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize};

/// The icon of the running app `pid`, drawn `pixels` wide and high. `None` if the process is gone
/// or has no icon.
pub fn png(pid: i32, pixels: f64) -> Option<Vec<u8>> {
    let icon = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?.icon()?;
    let mut rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(pixels, pixels));
    // AppKit picks the icon's best size for the rect. No context: one pixel per point.
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
        assert!(super::png(-1, 64.).is_none(), "no such process");
        // Finder runs in every login session. Without one (a bare SSH session) there is no app to ask.
        let finder = Command::new("pgrep").args(["-x", "Finder"]).output().expect("pgrep");
        let Some(pid) = String::from_utf8_lossy(&finder.stdout).lines().next().and_then(|l| l.parse().ok()) else {
            return;
        };
        let png = super::png(pid, 64.).expect("Finder's icon");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        // Width and height from the PNG header: the size asked for.
        assert_eq!((&png[16..20], &png[20..24]), (&[0, 0, 0, 64][..], &[0, 0, 0, 64][..]));
    }
}
