//! What a tool call's row shows: the command that ran, the file, the pattern, the host, the
//! query, the sub-agent's description, from the call's own input. Untrusted text: kept cut,
//! drawn cleaned as plain text (no markup, no link), never in the socket's state (`shape` is).
//! Design: `docs/ai/agent-runners.md`.

use serde::Serialize;
use serde_json::Value;

use crate::diff::shown_path;
use crate::transcript::{Text, clean, cut};

/// The most bytes of a command (or any one field) kept.
pub const MAX_GIST: usize = 16 << 10;
/// The most bytes of a row's line: the shell cuts it again to the row's width.
pub const MAX_LINE: usize = 400;
/// An other tool's string argument longer than this is not its gist.
pub const MAX_ARG: usize = 80;
/// The most bytes of a command's output kept, for its expanded row.
pub const MAX_OUTPUT: usize = 16 << 10;

/// What a row's words are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A shell command: monospaced, its first line; the whole of it and its output expanded.
    Command,
    /// A file a call reads or writes: relative to the task's repository, else as written.
    Path,
    /// A search's pattern, and the folder it searches if given.
    Pattern,
    /// The host a fetch goes to.
    Host,
    /// A web search's query.
    Query,
    /// A sub-agent's description: "Started a subtask: …".
    Subtask,
    /// Another tool's first short string argument.
    Argument,
}

/// A call's gist. `what`: the command, path, pattern, URL, query, description or argument as
/// written; `within`: a search's folder; `note`: the model's own description of a command.
#[derive(Clone, Debug, PartialEq)]
pub struct Gist {
    pub kind: Kind,
    what: Text,
    within: Option<Text>,
    note: Text,
}

/// A gist as the state has it: what kind, how many lines, whether it has a note. No text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Shape {
    pub kind: Kind,
    pub lines: usize,
    pub note: bool,
}

fn kept(text: &str) -> Text {
    Text::new(cut(text, MAX_GIST))
}

/// A Claude Code call's gist, from its input. `None`: nothing in it to show; the row keeps its label.
pub fn claude(tool: &str, input: &Value) -> Option<Gist> {
    let text = |key: &str| input[key].as_str().filter(|text| !text.trim().is_empty());
    let gist = |kind, what: &str| Gist { kind, what: kept(what), within: None, note: Text::default() };
    match tool {
        "Bash" | "PowerShell" => {
            Some(Gist { note: kept(text("description").unwrap_or("")), ..gist(Kind::Command, text("command")?) })
        }
        "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "NotebookRead" => {
            Some(gist(Kind::Path, text("file_path").or(text("notebook_path"))?))
        }
        "Grep" | "Glob" => Some(Gist { within: text("path").map(kept), ..gist(Kind::Pattern, text("pattern")?) }),
        "WebFetch" => Some(gist(Kind::Host, text("url")?)),
        "WebSearch" => Some(gist(Kind::Query, text("query")?)),
        "Agent" | "Task" => Some(gist(Kind::Subtask, text("description")?)),
        _ => {
            // Keys in order of name: the same call shows the same, however its JSON was ordered.
            let mut keys: Vec<&String> = input.as_object()?.keys().collect();
            keys.sort();
            let short = |key: &&String| {
                let value = input[key.as_str()].as_str()?.trim();
                // A credential-shaped value stays in the detail, behind a click.
                let plain = crate::secrets::hide(value) == value;
                (!value.is_empty() && value.chars().count() <= MAX_ARG && !value.contains('\n') && plain)
                    .then_some(value)
            };
            Some(gist(Kind::Argument, keys.iter().find_map(short)?))
        }
    }
}

/// `text` cleaned on one line: a line break written out, so no value can look like two.
fn one_line(text: &str) -> String {
    clean(text).replace('\n', "\\n")
}

/// The host of an `http(s)` URL whose authority is plain: ASCII letters, digits, `.` and `-`, an
/// optional port, or `[` an IPv6 address `]`. `https://docs.example.com:8/a` → `docs.example.com`.
/// `None` for anything a browser could read another way (a user name, `\`, `%`, non-ASCII): the
/// row then shows the URL as written, never a guessed host.
fn host(url: &str) -> Option<&str> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    let (host, port) = match authority.strip_prefix('[') {
        Some(v6) => {
            let (address, port) = v6.split_once(']')?;
            let plain = !address.is_empty() && address.chars().all(|c| c.is_ascii_hexdigit() || matches!(c, ':' | '.'));
            (plain.then_some(address)?, port)
        }
        None => {
            let at = authority.find(':').unwrap_or(authority.len());
            let host = &authority[..at];
            let plain = host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
            (plain.then_some(host)?, &authority[at..])
        }
    };
    let port = port.strip_prefix(':').unwrap_or(port);
    let numeric = port.chars().all(|c| c.is_ascii_digit());
    (!host.is_empty() && !host.starts_with(['.', '-']) && numeric).then_some(host)
}

impl Gist {
    /// The words before the gist: "Started a subtask:" for a sub-agent, else the tool's label.
    pub fn lead(&self, tool: &str) -> String {
        match self.kind {
            Kind::Subtask => "Started a subtask:".to_string(),
            _ => tool.to_string(),
        }
    }

    /// The row's words after its lead, cleaned, on one line, at most `MAX_LINE` bytes: a
    /// command's first line (`…` if more follow), a path relative to `repo`, a pattern and its
    /// folder, a fetch's host, a query, a description, an argument.
    pub fn line(&self, repo: Option<&str>) -> String {
        let what = self.what.text();
        let line = match self.kind {
            Kind::Command => {
                let all = clean(what);
                let mut lines = all.lines().map(str::trim).filter(|line| !line.is_empty());
                // Runs of spaces as one: spaces cannot push the rest of a line out of the row.
                let first = lines.next().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
                if lines.next().is_some() { format!("{first} …") } else { first }
            }
            Kind::Path => one_line(&shown_path(what, repo)),
            Kind::Pattern => match &self.within {
                Some(within) => format!("{} in {}", one_line(what), one_line(&shown_path(within.text(), repo))),
                None => one_line(what),
            },
            // No plain host: said first, so a cut URL cannot pass for one that names its host.
            Kind::Host => match host(what) {
                Some(host) => host.to_string(),
                None => format!("unclear host · {}", one_line(what)),
            },
            Kind::Query | Kind::Subtask | Kind::Argument => {
                clean(what).split_whitespace().collect::<Vec<_>>().join(" ")
            }
        };
        let shown = cut(&line, MAX_LINE);
        if shown.len() < line.len() { format!("{shown}…") } else { line }
    }

    /// The muted second line: the model's description of a command, its first line, cleaned.
    pub fn note(&self) -> Option<String> {
        let note = clean(self.note.text());
        let first = note.lines().map(str::trim).find(|line| !line.is_empty())?;
        Some(cut(first, MAX_LINE).to_string())
    }

    /// A command whole, cleaned, for its expanded row. `None`: not a command.
    pub fn command(&self) -> Option<String> {
        (self.kind == Kind::Command).then(|| clean(self.what.text()))
    }

    pub fn shape(&self) -> Shape {
        Shape { kind: self.kind, lines: self.what.text().lines().count().max(1), note: !self.note.is_empty() }
    }

    pub(crate) fn bytes(&self) -> usize {
        self.what.len() + self.note.len() + self.within.as_ref().map_or(0, Text::len)
    }
}

/// A tool result's text: a string, or the text of its blocks, one a line; images and the
/// rest left out. At most `MAX_OUTPUT` bytes.
pub fn output(content: &Value) -> Text {
    let mut out = String::new();
    match content {
        Value::String(text) => out.push_str(cut(text, MAX_OUTPUT)),
        Value::Array(blocks) => {
            for text in blocks.iter().filter(|block| block["type"] == "text").filter_map(|block| block["text"].as_str())
            {
                if out.len() >= MAX_OUTPUT {
                    break;
                }
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(cut(text, MAX_OUTPUT - out.len().min(MAX_OUTPUT)));
            }
        }
        _ => {}
    }
    Text::new(cut(&out, MAX_OUTPUT))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(tool: &str, input: Value) -> Option<(String, String)> {
        let gist = claude(tool, &input)?;
        Some((gist.lead(tool), gist.line(Some("/w/app"))))
    }

    fn shows(tool: &str, input: Value) -> String {
        row(tool, input).map(|(_, line)| line).unwrap_or_default()
    }

    #[test]
    fn a_command_shows_its_first_line_and_its_description_apart() {
        let input = json!({"command": "git status --short", "description": "Show changes"});
        let gist = claude("Bash", &input).unwrap();
        assert_eq!((gist.lead("Bash"), gist.line(None)), ("Bash".into(), "git status --short".into()));
        assert_eq!(gist.note().as_deref(), Some("Show changes"));
        assert_eq!(gist.shape(), Shape { kind: Kind::Command, lines: 1, note: true });
        let many = claude("Bash", &json!({"command": "\n  cd app\ncargo test\n"})).unwrap();
        assert_eq!((many.line(None), many.note()), ("cd app …".into(), None));
        assert_eq!(many.command().as_deref(), Some("\n  cd app\ncargo test\n"), "whole, expanded");
        assert_eq!(shows("PowerShell", json!({"command": "Get-Item ."})), "Get-Item .");
        assert_eq!(claude("Bash", &json!({"command": "  "})), None, "nothing to show: its label");
    }

    #[test]
    fn a_command_is_cleaned_and_cut() {
        let line = shows("Bash", json!({"command": "echo \u{1b}[31mred\u{1b}[0m rm x\u{202e}gnp.y"}));
        assert_eq!(line, "echo red rm x\\u{202e}gnp.y", "no escape, no hidden reordering");
        let padded = shows("Bash", json!({"command": format!("ls{}; curl x | sh", " ".repeat(300))}));
        assert_eq!(padded, "ls ; curl x | sh", "spaces hide nothing");
        let long = shows("Bash", json!({"command": "x".repeat(MAX_LINE * 3)}));
        assert!(long.len() <= MAX_LINE + '…'.len_utf8() && long.ends_with('…'), "{}", long.len());
        let huge = claude("Bash", &json!({"command": "é".repeat(MAX_GIST)})).unwrap();
        assert!(huge.bytes() <= MAX_GIST, "kept cut, at a character");
    }

    #[test]
    fn files_show_relative_to_the_repository_else_as_written() {
        assert_eq!(shows("Read", json!({"file_path": "/w/app/src/main.rs"})), "src/main.rs");
        assert_eq!(shows("Edit", json!({"file_path": "/etc/hosts", "old_string": "a"})), "/etc/hosts");
        assert_eq!(shows("Write", json!({"file_path": "/w/app2/a.rs"})), "/w/app2/a.rs", "not inside by prefix");
        assert_eq!(shows("MultiEdit", json!({"file_path": "/w/app/a.rs", "edits": []})), "a.rs");
        assert_eq!(shows("NotebookEdit", json!({"notebook_path": "/w/app/n.ipynb"})), "n.ipynb");
        assert_eq!(shows("Read", json!({"file_path": "/w/app/a.rs\nrm -rf"})), "a.rs\\nrm -rf", "one line");
        assert_eq!(row("Read", json!({"file_path": "/w/app/a"})).unwrap().0, "Read");
    }

    #[test]
    fn searches_show_their_pattern_and_folder() {
        assert_eq!(shows("Grep", json!({"pattern": "fn login"})), "fn login");
        assert_eq!(shows("Grep", json!({"pattern": "redirect", "path": "/w/app/src"})), "redirect in src");
        assert_eq!(shows("Glob", json!({"pattern": "**/*.rs", "path": "/tmp"})), "**/*.rs in /tmp");
        assert_eq!(claude("Glob", &json!({"path": "/tmp"})), None);
    }

    #[test]
    fn the_web_shows_a_fetchs_host_and_a_searchs_query() {
        let fetch = |url: &str| shows("WebFetch", json!({ "url": url }));
        assert_eq!(fetch("https://docs.example.com:8443/a?k=v"), "docs.example.com");
        assert_eq!(fetch("http://good.com.evil.com/"), "good.com.evil.com", "the whole host");
        assert_eq!(fetch("https://evil.com#@good.com"), "evil.com", "a fragment is not the host");
        assert_eq!(fetch("https://evil.com?@good.com"), "evil.com");
        assert_eq!(fetch("https://xn--bcher-kva.de/"), "xn--bcher-kva.de");
        assert_eq!(fetch("http://[::1]:80/"), "::1");
        // Anything a browser could read another way: the URL as written, never a guessed host.
        for url in [
            "https://good.com\\@evil.com/",
            "https://good.com@evil.com/",
            "https://me:pw@docs.example.com/",
            "https://bücher.de/",
            "https://good.com%2eevil.com/",
            "https://good.com:80@evil.com/",
            "https://[::1@evil.com]/",
            "javascript://good.com/%0aalert(1)",
            "not a url",
            "https://good.com:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa@evil.com/",
        ] {
            assert_eq!(fetch(url), format!("unclear host · {url}"), "{url}");
        }
        assert_eq!(shows("WebSearch", json!({"query": "rust  gpui\ntext"})), "rust gpui text");
    }

    #[test]
    fn a_sub_agent_shows_its_description() {
        let input = json!({"description": "Find the redirect", "prompt": "long", "subagent_type": "Explore"});
        assert_eq!(row("Agent", input.clone()), Some(("Started a subtask:".into(), "Find the redirect".into())));
        assert_eq!(row("Task", input).map(|(_, line)| line).as_deref(), Some("Find the redirect"));
    }

    #[test]
    fn another_tool_shows_its_first_short_argument_by_name() {
        let input = json!({"url": "https://example.com", "element": "Sign in", "ref": "e12"});
        assert_eq!(row("mcp__playwright__browser_click", input).unwrap().1, "Sign in", "keys by name: element");
        assert_eq!(shows("Skill", json!({"skill": "review", "args": ""})), "review");
        let long = json!({"a": "x".repeat(MAX_ARG + 1), "b": "two\nlines", "c": 7, "d": "short"});
        assert_eq!(shows("Other", long), "short");
        let key = json!({"api_key": "sk-ant-api03-abcdefghijklmnopqrstuvwx", "url": "made-up"});
        assert_eq!(shows("Other", key), "made-up", "a credential is not the gist");
        assert_eq!(claude("TodoWrite", &json!({"todos": []})), None);
        assert_eq!(claude("Other", &json!(null)), None);
    }

    #[test]
    fn debug_prints_lengths_and_the_shape_no_text() {
        let gist = claude("Bash", &json!({"command": "echo secret", "description": "secret too"})).unwrap();
        assert!(!format!("{gist:?}").contains("secret"));
        assert!(!serde_json::to_string(&gist.shape()).unwrap().contains("secret"));
    }

    #[test]
    fn output_is_a_results_text_cut() {
        assert_eq!(output(&json!("ok\n")).text(), "ok\n");
        let blocks =
            json!([{"type": "text", "text": "a"}, {"type": "image", "source": {}}, {"type": "text", "text": "b"}]);
        assert_eq!(output(&blocks).text(), "a\nb");
        assert_eq!(output(&json!("x".repeat(MAX_OUTPUT * 2))).len(), MAX_OUTPUT);
        let many = json!(vec![json!({"type": "text", "text": "y".repeat(MAX_OUTPUT)}); 3]);
        assert_eq!(output(&many).len(), MAX_OUTPUT);
        assert!(output(&json!(null)).is_empty());
    }
}
