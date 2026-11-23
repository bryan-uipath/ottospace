//! Copy and paste: the text a terminal's selection copies, the bytes a paste sends, the cells a
//! copy lights, and the notice that says either happened. The shell holds the text; the core
//! knows only how much of it there is.

use serde::Serialize;

use crate::model::PaneId;

/// Why a client's paste is refused.
pub const USERS_ALONE: &str = "only the user pastes their clipboard";
/// Files dropped from Finder are the user's own act: no client types a path this way (`type` is theirs).
pub const DROP_USERS: &str = "only the user drops files";
/// What a pane or the message box says when a drop types nothing.
pub const DROP_NO_ROOM: &str = "Nothing added: the message has no room for the dropped paths";
pub const DROP_REFUSED: &str = "Nothing typed: a dropped name holds a control character, or is not text";

/// How long the title strip and the status bar say "Copied 3 lines".
pub const NOTICE_MS: u64 = 2_000;
/// How long the copied cells, or the pane a paste went to, pulse.
pub const PULSE_MS: u64 = 600;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verb {
    Copied,
    Cut,
    Pasted,
}

/// How much text was copied or is pasted. `newline`: it holds a line break, even a last one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Amount {
    pub lines: usize,
    pub chars: usize,
    pub newline: bool,
}

impl Amount {
    /// `"a\nb"`: 2 lines. `"a\n"`: 1 line, with a newline. `\r\n` is one break.
    pub fn of(text: &str) -> Self {
        let breaks = text.replace("\r\n", "\n").replace('\r', "\n");
        let lines = breaks.trim_end_matches('\n').split('\n').count();
        Self { lines, chars: text.chars().count(), newline: breaks.contains('\n') }
    }

    /// "3 lines", else "42 characters" or "1 character".
    pub fn words(&self) -> String {
        match (self.lines, self.chars) {
            (lines, _) if lines > 1 => format!("{lines} lines"),
            (_, 1) => "1 character".into(),
            (_, chars) => format!("{chars} characters"),
        }
    }
}

/// What the title strip of `pane` (none: a copy from no pane, e.g. a chat's code block) and the
/// status bar say for `NOTICE_MS` from `at`. For the first `PULSE_MS` the copied cells pulse, or
/// the pane (`Clip::lights`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Clip {
    pub pane: Option<PaneId>,
    pub verb: Verb,
    pub amount: Amount,
    /// "Copied 3 lines".
    pub text: String,
    pub at: u64,
    /// A copy's first and last cell in its terminal, as (line, column): line 0 the screen's top
    /// at no scroll, negative in the history.
    #[serde(skip)]
    pub range: Option<[(i32, usize); 2]>,
}

/// What a clip lights for its pulse: these cells of the pane's screen, each (row, first column,
/// last column), else (`ring`) the pane's border.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lights {
    pub cells: Vec<(usize, usize, usize)>,
    pub ring: bool,
}

impl Clip {
    pub fn new(pane: Option<PaneId>, verb: Verb, amount: Amount, at: u64) -> Self {
        let said = match verb {
            Verb::Copied => "Copied",
            Verb::Cut => "Cut",
            Verb::Pasted => "Pasted",
        };
        Self { pane, verb, amount, text: format!("{said} {}", amount.words()), at, range: None }
    }

    /// What pulses in a terminal scrolled `back` lines, `rows` by `cols`: the copied cells on
    /// screen, where its selection is now (`live`: moved with new output) else where it was. A
    /// paste, or a copy with none on screen (scrolled away), lights the border alone: never a fill
    /// over the text.
    pub fn lights(&self, live: Option<[(i32, usize); 2]>, back: usize, rows: usize, cols: usize) -> Lights {
        let range = live.or(self.range).filter(|_| self.range.is_some() && self.verb != Verb::Pasted);
        let Some([(first, from), (last, to)]) = range else {
            return Lights { cells: Vec::new(), ring: true };
        };
        let cells: Vec<_> = (first..=last)
            .filter_map(|line| {
                let row = usize::try_from(line + back as i32).ok().filter(|row| *row < rows)?;
                let a = if line == first { from } else { 0 };
                let b = if line == last { to } else { usize::MAX };
                // Past a narrowed grid's edge: nothing of it shows.
                (a < cols).then(|| (row, a, b.min(cols - 1)))
            })
            .collect();
        Lights { ring: cells.is_empty(), cells }
    }

    /// Still said at `now`.
    pub fn shows(&self, now: u64) -> bool {
        now < self.at.saturating_add(NOTICE_MS)
    }

    /// How strongly the pane's border glows at `now`, 0 to 1: one rise and fall over `PULSE_MS`.
    /// With Reduce motion, a steady highlight for as long.
    pub fn glow(&self, now: u64, reduced: bool) -> f32 {
        let t = now.saturating_sub(self.at);
        match (self.pane, t < PULSE_MS, reduced) {
            (None, ..) | (_, false, _) => 0.,
            (_, true, true) => 1.,
            (_, true, false) => (std::f32::consts::PI * t as f32 / PULSE_MS as f32).sin(),
        }
    }
}

/// What a terminal's selection copies, from the grid's own text of it: each line without its
/// trailing spaces, and no line break at the end (a copied line pasted at a prompt must not run).
/// A wrapped line is already one line.
pub fn copy_text(raw: &str) -> String {
    let lines: Vec<&str> = raw.split('\n').map(|line| line.trim_end_matches(' ')).collect();
    lines.join("\n").trim_end_matches('\n').to_string()
}

/// The bytes a paste of `text` sends. Line breaks become ↵ (CR), as typed. Control characters
/// but tab go, ESC among them, and the bracket's markers first: pasted text cannot end the
/// bracket early or send keys of its own. `bracketed`: the program asked to be told a paste.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\x1b[201~", "").replace("\x1b[200~", "").replace("\r\n", "\n");
    let body: String = text
        .chars()
        .filter_map(|c| match c {
            '\n' | '\r' => Some('\r'),
            '\t' => Some('\t'),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect();
    match bracketed {
        true => format!("\x1b[200~{body}\x1b[201~").into_bytes(),
        false => body.into_bytes(),
    }
}

/// Dropped files' paths as a terminal types them, as Terminal and iTerm do: each in single quotes
/// (a `'` in a name as `'\''`), a space between and one after, no line break, so nothing in a
/// name is read by a shell and nothing runs until the user presses ↵. `None`: a name holds a
/// control character or a line break of any kind (U+2028 too: a program may read it as one,
/// and run what follows), or there are none.
/// e.g. `["/tmp/a b", "/tmp/it's"]` → `'/tmp/a b' '/tmp/it'\''s' `.
pub fn dropped(paths: &[String]) -> Option<String> {
    let breaks = |c: char| c.is_control() || crate::capture::is_break(c);
    if paths.is_empty() || paths.iter().any(|path| path.chars().any(breaks)) {
        return None;
    }
    let quote = |path: &String| format!("'{}'", path.replace('\'', "'\\''"));
    Some(paths.iter().map(quote).collect::<Vec<_>>().join(" ") + " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_amount_counts_lines_and_says_characters_for_one() {
        let amount = |text| Amount::of(text);
        assert_eq!(amount("a\nb\r\nc"), Amount { lines: 3, chars: 6, newline: true });
        assert_eq!(amount("ls\n"), Amount { lines: 1, chars: 3, newline: true });
        assert_eq!(amount("héllo"), Amount { lines: 1, chars: 5, newline: false });
        assert_eq!((amount("a\nb\nc").words(), amount("x").words()), ("3 lines".into(), "1 character".into()));
        assert_eq!(amount("forty-two characters of text, give or take").words(), "42 characters");
    }

    #[test]
    fn the_notice_says_what_happened_and_goes_after_two_seconds() {
        let clip = Clip::new(Some(1), Verb::Copied, Amount::of("a\nb\nc"), 10_000);
        assert_eq!(clip.text, "Copied 3 lines");
        assert_eq!(Clip::new(None, Verb::Pasted, Amount::of("ab"), 0).text, "Pasted 2 characters");
        assert_eq!(Clip::new(None, Verb::Cut, Amount::of("a"), 0).text, "Cut 1 character");
        assert!(clip.shows(10_000) && clip.shows(10_000 + NOTICE_MS - 1));
        assert!(!clip.shows(10_000 + NOTICE_MS));
    }

    #[test]
    fn the_pane_pulses_once_or_holds_still_with_reduced_motion() {
        let clip = Clip::new(Some(1), Verb::Copied, Amount::of("a"), 1_000);
        assert_eq!(clip.glow(1_000, false), 0.);
        assert!(clip.glow(1_000 + PULSE_MS / 2, false) > 0.99, "brightest halfway");
        assert!(clip.glow(1_000 + PULSE_MS / 6, false) < clip.glow(1_000 + PULSE_MS / 3, false));
        assert_eq!((clip.glow(1_000 + PULSE_MS, false), clip.glow(1_000 + PULSE_MS, true)), (0., 0.));
        assert_eq!((clip.glow(1_000, true), clip.glow(1_000 + PULSE_MS - 1, true)), (1., 1.));
        // A copy from no pane pulses none.
        assert_eq!(Clip::new(None, Verb::Copied, Amount::of("a"), 1_000).glow(1_300, false), 0.);
    }

    #[test]
    fn copied_text_loses_trailing_spaces_and_the_last_newline() {
        assert_eq!(copy_text("alpha beta gamma\n"), "alpha beta gamma");
        assert_eq!(copy_text("one   \n  two  \n\nthree\n\n"), "one\n  two\n\nthree");
        assert_eq!(copy_text("\ttab\t"), "\ttab\t", "tabs stay");
        assert_eq!(copy_text("   "), "");
    }

    #[test]
    fn paste_sends_returns_and_no_control_characters() {
        assert_eq!(paste_bytes("a\nb\r\nc\rd", false), b"a\rb\rc\rd".to_vec());
        assert_eq!(paste_bytes("a\tb\x03\x7f\u{9b}c\x1b[31m", false), b"a\tbc[31m".to_vec());
        assert_eq!(paste_bytes("a\nb", true), b"\x1b[200~a\rb\x1b[201~".to_vec());
    }

    #[test]
    fn pasted_text_cannot_end_the_bracket() {
        let sent = paste_bytes("x\x1b[201~rm -rf ~\n\x1b[200~y", true);
        assert_eq!(sent, b"\x1b[200~xrm -rf ~\ry\x1b[201~".to_vec());
        // Nor a marker the removal itself would join: its ESC goes too.
        let sent = String::from_utf8(paste_bytes("\x1b[20\x1b[201~1~", true)).unwrap();
        assert_eq!(sent.matches("\x1b[201~").count(), 1, "{sent:?}");
        assert!(sent.ends_with("\x1b[201~"));
    }

    #[test]
    fn a_multi_line_paste_without_brackets_goes_as_typed() {
        // As Terminal does: each line break is a ↵, so the shell runs each line. Brackets, when
        // the program asked, hold them until the user's own ↵.
        assert_eq!(paste_bytes("make\nmake install\n", false), b"make\rmake install\r".to_vec());
    }

    #[test]
    fn a_copy_lights_its_cells_on_screen_else_the_border() {
        let copy = |range| Clip { range, ..Clip::new(Some(1), Verb::Copied, Amount::of("a"), 0) };
        // From column 3 of row 1 to column 2 of row 3, 10 wide: the middle row whole.
        let lit = copy(Some([(1, 3), (3, 2)])).lights(None, 0, 24, 10);
        assert_eq!(lit, Lights { cells: vec![(1, 3, 9), (2, 0, 9), (3, 0, 2)], ring: false });
        // Output moved the selection up a row: the cells follow it.
        assert_eq!(copy(Some([(1, 3), (1, 5)])).lights(Some([(0, 3), (0, 5)]), 0, 24, 10).cells, [(0, 3, 5)]);
        // Narrowed to 4 columns: what was past the edge is not drawn.
        assert_eq!(copy(Some([(1, 6), (2, 1)])).lights(None, 0, 24, 4).cells, [(2, 0, 1)]);
        // Scrolled back 2: two rows of history in view, the copy's rows lower by 2; its last row
        // past the screen's foot is not drawn.
        assert_eq!(copy(Some([(-2, 0), (1, 4)])).lights(None, 2, 3, 10).cells, [(0, 0, 9), (1, 0, 9), (2, 0, 9)]);
        // Scrolled out of view, or no range: the border.
        assert_eq!(copy(Some([(-9, 0), (-8, 4)])).lights(None, 0, 24, 10), Lights { cells: vec![], ring: true });
        assert!(
            copy(None).lights(Some([(0, 0), (0, 1)]), 0, 24, 10).ring,
            "no range: a selection made since is not it"
        );
        // A paste lights its pane's border, never cells.
        let paste = Clip { range: Some([(0, 0), (0, 1)]), ..Clip::new(Some(1), Verb::Pasted, Amount::of("a"), 0) };
        assert_eq!(paste.lights(None, 0, 24, 10), Lights { cells: vec![], ring: true });
    }

    #[test]
    fn dropped_names_are_quoted_whole_and_never_run() {
        let drop = |paths: &[&str]| dropped(&paths.iter().map(|p| p.to_string()).collect::<Vec<_>>());
        assert_eq!(drop(&["/tmp/a.txt"]).as_deref(), Some("'/tmp/a.txt' "));
        assert_eq!(drop(&["/tmp/a b", "/tmp/c"]).as_deref(), Some("'/tmp/a b' '/tmp/c' "));
        // A quote ends the quoted part, is escaped, and opens it again.
        assert_eq!(drop(&["/tmp/it's"]).as_deref(), Some("'/tmp/it'\\''s' "));
        // Nothing a shell would read: $(), backticks, ;, &, |, *, ~, $VAR, a leading -, unicode.
        for name in [
            "/tmp/$(rm -rf ~)",
            "/tmp/`id`",
            "/tmp/a;b",
            "/tmp/a&b|c",
            "/tmp/*",
            "~/x",
            "/tmp/$HOME",
            "-rf",
            "/tmp/日本 🚀",
        ] {
            assert_eq!(drop(&[name]), Some(format!("'{name}' ")), "{name}");
        }
        // A line break, a carriage return, a tab, an escape, a nul: nothing typed at all.
        for name in ["/tmp/a\nrm -rf ~", "/tmp/a\rb", "/tmp/a\tb", "/tmp/a\x1b[31m", "/tmp/a\0b", "/tmp/a\u{2028}b"] {
            assert_eq!(drop(&["/tmp/ok", name]), None, "{name:?}");
        }
        assert_eq!(drop(&[]), None);
    }
}
