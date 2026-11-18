//! Colours for a fenced code block, by its fence's language: a small lexer per language
//! (comments, strings, numbers, keywords, types, keys), not a grammar. Pure.

use std::ops::Range;

use ottospace_core::theme;
use ottospace_core::transcript::cut as head;

/// What a run of code is, for its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Keyword,
    /// A capitalised name (`Point`), a TOML table, a shell variable.
    Type,
    String,
    Number,
    Comment,
    /// A JSON, YAML or TOML key.
    Key,
    /// A diff's lines.
    Added,
    Removed,
    Hunk,
}

/// A coloured run: its bytes, and what it is.
pub type Mark = (Range<usize>, Token);

/// `marks` of `text` split by line as `str::lines` splits it, each run's bytes within its line;
/// a run over several lines (a block comment) is cut into one for each.
/// None at all for no marks: plain text costs no list per line.
pub fn by_line(text: &str, marks: &[Mark]) -> Vec<Vec<Mark>> {
    let mut out = Vec::new();
    if marks.is_empty() {
        return out;
    }
    let (mut start, mut next) = (0, 0);
    for line in text.split_inclusive('\n') {
        let shown = line.trim_end_matches('\n').trim_end_matches('\r');
        let end = start + shown.len();
        // Runs before this line are done: each is looked at once, or once per line it spans.
        while marks.get(next).is_some_and(|(range, _)| range.end <= start) {
            next += 1;
        }
        let mut mine = Vec::new();
        for (range, token) in marks[next..].iter().take_while(|(range, _)| range.start < end) {
            let (from, to) = (range.start.max(start), range.end.min(end));
            if from < to {
                mine.push((from - start..to - start, *token));
            }
        }
        out.push(mine);
        start += line.len();
    }
    out
}

/// `marks` (of a line, in order) cut to `shown`, what is drawn of it: each end snapped down to a
/// character of `shown`, a run left empty dropped. Never a range that would split a character.
pub fn within(marks: &[Mark], shown: &str) -> Vec<Mark> {
    let snap = |mut at: usize| {
        at = at.min(shown.len());
        while !shown.is_char_boundary(at) {
            at -= 1;
        }
        at
    };
    marks
        .iter()
        .take_while(|(range, _)| range.start < shown.len())
        .filter_map(|(range, token)| {
            let (start, end) = (snap(range.start), snap(range.end));
            (start < end).then_some((start..end, *token))
        })
        .collect()
}

/// A token's colour, from the theme's hues.
pub fn colour(token: Token) -> u32 {
    match token {
        Token::Keyword => theme::VISUAL,
        Token::Type => theme::INSPECT,
        Token::String | Token::Added => theme::PICK,
        Token::Number => theme::APP,
        Token::Comment => theme::MUTED,
        Token::Key => theme::PRIMARY,
        Token::Removed => theme::ERROR,
        Token::Hunk => theme::MARK,
    }
}

/// How keys are written: before `:` as a string (JSON), a word at a line's start (YAML), before `=` (TOML).
#[derive(Clone, Copy, PartialEq)]
enum Keys {
    None,
    Json,
    Yaml,
    Toml,
}

struct Lang {
    line: &'static [&'static str],
    block: Option<(&'static str, &'static str)>,
    quotes: &'static [char],
    /// `"""` and `'''` strings (Python, TOML).
    triple: bool,
    /// `'a'` is a character, `'a` a lifetime (Rust, Go).
    chars: bool,
    keywords: &'static [&'static str],
    /// Keywords in any case (SQL).
    fold: bool,
    types: bool,
    keys: Keys,
    /// `$name` and `${name}` (shell).
    vars: bool,
}

const BASE: Lang = Lang {
    line: &[],
    block: None,
    quotes: &['"'],
    triple: false,
    chars: false,
    keywords: &[],
    fold: false,
    types: false,
    keys: Keys::None,
    vars: false,
};
const C_COMMENTS: (&[&str], Option<(&str, &str)>) = (&["//"], Some(("/*", "*/")));

const RUST: Lang = Lang {
    line: C_COMMENTS.0,
    quotes: &['"', '\''],
    block: C_COMMENTS.1,
    chars: true,
    types: true,
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn",
        "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self",
        "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "u8", "u16", "u32",
        "u64", "usize", "i8", "i16", "i32", "i64", "isize", "f32", "f64", "bool", "char", "str",
    ],
    ..BASE
};
const SCRIPT: Lang = Lang {
    line: C_COMMENTS.0,
    block: C_COMMENTS.1,
    quotes: &['"', '\'', '`'],
    types: true,
    keywords: &[
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "default",
        "delete",
        "do",
        "else",
        "export",
        "extends",
        "false",
        "finally",
        "for",
        "from",
        "function",
        "if",
        "import",
        "in",
        "instanceof",
        "interface",
        "let",
        "new",
        "null",
        "of",
        "return",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "true",
        "try",
        "type",
        "typeof",
        "undefined",
        "var",
        "void",
        "while",
        "yield",
        "string",
        "number",
        "boolean",
        "any",
        "unknown",
        "never",
    ],
    ..BASE
};
const PYTHON: Lang = Lang {
    line: &["#"],
    quotes: &['"', '\''],
    triple: true,
    types: true,
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except",
        "False", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "None", "nonlocal", "not",
        "or", "pass", "raise", "return", "True", "try", "while", "with", "yield", "float", "int", "str", "list",
        "dict", "bool",
    ],
    ..BASE
};
const SHELL: Lang = Lang {
    line: &["#"],
    quotes: &['"', '\''],
    vars: true,
    keywords: &[
        "if", "then", "else", "elif", "fi", "for", "in", "do", "done", "while", "until", "case", "esac", "function",
        "return", "set", "export", "local", "echo", "exit", "cd", "source",
    ],
    ..BASE
};
const JSON: Lang = Lang { keywords: &["true", "false", "null"], keys: Keys::Json, ..BASE };
const YAML: Lang = Lang {
    line: &["#"],
    quotes: &['"', '\''],
    keywords: &["true", "false", "null", "yes", "no", "on", "off"],
    keys: Keys::Yaml,
    ..BASE
};
const TOML: Lang =
    Lang { line: &["#"], quotes: &['"', '\''], triple: true, keywords: &["true", "false"], keys: Keys::Toml, ..BASE };
const GO: Lang = Lang {
    line: C_COMMENTS.0,
    block: C_COMMENTS.1,
    quotes: &['"', '`', '\''],
    chars: true,
    types: true,
    keywords: &[
        "break",
        "case",
        "chan",
        "const",
        "continue",
        "default",
        "defer",
        "else",
        "fallthrough",
        "for",
        "func",
        "go",
        "goto",
        "if",
        "import",
        "interface",
        "map",
        "package",
        "range",
        "return",
        "select",
        "struct",
        "switch",
        "type",
        "var",
        "true",
        "false",
        "nil",
        "string",
        "int",
        "bool",
        "error",
        "byte",
        "rune",
        "float64",
    ],
    ..BASE
};
const SWIFT: Lang = Lang {
    line: C_COMMENTS.0,
    block: C_COMMENTS.1,
    types: true,
    keywords: &[
        "as",
        "break",
        "case",
        "class",
        "continue",
        "default",
        "defer",
        "do",
        "else",
        "enum",
        "extension",
        "false",
        "for",
        "func",
        "guard",
        "if",
        "import",
        "in",
        "init",
        "let",
        "nil",
        "private",
        "protocol",
        "public",
        "return",
        "self",
        "static",
        "struct",
        "switch",
        "throw",
        "throws",
        "true",
        "try",
        "var",
        "while",
    ],
    ..BASE
};
const SQL: Lang = Lang {
    line: &["--"],
    block: C_COMMENTS.1,
    quotes: &['\''],
    fold: true,
    keywords: &[
        "select", "from", "where", "and", "or", "not", "order", "by", "group", "having", "limit", "offset", "insert",
        "into", "values", "update", "set", "delete", "create", "table", "drop", "alter", "join", "left", "right",
        "inner", "outer", "on", "as", "asc", "desc", "null", "is", "in", "like", "distinct", "union", "primary", "key",
    ],
    ..BASE
};

/// The language a fence's label names, if this file colours it.
fn lang(label: &str) -> Option<&'static Lang> {
    Some(match label.to_ascii_lowercase().as_str() {
        "rust" | "rs" => &RUST,
        "ts" | "typescript" | "tsx" | "js" | "javascript" | "jsx" | "mjs" | "cjs" => &SCRIPT,
        "python" | "py" => &PYTHON,
        "sh" | "bash" | "shell" | "zsh" => &SHELL,
        "json" | "jsonc" => &JSON,
        "yaml" | "yml" => &YAML,
        "toml" => &TOML,
        "go" | "golang" => &GO,
        "swift" => &SWIFT,
        "sql" => &SQL,
        _ => return None,
    })
}

/// The coloured runs of `code`, a block fenced as `label`, by byte range in order. None for a
/// language this file does not know.
pub fn marks(label: &str, code: &str) -> Vec<Mark> {
    if matches!(label.to_ascii_lowercase().as_str(), "diff" | "patch") {
        return diff(code);
    }
    lang(label).map_or_else(Vec::new, |lang| lex(lang, code))
}

/// A diff by line: `+` added, `-` removed, `@@` a hunk; its file names (`+++`, `---`) as hunks.
fn diff(code: &str) -> Vec<Mark> {
    let mut out = Vec::new();
    let mut at = 0;
    for line in code.split_inclusive('\n') {
        let body = line.trim_end_matches('\n');
        let token = if body.starts_with("+++") || body.starts_with("---") || body.starts_with("@@") {
            Some(Token::Hunk)
        } else if body.starts_with('+') {
            Some(Token::Added)
        } else if body.starts_with('-') {
            Some(Token::Removed)
        } else {
            None
        };
        if let Some(token) = token.filter(|_| !body.is_empty()) {
            out.push((at..at + body.len(), token));
        }
        at += line.len();
    }
    out
}

/// The longest `${name}` read as one.
const MAX_VAR: usize = 64;

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn lex(lang: &Lang, code: &str) -> Vec<Mark> {
    let mut out: Vec<Mark> = Vec::new();
    let mut i = 0;
    // Whether only spaces (and a YAML list's `- `) came before `i` on its line.
    let mut line_start = true;
    while i < code.len() {
        let rest = &code[i..];
        let Some(c) = rest.chars().next() else { break };
        let prev = code[..i].chars().next_back();
        let after_space = prev.is_none_or(char::is_whitespace);
        if c == '\n' {
            line_start = true;
            i += 1;
            continue;
        }
        // Comments: to the line's end, or the block's close. A `#` only after a space: `$#`, `a#b` are not.
        if lang.line.iter().any(|start| rest.starts_with(start) && (*start != "#" || after_space)) {
            let end = rest.find('\n').map_or(code.len(), |n| i + n);
            out.push((i..end, Token::Comment));
            i = end;
            continue;
        }
        if let Some((open, close)) = lang.block.filter(|(open, _)| rest.starts_with(open)) {
            let end = rest[open.len()..].find(close).map_or(code.len(), |n| i + open.len() + n + close.len());
            out.push((i..end, Token::Comment));
            i = end;
            continue;
        }
        if lang.quotes.contains(&c)
            && let Some(end) = string_end(lang, code, i, c)
        {
            let token = match lang.keys {
                Keys::Json if code[end..].trim_start_matches([' ', '\t']).starts_with(':') => Token::Key,
                Keys::Toml if line_start && code[end..].trim_start_matches([' ', '\t']).starts_with('=') => Token::Key,
                _ => Token::String,
            };
            out.push((i..end, token));
            line_start = false;
            i = end;
            continue;
        }
        // A TOML table: `[package]` alone on its line.
        if lang.keys == Keys::Toml && line_start && c == '[' {
            let end = rest.find('\n').map_or(code.len(), |n| i + n);
            out.push((i..end, Token::Type));
            line_start = false;
            i = end;
            continue;
        }
        if lang.vars && c == '$' {
            let body = &rest[1..];
            // `${…}` closed within `MAX_VAR` on its line: an unclosed one never scans on.
            let len = if body.starts_with('{') {
                let near = head(body, MAX_VAR);
                near.split('\n').next().unwrap_or("").find('}').map_or(0, |n| n + 1)
            } else {
                body.find(|c: char| !word_char(c)).unwrap_or(body.len())
            };
            if len > 0 {
                out.push((i..i + 1 + len, Token::Type));
                i += 1 + len;
                line_start = false;
                continue;
            }
        }
        if c.is_ascii_digit() && !prev.is_some_and(word_char) {
            let len = rest.find(|c: char| !(word_char(c) || c == '.')).unwrap_or(rest.len());
            // `1.` before a method (`1.max`) keeps its dot out.
            let len = rest[..len].trim_end_matches('.').len();
            out.push((i..i + len, Token::Number));
            i += len;
            line_start = false;
            continue;
        }
        if word_char(c) {
            let len =
                rest.find(|c: char| !(word_char(c) || (lang.keys == Keys::Yaml && c == '-'))).unwrap_or(rest.len());
            let word = &rest[..len];
            let next = rest[len..].trim_start_matches([' ', '\t']);
            let key = match lang.keys {
                Keys::Yaml => next.starts_with(':'),
                Keys::Toml => next.starts_with('='),
                Keys::None | Keys::Json => false,
            };
            let token = if key && line_start {
                Some(Token::Key)
            } else if lang.keywords.iter().any(|k| if lang.fold { k.eq_ignore_ascii_case(word) } else { *k == word }) {
                Some(Token::Keyword)
            } else if lang.types && word.starts_with(|c: char| c.is_ascii_uppercase()) {
                Some(Token::Type)
            } else {
                None
            };
            if let Some(token) = token {
                out.push((i..i + len, token));
            }
            i += len;
            line_start = false;
            continue;
        }
        // A YAML list item's `- ` keeps the line's start: `- name: x` has a key.
        if !(c.is_whitespace() || (lang.keys == Keys::Yaml && c == '-' && line_start)) {
            line_start = false;
        }
        i += c.len_utf8();
    }
    out
}

/// Where a string opened by `quote` at `at` ends (past its close), or `None` if `quote` opens no
/// string here (a Rust lifetime). An unclosed string ends at its line's end; a triple-quoted or
/// backtick one may span lines.
fn string_end(lang: &Lang, code: &str, at: usize, quote: char) -> Option<usize> {
    let rest = &code[at..];
    if lang.triple {
        let three: String = std::iter::repeat_n(quote, 3).collect();
        if let Some(body) = rest.strip_prefix(three.as_str()) {
            return Some(body.find(three.as_str()).map_or(code.len(), |n| at + 3 + n + 3));
        }
    }
    if lang.chars && quote == '\'' {
        // `'a'`, `'\n'`, `'\u{1F600}'`: closed soon; else a lifetime or label.
        let close = rest[1..].char_indices().skip(1).take(10).find(|(_, c)| *c == '\'')?.0;
        let body = &rest[1..1 + close];
        return (body.chars().count() == 1 || body.starts_with('\\')).then_some(at + 1 + close + 1);
    }
    let multi = quote == '`';
    // A shell's `'…'` takes `\` as it is.
    let escapes = !(lang.vars && quote == '\'');
    let mut escaped = false;
    for (n, c) in rest.char_indices().skip(1) {
        match c {
            '\n' if !multi => return Some(at + n),
            '\\' if escapes && !escaped => escaped = true,
            _ if escaped => escaped = false,
            _ if c == quote => return Some(at + n + c.len_utf8()),
            _ => {}
        }
    }
    Some(code.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each coloured run as its text, for reading.
    fn runs<'a>(label: &str, code: &'a str) -> Vec<(&'a str, Token)> {
        marks(label, code).into_iter().map(|(range, token)| (&code[range], token)).collect()
    }

    #[test]
    fn rust_has_keywords_types_strings_comments_and_lifetimes_are_not_strings() {
        let code = "/// Adds one.\nfn bump<'a>(n: u32, s: &'a str) -> Point { let c = '\\n'; \"x\" // never wraps\n}";
        let got = runs("rust", code);
        assert!(got.contains(&("/// Adds one.", Token::Comment)));
        assert!(got.contains(&("fn", Token::Keyword)) && got.contains(&("u32", Token::Keyword)));
        assert!(got.contains(&("Point", Token::Type)));
        assert!(got.contains(&("'\\n'", Token::String)), "{got:?}");
        assert!(got.contains(&("\"x\"", Token::String)));
        assert!(got.contains(&("// never wraps", Token::Comment)));
        assert!(!got.iter().any(|(text, _)| text.starts_with("'a")), "a lifetime is no string: {got:?}");
    }

    #[test]
    fn each_language_of_the_fixture_is_coloured() {
        let cases: &[(&str, &str, &str, Token)] = &[
            ("ts", "return `hello, ${name}`;", "`hello, ${name}`", Token::String),
            ("js", "const items = [1, 2];", "const", Token::Keyword),
            ("python", "def mean():\n    \"\"\"The average.\"\"\"", "\"\"\"The average.\"\"\"", Token::String),
            ("sh", "#!/bin/sh\nwc -l \"$f\"; echo $#", "#!/bin/sh", Token::Comment),
            ("sh", "for f in *.md; do wc -l $f; done", "$f", Token::Type),
            ("json", "{ \"name\": \"fixture\", \"ok\": true }", "\"name\"", Token::Key),
            ("json", "{ \"name\": \"fixture\" }", "\"fixture\"", Token::String),
            ("yaml", "jobs:\n  test:\n    runs-on: macos-latest", "runs-on", Token::Key),
            ("yaml", "- name: x", "name", Token::Key),
            ("toml", "[package]\nname = \"fixture\"", "[package]", Token::Type),
            ("toml", "[package]\nname = \"fixture\"", "name", Token::Key),
            ("go", "func main() { fmt.Println(\"hello\") }", "func", Token::Keyword),
            ("swift", "struct Point { var x = 0.0 }", "0.0", Token::Number),
            ("sql", "SELECT id FROM tasks WHERE state = 'open'", "SELECT", Token::Keyword),
            ("sql", "SELECT id FROM tasks WHERE state = 'open'", "'open'", Token::String),
            ("diff", "--- a/x\n+++ b/x\n@@ -1 +1 @@\n unchanged\n-removed\n+added", "-removed", Token::Removed),
            ("diff", "+added", "+added", Token::Added),
        ];
        for (label, code, text, token) in cases {
            let got = runs(label, code);
            assert!(got.contains(&(text, *token)), "{label}: {text:?} as {token:?} in {got:?}");
        }
        // Nothing for a language not known, or none named.
        assert!(marks("brainfuck", "+++").is_empty() && marks("", "fn x").is_empty());
    }

    /// An unclosed `${` is read within `MAX_VAR`, not to the end: 100 KB of them is still linear.
    #[test]
    fn runs_split_by_line_as_the_viewer_draws_them() {
        let code = "let a = 1; /* one\r\ntwo */ \"s\"\n\nfn";
        let lines = by_line(code, &marks("rust", code));
        assert_eq!(lines.len(), code.lines().count());
        let texts: Vec<Vec<(&str, Token)>> = lines
            .iter()
            .zip(code.lines())
            .map(|(marks, line)| marks.iter().map(|(range, token)| (&line[range.clone()], *token)).collect())
            .collect();
        assert_eq!(texts[0], [("let", Token::Keyword), ("1", Token::Number), ("/* one", Token::Comment)]);
        assert_eq!(texts[1], [("two */", Token::Comment), ("\"s\"", Token::String)]);
        assert_eq!((texts[2].len(), &texts[3][..]), (0, &[("fn", Token::Keyword)][..]));
        assert_eq!(by_line("a\nb", &[]), Vec::<Vec<Mark>>::new());
    }

    /// A run over the drawn end of a line of wide characters is cut at a character, never inside one.
    #[test]
    fn runs_cut_to_what_is_drawn_never_split_a_character() {
        let line = "// é日本😀 and more";
        let marks = marks("rust", line);
        for most in 0..=line.chars().count() {
            let shown: String = line.chars().take(most).collect();
            for (range, _) in within(&marks, &shown) {
                assert!(shown.get(range.clone()).is_some_and(|run| !run.is_empty()), "{shown:?} {range:?}");
            }
            assert_eq!(within(&marks, &shown).last().map(|(r, _)| r.end), (!shown.is_empty()).then_some(shown.len()));
        }
        // Ranges from another string, inside its characters: snapped down, or dropped when empty.
        let shown = "日本";
        assert_eq!(within(&[(1..5, Token::String)], shown), [(0..3, Token::String)]);
        assert!(within(&[(1..2, Token::String)], shown).is_empty());
        assert_eq!(within(&[(0..99, Token::Comment)], "😀é"), [(0..6, Token::Comment)]);
    }

    #[test]
    fn an_unclosed_variable_is_read_within_a_bound() {
        assert_eq!(runs("sh", "${ab}"), [("${ab}", Token::Type)]);
        let far = format!("${{{}}}", "a".repeat(MAX_VAR));
        assert!(runs("sh", &far).is_empty(), "a close past MAX_VAR is not looked for");
        assert!(runs("sh", "${a\n}").is_empty(), "nor on the next line");
        let many = "${".repeat(50_000);
        assert!(marks("sh", &many).is_empty());
    }

    #[test]
    fn runs_are_in_order_apart_and_on_char_boundaries_whatever_the_text() {
        let pieces =
            ["\"", "'", "`", "/*", "*/", "//", "#", "$", "{", "}", "é", "\n", " ", "a", "1", ":", "=", "[", "\\", "-"];
        let mut seed = 7u64;
        let mut next = |n: usize| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize % n
        };
        for label in ["rust", "ts", "python", "sh", "json", "yaml", "toml", "go", "swift", "sql", "diff"] {
            for _ in 0..500 {
                let code: String = (0..next(40)).map(|_| pieces[next(pieces.len())]).collect();
                let mut end = 0;
                for (range, _) in marks(label, &code) {
                    assert!(range.start >= end && range.end > range.start && range.end <= code.len(), "{code:?}");
                    assert!(code.is_char_boundary(range.start) && code.is_char_boundary(range.end), "{code:?}");
                    end = range.end;
                }
            }
        }
    }
}
