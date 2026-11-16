//! The dial's motion: a spring for each station's size, the pulse of a lit one, the glyph's turn
//! while the model thinks. Pure: time comes in as the ms since the last frame. The shell keeps a
//! `Motion` for the shown dial and steps it each frame it draws. Design: `docs/ai/agent-dial.md`.

use std::f32::consts::TAU;

use crate::activity::{Dial, Spot, Station};

/// The spring: slightly under-damped (ratio 0.6), so a change lands with a small overshoot.
pub const STIFFNESS: f32 = 180.;
pub const DAMPING: f32 = 16.;
/// A call that starts pushes its dot outward, points a second: the kick of a second call.
const KICK: f32 = 28.;
/// Steps no longer than this; a gap longer than `GAP_MS` (the window was hidden) is no travel.
const SUB_MS: f32 = 4.;
pub const GAP_MS: f32 = 250.;
/// Settled: this near its target (points) and this slow (points a second).
const NEAR: f32 = 0.02;
const SLOW: f32 = 0.2;

/// A station's dot, by radius: unused, used, lit, and more for each further call up to `DOT_MOST`.
pub const DOT_UNUSED: f32 = 1.2;
pub const DOT_USED: f32 = 2.2;
pub const DOT_LIT: f32 = 3.5;
pub const DOT_STEP: f32 = 0.7;
pub const DOT_MOST: u32 = 4;
/// A lit dot breathes this much wider, once every `PULSE_MS`.
pub const PULSE_GROW: f32 = 0.5;
pub const PULSE_MS: f32 = 1200.;
/// The glyph turns once in this long.
pub const SPIN_MS: f32 = 2400.;

/// A value on its way to `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub at: f32,
    pub speed: f32,
    pub to: f32,
}

impl Spring {
    pub fn rest(at: f32) -> Spring {
        Spring { at, speed: 0., to: at }
    }

    /// A new target; a higher one kicks.
    pub fn aim(&mut self, to: f32) {
        if to > self.to {
            self.speed += KICK;
        }
        self.to = to;
    }

    pub fn settled(&self) -> bool {
        (self.at - self.to).abs() < NEAR && self.speed.abs() < SLOW
    }

    /// `dt_ms` later. A long gap, or anything not a number, lands on the target.
    pub fn step(&mut self, dt_ms: f32) {
        if !(dt_ms.is_finite() && dt_ms <= GAP_MS && self.at.is_finite() && self.speed.is_finite()) {
            *self = Spring::rest(self.to);
            return;
        }
        let steps = (dt_ms / SUB_MS).ceil().max(1.);
        let h = dt_ms.max(0.) / steps / 1000.;
        for _ in 0..steps as u32 {
            self.speed += (STIFFNESS * (self.to - self.at) - DAMPING * self.speed) * h;
            self.at += self.speed * h;
        }
        if self.settled() {
            *self = Spring::rest(self.to);
        }
    }
}

/// The radius a station's dot goes to.
pub fn dot(spot: &Spot) -> f32 {
    match (spot.lit, spot.calls) {
        (true, _) => DOT_LIT + DOT_STEP * (spot.running.clamp(1, DOT_MOST) - 1) as f32,
        (false, 0) => DOT_UNUSED,
        (false, _) => DOT_USED,
    }
}

/// The pulse `ms` into it: 0 (narrow, bright) to 1 (wide, dim) and back.
pub fn pulse(ms: f32) -> f32 {
    if !ms.is_finite() {
        return 0.;
    }
    (1. - (TAU * (ms.rem_euclid(PULSE_MS) / PULSE_MS)).cos()) / 2.
}

/// The glyph's angle about its upright axis, `dt_ms` later: it turns while the model thinks,
/// then goes on, twice as fast, to face the front (0) and stays.
pub fn spin(angle: f32, thinking: bool, dt_ms: f32) -> f32 {
    if !(angle.is_finite() && dt_ms.is_finite()) {
        return 0.;
    }
    let turned = TAU * dt_ms.max(0.) / SPIN_MS;
    match thinking {
        true => (angle + turned).rem_euclid(TAU),
        false if angle <= 0. || angle + 2. * turned >= TAU => 0.,
        false => angle + 2. * turned,
    }
}

/// The glyph at `angle`, as drawn flat: its width (negative: seen from behind, mirrored) and
/// its light (1 facing, dimmer edge on and from behind).
pub fn facing(angle: f32) -> (f32, f32) {
    let width = angle.cos();
    let light = if width >= 0. { 1. - 0.4 * (1. - width) } else { 0.6 + 0.25 * width };
    (width, light)
}

/// This station's dot pulses: a call runs there or the model thinks, with times known. Not
/// while You is lit (a request or a question may wait for hours: neither You nor the call
/// asked about), nor in a watched session (a call its file never ended stays lit).
pub fn pulsing(spot: &Spot, dial: &Dial) -> bool {
    dial.timed && spot.lit && !dial.spot(Station::You).is_some_and(|you| you.lit)
}

/// The shown dial's moving parts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    /// Each station's dot radius, in `Station::DIAL`'s order.
    pub dots: [Spring; 11],
    /// The glyph's angle: 0 at rest.
    pub angle: f32,
    /// How far into the pulse: 0 while nothing pulses.
    pub pulse_ms: f32,
    /// The last step wanted another frame. After rest the clock starts at the change: the
    /// time since the last frame drawn (a second, an hour) is not travel.
    pub moving: bool,
}

impl Motion {
    /// `dial` at rest: every dot at its size, nothing turning.
    pub fn rest(dial: &Dial) -> Motion {
        let size = |i: usize| dial.stations.get(i).map_or(DOT_UNUSED, dot);
        Motion { dots: std::array::from_fn(|i| Spring::rest(size(i))), angle: 0., pulse_ms: 0., moving: false }
    }

    /// `dt_ms` later, toward `dial`. True while something moves: the next frame is wanted.
    /// `calm` (Reduce motion): sizes jump, nothing pulses or turns, never true.
    pub fn step(&mut self, dial: &Dial, dt_ms: f32, calm: bool) -> bool {
        if calm {
            *self = Motion::rest(dial);
            return false;
        }
        let dt_ms = if self.moving { dt_ms } else { 0. };
        for (spring, spot) in self.dots.iter_mut().zip(&dial.stations) {
            spring.aim(dot(spot));
            spring.step(dt_ms);
        }
        let thinking = dial.thinking();
        self.angle = spin(self.angle, thinking, dt_ms);
        let pulses = dial.stations.iter().any(|spot| pulsing(spot, dial));
        let on = if dt_ms.is_finite() { self.pulse_ms + dt_ms.max(0.) } else { 0. };
        self.pulse_ms = if pulses { on.rem_euclid(PULSE_MS) } else { 0. };
        self.moving = pulses || thinking || self.angle != 0. || self.dots.iter().any(|dot| !dot.settled());
        self.moving
    }

    /// A lit dot's pulse now, 0 to 1.
    pub fn pulse(&self) -> f32 {
        pulse(self.pulse_ms)
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::PI;

    use super::*;
    use crate::activity::{Activity, Is};

    /// A quarter turn: the glyph seen edge on.
    const EDGE: f32 = PI / 2.;

    /// Steps `spring` by `dt` ms `n` times; the extremes it reached.
    fn run(spring: &mut Spring, dt: f32, n: usize) -> (f32, f32) {
        let (mut low, mut high) = (spring.at, spring.at);
        for _ in 0..n {
            spring.step(dt);
            (low, high) = (low.min(spring.at), high.max(spring.at));
        }
        (low, high)
    }

    #[test]
    fn a_spring_settles_on_its_target_with_a_small_overshoot() {
        for dt in [1., 4., 8.3, 16.7, 33., 100., 250.] {
            let mut spring = Spring { to: 1., ..Spring::rest(0.) };
            let (low, high) = run(&mut spring, dt, (3000. / dt) as usize);
            assert_eq!(spring, Spring::rest(1.), "settled, exactly, in three seconds at {dt} ms a frame");
            assert!(low >= 0. && high > 1. && high < 1.12, "{dt}: {low}..{high}");
        }
    }

    #[test]
    fn a_spring_survives_a_long_gap_and_numbers_that_are_not() {
        // The window was hidden for five seconds, mid-travel: on the target, no jump back.
        let moving = Spring { at: 2., speed: 40., to: 5. };
        for dt in [5_000., 251., f32::NAN, f32::INFINITY, -f32::INFINITY, f32::MAX] {
            let mut spring = moving;
            spring.step(dt);
            assert_eq!(spring, Spring::rest(5.), "{dt}");
        }
        for broken in [Spring { at: f32::NAN, speed: 0., to: 3. }, Spring { at: 0., speed: f32::INFINITY, to: 3. }] {
            let mut spring = broken;
            spring.step(16.);
            assert_eq!(spring, Spring::rest(3.));
        }
        // A clock that went back, or did not move: nothing moves, nothing breaks.
        let mut spring = Spring { at: 2., speed: 0., to: 5. };
        spring.step(-16.);
        spring.step(0.);
        assert_eq!(spring, Spring { at: 2., speed: 0., to: 5. });
    }

    #[test]
    fn a_higher_target_kicks_and_a_lower_one_eases_back() {
        let mut up = Spring::rest(DOT_LIT);
        up.aim(DOT_LIT + DOT_STEP);
        let (_, high) = run(&mut up, 8., 400);
        assert_eq!(up, Spring::rest(DOT_LIT + DOT_STEP));
        // A pop past the new size, well short of the next station (21 points away).
        assert!(high > DOT_LIT + DOT_STEP + 0.5 && high < DOT_LIT + DOT_STEP + 2., "{high}");
        let mut down = Spring::rest(DOT_LIT + DOT_STEP);
        down.aim(DOT_LIT);
        assert_eq!(down.speed, 0., "no kick on the way down");
        let (low, high) = run(&mut down, 8., 400);
        assert_eq!(down, Spring::rest(DOT_LIT));
        assert!(high <= DOT_LIT + DOT_STEP && low > DOT_LIT - 0.1, "{low}..{high}");
    }

    #[test]
    fn the_pulse_and_the_turn_are_periodic_and_bounded() {
        assert_eq!((pulse(0.), pulse(PULSE_MS)), (0., 0.));
        assert!((pulse(PULSE_MS / 2.) - 1.).abs() < 1e-5);
        for ms in [-5., 1., 599., 1e9, f32::MAX, f32::NAN, f32::INFINITY] {
            assert!((0. ..=1.).contains(&pulse(ms)), "{ms}");
        }
        // Thinking: a quarter turn in a quarter of the time, round and round.
        assert!((spin(0., true, SPIN_MS / 4.) - EDGE).abs() < 1e-4);
        let far = spin(1., true, 5_000_000.);
        assert!((0. ..TAU).contains(&far), "{far}");
        // Done thinking: on to the front at twice the speed, then still.
        let after = spin(EDGE, false, SPIN_MS / 8.);
        assert!((after - PI).abs() < 1e-4, "{after}");
        assert_eq!(spin(after, false, SPIN_MS / 4.), 0.);
        assert_eq!((spin(0., false, 16.), spin(3., false, 5_000.), spin(f32::NAN, true, 1.)), (0., 0., 0.));
        assert_eq!(spin(1., false, f32::NAN), 0.);
        // Facing: full width and light at the front, a line edge on, mirrored and dim behind.
        assert_eq!(facing(0.), (1., 1.));
        let (edge, behind) = (facing(EDGE), facing(PI));
        assert!(edge.0.abs() < 1e-6 && (edge.1 - 0.6).abs() < 1e-6, "{edge:?}");
        assert!((behind.0 + 1.).abs() < 1e-6 && (behind.1 - 0.35).abs() < 1e-6, "{behind:?}");
    }

    fn live<'a>(turn: bool) -> Is<'a> {
        Is { live: true, timed: true, turn, ..Is::default() }
    }

    /// Steps at 60 frames a second for `ms`; whether the last step wanted another frame.
    fn frames(motion: &mut Motion, dial: &Dial, ms: u32) -> bool {
        (0..ms / 16).map(|_| motion.step(dial, 16., false)).last().unwrap_or(false)
    }

    #[test]
    fn a_dial_at_rest_wants_no_frame_and_one_at_work_does() {
        let mut a = Activity::default();
        a.message(0);
        // Idle: nothing moves, from the first frame.
        let idle = a.dial(1_000, &live(false));
        let mut motion = Motion::rest(&idle);
        assert!(!motion.step(&idle, 16., false));
        assert_eq!(motion, Motion::rest(&idle));
        // The model thinks: its dot pulses and the glyph turns, for as long as it does.
        let thinking = a.dial(1_000, &live(true));
        assert!(thinking.thinking());
        assert!(frames(&mut motion, &thinking, 4_000));
        assert!(motion.angle > 0. && motion.dots[1].to == DOT_LIT);
        // A call: the glyph comes to rest, the call's dot pulses on.
        a.start("1", Station::Shell, "cargo test", 2_000);
        let one = a.dial(3_000, &live(true));
        assert!(!one.thinking());
        assert!(frames(&mut motion, &one, 2_000));
        assert_eq!((motion.angle, motion.dots[3]), (0., Spring::rest(DOT_LIT)));
        // A second call there: a kick, then a larger dot. A fifth is no larger than a fourth.
        a.start("2", Station::Shell, "cargo build", 3_000);
        let two = a.dial(4_000, &live(true));
        assert_eq!(two.spot(Station::Shell).map(|s| s.running), Some(2));
        motion.step(&two, 16., false);
        assert!(motion.dots[3].speed > 20.);
        frames(&mut motion, &two, 2_000);
        assert_eq!(motion.dots[3], Spring::rest(DOT_LIT + DOT_STEP));
        for id in ["3", "4", "5"] {
            a.start(id, Station::Shell, "ls", 4_000);
        }
        let five = a.dial(4_000, &live(true));
        assert_eq!(five.spot(Station::Shell).map(dot), Some(DOT_LIT + 3. * DOT_STEP));
        // The turn ends: the dots settle back, and then no frame is wanted.
        a.settle(None, 5_000);
        let done = a.dial(6_000, &live(false));
        assert!(motion.step(&done, 16., false), "still on its way back");
        assert!(!frames(&mut motion, &done, 3_000));
        assert_eq!(motion, Motion::rest(&done));
    }

    #[test]
    fn reduce_motion_jumps_and_a_request_or_a_watched_call_does_not_pulse() {
        let mut a = Activity::default();
        a.message(0);
        a.start("1", Station::Files, "a.rs", 1_000);
        let working = a.dial(2_000, &live(true));
        let mut motion = Motion::rest(&a.dial(0, &live(false)));
        assert!(!motion.step(&working, 16., true), "calm: no frame");
        assert_eq!(motion, Motion::rest(&working), "at its sizes at once");
        assert_eq!(motion.dots[2].at, DOT_LIT);
        // A long gap (another space, a hidden window): there at once, then only the pulse.
        // A call that starts after a still hour: the clock starts there, its dot travels.
        let mut motion = Motion::rest(&Activity::default().dial(0, &live(false)));
        assert!(motion.step(&working, 3_600_000., false));
        assert_eq!((motion.dots[2].at, motion.dots[2].to), (DOT_UNUSED, DOT_LIT));
        assert!(motion.step(&working, 16., false) && motion.dots[2].at > DOT_UNUSED);
        // Mid-travel, a long gap (a hidden window): there at once, then only the pulse.
        assert!(motion.step(&working, 5_000., false));
        assert!(motion.dots.iter().all(|dot| *dot == Spring::rest(dot.to)));
        // A request waits: You is lit, larger, and still. So is a watched session's call.
        let asking = Is { asking: Some("git push"), ..live(true) };
        let watched = Is { watched: true, ..Is::default() };
        let mut b = Activity::default();
        b.message(0);
        for is in [asking.clone(), watched.clone()] {
            let dial = b.dial(1_000, &is);
            let mut motion = Motion::rest(&dial);
            assert!(!motion.step(&dial, 16., false), "{is:?}");
        }
        // The call asked about is lit as long: it does not pulse either.
        let waiting = a.dial(2_000, &asking);
        assert_eq!(waiting.spot(Station::Files).map(|spot| spot.lit), Some(true));
        let mut motion = Motion::rest(&waiting);
        assert!(!motion.step(&waiting, 16., false));
        let mut motion = Motion::rest(&b.dial(1_000, &watched));
        b.start("w", Station::Network, "example.com", 1_000);
        let dial = b.dial(2_000, &watched);
        assert!(motion.step(&dial, 16., false), "the dot travels to its lit size");
        assert!(!frames(&mut motion, &dial, 3_000), "and rests there");
        assert_eq!(motion.dots[4], Spring::rest(DOT_LIT));
    }
}
