//! UI colours as `0xRRGGBB`: the apollo-ui future-dark scheme, plus one accent per key mode.

use crate::model::Mode;

pub const BACKGROUND: u32 = 0x09090b;
/// Bars, pane headers, the secondary bar.
pub const RAISED: u32 = 0x18181b;
/// Selected rows and hovered controls.
pub const SELECTED: u32 = 0x27272a;
pub const BORDER: u32 = 0x27272a;
pub const BORDER_STRONG: u32 = 0x3f3f46;
pub const TEXT: u32 = 0xfafafa;
pub const MUTED: u32 = 0x9f9fa9;
pub const PRIMARY: u32 = 0x00b8db;
pub const APP: u32 = 0xff8904;
pub const ERROR: u32 = 0xff8484;

/// The one colour that changes with the mode. It marks the status pill and the focused pane.
pub fn accent(mode: Mode) -> u32 {
    match mode {
        Mode::Normal => PRIMARY,
        Mode::App => APP,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_mode_has_its_own_accent() {
        assert_ne!(accent(Mode::Normal), accent(Mode::App));
    }
}
