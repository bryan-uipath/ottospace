//! ⌘⌘, the return key's other key: ⌘ tapped twice, alone. Read from modifier changes only;
//! when the last other key or mouse button went down comes with each, as a time, never which.

use serde::Deserialize;

/// Longest a tap may hold ⌘ down: longer is ⌘ held.
pub const HOLD_MS: u64 = 250;
/// Longest from the first tap's release to the second's press.
pub const GAP_MS: u64 = 300;

/// The modifiers as one change left them, at `at` (ms, any one clock).
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Flags {
    pub at: u64,
    /// Either Command key is down.
    pub cmd: bool,
    /// ⇧, ⌃, ⌥ or fn is down (Caps Lock is a state, not a press: left out).
    #[serde(default)]
    pub others: bool,
    /// When a key that is no modifier, or a mouse button, last went down.
    #[serde(default)]
    pub input_at: Option<u64>,
}

/// Where a double tap has got to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Taps {
    #[default]
    Idle,
    /// ⌘ down alone at `at`; `first`: the first tap's release, if this is the second.
    Down { at: u64, first: Option<u64> },
    /// One clean tap, released at `at`.
    Up { at: u64 },
    /// ⌘ down with something else: nothing counts until it is up.
    Spoiled,
}

impl Taps {
    /// The state after `flags`, and whether they finished a double tap.
    /// E.g. ⌘ down 0, up 80, down 200, up 270: true at 270.
    pub fn step(self, flags: Flags) -> (Taps, bool) {
        let since = |t: u64| flags.input_at.is_some_and(|input| input >= t);
        if flags.others {
            return (if flags.cmd { Taps::Spoiled } else { Taps::Idle }, false);
        }
        let next = match (self, flags.cmd) {
            (Taps::Spoiled, true) | (Taps::Down { .. }, true) => self,
            (Taps::Idle, true) => Taps::Down { at: flags.at, first: None },
            (Taps::Up { at: up }, true) => {
                let near = flags.at.saturating_sub(up) <= GAP_MS && !since(up);
                Taps::Down { at: flags.at, first: near.then_some(up) }
            }
            (Taps::Down { at, first }, false) => {
                let clean = flags.at.saturating_sub(at) <= HOLD_MS && !since(at);
                match (clean, first) {
                    (true, Some(_)) => return (Taps::Idle, true),
                    (true, None) => Taps::Up { at: flags.at },
                    (false, _) => Taps::Idle,
                }
            }
            (Taps::Up { .. }, false) => self,
            (Taps::Idle | Taps::Spoiled, false) => Taps::Idle,
        };
        (next, false)
    }

    /// `step` over each of `flags`: how many double taps they made.
    pub fn run(&mut self, flags: impl IntoIterator<Item = Flags>) -> usize {
        let mut fired = 0;
        for f in flags {
            let (next, done) = self.step(f);
            *self = next;
            fired += usize::from(done);
        }
        fired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(at: u64, down: bool) -> Flags {
        Flags { at, cmd: down, ..Flags::default() }
    }

    /// Two taps: ⌘ down at `a`, up at `b`, down at `c`, up at `d`.
    fn taps(a: u64, b: u64, c: u64, d: u64) -> Vec<Flags> {
        vec![cmd(a, true), cmd(b, false), cmd(c, true), cmd(d, false)]
    }

    #[test]
    fn two_quick_taps_of_cmd_alone_return() {
        assert_eq!(Taps::default().run(taps(1000, 1080, 1200, 1270)), 1);
        // At the limits: each held 250 ms, 300 ms between.
        assert_eq!(Taps::default().run(taps(0, 250, 550, 800)), 1);
        // Three taps are one double tap; four are two.
        let mut four = taps(0, 50, 100, 150);
        four.extend(taps(200, 250, 300, 350));
        assert_eq!(Taps::default().run(four.clone()), 2);
        assert_eq!(Taps::default().run(four[..6].to_vec()), 1);
    }

    #[test]
    fn slow_or_held_taps_do_not() {
        // Too far apart, or one held: ⌘ held is not a tap.
        assert_eq!(Taps::default().run(taps(0, 80, 381, 450)), 0);
        assert_eq!(Taps::default().run(taps(0, 251, 300, 350)), 0);
        assert_eq!(Taps::default().run(taps(0, 80, 200, 451)), 0);
        // A slow pair, then a quick one: the quick one counts.
        let mut late = taps(0, 80, 1000, 1080);
        late.extend([cmd(1200, true), cmd(1260, false)]);
        assert_eq!(Taps::default().run(late), 1);
    }

    #[test]
    fn a_shortcut_click_or_other_modifier_spoils_it() {
        // ⌘C (or ⌘Tab, ⌘-click: a key or button down while ⌘ is), then a tap.
        let mut shortcut = taps(0, 80, 200, 270);
        shortcut[1].input_at = Some(40);
        assert_eq!(Taps::default().run(shortcut.clone()), 0);
        // During the second tap, or between the two.
        let mut second = taps(0, 80, 200, 270);
        second[3].input_at = Some(230);
        assert_eq!(Taps::default().run(second), 0);
        let mut between = taps(0, 80, 200, 270);
        between[2].input_at = Some(150);
        assert_eq!(Taps::default().run(between), 0);
        // A key long before the first tap does not count.
        let mut before = taps(1000, 1080, 1200, 1270);
        for f in &mut before {
            f.input_at = Some(500);
        }
        assert_eq!(Taps::default().run(before), 1);
        // ⌘⇧: ⇧ released before ⌘ still spoils that tap.
        let shift = |at, cmd| Flags { at, cmd, others: true, input_at: None };
        let mixed = vec![cmd(0, true), shift(20, true), cmd(40, true), cmd(60, false), cmd(150, true), cmd(200, false)];
        assert_eq!(Taps::default().run(mixed), 0);
        // ⇧ alone between the taps.
        let between =
            vec![cmd(0, true), cmd(60, false), shift(100, false), cmd(110, false), cmd(150, true), cmd(200, false)];
        assert_eq!(Taps::default().run(between), 0);
    }

    #[test]
    fn the_other_command_key_while_one_is_down_is_one_press() {
        // Left ⌘ down, right ⌘ down (still ⌘), both up: one tap.
        let both = vec![cmd(0, true), cmd(30, true), cmd(80, false), cmd(200, true), cmd(260, false)];
        assert_eq!(Taps::default().run(both), 1);
    }
}
