//! Where sneak mode draws each label. On a small control (an icon, a ×, a row of text) the label
//! sits beside it, touching its edge, so it never hides what it names; on a large one (a pane) it
//! sits inside, as it always did. The shell draws the pills at these rects.

use crate::ax::{Frame, Labelled, Source};

/// A pill's height in points. Its width is padding and a letter's advance each: the shell draws it
/// this size, so what the state says is what shows.
pub const PILL_H: f64 = 15.;
const PILL_PAD: f64 = 6.;
/// The widest letter, `W`, bold at 12 points, with room.
const PILL_LETTER: f64 = 12.;
/// A target is large when it is this many pills wide and this many high: a label inside it leaves
/// most of it seen. Anything smaller (a 28-point row, a tab, a rail icon) is small.
const ROOM: f64 = 3.;
/// Rounding the shell's points may add.
const EPS: f64 = 0.5;
/// How far a pill may reach over a neighbouring control's edge: its padding, not its glyph.
/// e.g. a × 2 points into the spot below a button.
const GRAZE: f64 = 2.5;
/// In the last resort, covering a pill costs this much more than covering a control.
const PILL_COST: f64 = 1000.;

/// The pill of `label`: width, height.
pub fn pill_size(label: &str) -> (f64, f64) {
    (PILL_PAD + PILL_LETTER * label.chars().count() as f64, PILL_H)
}

/// Whether a label inside `target` would hide much of it: under `ROOM` pills wide or high.
pub fn is_small(target: &Frame, label: &str) -> bool {
    let (w, h) = pill_size(label);
    !(target.w >= ROOM * w && target.h >= ROOM * h)
}

/// A large target's spot: a pane's body in its middle, anything else at its left, centred down.
fn inside(t: &Frame, (w, h): (f64, f64), body: bool) -> Frame {
    let y = t.y + (t.h - h) / 2.;
    match body {
        true => Frame::new(t.x + (t.w - w) / 2., y, w, h),
        false => Frame::new(t.x + 2., y, w, h),
    }
}

/// The spots touching `t` from outside, in the order tried: above, below, left, right. Above and
/// below first: a row of icons (the top row, a tab bar) keeps its labels in a line over or under
/// it, each centred on its own icon. In a column (`column`: the rail, a list) right, left, above,
/// below: its labels line up at its side. No diagonal corner: it touches its target at one point,
/// and sits as near a neighbour.
fn beside(t: &Frame, (w, h): (f64, f64), column: bool) -> [Frame; 4] {
    // Centred on a narrow target; at a wide one's left end, where its text starts.
    let x = if t.w < 2. * w { t.x + (t.w - w) / 2. } else { t.x };
    let y = t.y + (t.h - h) / 2.;
    let (above, below) = (Frame::new(x, t.y - h, w, h), Frame::new(x, t.y + t.h, w, h));
    let (left, right) = (Frame::new(t.x - w, y, w, h), Frame::new(t.x + t.w, y, w, h));
    match column {
        true => [right, left, above, below],
        false => [above, below, left, right],
    }
}

/// Whether `t` is one of a column: `other` lines up with it (same left and width, within 2 points)
/// less than a pill's height above or below it.
fn column(t: &Frame, other: &Frame) -> bool {
    let near = (other.y - (t.y + t.h)).max(t.y - (other.y + other.h)) < PILL_H;
    near && (other.x - t.x).abs() <= 2. && (other.w - t.w).abs() <= 2.
}

/// The last resort: inside `t`, at a corner. Top left first.
fn corners(t: &Frame, (w, h): (f64, f64)) -> [Frame; 4] {
    let (r, b) = (t.x + t.w - w, t.y + t.h - h);
    [Frame::new(t.x, t.y, w, h), Frame::new(r, t.y, w, h), Frame::new(t.x, b, w, h), Frame::new(r, b, w, h)]
}

fn within(r: &Frame, b: &Frame) -> bool {
    r.x >= b.x - EPS && r.y >= b.y - EPS && r.x + r.w <= b.x + b.w + EPS && r.y + r.h <= b.y + b.h + EPS
}

/// `r` moved the least to lie in `b`; at `b`'s top left if it is larger.
fn clamp(r: Frame, b: Option<Frame>) -> Frame {
    let Some(b) = b else { return r };
    let x = r.x.min(b.x + b.w - r.w).max(b.x);
    let y = r.y.min(b.y + b.h - r.h).max(b.y);
    Frame { x, y, ..r }
}

fn overlap(a: &Frame, b: &Frame) -> f64 {
    if !a.intersects(b) {
        return 0.;
    }
    let c = a.clip(b);
    c.w * c.h
}

fn holds(a: &Frame, b: &Frame) -> bool {
    a.x <= b.x && a.y <= b.y && a.x + a.w >= b.x + b.w && a.y + a.h >= b.y + b.h
}

/// Sets each label's `pill`: ottospace's own first, then an app pane's around them, so an app's
/// labels arriving never move ottospace's. In each, large targets first, at their old spot (a
/// corner if a pill is there); then each small one in reading order: the first spot beside it
/// (`beside`) inside its clip that covers no other small target (for ottospace's own, no other of
/// its own), no pill placed before, and no more than a third of one holding it (a header round its
/// ×); if none is free, the corner inside it that covers the least, pills above all. The same
/// labels always get the same rects.
pub fn place(labels: &mut [Labelled]) {
    let size: Vec<(f64, f64)> = labels.iter().map(|l| pill_size(&l.label)).collect();
    let small: Vec<bool> = labels.iter().map(|l| is_small(&l.target.frame, &l.label)).collect();
    let app: Vec<bool> = labels.iter().map(|l| matches!(l.target.source, Source::App { .. })).collect();
    let mut order: Vec<usize> = (0..labels.len()).collect();
    order.sort_by_key(|&i| (app[i], small[i]));
    let mut pills: Vec<Option<Frame>> = vec![None; labels.len()];
    for i in order {
        let (t, clip) = (labels[i].target.frame, labels[i].target.clip);
        let placed: Vec<Frame> = pills.iter().flatten().copied().collect();
        let on_pills = |r: &Frame| placed.iter().map(|b| overlap(r, b)).sum::<f64>();
        if !small[i] {
            let body = matches!(&labels[i].target.source, Source::Own { id } if id.starts_with("pane:"));
            // Today's spot, or if a pill is there (an app's column over its pane's middle), a corner.
            let spots = std::iter::once(inside(&t, size[i], body)).chain(corners(&t, size[i]));
            pills[i] = spots.min_by(|a, b| on_pills(a).total_cmp(&on_pills(b)));
            continue;
        }
        let kin =
            (0..labels.len()).filter(|&j| j != i && small[j] && (app[i] || !app[j])).map(|j| labels[j].target.frame);
        let (holders, kin): (Vec<Frame>, Vec<Frame>) = kin.partition(|o| holds(o, &t));
        let in_column = kin.iter().any(|o| column(&t, o));
        // A control's edge padding may take a sliver (`GRAZE`) of a pill; another pill none.
        let g = GRAZE;
        let grazes = |r: &Frame, b: &Frame| Frame::new(r.x + g, r.y + g, r.w - 2. * g, r.h - 2. * g).intersects(b);
        let cost = |r: &Frame| kin.iter().filter(|b| grazes(r, b)).map(|b| overlap(r, b)).sum::<f64>() + on_pills(r);
        let fits = |r: &Frame| clip.is_none_or(|c| within(r, &c));
        // A spot may reach into what holds the target (a pane's header round its ×) by a third
        // of the pill at most: deeper, it would cover the holder's own text.
        let (w, h) = size[i];
        let shallow = |r: &Frame| holders.iter().all(|o| overlap(r, o) <= w * h / 3. + EPS);
        let free = beside(&t, size[i], in_column).into_iter().find(|r| fits(r) && shallow(r) && cost(r) == 0.);
        let last = |r: &Frame| cost(r) + PILL_COST * on_pills(r);
        let corner =
            || corners(&t, size[i]).map(|r| clamp(r, clip)).into_iter().min_by(|a, b| last(a).total_cmp(&last(b)));
        pills[i] = free.or_else(corner);
    }
    for (l, pill) in labels.iter_mut().zip(pills) {
        l.pill = pill;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ax::Target;

    const WINDOW: Frame = Frame { x: 0., y: 0., w: 1400., h: 900. };

    fn at(id: &str, label: &str, f: [f64; 4]) -> Labelled {
        let source = Source::Own { id: id.into() };
        let target = Target { frame: Frame::new(f[0], f[1], f[2], f[3]), source, clip: Some(WINDOW) };
        Labelled { label: label.into(), target, pill: None }
    }

    fn placed(mut labels: Vec<Labelled>) -> Vec<Frame> {
        place(&mut labels);
        labels.into_iter().map(|l| l.pill.expect("a pill")).collect()
    }

    #[test]
    fn small_means_under_three_pills_either_way() {
        // A one-letter pill is 18 × 15: large from 54 × 45.
        let cases = [
            ([28., 22.], true),    // a top-row button
            ([56., 40.], true),    // a rail icon
            ([241., 28.], true),   // a row of text
            ([540., 27.], true),   // a pane's header
            ([16., 19.], true),    // a ×
            ([54., 45.], false),   // just large
            ([536., 772.], false), // a pane
        ];
        for ([w, h], small) in cases {
            assert_eq!(is_small(&Frame::new(0., 0., w, h), "a"), small, "{w} × {h}");
        }
        // A longer label needs a wider target.
        assert!(is_small(&Frame::new(0., 0., 80., 60.), "ab") && !is_small(&Frame::new(0., 0., 90., 60.), "ab"));
    }

    /// Each case: targets as (label, [x, y, w, h]), and where each label goes.
    #[test]
    fn a_small_targets_label_sits_beside_it() {
        let cases: &[(&str, &[(&str, [f64; 4])], &[[f64; 2]])] = &[
            // Free all round: above, centred on it.
            ("alone", &[("a", [100., 100., 20., 20.])], &[[101., 85.]]),
            // At the window's top: below.
            ("top row", &[("a", [1300., 8.5, 28., 22.])], &[[1305., 30.5]]),
            // Top-row buttons 4 apart: each below its own, in a line.
            (
                "buttons in a row",
                &[("a", [1300., 8.5, 28., 22.]), ("s", [1332., 8.5, 28., 22.]), ("d", [1364., 8.5, 28., 22.])],
                &[[1305., 30.5], [1337., 30.5], [1369., 30.5]],
            ),
            // A column touching the window's left: above and below are its neighbours, left is
            // off the window. Right, centred down.
            (
                "rail",
                &[("a", [0., 40., 56., 40.]), ("s", [0., 80., 56., 40.]), ("d", [0., 120., 56., 40.])],
                &[[56., 52.5], [56., 92.5], [56., 132.5]],
            ),
            // A wide row: above, at its left end.
            ("row", &[("a", [65., 120., 241., 28.])], &[[65., 105.]]),
            // A two-letter pill over a 16-point ×: centred on it.
            ("×", &[("pa", [376., 45., 16., 19.])], &[[369., 30.]]),
            // A header goes above itself; a large target below keeps today's spot: at its left,
            // centred down.
            ("large", &[("r", [317., 71., 540., 27.]), ("i", [319., 101., 536., 772.])], &[[317., 56.], [321., 479.5]]),
        ];
        for (name, targets, want) in cases {
            let labels = targets.iter().enumerate().map(|(i, (l, f))| at(&format!("c{i}"), l, *f)).collect();
            let got: Vec<[f64; 2]> = placed(labels).iter().map(|p| [p.x, p.y]).collect();
            assert_eq!(&got, want, "{name}");
        }
    }

    #[test]
    fn a_tabs_label_and_its_xs_do_not_meet() {
        // A tab holds its ×: the × does not block the tab's spot, and the tab's pill blocks the ×'s.
        let got = placed(vec![at("tab:1", "j", [316., 40., 80., 29.]), at("tab-close:1", "pa", [376., 45., 16., 19.])]);
        assert_eq!([got[0].x, got[0].y], [316., 25.], "above the tab, at its left");
        assert_eq!([got[1].x, got[1].y], [369., 30.], "above the ×");
        assert!(!got[0].intersects(&got[1]));
        // A pane's body keeps its middle.
        let got = placed(vec![at("pane:1", "i", [319., 101., 536., 772.])]);
        assert_eq!([got[0].x, got[0].y], [578., 479.5]);
    }

    #[test]
    fn a_label_stays_in_its_clip_and_falls_back_to_the_freest_corner() {
        // A row in a scroll area that ends at its edges: no outside spot is inside the clip.
        let mut row = at("task:1", "a", [65., 120., 241., 28.]);
        row.target.clip = Some(Frame::new(65., 120., 241., 28.));
        // A button at its right end: the corner fallback keeps off it, at the top left.
        let mut button = at("task-dismiss:1", "s", [262., 122., 38., 20.]);
        button.target.clip = row.target.clip;
        let got = placed(vec![row, button]);
        assert_eq!([got[0].x, got[0].y], [65., 120.]);
        assert!(got.iter().all(|p| within(p, &Frame::new(65., 120., 241., 28.))), "{got:?}");
        // Off the window's right and bottom: clamped in.
        let got = placed(vec![at("c", "a", [1390., 890., 10., 10.])]);
        assert!(within(&got[0], &WINDOW), "{got:?}");
    }

    #[test]
    fn an_app_panes_labels_stay_in_the_pane_and_go_round_ottospaces() {
        // A tab over a pane's header: the header's label goes under it, into the pane.
        let own = || vec![at("tab:1", "a", [320., 40., 80., 29.]), at("header:1", "s", [320., 70., 500., 29.])];
        // A button at the pane's top left, under that label: above would leave the pane, below and
        // left cross its left edge. Right, clear of the header's label.
        let pane = Frame::new(320., 99., 500., 400.);
        let source = Source::App { pane: 1, path: vec![0], role: "AXButton".into(), at: Frame::default() };
        let target = Target { frame: Frame::new(322., 100., 20., 20.), source, clip: Some(pane) };
        let mut labels = own();
        labels.push(Labelled { label: "ta".into(), target, pill: None });
        place(&mut labels);
        let pills: Vec<Frame> = labels.iter().map(|l| l.pill.unwrap_or_default()).collect();
        assert_eq!(([pills[1].x, pills[1].y], [pills[2].x, pills[2].y]), ([320., 99.], [342., 102.5]));
        assert!(within(&pills[2], &pane) && !pills[2].intersects(&pills[1]));
        // ottospace's own labels never move for an app's: the same alone.
        assert_eq!(placed(own()), pills[..2]);
    }

    #[test]
    fn a_large_targets_label_moves_off_a_pill_there() {
        // An app pane's own label in its middle, and an app's column holding that middle: a corner.
        let mut column = at("c", "ta", [290., 0., 310., 400.]);
        column.target.source = Source::App { pane: 1, path: vec![0], role: "AXGroup".into(), at: Frame::default() };
        let got = placed(vec![at("pane:1", "a", [0., 0., 600., 400.]), column]);
        assert_eq!([got[1].x, got[1].y], [290., 0.]);
        assert!(!got[0].intersects(&got[1]));
    }

    #[test]
    fn a_pill_may_graze_a_neighbours_padding() {
        // A button, and a × 2 points into its spot below (once seen in CI): still below.
        let got = placed(vec![at("top", "a", [988., 9., 28., 22.]), at("notice:close", "sd", [992., 44., 20., 20.])]);
        assert_eq!([got[0].x, got[0].y], [993., 31.]);
    }

    #[test]
    fn placing_is_the_same_each_time() {
        let labels: Vec<Labelled> = (0..30)
            .map(|i| at(&format!("c{i}"), "a", [(i % 6) as f64 * 30., (i / 6) as f64 * 25., 28., 22.]))
            .collect();
        assert_eq!(placed(labels.clone()), placed(labels));
    }

    /// Seeded random grids of controls of any size, in cells that do not overlap: no two pills
    /// meet, each lies in the window, and a small target's pill misses it unless no spot beside
    /// it was free.
    #[test]
    fn random_layouts_keep_pills_apart_and_off_their_targets() {
        let mut seed: u64 = 0x5eed;
        let mut next = |n: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        for round in 0..300 {
            let (cols, rows) = (1 + next(8), 1 + next(8));
            let (cw, ch) = (24. + next(80) as f64, 20. + next(60) as f64);
            let gap = next(3) as f64 * 4.;
            let mut labels = Vec::new();
            for r in 0..rows {
                for c in 0..cols {
                    if next(4) == 0 {
                        continue;
                    }
                    let label = if next(3) == 0 { "ab" } else { "a" };
                    let (x, y) = (c as f64 * (cw + gap), r as f64 * (ch + gap));
                    labels.push(at(&format!("c{r}-{c}"), label, [x, y, cw, ch]));
                }
            }
            // Row by row: reading order, as `Sneak` keeps them.
            place(&mut labels);
            let pills: Vec<Frame> = labels.iter().map(|l| l.pill.expect("a pill")).collect();
            for (i, p) in pills.iter().enumerate() {
                assert!(within(p, &WINDOW), "round {round}: {p:?} leaves the window");
                for q in &pills[i + 1..] {
                    assert!(!p.intersects(q), "round {round}: {p:?} meets {q:?}");
                }
                let l = &labels[i];
                let t = l.target.frame;
                if is_small(&t, &l.label) && p.intersects(&t) {
                    let others: Vec<Frame> =
                        labels.iter().filter(|o| o.target.frame != t).map(|o| o.target.frame).collect();
                    let blocked = |r: &Frame| !within(r, &WINDOW) || others.iter().any(|o| o.intersects(r));
                    let open: Vec<Frame> =
                        beside(&t, pill_size(&l.label), false).into_iter().filter(|r| !blocked(r)).collect();
                    // A spot no target blocks may still hold an earlier pill.
                    assert!(
                        open.iter().all(|r| pills[..i].iter().any(|q| q.intersects(r))),
                        "round {round}: {l:?} {open:?}"
                    );
                }
            }
        }
    }
}
