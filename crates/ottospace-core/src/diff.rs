//! A file edit's change, as the transcript shows it under its call: the lines removed and added,
//! from the call's own input (Claude `Edit`, `MultiEdit`, `Write`, `NotebookEdit`; Codex's
//! `fileChange` item and `apply_patch` patch). Untrusted text: cut, cleaned, drawn as plain
//! monospaced text. Built from borrowed JSON, keeping only what is drawn, so a huge input is
//! counted, never copied. Design: `docs/ai/agent-runners.md`.

use serde::Serialize;
use serde_json::Value;

use crate::links::{self, Target};
use crate::transcript::{Text, clean, cut};

/// The most lines one card keeps, over all its files; the rest are counted.
pub const MAX_LINES: usize = 40;
/// The most bytes of one line kept.
pub const MAX_LINE: usize = 240;
/// The most files one card names; the rest are counted.
pub const MAX_FILES: usize = 8;
/// A longer path is not a path: the call gets no card.
pub const MAX_PATH: usize = 1024;
/// Unchanged lines shown on each side of an edit.
const CONTEXT: usize = 2;

/// What a line of a card is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sign {
    Context,
    Removed,
    Added,
    /// A hunk's header (`@@ -1,3 +1,4 @@`), or the break between two edits of one file.
    Hunk,
}

/// One line as drawn: cleaned, at most `MAX_LINE` bytes of what was written.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub sign: Sign,
    pub text: Text,
}

/// What a call does to a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    /// Claude `Edit`, `MultiEdit`, `NotebookEdit`; Codex `update`.
    Edit,
    /// Claude `Write`: the file's whole new content.
    Write,
    Add,
    Delete,
}

/// One file of a card. `path` as written (display only: `shown`); `added` and `removed` count
/// every line, kept or not; `more` the lines not kept.
#[derive(Clone, Debug, PartialEq)]
pub struct File {
    pub path: Text,
    /// A move's destination (Codex).
    pub to: Option<Text>,
    pub change: Change,
    /// Claude `replace_all`: every occurrence.
    pub all: bool,
    pub lines: Vec<Line>,
    pub added: usize,
    pub removed: usize,
    pub more: usize,
}

impl File {
    /// The path as drawn: relative to `repo` when inside it, cleaned.
    pub fn shown(&self, repo: Option<&str>) -> String {
        shown_path(self.path.text(), repo)
    }

    /// What a click on the path may do: reveal it, if `links::target` allows it. Only an
    /// absolute path: a relative one is the session's folder's, not surely the repository's.
    pub fn target(&self, repo: Option<&str>) -> Option<Target> {
        let path = Some(self.path.text()).filter(|path| path.starts_with('/'))?;
        links::target(path, repo).filter(|target| matches!(target, Target::File(_)))
    }
}

/// An edit call's card.
#[derive(Clone, Debug, PartialEq)]
pub struct Diff {
    pub files: Vec<File>,
    /// Files past `MAX_FILES`.
    pub more_files: usize,
}

/// What the state says of a card: counts only, never a line or a path.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DiffShape {
    pub changes: Vec<Change>,
    pub more_files: usize,
    pub added: usize,
    pub removed: usize,
    /// Lines kept, and lines counted but not kept.
    pub lines: usize,
    pub more: usize,
}

impl Diff {
    pub fn shape(&self) -> DiffShape {
        let sum = |of: fn(&File) -> usize| self.files.iter().map(of).sum();
        DiffShape {
            changes: self.files.iter().map(|file| file.change).collect(),
            more_files: self.more_files,
            added: sum(|f| f.added),
            removed: sum(|f| f.removed),
            lines: sum(|f| f.lines.len()),
            more: sum(|f| f.more),
        }
    }

    /// Bytes kept, for the transcript's cap.
    pub fn bytes(&self) -> usize {
        let file = |f: &File| {
            f.path.len() + f.to.as_ref().map_or(0, Text::len) + f.lines.iter().map(|l| l.text.len() + 8).sum::<usize>()
        };
        self.files.iter().map(file).sum::<usize>() + 32
    }
}

/// `path` relative to `repo` when inside it, cleaned of what hides or reorders text.
pub fn shown_path(path: &str, repo: Option<&str>) -> String {
    let repo = repo.map(|repo| repo.trim_end_matches('/')).filter(|repo| !repo.is_empty());
    let inside = repo.and_then(|repo| path.strip_prefix(repo)?.strip_prefix('/')).filter(|rest| !rest.is_empty());
    clean(inside.unwrap_or(path))
}

/// A Claude Code tool call's card, from its input. `None`: not an edit, or not one it can read.
pub fn claude(tool: &str, input: &Value) -> Option<Diff> {
    let text = |key: &str| input[key].as_str();
    let mut cards = Cards::default();
    match tool {
        "Edit" => {
            let mut file = cards.file(text("file_path")?, Change::Edit)?;
            file.all = input["replace_all"] == true;
            cards.edit(&mut file, text("old_string")?, text("new_string")?);
            cards.push(file);
        }
        "MultiEdit" => {
            let mut file = cards.file(text("file_path")?, Change::Edit)?;
            for (n, edit) in input["edits"].as_array()?.iter().enumerate() {
                let (Some(old), Some(new)) = (edit["old_string"].as_str(), edit["new_string"].as_str()) else {
                    continue;
                };
                if n > 0 {
                    cards.line(&mut file, Sign::Hunk, "");
                }
                file.all |= edit["replace_all"] == true;
                cards.edit(&mut file, old, new);
            }
            cards.push(file);
        }
        "Write" => {
            let mut file = cards.file(text("file_path")?, Change::Write)?;
            cards.all(&mut file, Sign::Added, text("content")?);
            cards.push(file);
        }
        "NotebookEdit" if text("edit_mode") == Some("delete") => {
            let file = cards.file(text("notebook_path")?, Change::Delete)?;
            cards.push(file);
        }
        "NotebookEdit" => {
            let mut file = cards.file(text("notebook_path")?, Change::Edit)?;
            cards.all(&mut file, Sign::Added, text("new_source")?);
            cards.push(file);
        }
        _ => return None,
    }
    cards.done()
}

/// A Codex `fileChange` item's card, from its `changes`: `[{path, kind: {type, move_path}, diff}]`.
/// An added file's `diff` is its content, a deleted one's what it held, an update's a unified diff.
pub fn codex_changes(changes: &Value) -> Option<Diff> {
    let mut cards = Cards::default();
    for change in changes.as_array()? {
        let (Some(path), Some(kind)) = (change["path"].as_str(), change["kind"]["type"].as_str()) else { continue };
        let change_of = match kind {
            "add" => Change::Add,
            "delete" => Change::Delete,
            "update" => Change::Edit,
            _ => continue,
        };
        let Some(mut file) = cards.file(path, change_of) else { continue };
        let body = change["diff"].as_str().unwrap_or("");
        match change_of {
            Change::Add => cards.all(&mut file, Sign::Added, body),
            Change::Delete => cards.all(&mut file, Sign::Removed, body),
            _ => {
                file.to = change["kind"]["move_path"].as_str().and_then(|to| cards.path(to));
                cards.unified(&mut file, body);
            }
        }
        cards.push(file);
    }
    cards.done()
}

/// A Codex `apply_patch` call's card, from its patch: `*** Add File: p`, `*** Update File: p`
/// (`*** Move to: q`), `*** Delete File: p`, each with its `@@` hunks and `+`/`-`/` ` lines.
pub fn codex_patch(patch: &str) -> Option<Diff> {
    let mut cards = Cards::default();
    let mut file: Option<File> = None;
    for raw in patch.lines() {
        let header = raw.strip_prefix("*** ");
        let named = header.and_then(|h| h.split_once(" File: "));
        if let Some((kind, path)) = named {
            file.take().into_iter().for_each(|file| cards.push(file));
            let change = match kind {
                "Add" => Change::Add,
                "Delete" => Change::Delete,
                _ => Change::Edit,
            };
            file = cards.file(path, change);
        } else if let (Some(to), Some(file)) = (header.and_then(|h| h.strip_prefix("Move to: ")), &mut file) {
            file.to = cards.path(to);
        } else if header.is_some() {
            // `*** Begin Patch`, `*** End Patch`, `*** End of File`.
        } else if let Some(file) = &mut file {
            cards.body(file, raw);
        }
    }
    file.into_iter().for_each(|file| cards.push(file));
    cards.done()
}

/// A card being built: the lines it may still keep.
struct Cards {
    files: Vec<File>,
    more_files: usize,
    room: usize,
}

impl Default for Cards {
    fn default() -> Self {
        Self { files: Vec::new(), more_files: 0, room: MAX_LINES }
    }
}

impl Cards {
    /// A path as kept: non-empty, at most `MAX_PATH` bytes.
    fn path(&self, path: &str) -> Option<Text> {
        (!path.is_empty() && path.len() <= MAX_PATH).then(|| Text::new(path))
    }

    fn file(&self, path: &str, change: Change) -> Option<File> {
        let path = self.path(path)?;
        Some(File { path, to: None, change, all: false, lines: Vec::new(), added: 0, removed: 0, more: 0 })
    }

    fn push(&mut self, file: File) {
        match self.files.len() < MAX_FILES {
            true => self.files.push(file),
            false => self.more_files += 1,
        }
    }

    fn done(self) -> Option<Diff> {
        (!self.files.is_empty()).then_some(Diff { files: self.files, more_files: self.more_files })
    }

    /// Keeps a line while there is room, else counts it. A file past `MAX_FILES` keeps none.
    fn line(&mut self, file: &mut File, sign: Sign, text: &str) {
        match sign {
            Sign::Added => file.added += 1,
            Sign::Removed => file.removed += 1,
            Sign::Context | Sign::Hunk => {}
        }
        if self.room == 0 || self.files.len() >= MAX_FILES {
            file.more += 1;
            return;
        }
        self.room -= 1;
        let kept = cut(text, MAX_LINE);
        let mut shown = clean(kept);
        if kept.len() < text.len() {
            shown.push('…');
        }
        file.lines.push(Line { sign, text: Text::new(&shown) });
    }

    /// `count` lines of `lines` as `sign`: those with room kept, the rest counted unread.
    fn run<'a>(&mut self, file: &mut File, sign: Sign, lines: impl Iterator<Item = &'a str>, count: usize) {
        let mut read = 0;
        for line in lines.take(count.min(self.room)) {
            self.line(file, sign, line);
            read += 1;
        }
        let rest = count - read;
        file.more += rest;
        match sign {
            Sign::Added => file.added += rest,
            Sign::Removed => file.removed += rest,
            Sign::Context | Sign::Hunk => {}
        }
    }

    /// Every line of `text` as `sign`.
    fn all(&mut self, file: &mut File, sign: Sign, text: &str) {
        self.run(file, sign, text.lines(), text.lines().count());
    }

    /// `old` replaced by `new`: the lines both start and end with shown as context around the
    /// ones that differ. Linear in their sizes; nothing copied but what is kept.
    fn edit(&mut self, file: &mut File, old: &str, new: &str) {
        let (olds, news) = (old.lines().count(), new.lines().count());
        let head = old.lines().zip(new.lines()).take_while(|(a, b)| a == b).count();
        let tail = old.lines().rev().zip(new.lines().rev()).take_while(|(a, b)| a == b).count();
        let tail = tail.min(olds.min(news) - head);
        let before = head.min(CONTEXT);
        self.run(file, Sign::Context, old.lines().skip(head - before), before);
        self.run(file, Sign::Removed, old.lines().skip(head), olds - head - tail);
        self.run(file, Sign::Added, new.lines().skip(head), news - head - tail);
        self.run(file, Sign::Context, old.lines().skip(olds - tail), tail.min(CONTEXT));
    }

    /// A unified diff's lines; its `---`/`+++` headers before the first hunk left out.
    fn unified(&mut self, file: &mut File, diff: &str) {
        let mut hunks = false;
        for raw in diff.lines() {
            hunks |= raw.starts_with("@@");
            let header = ["--- ", "+++ ", "diff ", "index "].iter().any(|h| raw.starts_with(h));
            if hunks || !header {
                self.body(file, raw);
            }
        }
    }

    /// One line of a hunk, by its first character.
    fn body(&mut self, file: &mut File, raw: &str) {
        let (sign, text) = match raw.as_bytes().first() {
            Some(b'+') => (Sign::Added, &raw[1..]),
            Some(b'-') => (Sign::Removed, &raw[1..]),
            Some(b' ') => (Sign::Context, &raw[1..]),
            Some(b'@') if raw.starts_with("@@") => (Sign::Hunk, raw),
            // "\ No newline at end of file".
            Some(b'\\') => return,
            _ => (Sign::Context, raw),
        };
        self.line(file, sign, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Each kept line as `<sign><text>`.
    fn lines(file: &File) -> Vec<String> {
        let sign = |s: Sign| match s {
            Sign::Context => ' ',
            Sign::Removed => '-',
            Sign::Added => '+',
            Sign::Hunk => '@',
        };
        file.lines.iter().map(|l| format!("{}{}", sign(l.sign), l.text.text())).collect()
    }

    fn one(diff: Option<Diff>) -> File {
        let diff = diff.expect("a card");
        assert_eq!(diff.files.len(), 1);
        diff.files.into_iter().next().unwrap()
    }

    #[test]
    fn an_edit_shows_what_changed_with_two_lines_around_it() {
        let input = json!({
            "file_path": "/w/app/src/main.rs",
            "old_string": "a\nb\nc\nold\nd\ne\nf",
            "new_string": "a\nb\nc\nnew\nnewer\nd\ne\nf",
        });
        let file = one(claude("Edit", &input));
        assert_eq!(lines(&file), [" b", " c", "-old", "+new", "+newer", " d", " e"]);
        assert_eq!((file.added, file.removed, file.more, file.all, file.change), (2, 1, 0, false, Change::Edit));
        assert_eq!(file.shown(Some("/w/app")), "src/main.rs");
        assert_eq!(file.shown(Some("/w/app/")), "src/main.rs");
        assert_eq!(file.shown(Some("/w/ap")), "/w/app/src/main.rs", "a sibling folder is not inside");
        assert_eq!(file.shown(None), "/w/app/src/main.rs");
        // A click reveals it only inside the repository, and only by an absolute path.
        assert_eq!(file.target(Some("/w/app")), Some(Target::File("/w/app/src/main.rs".into())));
        assert_eq!((file.target(Some("/w/other")), file.target(None)), (None, None));
        let relative = one(claude("Write", &json!({"file_path": "src/main.rs", "content": ""})));
        assert_eq!(relative.target(Some("/w/app")), None);
        let out = one(claude("Write", &json!({"file_path": "/w/app/../../etc/passwd", "content": ""})));
        assert_eq!(out.target(Some("/w/app")), None);
        let all = json!({"file_path": "a.rs", "old_string": "x", "new_string": "y", "replace_all": true});
        let file = one(claude("Edit", &all));
        assert!(file.all && lines(&file) == ["-x", "+y"]);
        // Nothing in common, or nothing at all.
        let file = one(claude("Edit", &json!({"file_path": "a", "old_string": "", "new_string": "x\ny"})));
        assert_eq!(lines(&file), ["+x", "+y"]);
        let same = one(claude("Edit", &json!({"file_path": "a", "old_string": "x\nx", "new_string": "x\nx"})));
        assert_eq!((lines(&same).len(), same.added, same.removed), (2, 0, 0));
    }

    #[test]
    fn a_write_is_all_added_and_a_multi_edit_one_file() {
        let write = one(claude("Write", &json!({"file_path": "/w/app/new.md", "content": "# Title\n\nbody\n"})));
        assert_eq!((lines(&write), write.change), (vec!["+# Title".into(), "+".into(), "+body".into()], Change::Write));
        let multi = json!({"file_path": "/w/app/a.rs", "edits": [
            {"old_string": "one", "new_string": "1"},
            {"old_string": "two", "new_string": "2", "replace_all": true},
            {"old_string": 3},
        ]});
        let multi = one(claude("MultiEdit", &multi));
        assert_eq!(lines(&multi), ["-one", "+1", "@", "-two", "+2"]);
        assert!(multi.all);
        let cell = json!({"notebook_path": "/w/n.ipynb", "new_source": "print(1)", "edit_mode": "replace"});
        assert_eq!(lines(&one(claude("NotebookEdit", &cell))), ["+print(1)"]);
        let gone = one(claude("NotebookEdit", &json!({"notebook_path": "/w/n.ipynb", "edit_mode": "delete"})));
        assert_eq!((gone.change, gone.lines.len()), (Change::Delete, 0));
        // Not an edit: no card.
        assert_eq!(claude("Bash", &json!({"command": "ls"})), None);
        assert_eq!(claude("Read", &json!({"file_path": "/w/a"})), None);
    }

    #[test]
    fn codex_changes_and_patches_become_cards() {
        // As seen in the attach probe: an added file's `diff` is its content.
        let changes = json!([
            {"path": "/w/app/a.txt", "kind": {"type": "add"}, "diff": "1\n"},
            {"path": "/w/app/b.rs", "kind": {"type": "update", "move_path": "/w/app/c.rs"},
             "diff": "--- a/b.rs\n+++ b/c.rs\n@@ -1,2 +1,2 @@\n keep\n-old\n+new\n\\ No newline at end of file\n"},
            {"path": "/w/app/d.rs", "kind": {"type": "delete"}, "diff": "gone\n"},
            {"path": "/w/app/e.rs", "kind": {"type": "rename?"}},
        ]);
        let diff = codex_changes(&changes).expect("a card");
        let kinds: Vec<Change> = diff.files.iter().map(|f| f.change).collect();
        assert_eq!(kinds, [Change::Add, Change::Edit, Change::Delete]);
        assert_eq!(lines(&diff.files[0]), ["+1"]);
        assert_eq!(lines(&diff.files[1]), ["@@@ -1,2 +1,2 @@", " keep", "-old", "+new"]);
        assert_eq!(diff.files[1].to.as_ref().map(Text::text), Some("/w/app/c.rs"));
        assert_eq!(lines(&diff.files[2]), ["-gone"]);
        let patch = "*** Begin Patch\n*** Update File: src/main.rs\n*** Move to: src/lib.rs\n@@ fn main\n ctx\n-a\n+b\n\
                     *** Add File: notes.txt\n+hello\n*** Delete File: old.txt\n*** End Patch\n";
        let diff = codex_patch(patch).expect("a card");
        let shown: Vec<(String, Change)> = diff.files.iter().map(|f| (f.shown(None), f.change)).collect();
        let names = [
            ("src/main.rs".into(), Change::Edit),
            ("notes.txt".into(), Change::Add),
            ("old.txt".into(), Change::Delete),
        ];
        assert_eq!(shown, names);
        assert_eq!(lines(&diff.files[0]), ["@@@ fn main", " ctx", "-a", "+b"]);
        assert_eq!(diff.files[0].to.as_ref().map(Text::text), Some("src/lib.rs"));
        assert_eq!(codex_patch("garbage\n+x"), None, "no file named");
    }

    #[test]
    fn caps_keep_forty_lines_and_count_the_rest() {
        let big = (0..10_000).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
        let file = one(claude("Write", &json!({"file_path": "/w/big", "content": big})));
        assert_eq!((file.lines.len(), file.more, file.added), (MAX_LINES, 10_000 - MAX_LINES, 10_000));
        // The room is the card's, over its edits and files.
        let edit = json!({"file_path": "/w/a", "old_string": big, "new_string": "x"});
        let file = one(claude("Edit", &edit));
        assert_eq!((file.lines.len(), file.removed, file.added), (MAX_LINES, 10_000, 1));
        let changes: Vec<Value> = (0..20)
            .map(|n| json!({"path": format!("/w/{n}"), "kind": {"type": "add"}, "diff": "a\nb\nc\nd\ne"}))
            .collect();
        let diff = codex_changes(&Value::Array(changes)).unwrap();
        let shape = diff.shape();
        assert_eq!((diff.files.len(), diff.more_files), (MAX_FILES, 12));
        assert_eq!((shape.lines, shape.lines + shape.more, shape.added), (MAX_LINES, 5 * MAX_FILES, 5 * MAX_FILES));
        // A long line is cut at a character, and says so.
        let long = "é".repeat(MAX_LINE);
        let file = one(claude("Write", &json!({"file_path": "/w/a", "content": long})));
        let text = file.lines[0].text.text();
        assert!(text.ends_with('…') && text.len() <= MAX_LINE + '…'.len_utf8(), "{}", text.len());
        // A 50 MB string: kept to the cap, its lines counted.
        let huge = format!("{}\n", "y".repeat(99)).repeat(500_000);
        let file = one(claude("Write", &json!({"file_path": "/w/a", "content": huge})));
        assert_eq!((file.lines.len(), file.added), (MAX_LINES, 500_000));
        assert!(Diff { files: vec![file], more_files: 0 }.bytes() < 16 << 10);
    }

    #[test]
    fn text_is_cleaned_and_never_printed() {
        let input = json!({"file_path": "/w/a\u{202e}b.rs", "content": "x\u{1b}[2Jy\u{202e}z\tw\u{7}"});
        let file = one(claude("Write", &input));
        assert_eq!(lines(&file), ["+xy\\u{202e}z    w"]);
        assert_eq!(file.shown(None), "/w/a\\u{202e}b.rs");
        let debug = format!("{:?}", claude("Write", &json!({"file_path": "/secret/path", "content": "secret words"})));
        assert!(!debug.contains("secret"), "{debug}");
        let shape = serde_json::to_string(&one(claude("Write", &input)).change).unwrap();
        assert_eq!(shape, r#""write""#);
    }

    #[test]
    fn malformed_inputs_give_no_card_and_never_panic() {
        let bad = [
            json!(null),
            json!({}),
            json!({"file_path": 3, "old_string": "a", "new_string": "b"}),
            json!({"file_path": "/a", "old_string": "a"}),
            json!({"file_path": "", "content": "x"}),
            json!({"file_path": "x".repeat(MAX_PATH + 1), "content": "x"}),
            json!({"file_path": "/a", "edits": "no"}),
            json!({"file_path": "/a", "content": ["x"]}),
        ];
        for input in &bad {
            for tool in ["Edit", "MultiEdit", "Write", "NotebookEdit"] {
                assert_eq!(claude(tool, input), None, "{tool} {input}");
            }
            assert_eq!(codex_changes(input), None, "{input}");
        }
        assert_eq!(codex_changes(&json!([{"path": 1, "kind": {"type": "add"}}, "x", null])), None);
        for patch in ["", "*** Update File: ", "*** Add File: a\n\u{0}\u{ffff}+\n-\n@@", "+\n-\n \n@\n\\"] {
            let _ = codex_patch(patch);
        }
        // Lines that are only a sign, a lone `\r`, a multi-byte first character.
        let file = one(codex_changes(&json!([{"path": "/a", "kind": {"type": "update"}, "diff": "+\n-\n\r\né\n@"}])));
        assert_eq!(lines(&file), ["+", "-", " ", " é", " @"]);
    }
}
