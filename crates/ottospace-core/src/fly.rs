//! The centred new-task box leaving (`overlay_panels`, the canvas "NewTaskFly"): on ↵ it grows into
//! the drawer as the task's pane; closed any other way it fades. Where it is `t` ms after: pure, the
//! shell keeps the clock and draws a ghost over a model that has already moved on. Under Reduce
//! Motion (`calm`) nothing plays: the drawer is there at once.

/// The grow: the box moves right, stretches to the drawer's height and squares its corners.
pub const GROW_MS: f32 = 350.;
/// The fade, for Esc, ⇧↵ and ⌘↵.
pub const FADE_MS: f32 = 150.;
/// The box's corner radius at rest, in points.
pub const RADIUS: f32 = 10.;

/// The ghost at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// How far from the box's place to the drawer's: 0 at the box, 1 at the drawer.
    pub at: f32,
    /// How much of the ghost shows.
    pub alpha: f32,
    /// How much of the dim over the panes is left.
    pub dim: f32,
    /// Its corner radius, in points.
    pub radius: f32,
}

/// The ghost `t` ms after the box left; `grow`: into the drawer, else a fade where it was. None once
/// it is over, and always under Reduce Motion.
pub fn frame(t: f32, grow: bool, calm: bool) -> Option<Frame> {
    use crate::seal::part;
    let end = if grow { GROW_MS } else { FADE_MS };
    if calm || !(0. ..end).contains(&t) {
        return None;
    }
    if !grow {
        let left = 1. - part(t, 0., FADE_MS);
        return Some(Frame { at: 0., alpha: left, dim: left, radius: RADIUS });
    }
    // It travels for most of it; it fades into the drawer drawn under it over the last part.
    let at = crate::seal::ease_out(part(t, 0., GROW_MS * 0.8));
    let alpha = 1. - part(t, GROW_MS * 0.6, GROW_MS);
    let dim = 1. - part(t, 0., GROW_MS * 0.6);
    Some(Frame { at, alpha, dim, radius: RADIUS * (1. - at) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grow_travels_to_the_drawer_squares_its_corners_and_ends() {
        let start = frame(0., true, false).expect("plays");
        assert_eq!((start.at, start.alpha, start.dim, start.radius), (0., 1., 1., RADIUS));
        let late = frame(GROW_MS * 0.9, true, false).expect("plays");
        assert_eq!((late.at, late.dim, late.radius), (1., 0., 0.));
        assert!(late.alpha > 0. && late.alpha < 1.);
        assert!(frame(GROW_MS * 0.2, true, false).is_some_and(|f| f.at > 0.4), "most of the way early: eased out");
        assert_eq!(frame(GROW_MS, true, false), None);
    }

    #[test]
    fn the_fade_stays_where_it_was() {
        let mid = frame(FADE_MS / 2., false, false).expect("plays");
        assert_eq!((mid.at, mid.alpha, mid.dim), (0., 0.5, 0.5));
        assert_eq!(frame(FADE_MS, false, false), None);
    }

    #[test]
    fn reduce_motion_plays_nothing() {
        assert_eq!((frame(0., true, true), frame(0., false, true)), (None, None));
        assert_eq!(frame(-1., true, false), None);
    }
}
