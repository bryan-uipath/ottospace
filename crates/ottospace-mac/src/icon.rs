//! App icons, as PNG bytes the shell can draw.

use objc2::AnyThread;
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSDeviceRGBColorSpace, NSGraphicsContext, NSImageInterpolation,
    NSRunningApplication,
};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize};

/// The icon of the running app `pid`, drawn `pixels` wide and high, so the shell can show it
/// pixel for pixel. AppKit scales it from the icon's nearest size, which is far smoother than
/// scaling in the shell. `None` if the process is gone or has no icon.
pub fn png(pid: i32, pixels: u8) -> Option<Vec<u8>> {
    let icon = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?.icon()?;
    let side = isize::from(pixels);
    // An empty RGBA bitmap of that size (null planes: AppKit allocates it), one pixel per point.
    let bitmap = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(), std::ptr::null_mut(), side, side, 8, 4, true, false, NSDeviceRGBColorSpace, 0, 0,
        )
    }?;
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap)?;
    // Draw into the bitmap, then put back whatever context this thread was drawing to.
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&context));
    context.setImageInterpolation(NSImageInterpolation::High);
    icon.drawInRect(NSRect::new(NSPoint::new(0., 0.), NSSize::new(f64::from(pixels), f64::from(pixels))));
    NSGraphicsContext::restoreGraphicsState_class();
    let data = unsafe { bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
    Some(data.to_vec())
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    #[test]
    fn a_running_app_has_a_png_icon() {
        assert!(super::png(-1, 48).is_none(), "no such process");
        // Finder runs in every login session. Without one (a bare SSH session) there is no app to ask.
        let finder = Command::new("pgrep").args(["-x", "Finder"]).output().expect("pgrep");
        let Some(pid) = String::from_utf8_lossy(&finder.stdout).lines().next().and_then(|l| l.parse().ok()) else {
            return;
        };
        let png = super::png(pid, 48).expect("Finder's icon");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        // Width and height from the PNG header: the size asked for, whatever the display's scale.
        assert_eq!((&png[16..20], &png[20..24]), (&[0, 0, 0, 48][..], &[0, 0, 0, 48][..]));
    }
}
