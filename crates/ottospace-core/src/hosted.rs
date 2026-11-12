//! Hosted windows that refuse their pane's size: what the pane shows, and where a click goes.
//! The preview and the live window both sit at the pane's top left, unscaled. A larger window
//! is cut off; a smaller one leaves a gap of ottospace's own surface beside it.

/// Where to replay a click that makes a parked pane live: the same point in the window, or
/// `None` in the gap beside a smaller window. A replay there would land on ottospace, which
/// takes the keyboard back. `click` is from the pane's top left; `shown` is the preview's size.
pub fn replay(click: (f64, f64), shown: (f64, f64)) -> Option<(f64, f64)> {
    let inside = |at: f64, size: f64| (0.0..size).contains(&at);
    (inside(click.0, shown.0) && inside(click.1, shown.1)).then_some(click)
}

/// What a click on an app pane's body does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Click {
    /// Make the pane live and replay the click at this point of the window.
    Replay((f64, f64)),
    /// Make the pane live. The click is spent.
    Enter,
    /// Park the pane: ottospace takes the keyboard back.
    Leave,
    Nothing,
}

/// A click that reaches ottospace on an app pane's body. `live`: the pane was live when the
/// click was made. `click` and `shown` are as for `replay`.
///
/// Parked: a click on the preview enters and is replayed; in the gap it only enters. Live: the
/// real window takes its own clicks, so one that reaches ottospace in the gap is a click on
/// ottospace and leaves, like any click on its chrome. That holds for the second click of a
/// double-click in the gap too. One over the preview came before the window was placed: dropped.
pub fn click(live: bool, click: (f64, f64), shown: (f64, f64)) -> Click {
    match (live, replay(click, shown)) {
        (false, Some(at)) => Click::Replay(at),
        (false, None) => Click::Enter,
        (true, None) => Click::Leave,
        (true, Some(_)) => Click::Nothing,
    }
}

/// Whether a click finds its pane live. `live`: it is live now. `left_on_activation`: it was
/// until ottospace became the active app, and has been parked for that. `first_mouse`: this
/// click is the one that made ottospace active.
///
/// The click that activates ottospace and the activation arrive in either order. Without this,
/// a gap click that arrives second would find the pane parked and enter it again.
pub fn was_live(live: bool, left_on_activation: bool, first_mouse: bool) -> bool {
    live || (left_on_activation && first_mouse)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CALCULATOR: (f64, f64) = (232., 320.);
    const ON: (f64, f64) = (100., 60.);
    const GAP: (f64, f64) = (100., 500.);

    #[test]
    fn a_click_on_a_parked_pane_enters_and_is_replayed_only_on_the_preview() {
        assert_eq!(click(false, ON, CALCULATOR), Click::Replay(ON));
        assert_eq!(click(false, GAP, CALCULATOR), Click::Enter);
    }

    #[test]
    fn a_click_in_the_gap_of_a_live_pane_leaves() {
        assert_eq!(click(true, GAP, CALCULATOR), Click::Leave);
        // Over the preview the real window should have taken it: no replay, which would loop.
        assert_eq!(click(true, ON, CALCULATOR), Click::Nothing);
    }

    #[test]
    fn a_double_click_in_the_gap_enters_then_leaves() {
        // The second click finds the pane live, however soon it comes.
        assert_eq!(click(false, GAP, CALCULATOR), Click::Enter);
        assert_eq!(click(true, GAP, CALCULATOR), Click::Leave);
    }

    #[test]
    fn the_click_that_activates_ottospace_leaves_once() {
        // Click first, activation second: the pane is still live.
        assert_eq!(click(was_live(true, false, true), GAP, CALCULATOR), Click::Leave);
        // Activation first: it parked the pane, and the click must not enter it again.
        assert_eq!(click(was_live(false, true, true), GAP, CALCULATOR), Click::Leave);
        // A later click on the parked pane is an ordinary one.
        assert_eq!(click(was_live(false, true, false), GAP, CALCULATOR), Click::Enter);
        // So is a first click from another app when no pane was live.
        assert_eq!(click(was_live(false, false, true), ON, CALCULATOR), Click::Replay(ON));
    }

    #[test]
    fn a_click_on_the_preview_is_replayed_at_the_same_point() {
        assert_eq!(replay((100., 60.), (232., 320.)), Some((100., 60.)));
        assert_eq!(replay((0., 0.), (232., 320.)), Some((0., 0.)));
    }

    #[test]
    fn a_click_beside_a_smaller_window_is_not_replayed() {
        // Calculator, 232×320, in a larger pane: right of it, below it, and on its far edges.
        assert_eq!(replay((400., 60.), (232., 320.)), None);
        assert_eq!(replay((100., 500.), (232., 320.)), None);
        assert_eq!(replay((232., 60.), (232., 320.)), None);
        assert_eq!(replay((100., 320.), (232., 320.)), None);
        assert_eq!(replay((-1., 60.), (232., 320.)), None);
    }
}
