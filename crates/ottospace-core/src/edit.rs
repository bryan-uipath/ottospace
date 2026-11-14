//! A single-line text field: text, a cursor, a selection, undo. The shell draws it and owns the
//! clipboard; every change is an [`Edit`]. Offsets are bytes, always on a grapheme boundary, so
//! an emoji or a letter with its accent moves and deletes as one.

use std::ops::Range;

use serde::{Serialize, Serializer};
use unicode_segmentation::UnicodeSegmentation;

use crate::keys::Mods;

/// Undo steps kept. Older ones are dropped.
const HISTORY: usize = 100;

/// Where a move or a delete goes, from the cursor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    Start,
    End,
}

/// One change to a field.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// Typed text replaces the selection. Control characters are dropped.
    Insert(String),
    /// As `Insert`, with each line break or tab made a space.
    Paste(String),
    /// Removes the selection, or with none what lies between the cursor and `to`.
    Delete {
        to: Motion,
    },
    /// Removes the selection (⌘X; the shell copied it first). Nothing without one.
    Cut,
    /// Moves the cursor. `select`: the selection's other end stays (Shift).
    Move {
        to: Motion,
        select: bool,
    },
    SelectAll,
    /// A mouse press at byte `at`. `count` 2 selects the word there, 3 everything. `shift`
    /// extends the selection to `at` instead.
    Click {
        at: usize,
        count: usize,
        shift: bool,
    },
    /// The pressed mouse moved to byte `to`: by characters, or by words after a double click.
    Drag {
        to: usize,
    },
    Undo,
    Redo,
    /// The input method replaces `range` (UTF-16; default: its marked text, else the selection).
    Replace {
        range: Option<Range<usize>>,
        text: String,
    },
    /// As `Replace`, and the new text stays marked: a composition in progress (a dead key,
    /// an IME). `selected` is the selection inside the new text, in UTF-16.
    Mark {
        range: Option<Range<usize>>,
        text: String,
        selected: Option<Range<usize>>,
    },
    /// Ends the composition and keeps its text.
    Unmark,
}

/// What a key does in a field.
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    Edit(Edit),
    /// The shell's part: the clipboard. `Copy` reads `selected()`, `Cut` then sends
    /// `Edit::Cut`, `Paste` sends `Edit::Paste`.
    Copy,
    Cut,
    Paste,
}

/// The editing key for a press, by macOS convention: ⌥ is a word, ⌘ the line, Shift selects,
/// and the Emacs keys (⌃A ⌃E ⌃B ⌃F ⌃D ⌃H ⌃K ⌃U ⌃W). `None`: the key types text, or is not
/// the field's. `key` is GPUI's name for it (`"left"`, `"backspace"`, `"a"`).
pub fn key(key: &str, mods: Mods) -> Option<Key> {
    use Motion::*;
    let go = |to| Some(Key::Edit(Edit::Move { to, select: mods.shift }));
    let delete = |to| Some(Key::Edit(Edit::Delete { to }));
    match key {
        "left" if mods.cmd => go(Start),
        "right" if mods.cmd => go(End),
        "left" if mods.alt => go(WordLeft),
        "right" if mods.alt => go(WordRight),
        "left" => go(Left),
        "right" => go(Right),
        "home" | "up" | "pageup" => go(Start),
        "end" | "down" | "pagedown" => go(End),
        "backspace" if mods.cmd => delete(Start),
        "backspace" if mods.alt || mods.ctrl => delete(WordLeft),
        "backspace" => delete(Left),
        "delete" if mods.cmd => delete(End),
        "delete" if mods.alt || mods.ctrl => delete(WordRight),
        "delete" => delete(Right),
        _ if mods.cmd && !mods.ctrl && !mods.alt => match key {
            "a" if !mods.shift => Some(Key::Edit(Edit::SelectAll)),
            "z" if mods.shift => Some(Key::Edit(Edit::Redo)),
            "z" => Some(Key::Edit(Edit::Undo)),
            "c" if !mods.shift => Some(Key::Copy),
            "x" if !mods.shift => Some(Key::Cut),
            "v" if !mods.shift => Some(Key::Paste),
            _ => None,
        },
        _ if mods.ctrl && !mods.cmd && !mods.alt => match key {
            "a" => go(Start),
            "e" => go(End),
            "b" => go(Left),
            "f" => go(Right),
            "d" => delete(Right),
            "h" => delete(Left),
            "k" => delete(End),
            "u" => delete(Start),
            "w" => delete(WordLeft),
            _ => None,
        },
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Snap {
    text: String,
    anchor: usize,
    head: usize,
}

/// The field. In JSON it is its text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Field {
    text: String,
    /// The selection's fixed end, and the cursor. Equal: no selection.
    anchor: usize,
    head: usize,
    /// The composition in progress: drawn underlined, replaced as a whole.
    marked: Option<Range<usize>>,
    /// The most characters it holds. 0: no limit.
    limit: usize,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    /// The last edit typed text: more typing joins its undo step.
    typing: bool,
    /// What the last click selected, and its count: a drag extends from it by that unit.
    /// `None` once the text has changed: a drag then goes on from the selection's fixed end.
    pressed: Option<(Range<usize>, usize)>,
    /// It holds several lines (a conversation's composer): `\n` is kept, typed or pasted.
    lines: bool,
}

impl Serialize for Field {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text)
    }
}

impl Field {
    /// A field holding `text` (cut to `limit` characters; 0: no limit), the cursor at its end.
    pub fn new(text: &str, limit: usize) -> Self {
        let mut field = Self { limit, ..Self::default() };
        field.text = field.fitting(&clean(text, false), 0);
        field.anchor = field.text.len();
        field.head = field.anchor;
        field
    }

    /// An empty field of several lines: a line break typed or pasted is kept.
    pub fn lines(limit: usize) -> Self {
        Self { limit, lines: true, ..Self::default() }
    }

    /// `text` as this field takes it: control characters out, line breaks kept if it holds lines.
    fn cleaned(&self, text: &str, paste: bool) -> String {
        if !self.lines {
            return clean(text, paste);
        }
        let text = text.replace("\r\n", "\n").replace('\r', "\n").replace('\t', "    ");
        text.split('\n').map(|line| clean(line, false)).collect::<Vec<_>>().join("\n")
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The text as the `String` it is kept in, to compare with another.
    pub fn line(&self) -> &String {
        &self.text
    }

    /// The cursor, in bytes.
    pub fn cursor(&self) -> usize {
        self.head
    }

    /// The selection, in bytes, in text order. Empty: only a cursor.
    pub fn selection(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    /// What ⌘C copies. Empty with no selection.
    pub fn selected(&self) -> &str {
        &self.text[self.selection()]
    }

    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// A byte offset as UTF-16 code units: what the input method and JavaScript count in.
    pub fn utf16(&self, byte: usize) -> usize {
        self.text[..self.snap(byte)].encode_utf16().count()
    }

    /// The byte offset of a UTF-16 offset. One inside a character or a cluster moves to its start.
    pub fn byte(&self, utf16: usize) -> usize {
        let mut units = 0;
        let at = self.text.char_indices().find(|(_, c)| {
            units += c.len_utf16();
            units > utf16
        });
        self.snap(at.map_or(self.text.len(), |(i, _)| i))
    }

    /// The nearest grapheme boundary at or before `byte`.
    fn snap(&self, byte: usize) -> usize {
        if byte >= self.text.len() {
            return self.text.len();
        }
        self.text.grapheme_indices(true).map(|(i, _)| i).take_while(|i| *i <= byte).last().unwrap_or(0)
    }

    /// The nearest grapheme boundary at or after `byte`.
    fn snap_up(&self, byte: usize) -> usize {
        let before = self.snap(byte);
        if before >= byte.min(self.text.len()) {
            return before;
        }
        self.text[before..].graphemes(true).next().map_or(self.text.len(), |g| before + g.len())
    }

    /// As much of `text` as fits beside `kept` characters that stay: whole graphemes only.
    fn fitting(&self, text: &str, kept: usize) -> String {
        if self.limit == 0 {
            return text.to_string();
        }
        let mut room = self.limit.saturating_sub(kept);
        let fits = |g: &&str| {
            let n = g.chars().count();
            let ok = n <= room;
            room = if ok { room - n } else { 0 };
            ok
        };
        text.graphemes(true).take_while(fits).collect()
    }

    /// The word segments (UAX #29) as (range, is a word). Spaces and punctuation are segments
    /// that are not words. Each CJK ideograph is its own word: there is no dictionary here.
    fn segments(&self) -> impl DoubleEndedIterator<Item = (Range<usize>, bool)> + '_ {
        self.text.split_word_bound_indices().map(|(i, s)| (i..i + s.len(), s.chars().any(char::is_alphanumeric)))
    }

    /// The segment a click at `at` is on: the one that starts at or before it.
    fn word_at(&self, at: usize) -> Range<usize> {
        let at = at.min(self.text.len());
        let hit = self.segments().map(|(r, _)| r).find(|r| r.start <= at && at < r.end);
        hit.or_else(|| self.segments().next_back().map(|(r, _)| r)).unwrap_or(0..0)
    }

    fn target(&self, to: Motion) -> usize {
        let at = self.head;
        match to {
            Motion::Left => self.text[..at].grapheme_indices(true).next_back().map_or(0, |(i, _)| i),
            Motion::Right => self.text[at..].graphemes(true).next().map_or(at, |g| at + g.len()),
            // The start of the word before the cursor, over any spaces and punctuation between.
            Motion::WordLeft => {
                self.segments().rev().find(|(r, word)| *word && r.start < at).map_or(0, |(r, _)| r.start)
            }
            Motion::WordRight => {
                self.segments().find(|(r, word)| *word && r.end > at).map_or(self.text.len(), |(r, _)| r.end)
            }
            Motion::Start => 0,
            Motion::End => self.text.len(),
        }
    }

    /// Before a change to the text: keeps what to go back to. Typing joins the step before it,
    /// until a new word starts, so ⌘Z takes back a word, not a letter.
    fn record(&mut self, typing: bool) {
        if !(typing && self.typing) {
            self.undo.push(Snap { text: self.text.clone(), anchor: self.anchor, head: self.head });
            if self.undo.len() > HISTORY {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.typing = typing;
    }

    /// True if `text` would go in whole in place of bytes `range`: the limit leaves room.
    /// `splice` cuts what does not fit; a caller that must not write half a name asks first.
    pub fn fits(&self, range: Range<usize>, text: &str) -> bool {
        let gone = self.text.get(range).map_or(0, |part| part.chars().count());
        self.limit == 0 || self.text.chars().count() - gone + text.chars().count() <= self.limit
    }

    /// Replaces bytes `range` with `text` as one undo step, the cursor after it. For a caller
    /// that completes a word; `range` must lie on character boundaries.
    pub fn splice(&mut self, range: Range<usize>, text: &str) {
        self.record(false);
        self.put(range, &self.cleaned(text, false));
    }

    /// Replaces `range` with as much of `text` as fits; the cursor goes after it.
    fn put(&mut self, range: Range<usize>, text: &str) -> Range<usize> {
        let kept = self.text.chars().count() - self.text[range.clone()].chars().count();
        let text = self.fitting(text, kept);
        self.text.replace_range(range.clone(), &text);
        let end = range.start + text.len();
        // What went in may have joined what follows (a lone joiner, an accent): the cursor
        // goes after the whole cluster.
        self.head = self.snap_up(end);
        self.anchor = self.head;
        (self.marked, self.pressed) = (None, None);
        range.start..end
    }

    /// Applies one edit. True if the text changed (not only the cursor or the selection).
    pub fn apply(&mut self, edit: Edit) -> bool {
        let before = self.text.clone();
        match edit {
            Edit::Insert(text) => self.insert(&self.cleaned(&text, false), true),
            Edit::Paste(text) => self.insert(&self.cleaned(&text, true), false),
            Edit::Delete { to } => {
                let range = match self.selection() {
                    selection if !selection.is_empty() => selection,
                    _ => self.head.min(self.target(to))..self.head.max(self.target(to)),
                };
                self.remove(range);
            }
            Edit::Cut => self.remove(self.selection()),
            Edit::Move { to, select } => {
                let selection = self.selection();
                self.head = match to {
                    // Without Shift an arrow first collapses the selection to that side.
                    Motion::Left if !select && !selection.is_empty() => selection.start,
                    Motion::Right if !select && !selection.is_empty() => selection.end,
                    _ => self.target(to),
                };
                if !select {
                    self.anchor = self.head;
                }
                self.typing = false;
            }
            Edit::SelectAll => {
                (self.anchor, self.head) = (0, self.text.len());
                self.typing = false;
            }
            // A drag after it extends from the selection's fixed end, by characters.
            Edit::Click { at, shift: true, .. } => {
                self.head = self.snap(at);
                self.pressed = Some((self.anchor..self.anchor, 1));
                self.typing = false;
            }
            Edit::Click { at, count, .. } => {
                let range = self.unit(self.snap(at), count);
                (self.anchor, self.head) = (range.start, range.end);
                self.pressed = Some((range, count));
                self.typing = false;
            }
            Edit::Drag { to } => {
                let (from, count) = self.pressed.clone().unwrap_or((self.anchor..self.anchor, 1));
                // The press is never trusted past the text as it is now.
                let from = self.snap(from.start)..self.snap(from.end).max(self.snap(from.start));
                let to = self.unit(self.snap(to), count);
                // Leftward the pressed unit stays selected and the cursor leads.
                (self.anchor, self.head) =
                    if to.start < from.start { (from.end, to.start) } else { (from.start, to.end.max(from.end)) };
            }
            Edit::Undo => self.step(true),
            Edit::Redo => self.step(false),
            // Plain typing arrives this way too: with nothing named or marked it is an insert.
            Edit::Replace { range: None, text } if self.marked.is_none() => {
                self.insert(&self.cleaned(&text, false), true)
            }
            Edit::Replace { range, text } => {
                let range = self.ime_range(range);
                // A composition recorded its step when it began.
                if self.marked.is_none() {
                    self.record(range.is_empty());
                }
                self.put(range, &self.cleaned(&text, false));
                self.forget_no_change();
            }
            Edit::Mark { range, text, selected } => {
                let range = self.ime_range(range);
                if self.marked.is_none() {
                    self.record(false);
                }
                let marked = self.put(range, &self.cleaned(&text, false));
                if let Some(selected) = selected {
                    let inside = &self.text[marked.clone()];
                    let at = |utf16: usize| {
                        let mut units = 0;
                        let hit = inside.char_indices().find(|(_, c)| {
                            units += c.len_utf16();
                            units > utf16
                        });
                        marked.start + hit.map_or(inside.len(), |(i, _)| i)
                    };
                    let (from, to) = (self.snap_up(at(selected.start)), self.snap_up(at(selected.end)));
                    (self.anchor, self.head) = (from.min(to), to.max(from));
                }
                self.marked = Some(marked).filter(|m| !m.is_empty());
                self.forget_no_change();
            }
            Edit::Unmark => self.marked = None,
        }
        self.text != before
    }

    /// A composition that ended where it began (cancelled) is no step to undo.
    fn forget_no_change(&mut self) {
        if self.marked.is_none() && self.undo.last().is_some_and(|snap| snap.text == self.text) {
            self.undo.pop();
        }
    }

    /// What a click of `count` at `at` selects.
    fn unit(&self, at: usize, count: usize) -> Range<usize> {
        match count {
            0 | 1 => at..at,
            2 => self.word_at(at),
            _ => 0..self.text.len(),
        }
    }

    fn ime_range(&self, range: Option<Range<usize>>) -> Range<usize> {
        let range = range.map(|r| self.byte(r.start)..self.byte(r.end).max(self.byte(r.start)));
        range.or(self.marked.clone()).unwrap_or(self.selection())
    }

    fn insert(&mut self, text: &str, typing: bool) {
        let selection = self.selection();
        if text.is_empty() && selection.is_empty() {
            return;
        }
        // A new word (a letter after a space) starts a new undo step.
        let before = self.text[..selection.start].chars().next_back();
        let new_word = before.is_none_or(char::is_whitespace) && !text.starts_with(char::is_whitespace);
        if new_word {
            self.typing = false;
        }
        self.record(typing && selection.is_empty());
        self.put(selection, text);
    }

    fn remove(&mut self, range: Range<usize>) {
        if !range.is_empty() {
            self.record(false);
            self.put(range, "");
        }
    }

    /// One step back (`undo`) or forward again.
    fn step(&mut self, undo: bool) {
        let (from, to) = if undo { (&mut self.undo, &mut self.redo) } else { (&mut self.redo, &mut self.undo) };
        let Some(snap) = from.pop() else { return };
        to.push(Snap { text: std::mem::replace(&mut self.text, snap.text), anchor: self.anchor, head: self.head });
        (self.anchor, self.head) = (snap.anchor, snap.head);
        (self.marked, self.pressed) = (None, None);
        self.typing = false;
    }
}

/// `text` as one line may hold it: no control characters. `breaks`: each run of a line break
/// or a tab becomes one space first (a paste).
fn clean(text: &str, breaks: bool) -> String {
    let text = if breaks { text.replace("\r\n", " ").replace(['\n', '\r', '\t'], " ") } else { text.to_string() };
    text.chars().filter(|c| !c.is_control()).collect()
}

/// Random edits for the property tests, the same on every run.
#[cfg(test)]
pub(crate) mod fuzz {
    use super::Motion::*;
    use super::*;

    /// xorshift64: the same edits on every run.
    pub(crate) struct Rng(pub u64);

    impl Rng {
        pub fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n.max(1) as u64) as usize
        }

        pub fn text(&mut self) -> String {
            const PIECES: &[&str] = &[
                "a",
                " ",
                "fix",
                "#tag",
                "-",
                "e\u{301}",
                "\u{301}",
                "👨‍👩‍👧",
                "\u{200d}",
                "👍🏽",
                "\u{1f3fd}",
                "🇯🇵",
                "\u{1f1ef}",
                "שלום",
                "مرحبا",
                "𝒳",
                "日本",
                "한",
                "\n",
                "\r\n",
                "\t",
                "\u{7}",
            ];
            (0..self.below(4)).map(|_| PIECES[self.below(PIECES.len())]).collect()
        }

        pub fn range(&mut self, len: usize) -> Option<Range<usize>> {
            let (a, b) = (self.below(len + 3), self.below(len + 3));
            (self.below(3) > 0).then_some(a.min(b)..a.max(b))
        }

        pub fn edit(&mut self, len: usize) -> Edit {
            let to = [Left, Right, WordLeft, WordRight, Start, End][self.below(6)];
            // Bytes and UTF-16 offsets from anywhere, inside a character and past the end too.
            let at = self.below(len + 3);
            match self.below(16) {
                0 | 1 => Edit::Insert(self.text()),
                2 => Edit::Paste(self.text()),
                3 => Edit::Delete { to },
                4 => Edit::Cut,
                5 | 6 => Edit::Move { to, select: self.below(2) == 0 },
                7 => Edit::SelectAll,
                8 => Edit::Click { at, count: 1 + self.below(3), shift: false },
                9 => Edit::Click { at, count: 1, shift: true },
                10 | 11 => Edit::Drag { to: at },
                12 => [Edit::Undo, Edit::Redo, Edit::Unmark][self.below(3)].clone(),
                13 => Edit::Replace { range: self.range(len), text: self.text() },
                _ => Edit::Mark { range: self.range(len), text: self.text(), selected: self.range(4) },
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::Motion::*;
    use super::fuzz::Rng;
    use super::*;

    /// `"fix [zoom]| now"`: `[` the anchor, `|` the cursor. No `[`: no selection.
    fn field(marked: &str) -> Field {
        let text: String = marked.chars().filter(|c| !matches!(c, '[' | ']' | '|')).collect();
        let at = |mark: char| {
            let before = marked.split(mark).next().filter(|_| marked.contains(mark))?;
            Some(before.chars().filter(|c| !matches!(c, '[' | ']' | '|')).map(char::len_utf8).sum::<usize>())
        };
        let mut field = Field::new(&text, 0);
        field.head = at('|').unwrap_or(text.len());
        field.anchor = match (at('['), at(']')) {
            (Some(open), Some(close)) => {
                if open == field.head {
                    close
                } else {
                    open
                }
            }
            _ => field.head,
        };
        field
    }

    fn show(field: &Field) -> String {
        let (selection, mut out) = (field.selection(), String::new());
        for (i, c) in field.text.char_indices().chain([(field.text.len(), '\0')]) {
            if i == selection.start && !selection.is_empty() {
                out.push('[');
            }
            if i == field.head && i == selection.start {
                out.push('|');
            }
            if i == selection.end && !selection.is_empty() {
                out.push(']');
            }
            if i == field.head && i != selection.start {
                out.push('|');
            }
            if c != '\0' {
                out.push(c);
            }
        }
        out
    }

    fn after(start: &str, edits: Vec<Edit>) -> String {
        let mut field = field(start);
        for edit in edits {
            field.apply(edit);
        }
        show(&field)
    }

    fn go(to: Motion) -> Edit {
        Edit::Move { to, select: false }
    }

    fn sel(to: Motion) -> Edit {
        Edit::Move { to, select: true }
    }

    fn ins(text: &str) -> Edit {
        Edit::Insert(text.into())
    }

    fn shift_click(at: usize) -> Edit {
        Edit::Click { at, count: 1, shift: true }
    }

    #[test]
    fn the_cursor_moves_by_character_word_and_line() {
        let cases = [
            ("fix zoom|", vec![go(Left)], "fix zoo|m"),
            ("|fix", vec![go(Left)], "|fix"),
            ("fix|", vec![go(Right)], "fix|"),
            ("fix zoom| now", vec![go(WordLeft)], "fix |zoom now"),
            ("fix |zoom now", vec![go(WordLeft)], "|fix zoom now"),
            ("fix zo|om, now", vec![go(WordRight)], "fix zoom|, now"),
            ("fix zoom|, now", vec![go(WordRight)], "fix zoom, now|"),
            ("fix #can|vas", vec![go(WordLeft)], "fix #|canvas"),
            ("fix zo|om", vec![go(Start)], "|fix zoom"),
            ("fix zo|om", vec![go(End)], "fix zoom|"),
            // An arrow leaves a selection at that side of it.
            ("a [bcd]| e", vec![go(Left)], "a |bcd e"),
            ("a [|bcd] e", vec![go(Right)], "a bcd| e"),
            // One step is one grapheme: an emoji family, a flag, a letter and its accent.
            ("a👨‍👩‍👧|", vec![go(Left)], "a|👨‍👩‍👧"),
            ("|🇯🇵x", vec![go(Right)], "🇯🇵|x"),
            ("e\u{301}|", vec![go(Left)], "|e\u{301}"),
            ("👍🏽|", vec![go(Left)], "|👍🏽"),
            // CJK: a character a step; an ideograph is a word of its own.
            ("日本語|", vec![go(Left)], "日本|語"),
            ("日本語|", vec![go(WordLeft)], "日本|語"),
            ("한국어 텍스트|", vec![go(WordLeft)], "한국어 |텍스트"),
        ];
        for (start, edits, want) in cases {
            assert_eq!(after(start, edits.clone()), want, "{start} {edits:?}");
        }
    }

    #[test]
    fn shift_extends_the_selection_and_typing_replaces_it() {
        let cases = [
            ("fix zoom|", vec![sel(Left), sel(Left)], "fix zo[|om]"),
            ("fix zoom|", vec![sel(WordLeft)], "fix [|zoom]"),
            ("fix |zoom", vec![sel(End)], "fix [zoom]|"),
            ("fix |zoom", vec![sel(Start)], "[|fix ]zoom"),
            // Back over the anchor: the selection turns around.
            ("fix| zoom", vec![sel(Right), sel(Left), sel(Left)], "fi[|x] zoom"),
            ("fix [zoom]|", vec![ins("pan")], "fix pan|"),
            ("fix zoom|", vec![Edit::SelectAll], "[fix zoom]|"),
            ("fix zoom|", vec![Edit::SelectAll, ins("x")], "x|"),
            ("a [bcd]| e", vec![Edit::Cut], "a | e"),
            ("a bcd| e", vec![Edit::Cut], "a bcd| e"),
            ("a👨‍👩‍👧|", vec![sel(Left), ins("b")], "ab|"),
        ];
        for (start, edits, want) in cases {
            assert_eq!(after(start, edits.clone()), want, "{start} {edits:?}");
        }
        assert_eq!(field("a [bcd]| e").selected(), "bcd");
        assert_eq!(field("a bcd| e").selected(), "");
    }

    #[test]
    fn deleting_takes_a_character_a_word_or_the_line() {
        let del = |to| Edit::Delete { to };
        let cases = [
            ("fix|", vec![del(Left)], "fi|"),
            ("|fix", vec![del(Left)], "|fix"),
            ("|fix", vec![del(Right)], "|ix"),
            ("fix zoom| now", vec![del(WordLeft)], "fix | now"),
            ("fix zoom |now", vec![del(WordLeft)], "fix |now"),
            ("fix| zoom now", vec![del(WordRight)], "fix| now"),
            ("fix zoom| now", vec![del(Start)], "| now"),
            ("fix zoom| now", vec![del(End)], "fix zoom|"),
            // A selection goes whole, whatever the key.
            ("f[ix zo]|om", vec![del(WordLeft)], "f|om"),
            ("a👨‍👩‍👧|", vec![del(Left)], "a|"),
            ("e\u{301}|x", vec![del(Left)], "|x"),
            ("|🇯🇵x", vec![del(Right)], "|x"),
            ("日本語|", vec![del(Left)], "日本|"),
        ];
        for (start, edits, want) in cases {
            assert_eq!(after(start, edits.clone()), want, "{start} {edits:?}");
        }
    }

    #[test]
    fn the_mouse_places_the_cursor_and_selects_words() {
        let click = |at, count| Edit::Click { at, count, shift: false };
        let cases = [
            ("fix zoom now|", vec![click(5, 1)], "fix z|oom now"),
            ("fix zoom now|", vec![click(5, 2)], "fix [zoom]| now"),
            ("fix zoom now|", vec![click(3, 2)], "fix[ ]|zoom now"),
            ("fix zoom now|", vec![click(12, 2)], "fix zoom [now]|"),
            ("fix zoom now|", vec![click(5, 3)], "[fix zoom now]|"),
            ("fix zoom now|", vec![click(5, 1), Edit::Drag { to: 10 }], "fix z[oom n]|ow"),
            ("fix zoom now|", vec![click(5, 1), Edit::Drag { to: 1 }], "f[|ix z]oom now"),
            // After a double click a drag takes whole words, either way.
            ("fix zoom now|", vec![click(5, 2), Edit::Drag { to: 5 }], "fix [zoom]| now"),
            ("fix zoom now|", vec![click(5, 2), Edit::Drag { to: 10 }], "fix [zoom now]|"),
            ("fix zoom now|", vec![click(5, 2), Edit::Drag { to: 1 }], "[|fix zoom] now"),
            ("fi|x zoom now", vec![Edit::Click { at: 8, count: 1, shift: true }], "fi[x zoom]| now"),
            // A press inside a cluster lands on its edge; one past the end, at the end.
            ("a👨‍👩‍👧b|", vec![click(6, 1)], "a|👨‍👩‍👧b"),
            ("fix|", vec![click(99, 1)], "fix|"),
            ("|", vec![click(0, 2)], "|"),
            // A drag after a shift-click goes on from the selection's fixed end, not from an
            // older press.
            ("fix zoom now|", vec![click(3, 1), shift_click(8), Edit::Drag { to: 9 }], "fix[ zoom ]|now"),
            ("fix zoom now|", vec![click(12, 2), go(Start), shift_click(3), Edit::Drag { to: 5 }], "[fix z]|oom now"),
            ("fix zoom now|", vec![click(5, 2), shift_click(1), Edit::Drag { to: 0 }], "[|fix ]zoom now"),
        ];
        for (start, edits, want) in cases {
            assert_eq!(after(start, edits.clone()), want, "{start} {edits:?}");
        }
    }

    #[test]
    fn a_drag_survives_text_that_changed_under_the_press() {
        // The press was at the end of a longer line; ⌘A and a letter left one character.
        let edits = vec![
            Edit::Click { at: 16, count: 1, shift: false },
            Edit::SelectAll,
            ins("x"),
            shift_click(1),
            Edit::Drag { to: 1 },
        ];
        assert_eq!(after("fix zoom flicker|", edits), "x|");
        // The text shrinks while the button is held: a key, ⌘Z. The drag goes on from the cursor.
        let click = Edit::Click { at: 12, count: 2, shift: false };
        assert_eq!(
            after("fix zoom now|", vec![click.clone(), Edit::Delete { to: Left }, Edit::Drag { to: 2 }]),
            "fi[|x zoom ]"
        );
        assert_eq!(after("fix|", vec![ins(" zoom"), click, Edit::Undo, Edit::Drag { to: 1 }]), "f[|ix]");
    }

    #[test]
    fn what_goes_in_never_leaves_the_cursor_inside_a_cluster() {
        // A lone accent joins the letter before it; a lone joiner or skin tone, the emoji.
        assert_eq!(after("e|x", vec![Edit::Paste("\u{301}".into())]), "e\u{301}|x");
        assert_eq!(after("|\u{301}x", vec![ins("e")]), "e\u{301}|x");
        assert_eq!(after("👍|", vec![Edit::Paste("\u{1f3fd}".into()), Edit::Delete { to: Left }]), "|");
        assert_eq!(after("👨|👩", vec![ins("\u{200d}")]), "👨\u{200d}👩|");
        // Taking a letter away joins its accent to the one before.
        assert_eq!(after("ax|\u{301}", vec![Edit::Delete { to: Left }]), "a\u{301}|");
        let mut field = field("e|");
        field.apply(Edit::Mark { range: None, text: "\u{301}a".into(), selected: Some(0..0) });
        assert_eq!(show(&field), "e\u{301}|a");
    }

    #[test]
    fn any_edits_in_any_order_leave_a_sound_field() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for round in 0..600 {
            let limit = [0, 5, 12, 500][round % 4];
            let mut field = Field::new(&rng.text(), limit);
            let mut done: Vec<Edit> = Vec::new();
            for _ in 0..80 {
                let edit = rng.edit(field.text.len());
                done.push(edit.clone());
                field.apply(edit);
                let text = field.text.clone();
                let clusters: Vec<usize> = text.grapheme_indices(true).map(|(i, _)| i).chain([text.len()]).collect();
                let why = format!("limit {limit}, {text:?} after {done:?}");
                // The cursor and the selection's other end sit between clusters; `selection`,
                // `selected` and the UTF-16 offsets the shell asks for do not panic.
                assert!(clusters.contains(&field.head) && clusters.contains(&field.anchor), "{why}");
                let selection = field.selection();
                assert!(selection.start <= selection.end && selection.end <= text.len(), "{why}");
                assert_eq!(field.selected(), &text[selection.clone()], "{why}");
                assert_eq!(field.byte(field.utf16(selection.end)), selection.end, "{why}");
                if let Some(marked) = field.marked() {
                    assert!(marked.start < marked.end && text.get(marked).is_some(), "{why}");
                }
                assert!(limit == 0 || text.chars().count() <= limit, "{why}");
                assert!(!text.chars().any(char::is_control), "{why}");
                // Undo, then redo: exactly where it was.
                if !field.undo.is_empty() {
                    let mut again = field.clone();
                    again.apply(Edit::Undo);
                    again.apply(Edit::Redo);
                    assert_eq!((&again.text, again.anchor, again.head), (&text, field.anchor, field.head), "{why}");
                }
            }
        }
    }

    #[test]
    fn undo_takes_back_a_word_and_redo_brings_it_back() {
        let typed = |text: &str| text.chars().map(|c| ins(&c.to_string())).collect::<Vec<_>>();
        let then = |mut edits: Vec<Edit>, more: Vec<Edit>| {
            edits.extend(more);
            edits
        };
        assert_eq!(after("|", then(typed("fix zoom"), vec![Edit::Undo])), "fix |");
        assert_eq!(after("|", then(typed("fix zoom"), vec![Edit::Undo, Edit::Undo])), "|");
        assert_eq!(after("|", then(typed("fix zoom"), vec![Edit::Undo, Edit::Undo, Edit::Undo])), "|");
        assert_eq!(after("|", then(typed("fix zoom"), vec![Edit::Undo, Edit::Undo, Edit::Redo])), "fix |");
        assert_eq!(after("|", then(typed("fix zoom"), vec![Edit::Undo, Edit::Redo, Edit::Redo])), "fix zoom|");
        // A move ends the step; a new edit drops what could be redone; the selection comes back.
        assert_eq!(after("|", then(typed("ab"), vec![go(Left), ins("c"), Edit::Undo])), "a|b");
        assert_eq!(after("|", then(typed("ab"), vec![Edit::Undo, ins("c"), Edit::Redo])), "c|");
        let delete = Edit::Delete { to: Left };
        assert_eq!(after("fix [zoom]|", vec![delete.clone(), Edit::Undo]), "fix [zoom]|");
        assert_eq!(after("fix [zoom]|", vec![delete, Edit::Undo, Edit::Redo]), "fix |");
        assert_eq!(after("ab|", vec![Edit::Paste("c\nd".into()), Edit::Undo]), "ab|");
        // A delete that removes nothing is not a step.
        assert_eq!(
            after("|ab", vec![ins("x"), Edit::Delete { to: Start }, Edit::Delete { to: Left }, Edit::Undo]),
            "x|ab"
        );
        // The history is bounded: the oldest steps go.
        let mut field = Field::default();
        for n in 0..HISTORY + 20 {
            field.apply(ins(&format!("w{n} ")));
        }
        for _ in 0..HISTORY + 20 {
            field.apply(Edit::Undo);
        }
        assert_eq!(field.text.split_whitespace().count(), 20);
    }

    #[test]
    fn text_is_one_line_within_the_limit() {
        assert_eq!(after("a|", vec![Edit::Paste("b\r\nc\td\n".into())]), "ab c d |");
        assert_eq!(after("a|", vec![ins("b\u{7}\n\u{1b}c")]), "abc|");
        assert_eq!(Field::new("a\nb", 0).text(), "ab");
        // The limit counts characters and never splits a cluster.
        let mut field = Field::new("abcdef", 4);
        assert_eq!(field.text(), "abcd");
        assert!(!field.apply(ins("x")));
        field.apply(Edit::Delete { to: Left });
        assert!(!field.apply(ins("👨‍👩‍👧")), "five characters do not fit in one");
        assert!(field.apply(ins("語y")));
        assert_eq!(field.text(), "abc語");
        field.apply(Edit::SelectAll);
        field.apply(Edit::Paste("123456".into()));
        assert_eq!(field.text(), "1234");
        assert_eq!(serde_json::json!(field), serde_json::json!("1234"));
    }

    #[test]
    fn an_input_method_composes_in_place() {
        // A dead key: ´ is marked, then e replaces it with é.
        let mut field = field("caf|");
        field.apply(Edit::Mark { range: None, text: "´".into(), selected: Some(1..1) });
        assert_eq!((field.text(), field.marked(), field.cursor()), ("caf´", Some(3..5), 5));
        field.apply(Edit::Replace { range: None, text: "é".into() });
        assert_eq!((show(&field).as_str(), field.marked()), ("café|", None));
        field.apply(Edit::Undo);
        assert_eq!(show(&field), "caf|");
        // An IME: the marked text grows, the selection sits inside it, then it commits.
        let mut field = Field::new("a😀", 0);
        field.apply(Edit::Mark { range: None, text: "ni".into(), selected: Some(2..2) });
        field.apply(Edit::Mark { range: None, text: "nihon".into(), selected: Some(1..3) });
        assert_eq!((field.text(), field.marked(), field.selection()), ("a😀nihon", Some(5..10), 6..8));
        field.apply(Edit::Unmark);
        assert_eq!((field.text(), field.marked()), ("a😀nihon", None));
        // Ranges from the platform are UTF-16: 😀 is two units.
        field.apply(Edit::Replace { range: Some(3..8), text: "日本".into() });
        assert_eq!(show(&field), "a😀日本|");
        assert_eq!((field.utf16(5), field.byte(3), field.byte(2), field.byte(99)), (3, 5, 1, 11));
        // An empty marked text ends the composition.
        field.apply(Edit::Mark { range: None, text: "x".into(), selected: None });
        field.apply(Edit::Mark { range: None, text: String::new(), selected: None });
        assert_eq!((show(&field).as_str(), field.marked()), ("a😀日本|", None));
        // A composition that is cancelled leaves no step: ⌘Z takes back what came before it.
        let mut field = Field::default();
        field.apply(ins("ab"));
        field.apply(Edit::Mark { range: None, text: "´".into(), selected: None });
        field.apply(Edit::Mark { range: None, text: String::new(), selected: None });
        field.apply(Edit::Undo);
        assert_eq!(show(&field), "|");
    }

    #[test]
    fn keys_follow_the_macos_conventions() {
        let m = |spec: &str| Mods {
            ctrl: spec.contains('^'),
            alt: spec.contains('~'),
            shift: spec.contains('$'),
            cmd: spec.contains('@'),
        };
        let edit = |edit| Some(Key::Edit(edit));
        let del = |to| edit(Edit::Delete { to });
        let cases = [
            ("left", "", edit(go(Left))),
            ("left", "$", edit(sel(Left))),
            ("left", "~", edit(go(WordLeft))),
            ("right", "~$", edit(sel(WordRight))),
            ("left", "@", edit(go(Start))),
            ("right", "@$", edit(sel(End))),
            ("home", "", edit(go(Start))),
            ("end", "$", edit(sel(End))),
            ("a", "^", edit(go(Start))),
            ("e", "^", edit(go(End))),
            ("b", "^", edit(go(Left))),
            ("f", "^", edit(go(Right))),
            ("backspace", "", del(Left)),
            ("backspace", "~", del(WordLeft)),
            ("backspace", "@", del(Start)),
            ("delete", "", del(Right)),
            ("delete", "~", del(WordRight)),
            ("a", "@", edit(Edit::SelectAll)),
            ("z", "@", edit(Edit::Undo)),
            ("z", "@$", edit(Edit::Redo)),
            ("c", "@", Some(Key::Copy)),
            ("x", "@", Some(Key::Cut)),
            ("v", "@", Some(Key::Paste)),
            // Text, and other apps' shortcuts, are not the field's.
            ("a", "", None),
            ("a", "$", None),
            ("b", "~", None),
            ("n", "@", None),
            ("c", "@$", None),
            ("a", "@^", None),
            ("enter", "", None),
        ];
        for (name, mods, want) in cases {
            assert_eq!(key(name, m(mods)), want, "{mods}{name}");
        }
    }
}
