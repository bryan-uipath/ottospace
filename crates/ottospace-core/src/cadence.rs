//! How often the shell's main loop runs. Each frame while something moves or waits, and for
//! `LINGER_MS` after anything happened (a request, a key, a frame, a terminal's output); else
//! once a `SLOW_MS` while a check with no event of its own needs it, at the next thing due, or
//! not at all until woken. The loop's jobs keep their periods in ticks of `FRAME_MS` (`Ticks`).

/// One frame: the loop's fastest step, and its tick.
pub const FRAME_MS: u64 = 16;
/// How long the loop runs each frame after anything happened: a job of a period up to this runs
/// at least once after it.
pub const LINGER_MS: u64 = 1_000;
/// The slow checks' step: what changes with no event to say so (a terminal's title, the settings
/// file, the calendar).
pub const SLOW_MS: u64 = 1_000;

/// What the loop has to look at, as the shell sees it after a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Load {
    /// Something moves, or waits for a frame or for a reply no event announces: each frame.
    pub busy: bool,
    /// A slow check needs looking at: each `SLOW_MS`.
    pub slow: bool,
    /// The next thing due (a file to write, a read to ask for), in the loop's ms.
    pub due: Option<u64>,
}

/// How long the loop waits before it runs again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    /// At most this many ms; a wakeup ends the wait sooner, unless it is one frame.
    For(u64),
    /// Until a wakeup.
    Woken,
}

/// The ticks a run of the loop covers: those since the run before, so a job of a period runs
/// once each period however long the loop slept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticks {
    from: Option<u64>,
    to: u64,
}

impl Ticks {
    /// Whether a tick `t` with `t % every == phase` came since the last run: `every(15, 0)`
    /// as `tick % 15 == 0` was, when the loop ran every tick.
    pub fn every(&self, every: u64, phase: u64) -> bool {
        // How many such ticks there are up to `t`, 0 included.
        let upto = |t: u64| if t < phase { 0 } else { (t - phase) / every + 1 };
        upto(self.to) > self.from.map_or(0, upto)
    }
}

/// The loop's own time: its last run's tick, and when something last happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clock {
    last: Option<u64>,
    woke: u64,
}

impl Default for Clock {
    /// A start is something happening: the loop runs each frame for its first `LINGER_MS`.
    fn default() -> Self {
        Clock { last: None, woke: 0 }
    }
}

impl Clock {
    /// A run at `now` (the loop's ms); `woke`: something happened since the last run.
    pub fn step(&mut self, now: u64, woke: bool) -> Ticks {
        if woke {
            self.woke = now;
        }
        let to = now / FRAME_MS;
        // Never back: the same tick again covers none.
        let from = self.last.map(|last| last.min(to));
        self.last = Some(to);
        Ticks { from, to }
    }

    /// After a run at `now`: how long to wait for the next.
    pub fn wait(&self, now: u64, load: Load) -> Wait {
        if load.busy || now < self.woke.saturating_add(LINGER_MS) {
            return Wait::For(FRAME_MS);
        }
        let slow = load.slow.then_some(SLOW_MS);
        // Not sooner than a frame: something due now runs at the next.
        let due = load.due.map(|at| at.saturating_sub(now).max(FRAME_MS));
        match slow.into_iter().chain(due).min() {
            Some(ms) => Wait::For(ms),
            None => Wait::Woken,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticks(from: Option<u64>, to: u64) -> Ticks {
        Ticks { from, to }
    }

    /// Run each tick, `every` is the old `tick % n == phase`.
    #[test]
    fn run_each_tick_a_job_runs_on_its_ticks() {
        for (every, phase) in [(1, 0), (3, 0), (15, 0), (15, 7), (30, 15), (60, 0)] {
            let mut clock = Clock::default();
            for tick in 0..200u64 {
                let ran = clock.step(tick * FRAME_MS, false).every(every, phase);
                assert_eq!(ran, tick % every == phase, "every {every} phase {phase} tick {tick}");
            }
        }
    }

    /// After a sleep, each job runs once, whatever its period, and only if its tick came.
    #[test]
    fn after_a_sleep_a_job_runs_once_if_its_tick_came() {
        assert!(ticks(Some(10), 100).every(60, 0));
        assert!(ticks(Some(10), 100).every(15, 7));
        assert!(!ticks(Some(61), 70).every(60, 0));
        assert!(ticks(Some(59), 60).every(60, 0));
        assert!(!ticks(Some(60), 60).every(1, 0));
        // The first run covers tick 0, as the first tick did.
        assert!(ticks(None, 0).every(30, 0));
        assert!(!ticks(None, 0).every(30, 15));
    }

    #[test]
    fn a_clock_that_goes_back_covers_no_tick() {
        let mut clock = Clock::default();
        clock.step(1_000, false);
        let back = clock.step(500, false);
        assert!(!back.every(1, 0));
    }

    /// Busy, or within `LINGER_MS` of a wakeup (a start is one): each frame.
    #[test]
    fn busy_or_just_woken_runs_each_frame() {
        let mut clock = Clock::default();
        let idle = Load::default();
        clock.step(0, false);
        assert_eq!(clock.wait(0, idle), Wait::For(FRAME_MS));
        assert_eq!(clock.wait(LINGER_MS - 1, idle), Wait::For(FRAME_MS));
        assert_eq!(clock.wait(LINGER_MS, idle), Wait::Woken);
        assert_eq!(clock.wait(LINGER_MS, Load { busy: true, ..idle }), Wait::For(FRAME_MS));
        clock.step(5_000, true);
        assert_eq!(clock.wait(5_500, idle), Wait::For(FRAME_MS));
        assert_eq!(clock.wait(6_000, idle), Wait::Woken);
    }

    /// At rest: the slow step, or sooner the next thing due, never sooner than a frame.
    #[test]
    fn at_rest_it_waits_for_the_slow_step_or_what_is_due() {
        let clock = Clock::default();
        let now = 10_000;
        let slow = Load { slow: true, ..Load::default() };
        assert_eq!(clock.wait(now, slow), Wait::For(SLOW_MS));
        assert_eq!(clock.wait(now, Load { due: Some(now + 300), ..slow }), Wait::For(300));
        assert_eq!(clock.wait(now, Load { due: Some(now + 5_000), ..slow }), Wait::For(SLOW_MS));
        assert_eq!(clock.wait(now, Load { due: Some(now + 5_000), ..Load::default() }), Wait::For(5_000));
        assert_eq!(clock.wait(now, Load { due: Some(now - 50), ..Load::default() }), Wait::For(FRAME_MS));
        assert_eq!(clock.wait(now, Load::default()), Wait::Woken);
    }
}
