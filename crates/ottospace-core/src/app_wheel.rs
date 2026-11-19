//! The user's wheel or trackpad over a parked app pane enters it, as a click on the pane does:
//! the app's window comes up over the pane with the keyboard (APP mode). The shell passes the
//! scrolls that reached ottospace on to the window once it is up. Only the user's own wheel
//! enters. See `docs/ai/sneak-apps.md`.

use crate::model::{Action, Envelope, Kind, Mode, Model, Origin, PaneId};

/// Why a scroll that is not the user's own enters no app.
pub const USERS_ALONE: &str =
    "the wheel over an app pane is the user's alone: no client brings up another app's window";

impl Model {
    /// The user's `AppWheel` that enters its pane is that pane's `Enter`: everything a click on
    /// it does follows. Any other is left as it is, and does nothing.
    pub(crate) fn app_wheel(&self, sent: Envelope) -> Envelope {
        let Action::AppWheel { pane, dragging, active } = sent.action else { return sent };
        if sent.origin == Origin::User && !dragging && active && self.wheel_enters(pane) {
            return Action::Enter { pane }.sent_by(sent.origin);
        }
        sent
    }

    /// A wheel may enter `pane`: a parked app pane on show, with nothing over the panes that
    /// holds the user's attention (sneak mode, the palette, a box, the quit question).
    fn wheel_enters(&self, pane: PaneId) -> bool {
        let app = self.pane(pane).is_some_and(|p| matches!(p.kind, Kind::App { .. }));
        let shown = self.layout.as_ref().is_some_and(|l| l.leaves().contains(&pane));
        let mode = matches!(self.mode(), Mode::Normal | Mode::List | Mode::Talk);
        app && shown && self.live != Some(pane) && mode && !self.keys && !self.ax_overlay
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ax::{Frame, Source, Target};
    use crate::layout::Axis;
    use crate::model::Effect;

    /// A model whose pane 1 is a parked app pane, beside terminal pane 2 (focused).
    fn hosted() -> Model {
        let mut m = Model::default();
        let app = Kind::App { window: 7, title: "Canary 1".into(), bundle: None, name: "canary".into() };
        m.update(Action::Split { axis: Axis::Right, kind: app });
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m
    }

    fn wheel(pane: PaneId) -> Action {
        Action::AppWheel { pane, dragging: false, active: true }
    }

    #[test]
    fn the_users_wheel_enters_as_a_click_does() {
        let mut m = hosted();
        assert_eq!(m.focused, Some(2));
        assert_eq!(m.update(wheel(1)), [Effect::Live(1)]);
        assert_eq!((m.live, m.focused, m.mode()), (Some(1), Some(1), Mode::App));
        // The same as the click's `Enter`.
        let mut clicked = hosted();
        assert_eq!(clicked.update(Action::Enter { pane: 1 }), [Effect::Live(1)]);
        assert_eq!((clicked.live, clicked.focused, clicked.mode()), (m.live, m.focused, m.mode()));
        // More scrolls over the live pane: nothing more.
        assert_eq!(m.update(wheel(1)), []);
        // Leaving is as ever.
        assert_eq!(m.update(Action::Leave), [Effect::Parked(1)]);
        assert_eq!(m.mode(), Mode::Normal);
    }

    #[test]
    fn only_the_users_wheel_enters() {
        // A client's or the system's wheel leaves the terminal the user types in with the keyboard.
        let mut m = hosted();
        for origin in [Origin::Socket, Origin::System] {
            assert_eq!(m.refusal(&wheel(1), origin), Some(USERS_ALONE));
            assert_eq!(m.update(wheel(1).sent_by(origin)), []);
            assert_eq!((m.live, m.focused), (None, Some(2)));
        }
    }

    #[test]
    fn a_wheel_enters_only_a_parked_app_pane_on_show() {
        let mut m = hosted();
        // Not a terminal, not a pane that is not there.
        assert_eq!(m.update(wheel(2)), []);
        assert_eq!(m.update(wheel(9)), []);
        // Not during a drag, not from behind another app.
        assert_eq!(m.update(Action::AppWheel { pane: 1, dragging: true, active: true }), []);
        assert_eq!(m.update(Action::AppWheel { pane: 1, dragging: false, active: false }), []);
        assert_eq!((m.live, m.focused), (None, Some(2)));
        // From list mode it gives up the list's keys, as a click does.
        m.update(Action::FocusList);
        assert_eq!(m.update(wheel(1)), [Effect::Live(1)]);
        assert_eq!((m.list, m.mode()), (None, Mode::App));
    }

    #[test]
    fn no_entry_under_sneak_mode_or_the_palette() {
        let mut m = hosted();
        let pane = Target {
            frame: Frame { x: 0., y: 0., w: 9., h: 9. },
            source: Source::Own { id: "pane:1".into() },
            clip: None,
        };
        m.update(Action::Sneak { targets: vec![pane] });
        // As the wheel over a terminal, a scroll ends no mode: the labels stay up.
        assert_eq!((m.update(wheel(1)), m.mode()), (vec![], Mode::Sneak));
        m.update(Action::LeaveSneak);
        m.update(Action::PaletteOpen);
        assert_eq!(m.update(wheel(1)), []);
        assert_eq!(m.live, None);
    }
}
