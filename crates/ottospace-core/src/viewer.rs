//! The file viewer's rules: what a file is shown as, what the shell read of it, where the view
//! is, and the path under a ⌘-click in a terminal. Pure: the shell reads and watches the file,
//! and never puts its text in the state. Design: `docs/ai/file-viewer.md`.

use serde::{Deserialize, Serialize};

use crate::model::PaneId;
use crate::vim::Keys;

/// The most bytes the viewer reads of a file: past it, "too large".
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;
/// The longest path a viewer opens.
pub const MAX_PATH: usize = 4096;

/// What a file is shown as, by its name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shows {
    /// Drawn by the conversation's markdown renderer.
    Markdown,
    /// Source code: lines with their numbers (colours come with the markdown pass).
    Code,
    /// Any other text: lines with their numbers.
    #[default]
    Text,
    /// A PNG, JPEG, GIF or WebP, drawn as a picture.
    Image,
}

const MARKDOWN: &[&str] = &["md", "markdown", "mdx", "mdown"];
const IMAGES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];
const CODE: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "rb", "go", "swift", "kt", "java", "c", "h", "cc", "cpp",
    "hpp", "m", "mm", "cs", "sh", "bash", "zsh", "fish", "json", "jsonl", "yaml", "yml", "toml", "sql", "diff",
    "patch", "html", "css", "scss", "xml", "svg", "lua", "php", "pl", "r", "scala", "zig", "nix", "proto", "graphql",
    "vue", "svelte",
];
const TEXT: &[&str] = &["txt", "text", "log", "csv", "tsv", "ini", "cfg", "conf", "env", "lock", "gitignore"];
/// Files known by their whole name.
const NAMED: &[&str] = &["Makefile", "Dockerfile", "LICENSE", "README", "CHANGELOG", "Gemfile", "Rakefile", "Procfile"];

fn extension(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() || name.starts_with('.')).then(|| ext.to_ascii_lowercase())
}

/// What `path` is shown as. A name it does not know is text: the shell says "binary" if it is not.
pub fn shows(path: &str) -> Shows {
    match extension(path) {
        Some(ext) if MARKDOWN.contains(&ext.as_str()) => Shows::Markdown,
        Some(ext) if IMAGES.contains(&ext.as_str()) => Shows::Image,
        Some(ext) if CODE.contains(&ext.as_str()) => Shows::Code,
        _ => Shows::Text,
    }
}

/// True if a ⌘-click on `path` opens it in the viewer: a name known as markdown, code or text.
/// Any other file opens as it did before (a chat's link: revealed in Finder).
pub fn opens_on_click(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let known = |list: &[&str]| extension(path).is_some_and(|ext| list.contains(&ext.as_str()));
    known(MARKDOWN) || known(CODE) || known(TEXT) || NAMED.contains(&name)
}

/// A path the viewer may be asked for: absolute, one line, not too long.
pub fn sound(path: &str) -> bool {
    path.starts_with('/') && path.len() <= MAX_PATH && !path.chars().any(char::is_control)
}

/// What the shell read of the file, never its text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Read {
    /// Not read yet.
    #[default]
    Reading,
    /// Read and drawn: its lines (an image has none).
    Shown { lines: u32 },
    /// "File not found".
    Missing,
    /// Larger than `MAX_BYTES`.
    TooLarge { bytes: u64 },
    /// A folder, a device, a pipe: only a regular file is read.
    NotFile,
    /// Text with a NUL byte or not UTF-8, or an image its first bytes do not make one.
    Binary,
    /// The file is there but could not be read (its permissions).
    Unreadable,
}

/// A viewer pane's state, by pane. Its path is the pane's `Kind::File`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Viewer {
    pub shows: Shows,
    pub read: Read,
    /// The first line on show, from 0: a line of the file for code and text, a row of text's
    /// height for markdown.
    pub top: u32,
    /// The last place `top` may be, as the shell laid the file out; `None` until it has.
    pub most: Option<u32>,
    /// Lines on show.
    pub page: u32,
    /// What is typed of a move (`10`, `g`), shown in the status bar.
    pub pending: Keys,
    /// The pane that opened it to be reused (`OpenFile`'s `reuse`): its next file goes here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opener: Option<PaneId>,
    /// The lines it was opened at, drawn marked: within the file once read. Until the pane is
    /// asked for another place or file, or Esc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marked: Option<Lines>,
}

impl Viewer {
    pub fn new(path: &str, top: u32, opener: Option<PaneId>) -> Self {
        Viewer { shows: shows(path), top, opener, ..Viewer::default() }
    }

    /// A new view of `path` at `at`: a markdown file shows its source, so the lines can be marked.
    pub fn at(path: &str, at: Option<Lines>, opener: Option<PaneId>) -> Self {
        let mut view = Viewer::new(path, 0, opener);
        if at.is_some() && view.shows == Shows::Markdown {
            view.shows = Shows::Code;
        }
        view.mark(at);
        view
    }

    /// `at` marked and put in view, `CONTEXT` lines below the top; past the end once read, the
    /// last line. `None`: no mark, the view stays.
    pub fn mark(&mut self, at: Option<Lines>) {
        let Some(at) = at else { return };
        let at = match self.read {
            Read::Shown { lines } => at.within(lines),
            _ => Some(at),
        };
        if let Some(at) = at {
            self.top = at.first.saturating_sub(1 + CONTEXT);
            self.top = self.most.map_or(self.top, |most| self.top.min(most));
        }
        self.marked = at;
    }
}

/// Lines shown above a marked range when the view goes to it.
pub const CONTEXT: u32 = 3;

/// Lines of a file, from 1, both ends in: `12` is `12-12`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lines {
    pub first: u32,
    pub last: u32,
}

impl Lines {
    /// `None` for line 0, which no file has. Written backwards (`14-12`), the same lines.
    pub fn new(first: u32, last: u32) -> Option<Lines> {
        let (first, last) = (first.min(last), first.max(last));
        (first > 0).then_some(Lines { first, last })
    }

    /// Cut to a file of `lines`: past its end, its last line. `None`: an empty file.
    pub fn within(self, lines: u32) -> Option<Lines> {
        Lines::new(self.first.min(lines), self.last.min(lines))
    }

    /// `12` or `12-14`. `None`: anything else, or line 0.
    pub fn parse(text: &str) -> Option<Lines> {
        let (first, last) = text.split_once('-').unwrap_or((text, text));
        let number = |part: &str| digits(part).then(|| part.parse().ok()).flatten();
        Lines::new(number(first)?, number(last)?)
    }
}

fn digits(part: &str) -> bool {
    !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())
}

/// `path` without a line written after it, and those lines: `a.rs:12` (`:12:3`, its column
/// dropped), `a.rs:12-14`. Line 0 is dropped and names none.
pub fn split_lines(path: &str) -> (&str, Option<Lines>) {
    let (mut path, mut lines) = (path, None);
    for _ in 0..2 {
        let Some((before, tail)) = path.rsplit_once(':') else { break };
        let (first, last) = tail.split_once('-').unwrap_or((tail, tail));
        if !(digits(first) && digits(last)) {
            break;
        }
        // Of two, the first is the line: `:12:3` is line 12.
        (path, lines) = (before, Lines::parse(tail));
    }
    (path, lines)
}

/// The path-like word at `col` of a terminal's `line`, as written, and the lines after it:
/// `src/main.rs:12:3` gives `src/main.rs` and line 12; quotes, brackets and a trailing `.` or `,`
/// are not of it. `None`: no word there, or one that names no file (`http://…`).
pub fn path_at(line: &str, col: usize) -> Option<(String, Option<Lines>)> {
    let chars: Vec<char> = line.chars().collect();
    let part = |c: char| !c.is_whitespace() && !matches!(c, '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '<' | '>' | '|');
    if !chars.get(col).is_some_and(|c| part(*c)) {
        return None;
    }
    let start = (0..col).rev().take_while(|at| part(chars[*at])).last().unwrap_or(col);
    let end = (col..chars.len()).take_while(|at| part(chars[*at])).last().map_or(col, |at| at + 1);
    let word: String = chars[start..end].iter().collect();
    let word = word.trim_end_matches(['.', ',', ';', ':']);
    if word.contains("://") {
        return None;
    }
    let (path, lines) = split_lines(word);
    (!path.is_empty() && !path.contains(':')).then(|| (path.to_string(), lines))
}

/// `typed` as an absolute path: as is, from `home` for `~/…`, else from `cwd`. `.` and `..`
/// are left for the disk to resolve. `None`: relative with no folder to start from.
pub fn absolute(typed: &str, cwd: Option<&str>, home: Option<&str>) -> Option<String> {
    let joined = match typed {
        _ if typed.starts_with('/') => typed.to_string(),
        "~" => home?.to_string(),
        _ if typed.starts_with("~/") => format!("{}/{}", home?.trim_end_matches('/'), &typed[2..]),
        _ => format!("{}/{typed}", cwd.filter(|dir| dir.starts_with('/'))?.trim_end_matches('/')),
    };
    sound(&joined).then_some(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_shown_by_its_name() {
        assert_eq!(shows("/a/README.md"), Shows::Markdown);
        assert_eq!(shows("/a/Notes.MARKDOWN"), Shows::Markdown);
        assert_eq!(shows("/a/src/main.rs"), Shows::Code);
        assert_eq!(shows("/a/shot.PNG"), Shows::Image);
        assert_eq!(
            (shows("/a/LICENSE"), shows("/a/notes.txt"), shows("/a/.md")),
            (Shows::Text, Shows::Text, Shows::Markdown)
        );
        assert_eq!(shows("/a.b/file"), Shows::Text, "a dot in a folder is no extension");
    }

    #[test]
    fn a_click_opens_text_in_the_viewer_and_anything_else_as_before() {
        assert!(opens_on_click("/a/README.md") && opens_on_click("/a/x.rs") && opens_on_click("/a/Makefile"));
        assert!(opens_on_click("/a/build.log"));
        assert!(!opens_on_click("/a/shot.png") && !opens_on_click("/a/doc.pdf") && !opens_on_click("/a/bin"));
    }

    #[test]
    fn only_an_absolute_one_line_path_is_sound() {
        assert!(sound("/a/b.md"));
        assert!(!sound("a/b.md") && !sound("/a\nb") && !sound(&format!("/{}", "a".repeat(MAX_PATH))));
    }

    #[test]
    fn the_path_under_a_click() {
        let path = |line: &str, col| path_at(line, col).map(|(path, _)| path);
        let line = "error: src/main.rs:12:3: expected `;`";
        assert_eq!(path_at(line, 9), Some(("src/main.rs".into(), Lines::new(12, 12))));
        assert_eq!(path(line, 20).as_deref(), Some("src/main.rs"), "on its line number too");
        assert_eq!(path_at(line, 6), None, "a space");
        assert_eq!(path_at("see a.rs:12-14.", 5), Some(("a.rs".into(), Lines::new(12, 14))));
        assert_eq!(path_at("a.rs:0", 0), Some(("a.rs".into(), None)), "line 0 is none");
        assert_eq!(path("see (docs/a.md).", 7).as_deref(), Some("docs/a.md"));
        assert_eq!(path("open 'My.md', now", 7).as_deref(), Some("My.md"));
        assert_eq!(path_at("https://example.com/a.md", 3), None);
        assert_eq!(path("~/notes/today.md", 0).as_deref(), Some("~/notes/today.md"));
        assert_eq!(path_at("a:b", 0), None);
        assert_eq!(path_at("x", 9), None, "past the end");
    }

    #[test]
    fn lines_are_from_one_both_ends_in() {
        assert_eq!(Lines::parse("12"), Some(Lines { first: 12, last: 12 }));
        assert_eq!(Lines::parse("14-12"), Some(Lines { first: 12, last: 14 }), "backwards, the same lines");
        for bad in ["0", "0-3", "", "-", "12-", "a", "1-2-3", "99999999999"] {
            assert_eq!(Lines::parse(bad), None, "{bad}");
        }
        let at = Lines { first: 40, last: 60 };
        assert_eq!((at.within(50), at.within(10), at.within(0)), (Lines::new(40, 50), Lines::new(10, 10), None));
        assert_eq!(split_lines("/a.rs:3:9"), ("/a.rs", Lines::new(3, 3)));
        assert_eq!(split_lines("/a.rs:x"), ("/a.rs:x", None));
    }

    #[test]
    fn a_view_opened_at_lines_marks_them_with_some_above() {
        let view = Viewer::at("/a/x.rs", Lines::new(40, 42), None);
        assert_eq!((view.top, view.marked, view.shows), (36, Lines::new(40, 42), Shows::Code));
        assert_eq!(Viewer::at("/a/x.rs", Lines::new(2, 2), None).top, 0, "near the top: from the top");
        // Markdown opened at a line shows its source; without one, drawn.
        assert_eq!(
            (Viewer::at("/a/x.md", Lines::new(1, 1), None).shows, Viewer::at("/a/x.md", None, None).shows),
            (Shows::Code, Shows::Markdown)
        );
        // Read: past the end is the last line, an empty file has none.
        let mut view = Viewer { read: Read::Shown { lines: 10 }, most: Some(4), ..Viewer::default() };
        view.mark(Lines::new(30, 30));
        assert_eq!((view.marked, view.top), (Lines::new(10, 10), 4));
        view.read = Read::Shown { lines: 0 };
        view.mark(Lines::new(3, 3));
        assert_eq!((view.marked, view.top), (None, 4));
    }

    #[test]
    fn a_typed_path_is_made_absolute() {
        assert_eq!(absolute("src/a.rs", Some("/w/app/"), None).as_deref(), Some("/w/app/src/a.rs"));
        assert_eq!(absolute("/etc/hosts", None, None).as_deref(), Some("/etc/hosts"));
        assert_eq!(absolute("~/n.md", None, Some("/Users/me")).as_deref(), Some("/Users/me/n.md"));
        assert_eq!((absolute("a.rs", None, None), absolute("a.rs", Some("rel"), None)), (None, None));
        assert_eq!(absolute("~/n.md", Some("/w"), None), None);
    }
}
