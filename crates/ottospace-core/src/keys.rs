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
    let arrow = |c: char| if app_cursor { format!("\x1bO{c}") } else { format!("\x1b[{c}") };
    let named = match key {
        "enter" => "\r".into(),
        "tab" if mods.shift => "\x1b[Z".into(),
        "tab" => "\t".into(),
        "backspace" => "\x7f".into(),
        "escape" => "\x1b".into(),
        "up" => arrow('A'),
        "down" => arrow('B'),
        "right" => arrow('C'),
        "left" => arrow('D'),
        "home" => arrow('H'),
        "end" => arrow('F'),
        "pageup" => "\x1b[5~".into(),
        "pagedown" => "\x1b[6~".into(),
        "delete" => "\x1b[3~".into(),
        "space" if mods.ctrl => "\0".into(),
        "space" => " ".into(),
        _ => String::new(),
    };
    if !named.is_empty() {
        return Some(named.into_bytes());
    }

    let mut chars = key.chars();
    let single = match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    };
    if mods.ctrl {
        // ⌃A..⌃Z and ⌃@ ⌃[ ⌃\ ⌃] ⌃^ ⌃_ are the control codes 0..31. e.g. ⌃C → 3.
        let upper = single?.to_ascii_uppercase();
        return ('@'..='_').contains(&upper).then(|| vec![upper as u8 & 0x1f]);
    }
    if mods.alt {
        // ⌥ is Meta: ESC, then the key. e.g. ⌥B → ESC b (back one word).
        let c = single?;
        let c = if mods.shift { c.to_ascii_uppercase() } else { c };
        return Some(format!("\x1b{c}").into_bytes());
    }
    typed.filter(|t| !t.is_empty()).map(|t| t.as_bytes().to_vec())
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
