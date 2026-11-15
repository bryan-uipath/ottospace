//! UI colours as `0xRRGGBB`: the apollo-ui future-dark scheme, plus one accent per key mode.

use crate::activity::{Level, Pulse, Station};
use crate::model::Mode;
use crate::tasks::Group;

pub const BACKGROUND: u32 = 0x09090b;
/// Bars, pane headers, the secondary bar.
pub const RAISED: u32 = 0x18181b;
/// Selected rows and hovered controls.
pub const SELECTED: u32 = 0x27272a;
pub const BORDER: u32 = 0x27272a;
pub const BORDER_STRONG: u32 = 0x3f3f46;
/// A scrollbar's thumb: at rest, under the pointer, held. The track is clear.
pub const SCROLL_THUMB: u32 = BORDER_STRONG;
pub const SCROLL_THUMB_HOVER: u32 = 0x52525b;
pub const SCROLL_THUMB_HELD: u32 = 0x71717a;
pub const TEXT: u32 = 0xfafafa;
pub const MUTED: u32 = 0x9f9fa9;
pub const PRIMARY: u32 = 0x00b8db;
pub const APP: u32 = 0xff8904;
pub const PICK: u32 = 0x00d492;
pub const VISUAL: u32 = 0xa684ff;
/// The new-task box is open.
pub const COMPOSE: u32 = 0x9ae600;
/// List mode: the secondary bar has the keyboard.
pub const LIST: u32 = 0xfb64b6;
/// The quit question is up.
pub const CONFIRM: u32 = 0xff6467;
/// A mark has the keyboard.
pub const MARK: u32 = 0x51a2ff;
/// The command palette is open.
pub const COMMAND: u32 = 0xed6aff;
/// The accessibility inspector's filter has the keyboard.
pub const INSPECT: u32 = 0x8ec5ff;
pub const ERROR: u32 = 0xff8484;
/// Selected terminal text: a muted cyan, dark enough for light text on it.
pub const SELECTION: u32 = 0x155e75;
/// Sneak labels: a solid amber pill with dark text, readable over any app.
pub const SNEAK: u32 = 0xffb900;

/// The notice bar by level, `(background, text)`: apollo-ui's future-dark semantic tokens
/// `--color-{error,warning,info}-background` and `--color-{error,warning,info}-text`.
pub fn notice(level: crate::notice::Level) -> (u32, u32) {
    match level {
        crate::notice::Level::Error => (0x460809, ERROR),
        crate::notice::Level::Warning => (0x461901, SNEAK),
        crate::notice::Level::Info => (0x053345, 0x00d3f2),
    }
}

/// A task's colour by its inbox heading: its node on the home screen.
pub fn task(group: Group) -> u32 {
    match group {
        Group::NeedsYou => SNEAK,
        Group::Running => PRIMARY,
        Group::Captured => 0x71717a,
        Group::OnHold => 0x52525c,
        Group::Done => PICK,
    }
}

/// A dial's centre, as a task of that state: idle as captured, ended as on hold.
pub fn pulse(pulse: Pulse) -> u32 {
    match pulse {
        Pulse::Running => task(Group::Running),
        Pulse::NeedsYou => task(Group::NeedsYou),
        Pulse::Idle => task(Group::Captured),
        Pulse::Ended => task(Group::OnHold),
        Pulse::Error => ERROR,
    }
}

/// A dial's context ring: muted, then amber, then red.
pub fn level(level: Level) -> u32 {
    match level {
        Level::Calm => MUTED,
        Level::Amber => SNEAK,
        Level::Red => ERROR,
    }
}

/// A dial's stations, each its own hue from the tokens above; the ones that are not tools grey.
pub fn station(station: Station) -> u32 {
    match station {
        Station::You => TEXT,
        Station::Model => VISUAL,
        Station::Files => PICK,
        Station::Shell => APP,
        Station::Network => PRIMARY,
        Station::Browser => INSPECT,
        Station::Agents => COMMAND,
        Station::Skills => COMPOSE,
        Station::Memory => LIST,
        Station::Hooks | Station::Other => MUTED,
    }
}

/// Tasks' identity colours (`Task::colour`): their badges and the underline of their tabs. Hues
/// apart from each other and from the state colours above.
pub const IDENTITY: [u32; crate::tasks::COLOURS as usize] =
    // Neighbours in the order given are far apart: the first tasks look the most different.
    [0xf87171, 0x818cf8, 0xfb923c, 0xf472b6, 0xbef264, 0xc084fc, 0xd6a77a, 0x5eead4];

/// The names of `IDENTITY`'s colours, in its order: what `calendar_colors` may name.
pub const IDENTITY_NAMES: [&str; crate::tasks::COLOURS as usize] =
    ["red", "indigo", "orange", "pink", "lime", "purple", "tan", "teal"];

/// A meeting with a call link, on the band and in the Calendar section.
pub const MEETING: u32 = SNEAK;
/// A meeting with no call link: time held, drawn under the calls.
pub const MEETING_BLOCK: u32 = MUTED;
/// Out of office.
pub const MEETING_AWAY: u32 = 0x7c86a2;

/// A task's identity colour; with none, the muted grey.
pub fn identity(colour: Option<u8>) -> u32 {
    colour.and_then(|at| IDENTITY.get(usize::from(at))).copied().unwrap_or(MUTED)
}

/// The one colour that changes with the mode. It marks the status pill and the focused pane.
pub fn accent(mode: Mode) -> u32 {
    match mode {
        Mode::Normal => PRIMARY,
        Mode::App => APP,
        Mode::Pick => PICK,
        Mode::Palette => COMMAND,
        Mode::Inspect => INSPECT,
        Mode::Visual => VISUAL,
        Mode::List => LIST,
        Mode::Compose => COMPOSE,
        Mode::Sneak => SNEAK,
        Mode::Confirm => CONFIRM,
        Mode::Mark => MARK,
        // The amber of what is drawn. Sneak mode, the other amber, is never on with it.
        Mode::Annotate => SNEAK,
        // Keys go to another program, as in an app pane.
        Mode::Talk => APP,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_colours_are_apart_from_each_other_and_from_the_state_colours() {
        let states = [Group::NeedsYou, Group::Running, Group::Captured, Group::OnHold, Group::Done].map(task);
        for (at, colour) in IDENTITY.iter().enumerate() {
            assert!(!IDENTITY[..at].contains(colour) && !states.contains(colour), "{colour:06x}");
        }
        assert_eq!((identity(Some(1)), identity(Some(200)), identity(None)), (IDENTITY[1], MUTED, MUTED));
    }

    #[test]
    fn each_mode_has_its_own_accent() {
        assert_ne!(accent(Mode::Normal), accent(Mode::App));
        assert_ne!(accent(Mode::Normal), accent(Mode::Pick));
        assert_ne!(accent(Mode::App), accent(Mode::Pick));
        let others = [Mode::Normal, Mode::App, Mode::Pick].map(accent);
        assert!(!others.contains(&accent(Mode::Visual)));
    }
}
