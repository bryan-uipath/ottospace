//! Frames that stop coming. GPUI 0.2 draws on a display link of the window's display, and after
//! the displays change (a laptop taken off its monitors) the link may be for a display that is
//! gone, or be none: nothing is drawn again, a quit question included. The tick asks `check`
//! whether to have GPUI draw again another way (`ottospace_mac::window::redraw`): a few times
//! after each change of the displays, and whenever the shown window has had something to draw
//! for `STALL` with no frame. View state, never the model's: it is about GPUI's drawing.

use std::time::{Duration, Instant};

/// How long a shown window may wait with something to draw before it is kicked: dozens of frames.
pub(crate) const STALL: Duration = Duration::from_millis(750);

/// After a change of the displays, when to kick: at once, then as the new setup settles (the
/// window may have no screen in the middle of it).
const AFTER_CHANGE: [Duration; 4] =
    [Duration::ZERO, Duration::from_millis(300), Duration::from_secs(1), Duration::from_secs(3)];

/// Ticks that must see the wait too: one update that held the main thread past `STALL` (a
/// stream starting) is no stall, the frame is next in line.
const TICKS: u32 = 3;

#[derive(Debug, PartialEq)]
pub(crate) enum Verdict {
    Nothing,
    /// Draw again by another path. `stalled`: how long the window had something to draw (none:
    /// a kick planned after a change of the displays). `first`: the first kick of this stall.
    Kick {
        stalled: Option<Duration>,
        first: bool,
    },
    /// A frame came after `kicks` kicks of one stall, which lasted `lasted`.
    Recovered {
        kicks: u32,
        lasted: Duration,
    },
}

pub(crate) struct Stall {
    renders: u64,
    /// Since when the window has had something to draw and drawn nothing.
    waiting: Option<Instant>,
    /// Ticks since then.
    ticks: u32,
    kicks: u32,
    next: Option<Instant>,
    /// Kicks planned after a change of the displays, soonest first.
    planned: Vec<Instant>,
}

impl Stall {
    pub(crate) fn new() -> Self {
        Stall { renders: 0, waiting: None, ticks: 0, kicks: 0, next: None, planned: Vec::new() }
    }

    /// The displays changed at `now`: kicks follow at `AFTER_CHANGE`.
    pub(crate) fn displays_changed(&mut self, now: Instant) {
        self.planned = AFTER_CHANGE.iter().map(|after| now + *after).collect();
    }

    /// Each tick. `renders`: frames drawn so far; `wants`: something asked for a frame not yet
    /// drawn (an action, a held reply); `shown`: some of the window is on a screen.
    pub(crate) fn check(&mut self, now: Instant, renders: u64, wants: bool, shown: bool) -> Verdict {
        if renders != self.renders {
            self.renders = renders;
            let verdict = match (self.kicks, self.waiting) {
                (0, _) | (_, None) => Verdict::Nothing,
                (kicks, Some(since)) => Verdict::Recovered { kicks, lasted: now - since },
            };
            (self.waiting, self.ticks, self.kicks, self.next) = (None, 0, 0, None);
            if wants {
                self.waiting = Some(now);
            }
            return verdict;
        }
        if !shown {
            // A hidden window is not drawn, by design.
            (self.waiting, self.ticks, self.kicks, self.next) = (None, 0, 0, None);
            return Verdict::Nothing;
        }
        let planned = self.planned.first().is_some_and(|at| *at <= now);
        if planned {
            self.planned.retain(|at| *at > now);
        }
        let since = match (wants, self.waiting) {
            (false, _) => {
                (self.waiting, self.ticks, self.kicks, self.next) = (None, 0, 0, None);
                None
            }
            (true, Some(since)) => {
                self.ticks += 1;
                Some(since)
            }
            (true, None) => {
                self.waiting = Some(now);
                Some(now)
            }
        };
        let stalled = since.map(|since| now - since).filter(|waited| *waited >= STALL && self.ticks >= TICKS);
        let due = stalled.is_some() && self.next.is_none_or(|next| next <= now);
        if due {
            self.kicks += 1;
            // 1, 2, 4, 8, 8.. seconds apart: a kick can block for a second while the window
            // server has no drawable to give.
            let between = Duration::from_secs(1 << (self.kicks - 1).min(3));
            self.next = Some(now + between);
            return Verdict::Kick { stalled, first: self.kicks == 1 };
        }
        if planned {
            return Verdict::Kick { stalled: None, first: false };
        }
        Verdict::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_window_that_draws_is_never_kicked() {
        let (mut stall, start) = (Stall::new(), Instant::now());
        for n in 0..200u64 {
            assert_eq!(stall.check(start + ms(16 * n), n, true, true), Verdict::Nothing);
        }
    }

    #[test]
    fn nothing_to_draw_or_a_hidden_window_is_no_stall() {
        let (mut stall, start) = (Stall::new(), Instant::now());
        for n in 0..200u64 {
            assert_eq!(stall.check(start + ms(16 * n), 0, false, true), Verdict::Nothing);
            assert_eq!(stall.check(start + ms(16 * n), 0, true, false), Verdict::Nothing);
        }
    }

    #[test]
    fn a_stall_is_kicked_further_apart_until_a_frame_comes() {
        let (mut stall, start) = (Stall::new(), Instant::now());
        assert_eq!(stall.check(start, 5, true, true), Verdict::Nothing);
        // A tick every 50 ms with no frame: kicked at 750 ms, a second later, then two.
        let kicks: Vec<(u64, Verdict)> = (1..=80u64)
            .map(|n| (n * 50, stall.check(start + ms(n * 50), 5, true, true)))
            .filter(|(_, verdict)| *verdict != Verdict::Nothing)
            .collect();
        let at: Vec<u64> = kicks.iter().map(|(t, _)| *t).collect();
        assert_eq!(at, [750, 1750, 3750]);
        assert_eq!(kicks[0].1, Verdict::Kick { stalled: Some(STALL), first: true });
        assert_eq!(kicks[1].1, Verdict::Kick { stalled: Some(ms(1750)), first: false });
        // A frame: over, and said once.
        let recovered = stall.check(start + ms(4050), 6, false, true);
        assert_eq!(recovered, Verdict::Recovered { kicks: 3, lasted: ms(4050) });
        assert_eq!(stall.check(start + ms(4066), 7, false, true), Verdict::Nothing);
    }

    #[test]
    fn one_long_update_is_no_stall() {
        let (mut stall, start) = (Stall::new(), Instant::now());
        assert_eq!(stall.check(start, 5, true, true), Verdict::Nothing);
        // The main thread held for a second: the next tick sees the wait, but the frame is next.
        assert_eq!(stall.check(start + ms(1000), 5, true, true), Verdict::Nothing);
        assert_eq!(stall.check(start + ms(1016), 6, false, true), Verdict::Nothing);
    }

    #[test]
    fn a_change_of_the_displays_kicks_as_it_settles() {
        let (mut stall, start) = (Stall::new(), Instant::now());
        stall.displays_changed(start);
        let kicks: Vec<u64> = (0..300u64)
            .map(|n| n * 16)
            .filter(|t| stall.check(start + ms(*t), 0, false, true) != Verdict::Nothing)
            .collect();
        // One tick each at 0, 300 ms, 1 s and 3 s (ticks fall on 16 ms).
        assert_eq!(kicks, [0, 304, 1008, 3008]);
    }
}
