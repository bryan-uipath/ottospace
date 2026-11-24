//! Where the code city's names and callouts go, in window points: the owner's web city's rule
//! (`docs/ai/code-city/original.md`, "Labels"). A thing is named when it is 40 points to 45% of
//! the view on screen, largest first, greedy, never over another name or what `avoid` holds (the
//! key hints), at most 50. Each name is centred on its thing's top: a folder's on its plinth,
//! which must hold it, so deeper folders are named as the camera comes in. Recomputed for every
//! picture; a name shown before keeps its place first.

/// x, y, width, height.
pub type Rect = [f32; 4];

/// What a name is for: the wall of a top-two-tier folder, a deeper folder, a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Wall,
    Folder,
    File,
}

/// A thing that may be named, as the picture shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    /// Which thing (its atlas place): the same across pictures of one plan.
    pub key: usize,
    pub tier: Tier,
    /// Where it is on screen.
    pub bounds: Rect,
    /// The middle of its top on screen: its name is centred there.
    pub top: [f32; 2],
    /// Its top face on screen, corner by corner: a folder's name must lie inside it.
    pub face: [[f32; 2]; 4],
    /// The name's size.
    pub size: [f32; 2],
}

/// Named from this many points on screen, up to `MOST_SHARE` of the view's height.
pub const LEAST: f32 = 40.;
pub const MOST_SHARE: f32 = 0.45;
/// At most this many names.
pub const MOST: usize = 50;
/// Names keep this far apart, and from what they avoid.
const GAP: f32 = 2.;
/// A name shown in the last picture counts as this much larger: a small camera move does not
/// swap two names of near one size.
const KEPT: f32 = 1.25;
/// A plinth holds a name with this much room on each side of it.
const INSET: f32 = 4.;

/// How large a thing is on screen: its bounds' longer side.
fn extent(bounds: Rect) -> f32 {
    bounds[2].max(bounds[3])
}

/// Whether a thing this large on screen may be named in a view this size: one partly in the view,
/// from `LEAST` to `MOST_SHARE` of the view's height (the original's band: far out the districts,
/// near in the files).
pub fn sized(bounds: Rect, view: [f32; 2]) -> bool {
    let e = extent(bounds);
    overlaps(bounds, [0., 0., view[0], view[1]]) && (LEAST..=MOST_SHARE * view[1]).contains(&e)
}

/// Where a face crosses the row at `y`, left and right; none if it does not.
fn across(face: [[f32; 2]; 4], y: f32) -> Option<(f32, f32)> {
    let xs: Vec<f32> = (0..4)
        .filter_map(|i| {
            let (a, b) = (face[i], face[(i + 1) % 4]);
            let (lo, hi) = (a[1].min(b[1]), a[1].max(b[1]));
            if y < lo || y > hi || (b[1] - a[1]).abs() < 1e-6 {
                return None;
            }
            Some(a[0] + (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]))
        })
        .collect();
    let (l, r) = xs.iter().fold((f32::MAX, f32::MIN), |(l, r), x| (l.min(*x), r.max(*x)));
    (l <= r).then_some((l, r))
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

fn grown(r: Rect, by: f32) -> Rect {
    [r[0] - by, r[1] - by, r[2] + 2. * by, r[3] + 2. * by]
}

fn inside(r: Rect, view: [f32; 2]) -> bool {
    r[0] >= 0. && r[1] >= 0. && r[0] + r[2] <= view[0] && r[1] + r[3] <= view[1]
}

/// Where `c`'s name goes: centred on the middle of its top. A folder's plinth must hold it: its
/// top face, as drawn, spans the name and an inset each side along the name's top, middle and
/// bottom rows; else it waits for the camera to come in.
fn spot(c: &Candidate) -> Option<Rect> {
    let [w, h] = c.size;
    let [x, y] = c.top;
    let (left, right) = (x - w / 2. - INSET, x + w / 2. + INSET);
    let row = |y: f32| across(c.face, y).is_some_and(|(l, r)| l <= left && right <= r);
    let holds = c.tier == Tier::File || [y - h / 2., y, y + h / 2.].into_iter().all(row);
    holds.then_some([x - w / 2., y - h / 2., w, h])
}

/// The names drawn, by key, and where: the largest on screen first, as the original; each inside
/// the view, clear of `avoid` and of every name before it. `kept`: the keys named in the last
/// picture.
pub fn place(candidates: &[Candidate], view: [f32; 2], avoid: &[Rect], kept: &[usize]) -> Vec<(usize, Rect)> {
    let mut order: Vec<&Candidate> = candidates.iter().filter(|c| sized(c.bounds, view)).collect();
    let weight = |c: &Candidate| extent(c.bounds) * if kept.contains(&c.key) { KEPT } else { 1. };
    order.sort_by(|a, b| weight(b).total_cmp(&weight(a)).then(a.key.cmp(&b.key)));
    let mut placed: Vec<(usize, Rect)> = Vec::new();
    for c in order {
        if placed.len() == MOST {
            break;
        }
        let Some(r) = spot(c) else { continue };
        let free = |other: &Rect| !overlaps(grown(r, GAP), *other);
        if inside(r, view) && avoid.iter().all(free) && placed.iter().all(|(_, p)| free(p)) {
            placed.push((c.key, r));
        }
    }
    placed
}

/// A hovered folder's callout: under the nearest corner of its outline (the edge nearest the
/// camera), centred on it, if in the view; else at the pointer, or by it just clear of what it
/// avoids. Never over `avoid` where any of those is clear, never off the view. With no outline (a
/// folder the camera is inside of): at the pointer.
pub fn callout(
    outline: Option<[[f32; 2]; 4]>,
    size: [f32; 2],
    pointer: [f32; 2],
    view: [f32; 2],
    avoid: &[Rect],
) -> Rect {
    let [w, h] = size;
    let fit = |x: f32, y: f32| [x.clamp(0., (view[0] - w).max(0.)), y.clamp(0., (view[1] - h).max(0.)), w, h];
    let clear = |r: &Rect| avoid.iter().all(|a| !overlaps(*r, *a));
    let near = outline.map(|o| o.iter().copied().fold(o[0], |a, b| if b[1] > a[1] { b } else { a }));
    let edge = near
        .into_iter()
        .flat_map(|n| [[n[0] - w / 2., n[1] + 2. * GAP, w, h], [n[0] - w / 2., n[1] - h - 2. * GAP, w, h]]);
    let by_pointer = [fit(pointer[0] + 12., pointer[1] + 12.), fit(pointer[0] - w - 12., pointer[1] - h - 12.)];
    let flush = avoid
        .iter()
        .flat_map(|a| [fit(pointer[0] - w / 2., a[1] - h - GAP), fit(pointer[0] - w / 2., a[1] + a[3] + GAP)]);
    let mut all = edge.filter(|r| inside(*r, view)).chain(by_pointer).chain(flush);
    all.find(clear).unwrap_or(by_pointer[0])
}

/// A beam's label beside its dot at `dot`, on the side with more free room (right first when
/// even), `gap` from the dot; the other side if that one is over `avoid` or off the view.
pub fn beside(dot: [f32; 2], size: [f32; 2], gap: f32, view: [f32; 2], avoid: &[Rect]) -> Option<Rect> {
    let [w, h] = size;
    let right = [dot[0] + gap, dot[1] - h / 2., w, h];
    let left = [dot[0] - gap - w, dot[1] - h / 2., w, h];
    let sides = if view[0] - dot[0] >= dot[0] { [right, left] } else { [left, right] };
    sides.into_iter().find(|r| inside(*r, view) && avoid.iter().all(|a| !overlaps(*r, *a)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: [f32; 2] = [800., 500.];

    fn thing(key: usize, tier: Tier, x: f32, y: f32, side: f32) -> Candidate {
        let face = [[x, y], [x + side, y], [x + side, y + side], [x, y + side]];
        Candidate { key, tier, bounds: [x, y, side, side], top: [x + side / 2., y + side / 2.], face, size: [60., 14.] }
    }

    /// A made-up city: a grid of things of many sizes, some overlapping.
    fn many() -> Vec<Candidate> {
        (0..300)
            .map(|i| {
                let tier = [Tier::Wall, Tier::Folder, Tier::File, Tier::File][i % 4];
                let (x, y) = ((i * 37 % 760) as f32, (i * 53 % 460) as f32 + 20.);
                thing(i, tier, x, y, 20. + (i * 7 % 150) as f32)
            })
            .collect()
    }

    #[test]
    fn names_never_overlap_nor_cover_the_hints_nor_leave_the_view() {
        let hints = [300., 200., 200., 60.];
        let placed = place(&many(), VIEW, &[hints], &[]);
        assert!(placed.len() > 5 && placed.len() <= MOST, "{}", placed.len());
        for (i, (_, a)) in placed.iter().enumerate() {
            assert!(inside(*a, VIEW) && !overlaps(*a, hints), "{a:?}");
            for (_, b) in &placed[i + 1..] {
                assert!(!overlaps(*a, *b), "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn a_thing_is_named_from_forty_points_to_under_half_the_view() {
        let size = |side: f32| !place(&[thing(1, Tier::File, 100., 100., side)], [1000., 1000.], &[], &[]).is_empty();
        assert_eq!((size(39.), size(40.), size(450.), size(451.)), (false, true, true, false));
        // Off the view: none.
        assert_eq!(place(&[thing(1, Tier::Wall, 900., 100., 70.)], VIEW, &[], &[]), []);
    }

    /// A folder's name is centred on its plinth, which must hold it: a plinth narrower than its
    /// name is named only once the camera brings it near enough (deeper names as you zoom in).
    #[test]
    fn a_folders_name_is_centred_on_its_plinth_once_it_holds_it() {
        for tier in [Tier::Wall, Tier::Folder] {
            assert_eq!(place(&[thing(1, tier, 100., 100., 50.)], VIEW, &[], &[]), [], "{tier:?}: 50 holds no 60");
            assert_eq!(place(&[thing(1, tier, 100., 100., 70.)], VIEW, &[], &[]), [(1, [105., 128., 60., 14.])]);
            // Zoomed in twice: the same plinth twice as large holds it.
            assert_eq!(place(&[thing(1, tier, 100., 100., 100.)], VIEW, &[], &[]), [(1, [120., 143., 60., 14.])]);
        }
        // A file's name is centred too, at any width its size allows.
        assert_eq!(place(&[thing(1, Tier::File, 100., 100., 40.)], VIEW, &[], &[]), [(1, [90., 113., 60., 14.])]);
        // A long thin plinth across the view's diagonal: wide bounds, a narrow face where its name
        // would sit. Not named.
        let mut thin = thing(1, Tier::Folder, 100., 100., 200.);
        thin.face = [[100., 110.], [110., 100.], [300., 290.], [290., 300.]];
        assert_eq!(place(&[thin], VIEW, &[], &[]), []);
    }

    /// The band's top is 45% of the view's height, as the original: a wide view names no district
    /// taller than that.
    #[test]
    fn the_largest_named_is_under_half_the_views_height() {
        let tall = thing(1, Tier::Folder, 100., 50., 240.);
        assert_eq!(place(&[tall], [1000., 400.], &[], &[]), []);
        assert_eq!(place(&[thing(1, Tier::Folder, 100., 50., 170.)], [1000., 400.], &[], &[]).len(), 1);
    }

    #[test]
    fn the_largest_first_and_at_most_fifty() {
        // Two over one spot: the larger wins, whatever its tier.
        let wall = thing(1, Tier::Wall, 100., 100., 70.);
        let folder = thing(2, Tier::Folder, 90., 90., 90.);
        assert_eq!(place(&[folder, wall], VIEW, &[], &[]).iter().map(|p| p.0).collect::<Vec<_>>(), [2]);
        let (small, large) = (thing(3, Tier::File, 100., 100., 50.), thing(4, Tier::File, 95., 100., 60.));
        assert_eq!(place(&[small, large], VIEW, &[], &[])[0].0, 4);
        let spread: Vec<Candidate> =
            (0..200).map(|i| thing(i, Tier::File, (i % 10) as f32 * 80., (i / 10) as f32 * 24. + 20., 41.)).collect();
        assert_eq!(place(&spread, [800., 600.], &[], &[]).len(), MOST);
    }

    #[test]
    fn a_small_camera_move_keeps_the_names() {
        // Two things of near one size over one spot: whichever was named stays named while the
        // camera moves a little and the other grows past it.
        let (a, mut b) = (thing(1, Tier::Folder, 100., 100., 100.), thing(2, Tier::Folder, 110., 100., 98.));
        assert_eq!(place(&[a, b], VIEW, &[], &[])[0].0, 1);
        b.bounds[2] = 104.;
        assert_eq!(place(&[a, b], VIEW, &[], &[1])[0].0, 1);
        // Moved by a point, every name is the same one, a point over.
        let all = many();
        let first = place(&all, VIEW, &[], &[]);
        let moved: Vec<Candidate> = (all.iter())
            .map(|c| Candidate {
                bounds: [c.bounds[0] + 1., c.bounds[1], c.bounds[2], c.bounds[3]],
                top: [c.top[0] + 1., c.top[1]],
                ..*c
            })
            .collect();
        let kept: Vec<usize> = first.iter().map(|p| p.0).collect();
        let again: Vec<usize> = place(&moved, VIEW, &[], &kept).iter().map(|p| p.0).collect();
        let same = again.iter().filter(|k| kept.contains(k)).count();
        assert!(same * 10 >= kept.len() * 9, "{same} of {}", kept.len());
    }

    #[test]
    fn a_folders_callout_hangs_under_its_near_edge_else_at_the_pointer() {
        let outline = [[100., 100.], [300., 120.], [250., 200.], [60., 170.]];
        let r = callout(Some(outline), [80., 18.], [150., 150.], VIEW, &[]);
        assert_eq!(r, [210., 204., 80., 18.]);
        // The hints there: above the edge's corner; both covered: by the pointer.
        let r = callout(Some(outline), [80., 18.], [150., 150.], VIEW, &[[200., 197., 100., 40.]]);
        assert_eq!(r, [210., 178., 80., 18.]);
        let r = callout(Some(outline), [80., 18.], [100., 100.], VIEW, &[[150., 170., 200., 60.]]);
        assert_eq!(r, [112., 112., 80., 18.]);
        // Its near corner off the view (zoomed in): by the pointer, never clamped to a far corner.
        let big = [[-500., -300.], [1200., -200.], [1100., 800.], [-400., 700.]];
        assert_eq!(callout(Some(big), [80., 18.], [200., 200.], VIEW, &[]), [212., 212., 80., 18.]);
        assert_eq!(callout(None, [80., 18.], [200., 200.], VIEW, &[]), [212., 212., 80., 18.]);
        // A long one by the pointer at the hints' edge: just above them.
        let hints = [200., 180., 400., 140.];
        let r = callout(Some([[400., 300.]; 4]), [300., 18.], [190., 240.], VIEW, &[hints]);
        assert!(!overlaps(r, hints) && inside(r, VIEW), "{r:?}");
    }

    #[test]
    fn a_beams_label_sits_beside_its_dot_on_the_freer_side() {
        let size = [100., 18.];
        assert_eq!(beside([200., 100.], size, 6., VIEW, &[]), Some([206., 91., 100., 18.]));
        assert_eq!(beside([700., 100.], size, 6., VIEW, &[]), Some([594., 91., 100., 18.]));
        // The freer side covered: the other.
        assert_eq!(beside([200., 100.], size, 6., VIEW, &[[220., 95., 10., 10.]]), Some([94., 91., 100., 18.]));
        assert_eq!(beside([50., 100.], size, 6., [120., 500.], &[]), None);
    }
}
