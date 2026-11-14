//! Copy and paste: the text a terminal's selection copies, the bytes a paste sends, when a paste
//! asks first, and the notice that says either happened. The shell holds the text; the core
//! knows only how much of it there is.

use serde::{Deserialize, Serialize};

use crate::keys::Mods;
use crate::model::{Action, PaneId};

/// Why a client's paste, or its answer to "Paste 4 lines?", is refused.
pub const USERS_ALONE: &str = "only the user pastes their clipboard";

/// How long the title strip and the status bar say "Copied 3 lines".
pub const NOTICE_MS: u64 = 2_000;
/// How long the pane the text came from, or went to, pulses.
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
/// status bar say for `NOTICE_MS` from `at`. The pane pulses for the first `PULSE_MS`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Clip {
    pub pane: Option<PaneId>,
    pub verb: Verb,
    pub amount: Amount,
    /// "Copied 3 lines".
    pub text: String,
    pub at: u64,
}

impl Clip {
    pub fn new(pane: Option<PaneId>, verb: Verb, amount: Amount, at: u64) -> Self {
        let said = match verb {
            Verb::Copied => "Copied",
            Verb::Cut => "Cut",
            Verb::Pasted => "Pasted",
        };
        Self { pane, verb, amount, text: format!("{said} {}", amount.words()), at }
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

/// When a paste into a terminal asks first (`paste_confirm` in the settings).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PasteConfirm {
    /// Text with a line break, bracketed paste or not: a program may run each line.
    #[default]
    Multiline,
    Never,
}

/// A paste waits for the user's ↵: it holds a line break and the settings ask about those.
pub fn asks(amount: Amount, setting: PasteConfirm) -> bool {
    amount.newline && setting == PasteConfirm::Multiline
}

/// "Paste 4 lines?" over pane `pane`, until the user answers. The text is the shell's.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct PasteAsk {
    pub pane: PaneId,
    pub amount: Amount,
}

impl PasteAsk {
    pub fn title(&self) -> String {
        match self.amount.lines {
            1 => "Paste 1 line?".into(),
            lines => format!("Paste {lines} lines?"),
        }
    }

    /// The user's ↵ pastes; any other key of theirs cancels. A client's key answers nothing: it
    /// would paste the user's clipboard.
    pub fn answer(key: &str, mods: Mods, by_user: bool) -> Option<Action> {
        match (key, mods == Mods::default()) {
            _ if !by_user => None,
            ("enter", true) => Some(Action::QuitConfirm),
            _ => Some(Action::QuitCancel),
        }
    }
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
    fn a_paste_with_a_line_break_asks_unless_told_never() {
        assert!(asks(Amount::of("a\nb"), PasteConfirm::Multiline));
        assert!(asks(Amount::of("rm -rf build\n"), PasteConfirm::Multiline), "a last newline runs it");
        assert!(!asks(Amount::of("one line"), PasteConfirm::Multiline));
        assert!(!asks(Amount::of("a\nb"), PasteConfirm::Never));
    }

    #[test]
    fn only_the_users_enter_pastes() {
        let (plain, cmd) = (Mods::default(), Mods { cmd: true, ..Mods::default() });
        assert_eq!(PasteAsk::answer("enter", plain, true), Some(Action::QuitConfirm));
        assert_eq!(PasteAsk::answer("escape", plain, true), Some(Action::QuitCancel));
        assert_eq!(PasteAsk::answer("q", plain, true), Some(Action::QuitCancel), "not the quit question's q");
        assert_eq!(PasteAsk::answer("enter", cmd, true), Some(Action::QuitCancel));
        assert_eq!(PasteAsk::answer("enter", plain, false), None);
        assert_eq!(PasteAsk::answer("escape", plain, false), None);
        let ask = |lines| PasteAsk { pane: 1, amount: Amount { lines, chars: 9, newline: true } };
        assert_eq!((ask(1).title(), ask(4).title()), ("Paste 1 line?".into(), "Paste 4 lines?".into()));
    }
}
