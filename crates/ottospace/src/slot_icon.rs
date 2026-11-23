//! A slot's app icon, by bundle id, so a waiting slot shows it whether its app runs or not: read
//! from the installed app and kept as `<home>/icons/<bundle>.png`, read again once the app's bundle
//! is newer. At most `KEEP` files. A test run that names no home keeps none on disk. Draw-only.

use std::fs;
use std::path::Path;

use ottospace_mac::icon;

/// The most icons kept on disk: the oldest go first.
const KEEP: usize = 64;

/// The file `bundle`'s icon is kept in: its id when it is one (`us.zoom.xos.png`), else none.
fn file_name(bundle: &str) -> Option<String> {
    let ok = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_');
    let id = !bundle.is_empty() && !bundle.starts_with('.') && bundle.chars().all(ok);
    id.then(|| format!("{bundle}.png"))
}

/// The icon of the app with bundle id `bundle`, `pixels` wide: the kept one while the app is not
/// newer, else drawn from the installed app and kept in `dir`. None: neither.
pub(crate) fn png(bundle: &str, dir: Option<&Path>, pixels: u8) -> Option<Vec<u8>> {
    let name = file_name(bundle);
    let kept = dir.zip(name.as_deref()).map(|(dir, name)| dir.join(name));
    let app = icon::app_of_bundle(bundle);
    let changed = |path: &Path| fs::metadata(path).and_then(|meta| meta.modified()).ok();
    let (app_at, kept_at) = (app.as_deref().and_then(changed), kept.as_deref().and_then(changed));
    // A kept file cut short (a crash) is no PNG: drawn again.
    if let (Some(kept), Some(kept_at)) = (&kept, kept_at)
        && app_at.is_none_or(|app_at| app_at <= kept_at)
        && let Ok(png) = fs::read(kept)
        && png.starts_with(PNG)
    {
        return Some(png);
    }
    let png = icon::png_of_app(app.as_deref()?, pixels)?;
    if let (Some(dir), Some(name)) = (dir, name)
        && crate::config::write_private(dir, &name, &png).is_ok()
    {
        prune(dir, KEEP);
    }
    Some(png)
}

/// A PNG file's first bytes.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Leaves the `most` newest icons in `dir`.
fn prune(dir: &Path, most: usize) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut icons: Vec<_> = entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    icons.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in icons.into_iter().skip(most) {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    #[test]
    fn an_icon_is_kept_by_its_bundle_id_only() {
        assert_eq!(file_name("us.zoom.xos").as_deref(), Some("us.zoom.xos.png"));
        for bad in ["", ".hidden", "../x", "a/b", "a b"] {
            assert_eq!(file_name(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_kept_icon_is_read_back_unless_cut_short_and_the_oldest_go_past_the_most() {
        let dir = Scratch::dir(&format!("ottospace-slot-icon-{}", std::process::id()));
        let not_installed = "com.example.ottospace.none";
        let name = file_name(not_installed).unwrap_or_default();
        let icon = [PNG, b"rest"].concat();
        crate::config::write_private(&dir, &name, &icon).ok();
        // Not installed: the kept one.
        assert_eq!(png(not_installed, Some(&dir), 48), Some(icon));
        crate::config::write_private(&dir, &name, b"cut").ok();
        assert_eq!(png(not_installed, Some(&dir), 48), None, "no PNG, and nothing to draw it from");
        for n in 0..3 {
            crate::config::write_private(&dir, &format!("app{n}.png"), b"x").ok();
        }
        prune(&dir, 2);
        assert_eq!(fs::read_dir(&dir).map(Iterator::count).unwrap_or_default(), 2);
    }
}
