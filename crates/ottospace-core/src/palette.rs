//! Terminal colours as `0xRRGGBB` (Catppuccin Mocha).

pub const FOREGROUND: u32 = 0xcdd6f4;
pub const BACKGROUND: u32 = 0x1e1e2e;

/// The 16 ANSI colours: black, red, green, yellow, blue, magenta, cyan, white, then bright.
const ANSI: [u32; 16] = [
    0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de, //
    0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
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
