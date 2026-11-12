//! App icons, as PNG bytes the shell can draw.

use std::path::{Path, PathBuf};

use objc2::AnyThread;
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSDeviceRGBColorSpace, NSGraphicsContext, NSImage, NSImageInterpolation,
    NSRunningApplication, NSWorkspace,
};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

/// The icon of the running app `pid`, drawn `pixels` wide and high, so the shell can show it
/// pixel for pixel. AppKit scales it from the icon's nearest size, which is far smoother than
/// scaling in the shell. `None` if the process is gone or has no icon.
pub fn png(pid: i32, pixels: u8) -> Option<Vec<u8>> {
    let icon = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?.icon()?;
    draw(&icon, pixels)
}

/// The icon of the app bundle at `path` (an installed app that need not be running).
pub fn png_of_app(path: &Path, pixels: u8) -> Option<Vec<u8>> {
    path.is_dir().then(|| ())?;
    draw(&NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&path.to_string_lossy())), pixels)
}

/// Where the app called `name` is installed, looking where apps usually are; resolved, so it
/// compares equal to `bundle_of` for the same app whatever the case of the letters typed.
pub fn app_path(name: &str) -> Option<PathBuf> {
    // A name, not a path: it must not walk out of the folders below.
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        return None;
    }
    let found = app_folders().iter().map(|folder| folder.join(format!("{name}.app"))).find(|app| app.is_dir())?;
    found.canonicalize().ok()
}

/// Where apps usually are.
pub fn app_folders() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    vec![
        PathBuf::from("/Applications"),
        home.join("Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/Applications/Utilities"),
        PathBuf::from("/System/Applications/Utilities"),
    ]
}

/// Every app bundle directly in `folders`, as (name, resolved path): `("Notes", "/System/Applications/Notes.app")`.
/// Reads the folders only: slow on a cold disk, so not for the main thread.
pub fn apps_in(folders: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let entries = folders.iter().filter_map(|folder| std::fs::read_dir(folder).ok()).flatten().flatten();
    let app = |path: PathBuf| {
        let name = path.file_name()?.to_str()?.strip_suffix(".app")?.to_string();
        (!name.starts_with('.') && path.is_dir()).then_some(())?;
        Some((name, path.canonicalize().ok()?))
    };
    entries.filter_map(|entry| app(entry.path())).collect()
}

/// Where Launch Services says the app with this bundle id is installed. `None`: it is not.
pub fn app_of_bundle(bundle: &str) -> Option<PathBuf> {
    let url = NSWorkspace::sharedWorkspace().URLForApplicationWithBundleIdentifier(&NSString::from_str(bundle))?;
    PathBuf::from(url.path()?.to_string()).canonicalize().ok()
}

/// Where an app is installed: by its bundle id if it has one, else by its name where apps
/// usually are. `None`: it is not installed.
pub fn installed(bundle: Option<&str>, name: &str) -> Option<PathBuf> {
    bundle.and_then(app_of_bundle).or_else(|| app_path(name))
}

/// The app bundle the process `pid` runs from, resolved. `None` for a bare executable.
pub fn bundle_of(pid: i32) -> Option<PathBuf> {
    let url = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?.bundleURL()?;
    PathBuf::from(url.path()?.to_string()).canonicalize().ok()
}

fn draw(icon: &NSImage, pixels: u8) -> Option<Vec<u8>> {
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

    #[test]
    fn an_installed_app_is_found_by_name_whether_or_not_it_runs() {
        // A name is not a path, and its letters may be typed in any case.
        let calculator = super::app_path("calculator").expect("Calculator is part of macOS");
        assert_eq!(calculator.file_name().and_then(|name| name.to_str()), Some("Calculator.app"));
        assert!(super::png_of_app(&calculator, 48).is_some_and(|png| png.starts_with(b"\x89PNG")));
        assert_eq!((super::app_path("../Calculator"), super::app_path("No Such App 9")), (None, None));
        assert_eq!(super::bundle_of(-1), None);
        // The scan finds it where `app_path` does.
        let apps = super::apps_in(&super::app_folders());
        assert!(apps.contains(&("Calculator".to_string(), calculator)));
        assert!(super::apps_in(&["/no/such/folder".into()]).is_empty());
    }
}
