//! Hosted windows' native title bars: whether to crop one, and by how much.
//! An app pane shows one bar: ottospace's header while parked (the app's bar is cropped from
//! the stream), the app's own bar while live (the window is raised to cover the header).

use crate::Rect;

/// What AX reports about a window's top edge. Global top-left coordinates, points.
pub struct TitleBar {
    pub window: Rect,
    pub close: Option<Rect>,
    pub title: Option<Rect>,
    /// A toolbar child (unified title bar and toolbar, e.g. Finder). Unknown counts as one.
    pub toolbar: bool,
}

/// Height of a plain native title bar, or 0: leave the window uncropped.
/// Plain: a close button and title text centred in one strip, no toolbar. Chrome and Electron
/// windows report no title element; a unified toolbar makes the title as tall as the toolbar.
pub fn height(bar: &TitleBar) -> f32 {
    let (Some(close), Some(title), false) = (bar.close, bar.title, bar.toolbar) else { return 0.0 };
    // The buttons sit centred in the bar: a 16pt button 8pt down means a 32pt bar.
    let height = 2.0 * (close.y - bar.window.y) + close.h;
    let centred = ((title.y - bar.window.y + title.h / 2.0) - height / 2.0).abs() <= 1.0;
    let plain = (16.0..=40.0).contains(&height) && title.h <= close.h + 4.0 && centred;
    if plain { height } else { 0.0 }
}

/// The crop for a pane whose content top is at `top`: `bar`, unless the raised live window would
/// reach above `limit` (ottospace's top bar). Then 0, and the pane behaves as without a crop.
pub fn crop(bar: f32, top: f32, limit: f32) -> f32 {
    if top - bar >= limit { bar } else { 0.0 }
}

/// Top inset of an app pane's content below a `header` pt header, so a raised bar of `bar` pt
/// covers exactly the header and no more. At least 2pt, so a live window spares the pane border.
pub fn inset(bar: f32, header: f32) -> f32 {
    (bar - header).max(2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    // Measured 2026-11-11, macOS 26: window frame, close button, title element, toolbar child.
    fn terminal() -> TitleBar {
        let window = rect(3034., 349., 580., 385.);
        TitleBar {
            window,
            close: Some(rect(3042., 357., 16., 16.)),
            title: Some(rect(3134., 357., 199., 16.)),
            toolbar: false,
        }
    }

    #[test]
    fn a_plain_title_bar_is_cropped_by_its_height() {
        assert_eq!(height(&terminal()), 32.0);
    }

    #[test]
    fn a_unified_toolbar_is_not_cropped() {
        // Finder: the title is as tall as the 52pt toolbar.
        let window = rect(500., 544., 920., 436.);
        let finder = TitleBar {
            window,
            close: Some(rect(518., 562., 16., 16.)),
            title: Some(rect(746., 544., 265., 52.)),
            toolbar: true,
        };
        assert_eq!(height(&finder), 0.0);
        assert_eq!(height(&TitleBar { toolbar: false, ..finder }), 0.0, "a 52pt title is not plain");
    }

    #[test]
    fn a_window_without_a_title_element_is_not_cropped() {
        // Chrome, VS Code, Spotify: buttons drawn over their own tabs or chrome.
        assert_eq!(height(&TitleBar { title: None, ..terminal() }), 0.0);
        assert_eq!(height(&TitleBar { close: None, ..terminal() }), 0.0);
    }

    #[test]
    fn an_off_centre_title_is_not_cropped() {
        let bar = TitleBar { title: Some(rect(3134., 380., 199., 16.)), ..terminal() };
        assert_eq!(height(&bar), 0.0);
    }

    #[test]
    fn a_pane_whose_raised_window_would_cover_the_top_bar_is_not_cropped() {
        assert_eq!(crop(32., 100., 68.), 32.0);
        assert_eq!(crop(32., 99., 68.), 0.0);
        assert_eq!(crop(0., 50., 68.), 0.0);
    }

    #[test]
    fn the_inset_lets_the_bar_cover_the_header_exactly() {
        assert_eq!(inset(32., 29.), 3.0);
        assert_eq!(inset(28., 29.), 2.0);
        assert_eq!(inset(0., 29.), 2.0);
    }
}
