//! Key presses → the bytes a terminal program expects.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
}

/// Bytes to send to the PTY for one key press, or `None` if the key is not for the terminal.
/// `key` is the key's name (`"a"`, `"enter"`, `"up"`); `typed` is the text it would insert.
/// `app_cursor` is the terminal's application-cursor mode (vim and others turn it on).
pub fn encode(key: &str, typed: Option<&str>, mods: Mods, app_cursor: bool) -> Option<Vec<u8>> {
    if mods.cmd {
        return None; // ⌘ shortcuts belong to ottospace
    }
    if let Some(bytes) = named(key, mods, app_cursor) {
        return Some(bytes);
    }

    let mut chars = key.chars();
    let single = match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    };
    if mods.ctrl {
        // ⌃A..⌃Z and ⌃@ ⌃[ ⌃\ ⌃] ⌃^ ⌃_ are the control codes 0..31. e.g. ⌃C → 3.
        // Terminals also map ⌃2 → 0, ⌃/ and ⌃- → 31 (readline undo), ⌃? → 127.
        let code = match single? {
            '2' => 0,
            '/' | '-' => 0x1f,
            '?' => 0x7f,
            c if ('@'..='_').contains(&c.to_ascii_uppercase()) => c.to_ascii_uppercase() as u8 & 0x1f,
            _ => return None,
        };
        return Some(meta(mods.alt, vec![code]));
    }
    if mods.alt {
        // ⌥ is Meta: ESC, then the key. e.g. ⌥B → ESC b (back one word).
        let c = single?;
        let c = if mods.shift { c.to_ascii_uppercase() } else { c };
        return Some(meta(true, c.to_string().into_bytes()));
    }
    typed.filter(|t| !t.is_empty()).map(|t| t.as_bytes().to_vec())
}

/// Prefixes ESC when ⌥ (Meta) is held.
fn meta(alt: bool, mut bytes: Vec<u8>) -> Vec<u8> {
    if alt {
        bytes.insert(0, 0x1b);
    }
    bytes
}

/// Keys with a name instead of a character: enter, arrows, delete and so on.
fn named(key: &str, mods: Mods, app_cursor: bool) -> Option<Vec<u8>> {
    // xterm modifier parameter: 1 + shift(1) + alt(2) + ctrl(4). 1 means no modifier.
    let param = 1 + mods.shift as u8 + 2 * mods.alt as u8 + 4 * mods.ctrl as u8;
    // Cursor keys: `ESC [ A`, or `ESC O A` in application mode, or `ESC [ 1 ; m A` with modifiers.
    let cursor = |c: char| match (param, app_cursor) {
        (1, true) => format!("\x1bO{c}"),
        (1, false) => format!("\x1b[{c}"),
        _ => format!("\x1b[1;{param}{c}"),
    };
    // Editing keys: `ESC [ n ~`, or `ESC [ n ; m ~` with modifiers.
    let tilde = |n: u8| if param == 1 { format!("\x1b[{n}~") } else { format!("\x1b[{n};{param}~") };
    let text: &str = match key {
        "enter" => "\r",
        "tab" if mods.shift => return Some(b"\x1b[Z".to_vec()),
        "tab" => "\t",
        "backspace" => "\x7f",
        "escape" => "\x1b",
        "space" if mods.ctrl => "\0",
        "space" => " ",
        // ⌥← and ⌥→ move by word in shells (ESC b, ESC f), as in Terminal and iTerm.
        "left" if mods.alt && !mods.ctrl && !mods.shift => return Some(b"\x1bb".to_vec()),
        "right" if mods.alt && !mods.ctrl && !mods.shift => return Some(b"\x1bf".to_vec()),
        "up" => return Some(cursor('A').into_bytes()),
        "down" => return Some(cursor('B').into_bytes()),
        "right" => return Some(cursor('C').into_bytes()),
        "left" => return Some(cursor('D').into_bytes()),
        "home" => return Some(cursor('H').into_bytes()),
        "end" => return Some(cursor('F').into_bytes()),
        "pageup" => return Some(tilde(5).into_bytes()),
        "pagedown" => return Some(tilde(6).into_bytes()),
        "delete" => return Some(tilde(3).into_bytes()),
        _ => return None,
    };
    // e.g. ⌥⌫ → ESC DEL (delete a word), ⌥↵ → ESC CR.
    Some(meta(mods.alt, text.as_bytes().to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods { ctrl: false, alt: false, shift: false, cmd: false };

    fn enc(key: &str, typed: Option<&str>, mods: Mods) -> Option<Vec<u8>> {
        encode(key, typed, mods, false)
    }

    #[test]
    fn text_keys_send_what_they_type() {
        assert_eq!(enc("a", Some("a"), NONE), Some(b"a".to_vec()));
        assert_eq!(enc("a", Some("A"), Mods { shift: true, ..NONE }), Some(b"A".to_vec()));
        assert_eq!(enc("e", Some("é"), NONE), Some("é".as_bytes().to_vec()));
        assert_eq!(enc("space", Some(" "), NONE), Some(b" ".to_vec()));
    }

    #[test]
    fn named_keys_send_their_codes() {
        assert_eq!(enc("enter", Some("\n"), NONE), Some(b"\r".to_vec()));
        assert_eq!(enc("backspace", None, NONE), Some(vec![0x7f]));
        assert_eq!(enc("tab", None, Mods { shift: true, ..NONE }), Some(b"\x1b[Z".to_vec()));
        assert_eq!(enc("delete", None, NONE), Some(b"\x1b[3~".to_vec()));
    }

    #[test]
    fn arrows_follow_application_cursor_mode() {
        assert_eq!(encode("up", None, NONE, false), Some(b"\x1b[A".to_vec()));
        assert_eq!(encode("up", None, NONE, true), Some(b"\x1bOA".to_vec()));
    }

    #[test]
    fn ctrl_sends_control_codes() {
        let ctrl = Mods { ctrl: true, ..NONE };
        assert_eq!(enc("c", None, ctrl), Some(vec![3]));
        assert_eq!(enc("[", None, ctrl), Some(vec![0x1b]));
        assert_eq!(enc("space", None, ctrl), Some(vec![0]));
        assert_eq!(enc("1", None, ctrl), None);
    }

    #[test]
    fn modifiers_reach_named_keys() {
        let alt = Mods { alt: true, ..NONE };
        let ctrl = Mods { ctrl: true, ..NONE };
        assert_eq!(enc("backspace", None, alt), Some(b"\x1b\x7f".to_vec())); // delete a word
        assert_eq!(enc("left", None, alt), Some(b"\x1bb".to_vec())); // back a word
        assert_eq!(enc("right", None, alt), Some(b"\x1bf".to_vec()));
        assert_eq!(enc("right", None, ctrl), Some(b"\x1b[1;5C".to_vec()));
        assert_eq!(enc("up", None, Mods { shift: true, ..NONE }), Some(b"\x1b[1;2A".to_vec()));
        assert_eq!(enc("delete", None, ctrl), Some(b"\x1b[3;5~".to_vec()));
        // Modifiers win over application-cursor mode.
        assert_eq!(encode("up", None, ctrl, true), Some(b"\x1b[1;5A".to_vec()));
    }

    #[test]
    fn ctrl_combines_with_alt_and_covers_the_symbol_keys() {
        let ctrl = Mods { ctrl: true, ..NONE };
        assert_eq!(enc("x", None, Mods { ctrl: true, alt: true, ..NONE }), Some(vec![0x1b, 0x18]));
        assert_eq!(enc("/", None, ctrl), Some(vec![0x1f])); // readline undo
        assert_eq!(enc("-", None, ctrl), Some(vec![0x1f]));
        assert_eq!(enc("2", None, ctrl), Some(vec![0]));
        assert_eq!(enc("?", None, ctrl), Some(vec![0x7f]));
    }

    #[test]
    fn alt_is_meta() {
        let alt = Mods { alt: true, ..NONE };
        assert_eq!(enc("b", Some("∫"), alt), Some(b"\x1bb".to_vec()));
    }

    #[test]
    fn cmd_and_bare_modifiers_send_nothing() {
        assert_eq!(enc("d", None, Mods { cmd: true, ..NONE }), None);
        assert_eq!(enc("shift", None, Mods { shift: true, ..NONE }), None);
        assert_eq!(enc("f5", None, NONE), None);
    }
}
