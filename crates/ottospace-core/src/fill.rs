//! Fill the display: ottospace's window over a whole display, the menu bar and the Dock away
//! while ottospace is the app in front. The rules; the shell makes the AppKit calls.
//! Measured: `docs/ai/fill-display.md`.

use crate::ax::Frame;
use crate::restore;

/// Why a client's request to fill the display is refused.
pub const USERS_ALONE: &str = "only the user puts the window over the menu bar and the Dock";

/// The frame the mode gives the window: the whole display, less the strip its camera housing
/// cuts into (`safe_top`; 0 with none). The menu bar is as tall as that strip, and stays in it.
pub fn frame(display: Frame, safe_top: f64) -> Frame {
    Frame::new(display.x, display.y + safe_top, display.w, display.h - safe_top)
}

/// The part of pane content `rect` an app's window is sized to, its title bar `crop` above it.
/// `keep`: while the mode is on, what the bars leave when they show. Live, its app is in front and
/// the Dock back over the pane's foot: none of it below the Dock's top, where macOS lets no window
/// be resized. Parked, ottospace is in front and the Dock away: as much as a window no taller
/// than `keep` holds (macOS's limit, off screen too), or the picture leaves the Dock's strip empty.
pub fn app_rect(rect: Frame, keep: Option<Frame>, live: bool, crop: f64) -> Frame {
    let Some(visible) = keep else { return rect };
    if live {
        return within(rect, visible);
    }
    let h = rect.h.min(visible.h - crop);
    if h < 1.0 { rect } else { Frame { h, ..rect } }
}

/// None of `rect` below `visible`. A pane wholly in the Dock's strip keeps its rect.
fn within(rect: Frame, visible: Frame) -> Frame {
    let h = (visible.y + visible.h - rect.y).min(rect.h);
    if h < 1.0 { rect } else { Frame { h, ..rect } }
}

/// True while the window still has the frame the mode gave it, to a point.
pub fn holds(now: Frame, filled: Frame) -> bool {
    [now.x - filled.x, now.y - filled.y, now.w - filled.w, now.h - filled.h].iter().all(|d| d.abs() <= 1.0)
}

/// Where the window goes when the mode ends. Moved since (dragged, zoomed, its display gone): it
/// stays at `now`. Else back to `before`. Either way centred on `main`, no larger than it, if
/// the user could not reach it there.
pub fn back(now: Frame, filled: Frame, before: Frame, screens: &[Frame], main: Frame) -> Frame {
    restore::destination(if holds(now, filled) { before } else { now }, screens, main)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Action, Effect, Model, Origin};

    const DISPLAY: Frame = Frame { x: 0., y: 0., w: 1920., h: 1080. };
    const VISIBLE: Frame = Frame { x: 0., y: 25., w: 1920., h: 975. };

    #[test]
    fn the_window_takes_the_whole_display_but_not_the_strip_beside_a_camera_housing() {
        assert_eq!(frame(DISPLAY, 0.), DISPLAY);
        assert_eq!(frame(Frame::new(0., 0., 1512., 982.), 38.), Frame::new(0., 38., 1512., 944.));
    }

    #[test]
    fn an_apps_window_stops_at_the_docks_top() {
        // Measured on the runner: a 1024×768 display, the menu bar 31, the Dock from 708.
        let visible = Frame::new(0., 31., 1024., 677.);
        assert_eq!(within(Frame::new(319., 75., 702., 666.), visible), Frame::new(319., 75., 702., 633.));
        // Clear of the strip: as it is. Wholly in it: as it is, too.
        let upper = Frame::new(319., 75., 702., 300.);
        let strip = Frame::new(319., 720., 702., 20.);
        assert_eq!((within(upper, visible), within(strip, visible)), (upper, strip));
    }

    #[test]
    fn a_parked_apps_picture_goes_below_the_docks_top_and_only_a_live_window_stops_there() {
        // Measured on the runner: asked for 702×700 parked, the canary's window came out 679 tall.
        let visible = Frame::new(0., 31., 1024., 677.);
        let pane = Frame::new(319., 75., 702., 666.);
        // ottospace in front, the Dock away: as tall as a window may be, its 34 pt title bar too.
        assert_eq!(app_rect(pane, Some(visible), false, 34.), Frame::new(319., 75., 702., 643.));
        let short = Frame::new(319., 75., 702., 400.);
        assert_eq!(app_rect(short, Some(visible), false, 34.), short);
        // Live: no lower than the Dock's top.
        assert_eq!(app_rect(pane, Some(visible), true, 34.), Frame::new(319., 75., 702., 633.));
        // The mode off: the pane as it is, live or not.
        assert_eq!((app_rect(pane, None, true, 34.), app_rect(pane, None, false, 34.)), (pane, pane));
    }

    #[test]
    fn the_window_goes_back_where_it_was_unless_it_moved_or_its_place_is_gone() {
        let before = Frame::new(200., 100., 1400., 900.);
        assert_eq!(back(DISPLAY, DISPLAY, before, &[VISIBLE], VISIBLE), before);
        // Dragged or zoomed since: it stays there.
        let dragged = Frame::new(40., 60., 1920., 1080.);
        assert!(!holds(dragged, DISPLAY) && holds(Frame::new(0., 0.5, 1920., 1080.), DISPLAY));
        assert_eq!(back(dragged, DISPLAY, before, &[VISIBLE], VISIBLE), dragged);
        // Its old place was on a display since unplugged: the middle of the main screen.
        let gone = Frame::new(2500., 100., 1400., 900.);
        assert_eq!(back(DISPLAY, DISPLAY, gone, &[VISIBLE], VISIBLE), Frame::new(260., 62.5, 1400., 900.));
        // Moved by the system off its unplugged display, still as large: no larger than the screen left.
        assert_eq!(back(Frame::new(0., -400., 2560., 1440.), gone, before, &[VISIBLE], VISIBLE), VISIBLE);
    }

    #[test]
    fn only_the_user_fills_the_display_and_anyone_may_end_it() {
        let mut m = Model::default();
        assert_eq!(m.refusal(&Action::FillDisplay, Origin::Socket), Some(USERS_ALONE));
        assert_eq!(m.refusal(&Action::FillDisplay, Origin::System), Some(USERS_ALONE));
        assert_eq!(m.update(Action::FillDisplay.sent_by(Origin::Socket)), []);
        assert!(!m.fill_display);
        assert_eq!(m.update(Action::FillDisplay), [Effect::FillDisplay { on: true }]);
        assert_eq!(m.update(Action::FillDisplay), [Effect::FillDisplay { on: false }]);
        // Over already: nothing to tell the shell.
        assert_eq!(m.update(Action::FillEnded.sent_by(Origin::System)), []);
        m.update(Action::FillDisplay);
        assert_eq!(m.update(Action::FillEnded.sent_by(Origin::System)), [Effect::FillDisplay { on: false }]);
        assert!(!m.fill_display);
    }
}
