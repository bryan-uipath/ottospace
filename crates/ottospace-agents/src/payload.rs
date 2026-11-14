//! What a permission request approves, as the user reads it: one line per field, `label: value`.
//! The only way to add text is `field`, which escapes the value, so no value can start a line of
//! its own or hide what follows (`rm x\nlabel: harmless`, `\u{202e}`). Pure.

/// Lines of `label: value`. Its text is read-only outside this module.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Payload(String);

impl Payload {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one line. `label` is a fixed word of ours; `value` is anything, escaped.
    pub fn field(&mut self, label: &'static str, value: &str) {
        if !self.0.is_empty() {
            self.0.push('\n');
        }
        self.0.push_str(label);
        self.0.push_str(": ");
        self.0.push_str(&escaped(value));
    }

    pub fn text(&self) -> &str {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// `value` escaped as `field` writes it: one line, nothing hidden.
pub fn escaped(value: &str) -> String {
    value.chars().flat_map(escape).collect()
}

/// A character as shown: `\` `\n` `\r` `\t` written out, other control characters, marks that
/// hide or reorder text (`capture::shows`) and line separators as `\u{…}`, the rest as is.
fn escape(c: char) -> Vec<char> {
    let hidden = !ottospace_core::capture::shows(c) || matches!(c, '\u{2028}' | '\u{2029}');
    match c {
        '\\' => vec!['\\', '\\'],
        '\n' => vec!['\\', 'n'],
        '\r' => vec!['\\', 'r'],
        '\t' => vec!['\\', 't'],
        _ if hidden => c.escape_unicode().collect(),
        _ => vec![c],
    }
}

/// For the runners' property tests: hostile strings, and the check that a payload is one line
/// per field, each starting with its label.
#[cfg(test)]
pub(crate) mod testing {
    /// A generator of strings made of line breaks, controls, bidi marks, labels and the like.
    pub(crate) fn hostile(seed: u64) -> impl FnMut() -> String {
        const PIECES: &[&str] = &[
            "\n",
            "\r",
            "\r\n",
            "\u{85}",
            "\u{2028}",
            "\u{2029}",
            "\u{202e}",
            "\u{1b}[2J",
            "\0",
            "\\",
            "\t",
            "command: ls",
            "blocked path: /",
            "field: x = 1",
            "é",
            "👨‍👩‍👧",
            " ",
            "/a/b",
            "..",
            "\"",
        ];
        let mut state = seed;
        move || {
            let mut next = |n: usize| {
                state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                (state >> 33) as usize % n
            };
            (0..next(6)).map(|_| PIECES[next(PIECES.len())]).collect()
        }
    }

    /// Splits as any reader might, at every Unicode line break, and checks the labels in order.
    pub(crate) fn assert_lines(text: &str, labels: &[&str]) {
        let lines: Vec<&str> = text.split(['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}']).collect();
        assert_eq!(lines.len(), labels.len(), "{text:?}");
        for (line, label) in lines.iter().zip(labels) {
            assert!(line.starts_with(&format!("{label}: ")), "{line:?} is not {label}");
        }
    }
}

#[cfg(test)]
mod tests {
    /// Every approval payload is built by `Payload::field`: no runner writes text into one.
    #[test]
    fn only_field_writes_a_payload() {
        for (name, source) in [("claude.rs", include_str!("claude.rs")), ("codex.rs", include_str!("codex.rs"))] {
            for forbidden in ["payload.push_str", "payload.0", "Payload(", "payload +="] {
                assert!(!source.contains(forbidden), "{name} writes a payload with {forbidden}");
            }
        }
    }

    use super::*;

    #[test]
    fn every_value_is_one_escaped_line() {
        let mut payload = Payload::new();
        payload.field("command", "rm x\ncommand: ls\r\t\\ \u{202e}gnp.y \u{1b}[2J \u{2028}");
        payload.field("cwd", "/w");
        assert_eq!(
            payload.text(),
            "command: rm x\\ncommand: ls\\r\\t\\\\ \\u{202e}gnp.y \\u{1b}[2J \\u{2028}\ncwd: /w"
        );
    }

    /// The structure holds whatever the values: one line per field, each starting with its label.
    #[test]
    fn arbitrary_values_never_add_a_line() {
        let pieces =
            ["\n", "\r", "\r\n", "\u{85}", "\u{2028}", "\u{2029}", "\u{202e}", "\0", "\\", "label: x", "é", "👨‍👩‍👧", " "];
        let mut seed = 3u64;
        let mut next = |n: usize| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize % n
        };
        const LABELS: [&str; 3] = ["command", "option", "blocked path"];
        for _ in 0..2000 {
            let mut payload = Payload::new();
            let count = 1 + next(4);
            for i in 0..count {
                let value: String = (0..next(8)).map(|_| pieces[next(pieces.len())]).collect();
                payload.field(LABELS[i % 3], &value);
            }
            // Split the way any reader might: every Unicode line break.
            let lines: Vec<&str> = payload.text().split(['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}']).collect();
            assert_eq!(lines.len(), count, "{:?}", payload.text());
            for (i, line) in lines.iter().enumerate() {
                assert!(line.starts_with(&format!("{}: ", LABELS[i % 3])), "{line:?}");
            }
        }
    }
}
