//! The user's wheel or trackpad over a parked app pane scrolls that app's window, which stays
//! where it is: not raised, not activated, the pointer not moved. Only the user's own scroll is
//! forwarded; nothing else (no click, no key, no drag). See `docs/ai/sneak-apps.md`.

/// Why a scroll that is not the user's own reaches no app.
pub const USERS_ALONE: &str = "the wheel over an app pane is the user's alone: no client scrolls another app";

/// The most points one forwarded scroll moves, each way: a runaway delta (a stuck device, a
/// frame's worth added up) scrolls a page at most.
pub const MAX_POINTS: f64 = 600.0;

/// `delta` within `MAX_POINTS`; not a number: none.
pub fn cap(delta: f64) -> f64 {
    if delta.is_finite() { delta.clamp(-MAX_POINTS, MAX_POINTS) } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ax::{Frame, Source, Target};
    use crate::layout::Axis;
    use crate::model::{Action, Effect, Kind, Mode, Model, Origin};

    #[test]
    fn a_delta_is_capped_and_not_a_number_is_none() {
        assert_eq!((cap(12.5), cap(-9000.), cap(9000.)), (12.5, -MAX_POINTS, MAX_POINTS));
        assert_eq!((cap(f64::NAN), cap(f64::INFINITY)), (0.0, 0.0));
    }

    /// A model whose pane 1 is a parked app pane.
    fn hosted() -> Model {
        let mut m = Model::default();
        let app = Kind::App { window: 7, title: "Canary 1".into(), bundle: None, name: "canary".into() };
        m.update(Action::Split { axis: Axis::Right, kind: app });
        m
    }

    #[test]
    fn only_the_users_scroll_over_a_parked_app_pane_is_forwarded() {
        let mut m = hosted();
        let wheel = Action::AppWheel { pane: 1, dx: 0., dy: -40. };
        assert_eq!(m.update(wheel.clone()), [Effect::AppWheel { pane: 1, dx: 0., dy: -40. }]);
        // A client's or the system's: refused, with the reason, and nothing happens.
        for origin in [Origin::Socket, Origin::System] {
            assert_eq!(m.refusal(&wheel, origin), Some(USERS_ALONE));
            assert_eq!(m.update(wheel.clone().sent_by(origin)), []);
        }
        // Capped. A scroll of no distance goes too: it ends a trackpad gesture.
        let big = Action::AppWheel { pane: 1, dx: 1e9, dy: f64::NAN };
        assert_eq!(m.update(big), [Effect::AppWheel { pane: 1, dx: MAX_POINTS, dy: 0. }]);
        let end = Action::AppWheel { pane: 1, dx: 0., dy: 0. };
        assert_eq!(m.update(end), [Effect::AppWheel { pane: 1, dx: 0., dy: 0. }]);
        // Not a pane that is live (the real window takes its own scrolls), and not another kind.
        m.update(Action::Enter { pane: 1 });
        assert_eq!(m.update(Action::AppWheel { pane: 1, dx: 0., dy: 5. }), []);
        m.update(Action::Leave);
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        assert_eq!(m.update(Action::AppWheel { pane: 2, dx: 0., dy: 5. }), []);
        assert_eq!(m.update(Action::AppWheel { pane: 9, dx: 0., dy: 5. }), []);
    }

    #[test]
    fn a_scroll_ends_no_mode() {
        // As the wheel over a terminal: sneak mode's labels stay up, and the scroll goes through.
        let mut m = hosted();
        let pane = Target { frame: Frame { x: 0., y: 0., w: 9., h: 9. }, source: Source::Own { id: "pane:1".into() } };
        m.update(Action::Sneak { targets: vec![pane] });
        assert_eq!(m.mode(), Mode::Sneak);
        assert_eq!(m.update(Action::AppWheel { pane: 1, dx: 0., dy: 5. }).len(), 1);
        assert_eq!(m.mode(), Mode::Sneak);
    }
}
