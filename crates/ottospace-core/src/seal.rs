//! The seal, fold and land a user's Done plays (the canvas "Decision made"), about 0.8 s: where
//! each part is `t` ms after the click. Pure: the shell keeps the clock and draws. Under Reduce
//! Motion (`calm`) the tick shows still, the drawer fades, and there are no sparks and no pop.

/// Seal: the ring overshoots and settles, the tick draws itself, sparks fly out.
pub const SEAL_MS: f32 = 250.;
/// Fold: the drawer slides into the rail.
pub const FOLD_MS: f32 = 550.;
/// Land: the task's rail item pops, a ✓ flashes on its corner. Then rest.
pub const LAND_MS: f32 = 800.;
/// How long a spark flies.
const SPARK_MS: f32 = 400.;
/// The drawer's own content fades out under the ring in this long: a crossfade, not a shrink.
pub const CONTENT_MS: f32 = 150.;
/// Where each spark ends, from the ring's centre in points, and its radius.
pub const SPARKS: [(f32, f32, f32); 4] = [(-38., -22., 3.), (40., -28., 2.5), (34., 30., 3.), (-30., 32., 2.)];

/// A spark now: from the centre, and how much of it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spark {
    pub dx: f32,
    pub dy: f32,
    pub r: f32,
    pub alpha: f32,
}

/// Every part at one moment.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// How far the drawer has gone into the rail: 0 where it was, 1 in it.
    pub fold: f32,
    /// How much of the drawer shows.
    pub panel: f32,
    /// How much of the drawer's own content (its header and conversation) shows over it.
    pub content: f32,
    /// The green ring's scale, and how much of it shows.
    pub ring: f32,
    pub ring_alpha: f32,
    /// How much of the tick is drawn.
    pub tick: f32,
    pub sparks: Vec<Spark>,
    /// The rail item's scale, and how much of its corner's ✓ shows.
    pub pop: f32,
    pub badge: f32,
}

/// `at` of the way from `a` to `b`.
fn mix(a: f32, b: f32, at: f32) -> f32 {
    a + (b - a) * at.clamp(0., 1.)
}

/// How far `t` is from `from` to `to`, 0 to 1.
pub(crate) fn part(t: f32, from: f32, to: f32) -> f32 {
    ((t - from) / (to - from)).clamp(0., 1.)
}

pub(crate) fn ease_out(x: f32) -> f32 {
    1. - (1. - x).powi(3)
}

fn ease_in_out(x: f32) -> f32 {
    if x < 0.5 { 4. * x * x * x } else { 1. - (-2. * x + 2.).powi(3) / 2. }
}

/// Through keyframes `(at, value)`, `at` rising from 0 to 1.
fn keys(x: f32, frames: &[(f32, f32)]) -> f32 {
    let mut last = frames[0];
    for &(at, value) in frames {
        if x <= at {
            return mix(last.1, value, if at > last.0 { (x - last.0) / (at - last.0) } else { 1. });
        }
        last = (at, value);
    }
    last.1
}

/// The frame `t` ms after the click; `None` from `LAND_MS` on: at rest, nothing to draw.
pub fn frame(t: f32, calm: bool) -> Option<Frame> {
    if !(0. ..LAND_MS).contains(&t) {
        return None;
    }
    let (seal, fold, land) = (part(t, 0., SEAL_MS), part(t, SEAL_MS, FOLD_MS), part(t, FOLD_MS, LAND_MS));
    // The ✓ on the item's corner: in, held, out.
    let badge = keys(land, &[(0., 0.), (0.3, 1.), (0.8, 1.), (1., 0.)]);
    let content = 1. - part(t, 0., CONTENT_MS);
    if calm {
        let panel = 1. - fold;
        let sparks = Vec::new();
        return Some(Frame { fold: 0., panel, content, ring: 1., ring_alpha: 1., tick: 1., sparks, pop: 1., badge });
    }
    let ring = keys(seal, &[(0., 0.6), (0.3, 1.15), (0.55, 1.), (1., 1.)]);
    let ring_alpha = keys(seal, &[(0., 0.), (0.3, 1.), (1., 1.)]);
    let tick = ease_out(part(seal, 0.2, 1.));
    let fly = part(t, 0., SPARK_MS);
    let sparks = if fly < 1. {
        let (go, alpha) = (ease_out(fly), keys(fly, &[(0., 0.), (0.2, 1.), (1., 0.)]));
        SPARKS.iter().map(|&(dx, dy, r)| Spark { dx: dx * go, dy: dy * go, r, alpha }).collect()
    } else {
        Vec::new()
    };
    let fold = ease_in_out(fold);
    let pop = keys(land, &[(0., 1.), (0.4, 1.28), (0.8, 0.94), (1., 1.)]);
    Some(Frame { fold, panel: 1. - fold, content, ring, ring_alpha, tick, sparks, pop, badge })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seal_rings_ticks_and_sparks_then_folds_lands_and_rests() {
        let at = |t: f32| frame(t, false).expect("playing");
        // The click: a small ring, no tick yet, sparks about to fly, the drawer where it was, its
        // content still drawn; it fades out under the ring by 150 ms.
        let first = at(0.);
        assert_eq!((first.content, at(75.).content, at(CONTENT_MS).content), (1., 0.5, 0.));
        assert_eq!((first.ring, first.tick, first.fold, first.pop, first.sparks.len()), (0.6, 0., 0., 1., 4));
        // The ring overshoots, then settles; the tick draws in the seal.
        assert!(at(75.).ring > 1.1 && (at(250.).ring - 1.).abs() < 1e-6);
        assert!(at(100.).tick > 0. && at(100.).tick < 1. && at(250.).tick == 1.);
        // Sparks once: gone by 400 ms, flown out.
        assert!(at(300.).sparks.iter().all(|s| s.dx.abs() > 20.));
        assert!(at(400.).sparks.is_empty());
        // The fold: into the rail by 550 ms.
        assert!(at(400.).fold > 0. && at(400.).fold < 1. && at(550.).fold == 1.);
        assert_eq!(at(550.).panel, 0.);
        // The land: the item pops, the ✓ flashes, then both settle.
        assert!(at(640.).pop > 1.2 && at(640.).badge > 0.9);
        assert!((at(799.9).pop - 1.).abs() < 0.01 && at(799.9).badge < 0.01);
        assert_eq!(frame(LAND_MS, false), None, "at rest");
        assert_eq!((frame(-1., false), frame(f32::NAN, false)), (None, None));
    }

    #[test]
    fn with_reduce_motion_the_tick_is_still_the_drawer_fades_and_nothing_flies() {
        for t in [0., 100., 250., 400., 600., 799.] {
            let f = frame(t, true).expect("playing");
            assert_eq!((f.ring, f.tick, f.fold, f.pop, f.sparks.len()), (1., 1., 0., 1., 0), "{t}");
        }
        assert_eq!(frame(250., true).map(|f| f.panel), Some(1.));
        assert_eq!(frame(0., true).map(|f| f.content), Some(1.), "its content fades as without Reduce Motion");
        assert!(frame(400., true).is_some_and(|f| f.panel > 0. && f.panel < 1.));
        assert_eq!(frame(550., true).map(|f| f.panel), Some(0.));
    }
}
