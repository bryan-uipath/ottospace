//! Hosted windows that refuse their pane's size: what the pane shows, and where a click goes.
//! The preview and the live window both sit at the pane's top left, unscaled. A larger window
//! is cut off; a smaller one leaves a gap of ottospace's own surface beside it.
//! The pane header says so (`Fit::hint`), and the user may fit the pane to the window.

use serde::Serialize;

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

/// A window may differ from its pane by this much, in points, and still count as fitting.
pub const TOLERANCE: f32 = 2.0;

/// A window may be smaller than its pane by less than this, in points, and still count as
/// fitting: an app that snaps its size, e.g. a terminal emulator to its grid of cells.
pub const SNAP: f32 = 24.0;

/// Why a fit would move nothing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stuck {
    /// No split lies above the pane on an axis that does not fit: it spans the tab there.
    NoSplit,
    /// Every such split is at its minimum share already.
    AtLimit,
}

impl Stuck {
    pub fn why(self) -> &'static str {
        match self {
            Stuck::NoSplit => "no split to move",
            Stuck::AtLimit => "splits at their limit",
        }
    }
}

/// A window that kept another size when asked for its pane's. Sizes are `[width, height]` in points.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Fit {
    /// The size the window kept, without the title bar the pane crops.
    pub wants: [f32; 2],
    /// What the pane header says, e.g. `needs 120 pt more width` or `fixed size 232×320`.
    pub hint: String,
    /// Set when a fit would move nothing, and why. The model keeps it in step with the layout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stuck: Option<Stuck>,
    /// The pane's content size, and the pane's whole size (header, border and inset too).
    #[serde(skip)]
    content: [f32; 2],
    #[serde(skip)]
    outer: [f32; 2],
}

/// The misfit of a `window` in a pane, or `None` when it has the pane's `content` size.
pub fn fit(content: [f32; 2], outer: [f32; 2], window: [f32; 2]) -> Option<Fit> {
    let hint = hint(content, window)?;
    Some(Fit { wants: window, hint, stuck: None, content, outer })
}

/// How far the window is from the pane on one axis, in whole points: above 0 it is cut off,
/// below it leaves a gap. 0: it fits, within `TOLERANCE` above and `SNAP` below.
fn misfit(content: f32, window: f32) -> f32 {
    let by = (window - content).round();
    if by > TOLERANCE || by <= -SNAP { by } else { 0.0 }
}

/// A window smaller on either axis did not grow: it is of a fixed size. Else it is cut off, and
/// the hint says by how much.
fn hint(content: [f32; 2], window: [f32; 2]) -> Option<String> {
    let [w, h] = [misfit(content[0], window[0]), misfit(content[1], window[1])];
    let (wider, taller) = (w > 0.0, h > 0.0);
    if w < 0.0 || h < 0.0 {
        return Some(format!("fixed size {}×{}", window[0].round(), window[1].round()));
    }
    match (wider, taller) {
        (true, true) => Some(format!("needs {w}×{h} pt more")),
        (true, false) => Some(format!("needs {w} pt more width")),
        (false, true) => Some(format!("needs {h} pt more height")),
        (false, false) => None,
    }
}

impl Fit {
    /// How much the pane must grow (above 1) or shrink on `axis` (0: width, 1: height) for its
    /// content to be the window's size, e.g. 1.25. `None`: that axis fits.
    pub fn scale(&self, axis: usize) -> Option<f32> {
        let fits = misfit(self.content[axis], self.wants[axis]) == 0.0;
        let more = self.wants[axis] - self.content[axis];
        (!fits && self.outer[axis] > 0.0).then(|| (self.outer[axis] + more) / self.outer[axis])
    }

    /// The header's words: the hint, and why a fit would move nothing if it would not,
    /// e.g. `fixed size 160×120 · no split to move`.
    pub fn label(&self) -> String {
        match self.stuck {
            Some(stuck) => format!("{} · {}", self.hint, stuck.why()),
            None => self.hint.clone(),
        }
    }

    /// Whether this is the same measure: the shell reports only one that changed.
    pub fn measured(&self, content: [f32; 2], outer: [f32; 2], window: [f32; 2]) -> bool {
        (self.content, self.outer, self.wants) == (content, outer, window)
    }
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
    fn a_window_cut_off_says_how_much_more_it_needs() {
        let hint = |window| fit([400., 600.], [406., 638.], window).map(|f| f.hint);
        assert_eq!(hint([520., 600.]).as_deref(), Some("needs 120 pt more width"));
        assert_eq!(hint([400., 640.]).as_deref(), Some("needs 40 pt more height"));
        assert_eq!(hint([520., 640.]).as_deref(), Some("needs 120×40 pt more"));
    }

    #[test]
    fn a_window_smaller_than_its_pane_is_of_a_fixed_size() {
        let hint = |window| fit([400., 600.], [406., 638.], window).map(|f| f.hint);
        assert_eq!(hint([232., 320.]).as_deref(), Some("fixed size 232×320"));
        // Smaller on one axis is enough: a window that could resize would have grown there.
        assert_eq!(hint([520., 320.]).as_deref(), Some("fixed size 520×320"));
        assert_eq!(hint([232., 600.]).as_deref(), Some("fixed size 232×600"));
    }

    #[test]
    fn a_window_within_two_points_of_its_pane_fits() {
        assert_eq!(fit([400., 600.], [406., 638.], [400., 600.]), None);
        assert_eq!(fit([400., 600.], [406., 638.], [402., 598.]), None);
        assert!(fit([400., 600.], [406., 638.], [403., 600.]).is_some());
    }

    #[test]
    fn a_window_a_little_smaller_than_its_pane_fits() {
        // A terminal emulator keeps whole cells: 13 pt short in width, 20 in height.
        assert_eq!(fit([400., 600.], [406., 638.], [387., 580.]), None);
        // 24 pt short is a fixed size.
        let hint = |window| fit([400., 600.], [406., 638.], window).map(|f| f.hint);
        assert_eq!(hint([376., 600.]).as_deref(), Some("fixed size 376×600"));
        // A little short on one axis, cut off on the other: only the cut is named and fitted.
        let cut_off = fit([400., 600.], [406., 638.], [390., 700.]).unwrap();
        assert_eq!(cut_off.hint, "needs 100 pt more height");
        assert_eq!((cut_off.scale(0), cut_off.scale(1)), (None, Some(738. / 638.)));
    }

    #[test]
    fn the_label_says_why_a_fit_would_move_nothing() {
        let mut small = fit([400., 600.], [406., 638.], [160., 120.]).unwrap();
        assert_eq!(small.label(), "fixed size 160×120");
        small.stuck = Some(Stuck::NoSplit);
        assert_eq!(small.label(), "fixed size 160×120 · no split to move");
        small.stuck = Some(Stuck::AtLimit);
        assert_eq!(small.label(), "fixed size 160×120 · splits at their limit");
    }

    #[test]
    fn the_pane_scales_by_what_its_content_lacks_chrome_included() {
        // 120 pt more content in a pane 406 pt wide: 526 / 406. The height fits.
        let cut_off = fit([400., 600.], [406., 638.], [520., 600.]).unwrap();
        assert_eq!(cut_off.scale(0), Some(526. / 406.));
        assert_eq!(cut_off.scale(1), None);
        let small = fit([400., 600.], [406., 638.], [232., 320.]).unwrap();
        assert_eq!(small.scale(0), Some(238. / 406.));
        assert_eq!(small.scale(1), Some(358. / 638.));
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
