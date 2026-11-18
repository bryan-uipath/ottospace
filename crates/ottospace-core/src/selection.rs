//! A selection across a native transcript's drawn texts: a reply's paragraphs, a message, a
//! call's input and output, a note. Read-only, as a text field of several lines over the texts
//! joined by line breaks (`edit::Field`): a press, a double or triple click and a drag select as
//! there. Each text has a key (`in:12`) that stays while the transcript grows. View state: the
//! shell keeps it, and ⌘C copies `selected()`. The state shows only `extent()`.

use std::ops::Range;

use serde::Serialize;

use crate::edit::{Edit, Field};

/// The texts as last drawn, in order, and the selection over them.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    /// Each text's key and its range in `field`'s text.
    texts: Vec<(String, Range<usize>)>,
    field: Field,
    /// The last press that was no ⇧-click: its text's key, offset and click count. A drag after
    /// text changed goes on from it, by its unit (a word after a double click).
    pressed: Option<(String, usize, usize)>,
}

/// Where a point is, by the texts' rectangles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    /// Inside text `n`, or beside it on a line it has: its layout says where.
    In(usize),
    /// Before text `n` (in a gap above it): its start.
    Start(usize),
    /// After the last text: its end.
    End(usize),
}

/// How much is selected: the texts it touches, and its characters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Extent {
    pub rows: usize,
    pub chars: usize,
}

/// The text under `(x, y)`, among rectangles `[x, y, w, h]` in drawing order. Inside one, or
/// level with one (nearest by x), that one; in a gap, the start of the next; past all, the end
/// of the last. `None`: there are none.
pub fn hit(rects: &[[f32; 4]], (x, y): (f32, f32)) -> Option<Hit> {
    let level = rects.iter().enumerate().filter(|(_, r)| r[1] <= y && y < r[1] + r[3]);
    let dx = |r: &[f32; 4]| if x < r[0] { r[0] - x } else { (x - r[0] - r[2]).max(0.) };
    if let Some((n, _)) = level.min_by(|(_, a), (_, b)| dx(a).total_cmp(&dx(b))) {
        return Some(Hit::In(n));
    }
    match rects.iter().position(|r| y < r[1]) {
        Some(n) => Some(Hit::Start(n)),
        None => rects.len().checked_sub(1).map(Hit::End),
    }
}

impl Hit {
    /// The text it is on and an offset there, `offset` saying where in text `In(n)`.
    pub fn at(self, offset: impl FnOnce(usize) -> usize) -> (usize, usize) {
        match self {
            Hit::In(n) => (n, offset(n)),
            Hit::Start(n) => (n, 0),
            Hit::End(n) => (n, usize::MAX),
        }
    }
}

/// `styles` (sorted, apart) with `selected` laid over them: each part inside it styled by
/// `with` (given the style there, if any). As a text's highlights are given to draw.
pub fn overlay<T: Clone>(
    styles: Vec<(Range<usize>, T)>,
    selected: Range<usize>,
    with: impl Fn(Option<&T>) -> T,
) -> Vec<(Range<usize>, T)> {
    if selected.is_empty() {
        return styles;
    }
    let mut out = Vec::new();
    let mut at = 0;
    let gap = |out: &mut Vec<_>, from: usize, to: usize| {
        let (a, b) = (from.max(selected.start), to.min(selected.end));
        if a < b {
            out.push((a..b, with(None)));
        }
    };
    for (range, style) in styles {
        gap(&mut out, at, range.start);
        let (a, b) = (range.start.max(selected.start), range.end.min(selected.end));
        if a < b {
            out.extend([(range.start..a, style.clone()), (a..b, with(Some(&style))), (b..range.end, style)]);
        } else {
            out.push((range.clone(), style));
        }
        at = range.end;
    }
    gap(&mut out, at, usize::MAX);
    out.retain(|(range, _)| !range.is_empty());
    out
}

impl Selection {
    /// The texts drawn now, in order, as (key, text). A selection, or a press, stays while the
    /// texts at its ends are still there and unchanged. False: one went with its text.
    pub fn set<'a>(&mut self, texts: impl IntoIterator<Item = (&'a str, &'a str)>) -> bool {
        let mut joined = String::new();
        let mut kept = Vec::new();
        for (key, text) in texts {
            if !kept.is_empty() {
                joined.push('\n');
            }
            kept.push((key.to_string(), joined.len()..joined.len() + text.len()));
            joined.push_str(text);
        }
        if joined == self.field.text() && kept == self.texts {
            return true;
        }
        let (selection, head) = (self.field.selection(), self.field.cursor());
        let anchor = if head == selection.end { selection.start } else { selection.end };
        let place = |at: usize| self.place(at).map(|(key, offset)| (key.to_string(), offset));
        let (anchor, head) = (place(anchor), place(head));
        let old = std::mem::replace(self, Self { texts: kept, field: Field::of_lines(&joined), pressed: None });
        // Byte `offset` of text `key` now, if that text is as it was.
        let at = |(key, offset): (String, usize)| {
            let (_, range) = self.texts.iter().find(|(k, _)| *k == key)?;
            let (_, before) = old.texts.iter().find(|(k, _)| *k == key)?;
            let same = self.field.text().get(range.clone()) == old.field.text().get(before.clone());
            same.then_some(range.start + offset)
        };
        let (Some(anchor), Some(head)) = (anchor.and_then(at), head.and_then(at)) else { return false };
        let pressed =
            old.pressed.and_then(|(key, offset, count)| Some((at((key.clone(), offset))?, key, offset, count)));
        match pressed {
            Some((press, key, offset, count)) => {
                self.field.apply(Edit::Click { at: press, count, shift: false });
                // A drag had gone past the pressed unit: on to where it was.
                let unit = self.field.selection();
                if head < unit.start || head > unit.end {
                    self.field.apply(Edit::Drag { to: head });
                }
                self.pressed = Some((key, offset, count));
            }
            None => {
                self.field.apply(Edit::Click { at: anchor, count: 1, shift: false });
                self.field.apply(Edit::Click { at: head, count: 1, shift: true });
            }
        }
        true
    }

    /// Byte `at` of the joined text as the key of its text and an offset in it.
    fn place(&self, at: usize) -> Option<(&str, usize)> {
        let (key, range) = self.texts.iter().find(|(_, range)| range.start <= at && at <= range.end)?;
        Some((key.as_str(), at - range.start))
    }

    /// Text `n`'s byte `offset` in the joined text.
    fn joined(&self, n: usize, offset: usize) -> usize {
        self.texts.get(n).map_or(self.field.text().len(), |(_, range)| range.start + offset.min(range.len()))
    }

    /// A press on text `n` at byte `offset`: `clicks` 2 a word, 3 its line; `shift` extends.
    pub fn press(&mut self, n: usize, offset: usize, clicks: usize, shift: bool) {
        let at = self.joined(n, offset);
        let count = clicks.max(1);
        self.field.apply(Edit::Click { at, count, shift });
        self.pressed = self.place(at).filter(|_| !shift).map(|(key, offset)| (key.to_string(), offset, count));
    }

    /// The pressed mouse is over text `n` at byte `offset`.
    pub fn drag(&mut self, n: usize, offset: usize) {
        let to = self.joined(n, offset);
        self.field.apply(Edit::Drag { to });
    }

    /// ⌘A: every text.
    pub fn select_all(&mut self) {
        self.field.apply(Edit::SelectAll);
        self.pressed = None;
    }

    pub fn clear(&mut self) {
        let at = self.field.cursor();
        self.field.apply(Edit::Click { at, count: 1, shift: false });
        self.pressed = None;
    }

    /// What ⌘C copies: the texts selected, a line break between two. Empty: none.
    pub fn selected(&self) -> &str {
        self.field.selected()
    }

    /// The part of the text `key`, drawn as `text`, that is selected, in its own bytes. `None`:
    /// none of it, or its text is not the one selected in (it changed since).
    pub fn range_of(&self, key: &str, text: &str) -> Option<Range<usize>> {
        let selection = self.field.selection();
        let (_, range) = self
            .texts
            .iter()
            .find(|(k, _)| k == key)
            .filter(|(_, r)| self.field.text().get(r.clone()) == Some(text))?;
        let (a, b) = (selection.start.max(range.start), selection.end.min(range.end));
        (a < b).then(|| a - range.start..b - range.start)
    }

    pub fn extent(&self) -> Option<Extent> {
        let selection = self.field.selection();
        if selection.is_empty() {
            return None;
        }
        let touches = |range: &&Range<usize>| range.start < selection.end && selection.start < range.end;
        let rows = self.texts.iter().map(|(_, range)| range).filter(touches).count();
        Some(Extent { rows, chars: self.selected().chars().count() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(texts: &[(&str, &str)]) -> Selection {
        let mut s = Selection::default();
        s.set(texts.iter().copied());
        s
    }

    #[test]
    fn a_drag_across_two_texts_copies_both_a_line_break_between() {
        let mut s = of(&[("a", "Fix the login"), ("b", "It redirects twice")]);
        s.press(0, 4, 1, false);
        s.drag(1, 2);
        assert_eq!(s.selected(), "the login\nIt");
        assert_eq!(s.extent(), Some(Extent { rows: 2, chars: 12 }));
        assert_eq!(
            (s.range_of("a", "Fix the login"), s.range_of("b", "It redirects twice")),
            (Some(4..13), Some(0..2))
        );
        assert_eq!(s.range_of("b", "It redirects twice and more"), None);
        // Backwards too.
        s.press(1, 2, 1, false);
        s.drag(0, 4);
        assert_eq!(s.selected(), "the login\nIt");
    }

    #[test]
    fn clicks_pick_a_word_or_a_line_and_a_drag_goes_on_by_them() {
        let mut s = of(&[("a", "one two"), ("b", "three four\nfive")]);
        s.press(0, 5, 2, false);
        assert_eq!(s.selected(), "two");
        s.drag(1, 1);
        assert_eq!(s.selected(), "two\nthree");
        s.press(1, 2, 3, false);
        assert_eq!(s.selected(), "three four");
        s.press(0, 1, 1, false);
        assert_eq!((s.selected(), s.extent()), ("", None));
        s.press(1, 3, 1, true);
        assert_eq!(s.selected(), "ne two\nthr");
    }

    #[test]
    fn select_all_takes_every_text_and_clear_none() {
        let mut s = of(&[("a", "x"), ("b", "y z")]);
        s.select_all();
        assert_eq!((s.selected(), s.extent()), ("x\ny z", Some(Extent { rows: 2, chars: 5 })));
        s.clear();
        assert_eq!(s.selected(), "");
    }

    #[test]
    fn a_selection_holds_while_its_ends_texts_do() {
        let mut s = of(&[("a", "first reply"), ("b", "second")]);
        s.press(0, 6, 1, false);
        s.drag(1, 3);
        // A text added after, and one opened between: the ends are as they were.
        s.set([("a", "first reply"), ("in:1", "{}"), ("b", "second"), ("c", "more")]);
        assert_eq!(s.selected(), "reply\n{}\nsec");
        // Leftward drags keep their direction: the far end moves.
        s.press(1, 1, 1, false);
        s.drag(0, 0);
        s.set([("z", "new"), ("a", "first reply"), ("in:1", "{}"), ("b", "second")]);
        assert_eq!(s.selected(), "first reply\n{");
        s.drag(0, 1);
        assert_eq!(s.selected(), "ew\nfirst reply\n{");
        // A text between changed: it holds. The text at an end changed (a reply streaming): it goes.
        s.set([("z", "new"), ("a", "first reply, longer"), ("in:1", "{}")]);
        assert_eq!(s.selected(), "ew\nfirst reply, longer\n{");
        s.set([("z", "news"), ("a", "first reply, longer"), ("in:1", "{}")]);
        assert_eq!(s.selected(), "");
    }

    #[test]
    fn a_press_holds_its_place_and_unit_while_text_is_added() {
        // A press, then a row is added before the mouse moves: the drag goes on from the press.
        let mut s = of(&[("a", "first reply"), ("b", "second")]);
        s.press(0, 6, 1, false);
        assert!(s.set([("a", "first reply"), ("b", "second"), ("c", "new")]));
        s.drag(0, 9);
        assert_eq!(s.selected(), "rep");
        // A double click's word stays whole when a drag turns back after text changed.
        s.press(1, 2, 2, false);
        s.drag(2, 1);
        assert!(s.set([("z", "top"), ("a", "first reply"), ("b", "second"), ("c", "new")]));
        assert_eq!(s.selected(), "second\nnew");
        s.drag(1, 7);
        assert_eq!(s.selected(), "reply\nsecond");
        // The pressed text changed (streaming): the press goes, and the shell ends the drag.
        s.press(3, 1, 1, false);
        assert!(!s.set([("a", "first reply"), ("b", "second"), ("c", "newest")]));
        assert_eq!(s.selected(), "");
    }

    #[test]
    fn a_point_finds_its_text_or_the_nearest_end() {
        let rects = [[0., 0., 100., 20.], [0., 30., 40., 20.], [50., 30., 40., 20.]];
        assert_eq!(hit(&rects, (10., 5.)), Some(Hit::In(0)));
        // Level with two cells of a table: the nearer.
        assert_eq!(hit(&rects, (47., 35.)), Some(Hit::In(2)));
        assert_eq!(hit(&rects, (200., 35.)), Some(Hit::In(2)));
        assert_eq!(hit(&rects, (10., 25.)), Some(Hit::Start(1)));
        assert_eq!(hit(&rects, (10., -5.)), Some(Hit::Start(0)));
        assert_eq!(hit(&rects, (10., 99.)), Some(Hit::End(2)));
        assert_eq!(hit(&[], (0., 0.)), None);
        assert_eq!((Hit::End(1).at(|_| 0), Hit::In(0).at(|_| 1)), ((1, usize::MAX), (0, 1)));
    }

    #[test]
    fn the_selection_lies_over_a_texts_styles() {
        let styles = vec![(2..4, "b"), (6..8, "i")];
        let with = |style: Option<&&str>| if style.is_some() { "sel+" } else { "sel" };
        assert_eq!(
            overlay(styles.clone(), 3..7, with),
            vec![(2..3, "b"), (3..4, "sel+"), (4..6, "sel"), (6..7, "sel+"), (7..8, "i")]
        );
        assert_eq!(overlay(styles.clone(), 0..10, with)[0], (0..2, "sel"));
        assert_eq!(overlay(styles.clone(), 0..10, with).last(), Some(&(8..10, "sel")));
        assert_eq!(overlay(styles.clone(), 5..5, with), styles);
        assert_eq!(overlay(Vec::new(), 1..3, with), vec![(1..3, "sel")]);
    }
}
