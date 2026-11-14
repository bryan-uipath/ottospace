//! The new-task box's completion dropdown: which names finish the tag at the cursor, in what
//! order, and which letters of each were matched. The model gathers the names; this ranks them.

use serde::Serialize;

/// The most rows the dropdown shows.
pub const MAX_ROWS: usize = 8;

/// A name a tag may take, before it is matched.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    /// What goes into the line after the mark: `canvas`, `/src/export/`.
    pub name: String,
    /// What it is, for the small mark on its row: `project`, `repo`, `folder`, `tab`.
    pub kind: &'static str,
    /// A muted word of context: a repository's path, how many tasks a project has.
    pub detail: String,
}

/// One row of the dropdown.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Row {
    pub name: String,
    pub kind: &'static str,
    pub detail: String,
    /// The characters of `name` (by index, not byte) that match what is typed: drawn stronger.
    pub matched: Vec<usize>,
}

/// The dropdown as drawn and as the state gives it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Dropdown {
    /// The tag's mark: `#`, `>`, `@`, `$`, `/` or `^`.
    pub mark: char,
    /// What is typed of the name, before the cursor.
    pub typed: String,
    /// The byte where the tag's word starts: the dropdown hangs under it.
    #[serde(skip)]
    pub start: usize,
    pub rows: Vec<Row>,
    /// The highlighted row: ↵ and Tab take it. `None`: ↵ adds the task.
    pub selected: Option<usize>,
    /// What the dropdown says under its rows: `no match`, `new project` with none; `… 12 more`
    /// past the last of a folder's.
    pub note: Option<String>,
    /// The rows are those of the path as it was: what finishes it now is still being read. They
    /// stay on show, so the list does not blink, and none can be taken.
    #[serde(skip)]
    pub stale: bool,
}

/// How well `name` matches `typed`, without regard to case, and where. 0: it starts with it.
/// 1: a word of it does (`flow-app` for `ap`). 2: its letters are there in order (`fa`).
/// `None`: no match. Nothing typed matches everything.
pub fn matching(typed: &str, name: &str) -> Option<(u8, Vec<usize>)> {
    let lower = |text: &str| text.chars().flat_map(char::to_lowercase).collect::<Vec<char>>();
    let (typed, chars, name) = (lower(typed), name.chars().collect::<Vec<char>>(), lower(name));
    // A character that lowers to several would shift every index: match such a name loosely.
    if name.len() != chars.len() {
        return name.starts_with(&typed).then(|| (0, (0..typed.len().min(chars.len())).collect()));
    }
    let run = |at: usize| name[at..].starts_with(&typed).then(|| (at..at + typed.len()).collect::<Vec<usize>>());
    if let Some(found) = run(0) {
        return Some((0, found));
    }
    let word_start =
        |at: &usize| !chars[at - 1].is_alphanumeric() || (chars[at - 1].is_lowercase() && chars[*at].is_uppercase());
    if let Some(found) = (1..name.len()).filter(word_start).find_map(run) {
        return Some((1, found));
    }
    let mut found = Vec::new();
    let mut from = 0;
    for letter in &typed {
        let at = from + name[from..].iter().position(|c| c == letter)?;
        found.push(at);
        from = at + 1;
    }
    Some((2, found))
}

/// The rows for what is typed: the candidates that match, the best first, at most `MAX_ROWS`.
/// Among equals the order given stays: the caller puts the likelier first.
pub fn rows(typed: &str, candidates: Vec<Candidate>) -> Vec<Row> {
    let mut seen: Vec<String> = Vec::new();
    let mut ranked: Vec<(u8, Row)> = Vec::new();
    for Candidate { name, kind, detail } in candidates {
        if seen.contains(&name) {
            continue;
        }
        if let Some((rank, matched)) = matching(typed, &name) {
            seen.push(name.clone());
            ranked.push((rank, Row { name, kind, detail, matched }));
        }
    }
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().take(MAX_ROWS).map(|(_, row)| row).collect()
}

/// The highlighted row. One the user moved to stays (within the rows there are). Else the
/// first, but only once a letter of the name is typed: a bare `#` highlights nothing, so ↵
/// still adds the task.
pub fn selected(moved: Option<usize>, typed: &str, rows: usize) -> Option<usize> {
    let first = (!typed.is_empty()).then_some(0);
    moved.or(first).filter(|_| rows > 0).map(|at| at.min(rows - 1))
}

/// The row a move of `delta` lands on, from the highlighted one; it wraps. From none, down is
/// the first row and up the last.
pub fn moved(selected: Option<usize>, delta: i32, rows: usize) -> Option<usize> {
    if rows == 0 {
        return None;
    }
    let from = match selected {
        Some(at) => at as i64,
        None if delta > 0 => -1,
        None => rows as i64,
    };
    Some((from + delta as i64).rem_euclid(rows as i64) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(typed: &str, list: &[&str]) -> Vec<String> {
        let candidate = |name: &&str| Candidate { name: name.to_string(), kind: "repo", detail: String::new() };
        rows(typed, list.iter().map(candidate).collect()).into_iter().map(|row| row.name).collect()
    }

    #[test]
    fn names_are_ranked_by_how_they_match() {
        let list = ["flow-app", "apollo", "zap", "MyApp", "a-plan", "notes", "App", "apollo"];
        // Starts with it, then a word of it does, then its letters in order; each once.
        assert_eq!(names("ap", &list), ["apollo", "App", "flow-app", "MyApp", "zap", "a-plan"]);
        assert_eq!(names("", &list), ["flow-app", "apollo", "zap", "MyApp", "a-plan", "notes", "App"]);
        assert_eq!(names("xyz", &list), Vec::<String>::new());
        let cases = [
            ("ap", "apollo", Some((0, vec![0, 1]))),
            ("AP", "flow-app", Some((1, vec![5, 6]))),
            ("app", "MyApp", Some((1, vec![2, 3, 4]))),
            // `f` then `a`: letters in order, not one word.
            ("fa", "flow-app", Some((2, vec![0, 5]))),
            ("fp", "flow-app", Some((2, vec![0, 6]))),
            ("ex", "/src/export", Some((1, vec![5, 6]))),
            ("日", "今日本", Some((2, vec![1]))),
            ("pa", "ap", None),
            ("", "x", Some((0, vec![]))),
        ];
        for (typed, name, want) in cases {
            assert_eq!(matching(typed, name), want, "{typed} in {name}");
        }
        // At most eight rows.
        let many: Vec<String> = (0..20).map(|n| format!("app-{n}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(names("app", &many).len(), MAX_ROWS);
    }

    #[test]
    fn the_first_row_is_highlighted_only_once_a_letter_is_typed() {
        assert_eq!((selected(None, "", 3), selected(None, "a", 3), selected(None, "a", 0)), (None, Some(0), None));
        assert_eq!((selected(Some(5), "", 3), selected(Some(1), "a", 3)), (Some(2), Some(1)));
        // ↓ from nothing is the first row, ↑ the last; both wrap.
        assert_eq!(
            (moved(None, 1, 3), moved(None, -1, 3), moved(Some(2), 1, 3), moved(Some(0), -1, 3)),
            (Some(0), Some(2), Some(0), Some(2))
        );
        assert_eq!((moved(None, 1, 0), moved(Some(1), i32::MIN, 3).is_some()), (None, true));
    }
}
