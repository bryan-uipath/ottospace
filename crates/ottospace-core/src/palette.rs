//! Terminal colours as `0xRRGGBB`, matched to the future-dark UI theme.

pub const FOREGROUND: u32 = crate::theme::TEXT;
pub const BACKGROUND: u32 = crate::theme::BACKGROUND;

/// The 16 ANSI colours: black, red, green, yellow, blue, magenta, cyan, white, then bright.
const ANSI: [u32; 16] = [
    0x3f3f46, 0xff6467, 0x00d492, 0xffb900, 0x51a2ff, 0xc27aff, 0x00d3f2, 0xd4d4d8, //
    0x71717b, 0xff8484, 0x5ee9b5, 0xffd230, 0x8ec5ff, 0xdab2ff, 0x53eafd, 0xfafafa,
];

/// xterm 256-colour index → colour. 0-15 ANSI, 16-231 a 6×6×6 cube, 232-255 a grey ramp.
pub fn xterm(index: u8) -> u32 {
    match index {
        0..=15 => ANSI[index as usize],
        16..=231 => {
            let i = index as u32 - 16;
            let level = |n: u32| if n == 0 { 0 } else { 55 + 40 * n };
            level(i / 36) << 16 | level(i / 6 % 6) << 8 | level(i % 6)
        }
        232..=255 => {
            let grey = 8 + 10 * (index as u32 - 232);
            grey << 16 | grey << 8 | grey
        }
    }
}

#[cfg(test)]
mod tests {
    use super::xterm;

    #[test]
    fn cube_and_grey_ramp_match_xterm() {
        assert_eq!(xterm(16), 0x000000);
        assert_eq!(xterm(196), 0xff0000); // pure red corner
        assert_eq!(xterm(110), 0x87afd7);
        assert_eq!(xterm(231), 0xffffff);
        assert_eq!(xterm(232), 0x080808);
        assert_eq!(xterm(255), 0xeeeeee);
    }
}
