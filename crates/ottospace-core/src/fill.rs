//! Fill the display: ottospace's window over a whole display, the menu bar and the Dock away
//! while ottospace is the app in front. The rules; the shell makes the AppKit calls.
//! Measured: `docs/ai/fill-display.md`.

use crate::ax::Frame;
use crate::restore;

/// Why a client's request to fill the display is refused.
pub const USERS_ALONE: &str = "only the user puts the window over the menu bar and the Dock";

/// True while the window still has the frame the mode gave it, to a point.
pub fn holds(now: Frame, filled: Frame) -> bool {
    [now.x - filled.x, now.y - filled.y, now.w - filled.w, now.h - filled.h].iter().all(|d| d.abs() <= 1.0)
}

/// Where the window goes when the mode ends. Moved since (dragged, zoomed, its display gone): it
/// stays at `now`. Else back to `before`, or centred on `main` if the user could not reach it there.
pub fn back(now: Frame, filled: Frame, before: Frame, screens: &[Frame], main: Frame) -> Frame {
    if holds(now, filled) { restore::destination(before, screens, main) } else { now }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Action, Effect, Model, Origin};

    const DISPLAY: Frame = Frame { x: 0., y: 0., w: 1920., h: 1080. };
    const VISIBLE: Frame = Frame { x: 0., y: 25., w: 1920., h: 975. };

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
