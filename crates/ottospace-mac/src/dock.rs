//! The apps the user keeps in the Dock, read from the Dock's preferences file. Read-only.

use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{
    NSArray, NSData, NSDictionary, NSPropertyListMutabilityOptions, NSPropertyListSerialization, NSString, NSURL,
};

/// The bundle of each app kept in the Dock, in the Dock's order, as its preferences name it.
/// Not resolved, and not checked to exist. Empty if the preferences cannot be read.
pub fn persistent_apps() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let file = home.join("Library/Preferences/com.apple.dock.plist");
    std::fs::read(file).map(|plist| apps_in(&plist)).unwrap_or_default()
}

/// The same from the bytes of a `com.apple.dock.plist`, binary or XML: for each tile of
/// `persistent-apps`, the path in `tile-data` → `file-data` → `_CFURLString`.
pub fn apps_in(plist: &[u8]) -> Vec<PathBuf> {
    let data = NSData::with_bytes(plist);
    let options = NSPropertyListMutabilityOptions::Immutable;
    let read =
        unsafe { NSPropertyListSerialization::propertyListWithData_options_format_error(&data, options, std::ptr::null_mut()) };
    let Some(tiles) = read.ok().and_then(|root| child(root, "persistent-apps")) else { return Vec::new() };
    let Ok(tiles) = tiles.downcast::<NSArray>() else { return Vec::new() };
    let path = |tile: Retained<AnyObject>| {
        let url = child(child(child(tile, "tile-data")?, "file-data")?, "_CFURLString")?;
        let url = url.downcast::<NSString>().ok()?;
        // A URL (`file:///Applications/Notes.app/`) on current systems; older ones wrote a path.
        match url.to_string() {
            text if text.starts_with("file:") => Some(PathBuf::from(NSURL::URLWithString(&url)?.path()?.to_string())),
            text => Some(PathBuf::from(text)),
        }
    };
    tiles.iter().filter_map(path).collect()
}

/// `dict[key]`, if `dict` is a dictionary that has it.
fn child(dict: Retained<AnyObject>, key: &str) -> Option<Retained<AnyObject>> {
    let dict = dict.downcast::<NSDictionary>().ok()?;
    dict.objectForKey(&NSString::from_str(key))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    /// A made-up Dock: never the user's own preferences.
    const DOCK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
  <key>orientation</key><string>bottom</string>
  <key>persistent-apps</key><array>
    <dict><key>tile-data</key><dict>
      <key>file-data</key><dict>
        <key>_CFURLString</key><string>file:///Applications/Lumen%20Desk.app/</string>
        <key>_CFURLStringType</key><integer>15</integer>
      </dict>
      <key>file-label</key><string>Lumen Desk</string>
    </dict><key>tile-type</key><string>file-tile</string></dict>
    <dict><key>tile-data</key><dict>
      <key>file-data</key><dict>
        <key>_CFURLString</key><string>/Applications/Old Style.app</string>
        <key>_CFURLStringType</key><integer>0</integer>
      </dict>
    </dict></dict>
    <dict><key>tile-type</key><string>spacer-tile</string><key>tile-data</key><dict/></dict>
    <string>not a tile</string>
    <dict><key>tile-data</key><dict>
      <key>file-data</key><dict>
        <key>_CFURLString</key><string>file:///System/Applications/Notes.app/</string>
      </dict>
    </dict></dict>
  </array>
  <key>persistent-others</key><array>
    <dict><key>tile-data</key><dict><key>file-data</key><dict>
      <key>_CFURLString</key><string>file:///Users/me/Downloads/</string>
    </dict></dict></dict>
  </array>
</dict></plist>"#;

    #[test]
    fn the_docks_apps_are_read_in_order_and_what_is_no_app_tile_is_passed_over() {
        let apps = super::apps_in(DOCK.as_bytes());
        let wanted = ["/Applications/Lumen Desk.app", "/Applications/Old Style.app", "/System/Applications/Notes.app"];
        assert_eq!(apps, wanted.map(PathBuf::from));
        // Not a property list, a list with no Dock in it, a Dock of another shape: no apps.
        for other in ["", "not a plist", "<plist><array/></plist>", "<plist><dict><key>persistent-apps</key><dict/></dict></plist>"] {
            assert!(super::apps_in(other.as_bytes()).is_empty(), "{other}");
        }
    }
}
