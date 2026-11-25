//! Reviewing a change (`review` flag): one file's patch, as `git diff` writes it, read into the
//! rows the viewer draws, and the steps between its hunks. Pure: the shell runs git and draws;
//! the model holds the hunks' rows, never a line's text. Design: `docs/ai/review/plan.md`.

use crate::diff::Sign;

/// The most rows a patch keeps: past them it is cut, and says so.
pub const MAX_ROWS: usize = 10_000;

/// A row of a file's diff as drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A hunk's start: the unchanged lines folded before it (`⋯ 40 lines`), and its first line
    /// on each side.
    Hunk { skipped: u32, old: u32, new: u32 },
    /// A line, with its number on each side it is on.
    Line { sign: Sign, old: Option<u32>, new: Option<u32>, text: String },
}

/// One file's patch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Patch {
    pub rows: Vec<Row>,
    /// Git says it differs, as bytes: no lines.
    pub binary: bool,
    /// Renamed from this path (`rename from`).
    pub from: Option<String>,
    pub added: u32,
    pub removed: u32,
    /// More than `MAX_ROWS`: the rest not kept.
    pub cut: bool,
}

impl Patch {
    /// The row of each hunk's start, in order: what `]c` and `[c` step between.
    pub fn hunks(&self) -> Vec<u32> {
        let at = |(n, row): (usize, &Row)| matches!(row, Row::Hunk { .. }).then_some(n as u32);
        self.rows.iter().enumerate().filter_map(at).collect()
    }
}

/// `git diff`'s output for one file: its headers (rename, binary), then each hunk's lines. A
/// second file's `diff --git` ends it; `\ No newline at end of file` is no row.
pub fn parse(text: &str) -> Patch {
    let mut patch = Patch::default();
    let (mut old, mut new, mut next_new, mut files, mut in_hunk) = (0u32, 0u32, 1u32, 0, false);
    for line in text.lines() {
        if line.starts_with("diff --git ") {
            files += 1;
            if files > 1 {
                break;
            }
            continue;
        }
        if let Some((from_old, from_new, len_new)) = hunk_header(line) {
            in_hunk = true;
            // A hunk that adds nothing on the new side names the line before it.
            let first = if len_new == 0 { from_new + 1 } else { from_new };
            let skipped = first.saturating_sub(next_new);
            (old, new) = (from_old.max(1), first.max(1));
            if !push(&mut patch, Row::Hunk { skipped, old, new }) {
                break;
            }
            continue;
        }
        if !in_hunk {
            if let Some(path) = line.strip_prefix("rename from ") {
                patch.from = Some(path.to_string());
            } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
                patch.binary = true;
            }
            continue;
        }
        let (sign, rest) = match line.as_bytes().first() {
            Some(b' ') => (Sign::Context, &line[1..]),
            Some(b'-') => (Sign::Removed, &line[1..]),
            Some(b'+') => (Sign::Added, &line[1..]),
            // An empty context line, as some tools write it.
            None => (Sign::Context, ""),
            _ => continue,
        };
        let row = match sign {
            Sign::Removed => {
                old += 1;
                Row::Line { sign, old: Some(old - 1), new: None, text: rest.to_string() }
            }
            Sign::Added => {
                new += 1;
                Row::Line { sign, old: None, new: Some(new - 1), text: rest.to_string() }
            }
            _ => {
                (old, new) = (old + 1, new + 1);
                Row::Line { sign, old: Some(old - 1), new: Some(new - 1), text: rest.to_string() }
            }
        };
        next_new = new;
        // Counted as kept: a cut patch's counts are of the rows it shows.
        let counts = match sign {
            Sign::Added => (1, 0),
            Sign::Removed => (0, 1),
            _ => (0, 0),
        };
        if !push(&mut patch, row) {
            break;
        }
        (patch.added, patch.removed) = (patch.added + counts.0, patch.removed + counts.1);
    }
    patch
}

/// Kept if there is room; else the patch is cut.
fn push(patch: &mut Patch, row: Row) -> bool {
    if patch.rows.len() >= MAX_ROWS {
        patch.cut = true;
        return false;
    }
    patch.rows.push(row);
    true
}

/// `@@ -12,3 +14,5 @@ fn name`: the old side's first line, the new side's first line and its
/// length (1 when it is not written).
fn hunk_header(line: &str) -> Option<(u32, u32, u32)> {
    let rest = line.strip_prefix("@@ -")?;
    let (ranges, _) = rest.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let start = |range: &str| range.split(',').next()?.parse::<u32>().ok();
    let len = |range: &str| range.split_once(',').map_or(Some(1), |(_, len)| len.parse::<u32>().ok());
    Some((start(old)?, start(new)?, len(new)?))
}

/// The hunk `]c` (`forward`) or `[c` goes to from row `top`: the next one below it, or the last
/// one above it. `None`: there is none that way.
pub fn step(hunks: &[u32], top: u32, forward: bool) -> Option<u32> {
    if forward { hunks.iter().copied().find(|at| *at > top) } else { hunks.iter().rev().copied().find(|at| *at < top) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDIT: &str = "diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -3,3 +3,4 @@ fn one() {
 a
-b
+B
+c
 d
@@ -20,2 +21,2 @@
 x
-y
+Y
";

    fn line(row: &Row) -> Option<(Sign, Option<u32>, Option<u32>, &str)> {
        match row {
            Row::Line { sign, old, new, text } => Some((*sign, *old, *new, text.as_str())),
            Row::Hunk { .. } => None,
        }
    }

    /// Each hunk a row with the unchanged lines folded before it; each line numbered on its sides.
    #[test]
    fn a_patch_is_rows_numbered_on_each_side_and_folded_between_hunks() {
        let patch = parse(EDIT);
        assert_eq!((patch.added, patch.removed, patch.binary, patch.cut), (3, 2, false, false));
        assert_eq!(patch.rows[0], Row::Hunk { skipped: 2, old: 3, new: 3 });
        assert_eq!(line(&patch.rows[1]), Some((Sign::Context, Some(3), Some(3), "a")));
        assert_eq!(line(&patch.rows[2]), Some((Sign::Removed, Some(4), None, "b")));
        assert_eq!(line(&patch.rows[3]), Some((Sign::Added, None, Some(4), "B")));
        assert_eq!(line(&patch.rows[5]), Some((Sign::Context, Some(5), Some(6), "d")));
        // Lines 7 to 20 of the new side are unchanged: folded.
        assert_eq!(patch.rows[6], Row::Hunk { skipped: 14, old: 20, new: 21 });
        assert_eq!(patch.hunks(), [0, 6]);
    }

    /// `]c` goes to the next hunk below the top, `[c` to the last one above; none past the ends.
    #[test]
    fn the_hunk_steps_go_down_and_up_and_stop_at_the_ends() {
        let hunks = [0, 6, 30];
        assert_eq!((step(&hunks, 0, true), step(&hunks, 6, true), step(&hunks, 30, true)), (Some(6), Some(30), None));
        assert_eq!((step(&hunks, 10, false), step(&hunks, 6, false), step(&hunks, 0, false)), (Some(6), Some(0), None));
        assert_eq!(step(&[], 0, true), None);
    }

    /// A rename says where from; a binary file has no rows; "no newline" is no row; a new file's
    /// hunk starts at its first line; a whole file removed numbers only the old side.
    #[test]
    fn renames_binaries_new_and_removed_files_and_the_no_newline_mark() {
        let renamed = "diff --git a/old.rs b/new.rs\nsimilarity index 90%\nrename from old.rs\nrename to new.rs\n@@ -1 +1 @@\n-a\n+b\n\\ No newline at end of file\n";
        let patch = parse(renamed);
        assert_eq!((patch.from.as_deref(), patch.rows.len(), patch.added), (Some("old.rs"), 3, 1));
        assert_eq!(patch.rows[0], Row::Hunk { skipped: 0, old: 1, new: 1 });
        let binary = parse("diff --git a/a.png b/a.png\nindex 1..2 100644\nBinary files a/a.png and b/a.png differ\n");
        assert!(binary.binary && binary.rows.is_empty());
        let added = parse(
            "diff --git a/n.rs b/n.rs\nnew file mode 100644\n--- /dev/null\n+++ b/n.rs\n@@ -0,0 +1,2 @@\n+x\n+y\n",
        );
        assert_eq!(added.rows[0], Row::Hunk { skipped: 0, old: 1, new: 1 });
        assert_eq!(line(&added.rows[2]), Some((Sign::Added, None, Some(2), "y")));
        let gone = parse("diff --git a/g.rs b/g.rs\ndeleted file mode 100644\n@@ -1,2 +0,0 @@\n-x\n-y\n");
        assert_eq!((gone.removed, line(&gone.rows[2])), (2, Some((Sign::Removed, Some(2), None, "y"))));
    }

    /// A second file ends the patch; past `MAX_ROWS` it is cut; text that is no patch is none.
    #[test]
    fn one_file_only_within_its_cap_and_nothing_from_no_patch() {
        let two = format!("{EDIT}diff --git a/b.rs b/b.rs\n@@ -1 +1 @@\n-p\n+q\n");
        assert_eq!(parse(&two).rows.len(), parse(EDIT).rows.len());
        let huge = format!("diff --git a/a b/a\n@@ -1 +1,{n} @@\n{}", "+x\n".repeat(MAX_ROWS + 5), n = MAX_ROWS + 5);
        let cut = parse(&huge);
        assert_eq!((cut.rows.len(), cut.cut, cut.added), (MAX_ROWS, true, MAX_ROWS as u32 - 1), "the rows it keeps");
        assert_eq!(parse("not a patch\n+looks like one\n"), Patch::default());
        assert_eq!((hunk_header("@@ -a +2 @@"), hunk_header("@@ -1 +2,x @@")), (None, None));
    }
}
