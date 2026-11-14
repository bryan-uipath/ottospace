//! The user's wheel or trackpad over a parked app pane: the app's real window comes up over the
//! pane for the scroll (a peek), and macOS scrolls it as it scrolls any window under the pointer.
//! The shell activates the app to show it above ottospace. It goes back by itself, and ottospace
//! takes the keyboard back, when the pointer leaves the pane or no scroll came for `QUIET_MS`; a
//! click or a key in the app makes it a normal entry. Only the user's own wheel starts
//! one. See `docs/ai/sneak-apps.md`.

use crate::layout::Node;
use crate::model::PaneId;

/// Why a scroll that is not the user's own brings up no window.
pub const USERS_ALONE: &str =
    "the wheel over an app pane is the user's alone: no client brings up another app's window";

/// How long a peek stays up after the last scroll, in milliseconds.
pub const QUIET_MS: u64 = 1500;

/// A peek: the pane, when its last scroll came, and the view it was started over.
#[derive(Clone, Debug, PartialEq)]
pub struct Peek {
    pub pane: PaneId,
    pub last: u64,
    tab: usize,
    layout: Option<Node>,
}

impl Peek {
    pub(crate) fn new(pane: PaneId, now: u64, tab: usize, layout: Option<Node>) -> Self {
        Self { pane, last: now, tab, layout }
    }

    /// True while the tab and its tree are as they were: the window still covers its pane.
    pub(crate) fn over(&self, tab: usize, layout: Option<&Node>) -> bool {
        self.tab == tab && self.layout.as_ref() == layout
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ax::{Frame, Source, Target};
    use crate::layout::{Axis, Dir};
    use crate::model::{Action, Effect, Kind, Mode, Model, Origin};

    /// A model whose pane 1 is a parked app pane, beside terminal pane 2.
    fn hosted() -> Model {
        let mut m = Model::default();
        let app = Kind::App { window: 7, title: "Canary 1".into(), bundle: None, name: "canary".into() };
        m.update(Action::Split { axis: Axis::Right, kind: app });
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m
    }

    fn wheel(now: u64) -> Action {
        Action::AppWheel { pane: 1, now }
    }

    fn check(now: u64, over: bool, pressed: bool, scrolled: Option<u64>) -> Action {
        Action::PeekCheck { now, over, pressed, scrolled }
    }

    #[test]
    fn only_the_users_wheel_starts_a_peek() {
        let mut m = hosted();
        for origin in [Origin::Socket, Origin::System] {
            assert_eq!(m.refusal(&wheel(0), origin), Some(USERS_ALONE));
            assert_eq!(m.update(wheel(0).sent_by(origin)), []);
        }
        assert_eq!(m.peek, None);
        assert_eq!(m.update(wheel(0)), [Effect::Peek(1)]);
        assert_eq!((m.peek.as_ref().map(|p| p.pane), m.mode(), m.live), (Some(1), Mode::Normal, None));
        // Another scroll that came before the window was up: the same peek.
        assert_eq!(m.update(wheel(100)), []);
        // Not a terminal, not a pane that is not there.
        let mut m = hosted();
        assert_eq!(m.update(Action::AppWheel { pane: 2, now: 0 }), []);
        assert_eq!(m.update(Action::AppWheel { pane: 9, now: 0 }), []);
        assert_eq!(m.peek, None);
    }

    #[test]
    fn a_peek_ends_when_the_pointer_leaves_or_the_wheel_is_quiet() {
        let mut m = hosted();
        m.update(wheel(1000));
        // Scrolls keep it up while the pointer is over the pane.
        assert_eq!(m.update(check(2000, true, false, Some(1900))), []);
        assert_eq!(m.update(check(3399, true, false, None)), []);
        assert_eq!(m.update(check(3400, true, false, None)), [Effect::Parked(1)]);
        assert_eq!(m.peek, None);
        // A scroll that was not over the pane keeps nothing up; leaving ends it at once.
        m.update(wheel(5000));
        assert_eq!(m.update(check(5100, false, false, Some(5100))), [Effect::Parked(1)]);
        // A check with no peek does nothing.
        assert_eq!(m.update(check(9000, false, false, None)), []);
    }

    #[test]
    fn a_click_or_key_in_the_peeked_app_makes_it_an_entry() {
        let mut m = hosted();
        m.update(wheel(0));
        // The window is already up: no park in between.
        assert_eq!(m.update(check(100, true, true, None)), [Effect::Live(1)]);
        assert_eq!((m.peek.clone(), m.live, m.mode(), m.focused), (None, Some(1), Mode::App, Some(1)));
        // A live pane is not peeked; leaving parks it once.
        assert_eq!(m.update(wheel(200)), []);
        assert_eq!(m.update(Action::Leave), [Effect::Parked(1)]);
    }

    #[test]
    fn what_changes_the_view_ends_a_peek() {
        // Focus moves, the tree changes, a mode takes the panes, the pane closes.
        let ends = [Action::Focus { pane: 2 }, Action::Equalize, Action::PaletteOpen, Action::Close { pane: Some(1) }];
        for end in ends {
            let mut m = hosted();
            m.update(Action::Resize { dir: Dir::Right });
            m.update(wheel(0));
            let effects = m.update(end);
            assert_eq!(m.peek, None, "{effects:?}");
            let parked = effects.contains(&Effect::Parked(1));
            let closed = effects.iter().any(|e| matches!(e, Effect::Closed(p) if p.id == 1));
            assert!(parked || closed, "{effects:?}");
        }
    }

    #[test]
    fn no_peek_under_sneak_mode_or_the_palette() {
        let mut m = hosted();
        let pane = Target { frame: Frame { x: 0., y: 0., w: 9., h: 9. }, source: Source::Own { id: "pane:1".into() } };
        m.update(Action::Sneak { targets: vec![pane] });
        // As the wheel over a terminal, a scroll ends no mode: the labels stay up.
        assert_eq!((m.update(wheel(0)), m.mode()), (vec![], Mode::Sneak));
        m.update(Action::LeaveSneak);
        m.update(Action::PaletteOpen);
        assert_eq!(m.update(wheel(0)), []);
    }
}
