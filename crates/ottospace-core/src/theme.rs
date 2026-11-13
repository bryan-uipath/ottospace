//! UI colours as `0xRRGGBB`: the apollo-ui future-dark scheme, plus one accent per key mode.

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
pub const ERROR: u32 = 0xff8484;
/// Selected terminal text: a muted cyan, dark enough for light text on it.
pub const SELECTION: u32 = 0x155e75;
/// Sneak labels: a solid amber pill with dark text, readable over any app.
pub const SNEAK: u32 = 0xffb900;

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

/// The one colour that changes with the mode. It marks the status pill and the focused pane.
pub fn accent(mode: Mode) -> u32 {
    match mode {
        Mode::Normal => PRIMARY,
        Mode::App => APP,
        Mode::Pick => PICK,
        Mode::Visual => VISUAL,
        Mode::List => LIST,
        Mode::Compose => COMPOSE,
        Mode::Sneak => SNEAK,
        Mode::Confirm => CONFIRM,
        Mode::Mark => MARK,
        // The amber of what is drawn. Sneak mode, the other amber, is never on with it.
        Mode::Annotate => SNEAK,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_mode_has_its_own_accent() {
        assert_ne!(accent(Mode::Normal), accent(Mode::App));
        assert_ne!(accent(Mode::Normal), accent(Mode::Pick));
        assert_ne!(accent(Mode::App), accent(Mode::Pick));
        let others = [Mode::Normal, Mode::App, Mode::Pick].map(accent);
        assert!(!others.contains(&accent(Mode::Visual)));
    }
}
