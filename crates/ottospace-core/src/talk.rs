//! The right drawer: one conversation at a time, beside the workspace and in no tab. Today a
//! conversation is a task's agent in a terminal the shell hosts; what the drawer shows of it
//! is the shell's to draw. See `docs/ai/task-conversation.md`.

use serde::Serialize;

use crate::tasks::{Group, TaskId};

/// The drawer's width in points when nobody has dragged it, and the least and most it takes.
pub const WIDTH: f32 = 480.;
pub const MIN_WIDTH: f32 = 320.;
pub const MAX_WIDTH: f32 = 1600.;

/// The conversation the drawer is on.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Talk {
    /// Whose it is. A session that is no task's will be a second kind of subject here.
    pub task: TaskId,
    /// False: closed to the right rail. The agent goes on; the rail's button opens it again.
    pub open: bool,
    /// The keyboard is in it: keys go to the agent, as in an app pane they go to the app.
    pub keys: bool,
}

/// The most conversations the right rail shows. Past it the oldest idle ones drop off; they
/// stay in the Tasks list.
pub const RAIL: usize = 8;

/// A conversation's badge on the right rail: the first letters of the title's first two words,
/// or the first two letters of a one-word title, in capitals. Whole graphemes: `éte` → `ÉT`.
pub fn monogram(title: &str) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    let words: Vec<&str> = title.split_whitespace().collect();
    let letters: Vec<&str> = match words[..] {
        [] => Vec::new(),
        [one] => one.graphemes(true).take(2).collect(),
        [one, two, ..] => [one, two].iter().filter_map(|word| word.graphemes(true).next()).collect(),
    };
    letters.concat().to_uppercase()
}

/// The monograms of the rail's cells, in order: each `monogram`, unless an earlier cell has it;
/// then the first letter and the first letter (not a space) where this title differs from the
/// earlier one's, else the first letter and the cell's place.
pub fn monograms(titles: &[&str]) -> Vec<String> {
    use unicode_segmentation::UnicodeSegmentation;
    let letters = |title: &str| -> Vec<String> {
        title.graphemes(true).filter(|g| !g.trim().is_empty()).map(str::to_uppercase).collect()
    };
    let mut out: Vec<String> = Vec::new();
    for (at, title) in titles.iter().enumerate() {
        let mut mono = monogram(title);
        if let Some(earlier) = out.iter().position(|seen| *seen == mono) {
            let (mine, theirs) = (letters(title), letters(titles[earlier]));
            let first = mine.first().cloned().unwrap_or_default();
            let parted = (1..mine.len())
                .filter(|i| theirs.get(*i) != Some(&mine[*i]))
                .map(|i| format!("{first}{}", mine[i]))
                .find(|candidate| !out.contains(candidate));
            // Else its place, or the first number after it no cell has.
            let numbered = (at + 1..).map(|n| format!("{first}{n}")).find(|candidate| !out.contains(candidate));
            mono = parted.or(numbered).unwrap_or_default();
        }
        out.push(mono);
    }
    out
}

/// A conversation's cell on the right rail.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Cell {
    pub task: TaskId,
    /// Its badge: two letters, told apart from the other cells' (`monograms`).
    pub mono: String,
    /// The state its ring is coloured by. `None`: a task not listed (snoozed, dismissed).
    pub group: Option<Group>,
    /// Its agent is at work: the badge's dot blinks.
    pub working: bool,
    /// The drawer shows it.
    pub shown: bool,
    /// What hovering it says: the title, what it does now, and where, who and how old.
    pub tip: String,
}

/// A width asked for (a drag of the drawer's edge), as one it may have.
pub fn width(asked: f32) -> f32 {
    if asked.is_finite() { asked.clamp(MIN_WIDTH, MAX_WIDTH) } else { WIDTH }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_monogram_takes_whole_letters_of_the_first_two_words() {
        let cases = [
            ("fix the login loop", "FT"),
            ("refactor", "RE"),
            ("  spaced   out ", "SO"),
            ("été à Paris", "ÉÀ"),
            ("👩‍💻 ship it", "👩‍💻S"),
            ("e\u{301}clair", "E\u{301}C"),
            ("x", "X"),
            ("", ""),
        ];
        for (title, want) in cases {
            assert_eq!(monogram(title), want, "{title:?}");
        }
    }

    #[test]
    fn monograms_that_collide_take_the_first_letter_that_differs() {
        let titles = ["fix the login", "fix the logout", "fix tests", "fix the login", "refactor"];
        // `FT` three times: the later ones take the first letter where their titles part.
        assert_eq!(monograms(&titles), ["FT", "FO", "FE", "F4", "RE"]);
        assert_eq!(monograms(&["été", "étage"]), ["ÉT", "ÉA"]);
        assert_eq!(monograms(&[]), Vec::<String>::new());
        // A place already taken as a badge: the next number.
        assert_eq!(monograms(&["a3", "a", "a"]), ["A3", "A", "A4"]);
    }

    #[test]
    fn a_width_is_kept_within_bounds() {
        let asked = [0., 400., 5000., f32::NAN, f32::INFINITY, -1.];
        assert_eq!(asked.map(width), [MIN_WIDTH, 400., MAX_WIDTH, WIDTH, WIDTH, MIN_WIDTH]);
    }
}
