//! Files on the clipboard: what Copy in Finder puts there. Read-only.

use std::path::PathBuf;

use objc2_app_kit::{NSPasteboard, NSPasteboardTypeFileURL};
use objc2_foundation::NSURL;

/// The files and folders on the general pasteboard, in its order. None: it holds text, an
/// image, or nothing.
pub fn files() -> Vec<PathBuf> {
    let Some(items) = NSPasteboard::generalPasteboard().pasteboardItems() else { return Vec::new() };
    let file = |item: objc2::rc::Retained<objc2_app_kit::NSPasteboardItem>| {
        let url = item.stringForType(unsafe { NSPasteboardTypeFileURL })?;
        Some(PathBuf::from(NSURL::URLWithString(&url)?.path()?.to_string()))
    };
    items.into_iter().filter_map(file).collect()
}
