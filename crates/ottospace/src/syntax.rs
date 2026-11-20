//! A code file's colours from its tree-sitter grammar: parsed once off the main thread within a
//! budget, then only the lines drawn are coloured, a chunk at a time, kept. A language with no
//! grammar here, a file too large or a parse past its budget: `highlight`'s lexer, as before.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::{ControlFlow, Range};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tree_sitter::{Language, ParseOptions, Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::highlight::{Mark, Token};
use crate::viewer::MAX_DRAWN;

/// The most bytes parsed: past it, the lexer.
pub const MAX_PARSE: usize = 2 * 1024 * 1024;
/// How long a parse may take before it is dropped for the lexer.
pub const BUDGET: Duration = Duration::from_millis(500);
/// Lines coloured together: the lines drawn and those near them.
const CHUNK: usize = 128;

/// A language with a grammar here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Json,
    Toml,
    Python,
    Bash,
}

const LANGS: [Lang; 8] =
    [Lang::Rust, Lang::TypeScript, Lang::Tsx, Lang::JavaScript, Lang::Json, Lang::Toml, Lang::Python, Lang::Bash];

/// The language of a file by its extension (`rs`, `tsx`), any case. Markdown is drawn, not here.
pub fn lang(extension: &str) -> Option<Lang> {
    Some(match extension.to_ascii_lowercase().as_str() {
        "rs" => Lang::Rust,
        "ts" | "mts" | "cts" => Lang::TypeScript,
        "tsx" => Lang::Tsx,
        "js" | "jsx" | "mjs" | "cjs" => Lang::JavaScript,
        "json" => Lang::Json,
        "toml" => Lang::Toml,
        "py" => Lang::Python,
        "sh" | "bash" | "zsh" => Lang::Bash,
        _ => return None,
    })
}

impl Lang {
    /// Its name in the state (`viewers[n].syntax`).
    pub fn name(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::TypeScript => "typescript",
            Lang::Tsx => "tsx",
            Lang::JavaScript => "javascript",
            Lang::Json => "json",
            Lang::Toml => "toml",
            Lang::Python => "python",
            Lang::Bash => "bash",
        }
    }

    fn grammar(self) -> Language {
        match self {
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Lang::Json => tree_sitter_json::LANGUAGE.into(),
            Lang::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            Lang::Bash => tree_sitter_bash::LANGUAGE.into(),
        }
    }

    /// Its highlights query, its own patterns first. TypeScript's adds to JavaScript's; JSX's
    /// elements are JavaScript's and TSX's.
    fn highlights(self) -> String {
        let (script, jsx) = (tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_javascript::JSX_HIGHLIGHT_QUERY);
        let typed = tree_sitter_typescript::HIGHLIGHTS_QUERY;
        match self {
            Lang::Rust => tree_sitter_rust::HIGHLIGHTS_QUERY.into(),
            Lang::TypeScript => format!("{typed}\n{script}"),
            Lang::Tsx => format!("{typed}\n{jsx}\n{script}"),
            Lang::JavaScript => format!("{jsx}\n{script}"),
            Lang::Json => tree_sitter_json::HIGHLIGHTS_QUERY.into(),
            Lang::Toml => tree_sitter_toml_ng::HIGHLIGHTS_QUERY.into(),
            Lang::Python => tree_sitter_python::HIGHLIGHTS_QUERY.into(),
            Lang::Bash => tree_sitter_bash::HIGHLIGHT_QUERY.into(),
        }
    }

    /// The query, made once (its first file's read thread), and each capture's token by index.
    /// `None`: it does not compile.
    fn query(self) -> Option<&'static (Query, Vec<Option<Token>>)> {
        type Made = OnceLock<Option<(Query, Vec<Option<Token>>)>>;
        static QUERIES: [Made; LANGS.len()] = [const { OnceLock::new() }; LANGS.len()];
        let made = QUERIES[self as usize].get_or_init(|| {
            let query = Query::new(&self.grammar(), &self.highlights()).ok()?;
            let tokens = query.capture_names().iter().map(|name| token(name)).collect();
            Some((query, tokens))
        });
        made.as_ref()
    }
}

/// What a highlights capture is drawn as, by its name's most specific known part: `keyword`,
/// `string.special.key` (a key), `function.method`. `None`: plain (`variable`, `operator`).
pub fn token(capture: &str) -> Option<Token> {
    let first = capture.split('.').next().unwrap_or(capture);
    Some(match (first, capture) {
        (_, "string.special.key") | ("property", _) => Token::Key,
        ("keyword" | "label", _) => Token::Keyword,
        ("string" | "escape", _) => Token::String,
        ("comment", _) => Token::Comment,
        // Rust's numbers are `constant.builtin`, as JSON's `true`.
        ("number" | "constant" | "boolean", _) => Token::Number,
        ("type" | "constructor" | "attribute", _) => Token::Type,
        ("function", _) => Token::Function,
        ("punctuation", _) => Token::Punctuation,
        _ => return None,
    })
}

/// `text` parsed as `lang`, if within `MAX_PARSE` and `budget`.
pub fn parse(lang: Lang, text: &str, budget: Duration) -> Option<Tree> {
    if text.len() > MAX_PARSE {
        return None;
    }
    lang.query()?;
    let mut parser = Parser::new();
    parser.set_language(&lang.grammar()).ok()?;
    let start = Instant::now();
    let mut past = |_: &tree_sitter::ParseState| {
        if start.elapsed() > budget { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
    };
    let bytes = text.as_bytes();
    let mut read = |at: usize, _| &bytes[at.min(bytes.len())..];
    parser.parse_with_options(&mut read, None, Some(ParseOptions::new().progress_callback(&mut past)))
}

/// A file's tree, and its lines' colours as they are drawn.
pub struct Colours {
    lang: Lang,
    tree: Tree,
    text: String,
    /// Each line's bytes in `text`, without its newline: as `str::lines` splits it, cut as drawn.
    lines: Vec<Range<usize>>,
    chunks: RefCell<HashMap<usize, Vec<Vec<Mark>>>>,
}

impl Colours {
    pub fn new(lang: Lang, tree: Tree, text: String) -> Self {
        let mut lines = Vec::new();
        let mut start = 0;
        for line in text.split_inclusive('\n') {
            let shown = line.trim_end_matches('\n').trim_end_matches('\r');
            // No more than is drawn of it: a minified file's one long line is not coloured whole.
            let shown = shown.char_indices().nth(MAX_DRAWN).map_or(shown, |(cut, _)| &shown[..cut]);
            lines.push(start..start + shown.len());
            start += line.len();
        }
        Colours { lang, tree, text, lines, chunks: RefCell::default() }
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// Line `at`'s runs, by byte within it: its chunk coloured the first time it is asked for.
    pub fn line(&self, at: usize) -> Vec<Mark> {
        let chunk = at / CHUNK;
        if !self.chunks.borrow().contains_key(&chunk) {
            let made = self.colour(chunk * CHUNK..((chunk + 1) * CHUNK).min(self.lines.len()));
            self.chunks.borrow_mut().insert(chunk, made);
        }
        self.chunks.borrow().get(&chunk).and_then(|lines| lines.get(at % CHUNK)).cloned().unwrap_or_default()
    }

    /// The runs of lines `range`, queried together up to a line that was cut: its rest is skipped.
    fn colour(&self, range: Range<usize>) -> Vec<Vec<Mark>> {
        let lines = self.lines.get(range).unwrap_or_default();
        lines.chunk_by(|line, next| next.start - line.end <= 2).flat_map(|lines| self.span(lines)).collect()
    }

    /// The runs of `lines`, one after another in the text. A node inside another is drawn as its
    /// own; of two captures of one node, the query's first.
    fn span(&self, lines: &[Range<usize>]) -> Vec<Vec<Mark>> {
        let (Some(first), Some(last)) = (lines.first(), lines.last()) else { return Vec::new() };
        let Some((query, tokens)) = self.lang.query() else { return Vec::new() };
        let bytes = first.start..last.end;
        let mut found: Vec<(usize, usize, usize, Token)> = Vec::new();
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(bytes.clone());
        let mut captures = cursor.captures(query, self.tree.root_node(), self.text.as_bytes());
        while let Some((matched, index)) = captures.next() {
            let capture = matched.captures()[*index];
            let Some(token) = tokens.get(capture.index as usize).copied().flatten() else { continue };
            let node = capture.node.byte_range();
            let (start, end) = (node.start.max(bytes.start), node.end.min(bytes.end));
            if start < end {
                found.push((start, end, matched.pattern_index, token));
            }
        }
        // Outer before inner, so the inner is painted over it; one node's first pattern first.
        found.sort_by_key(|(start, end, pattern, _)| (*start, std::cmp::Reverse(*end), *pattern));
        found.dedup_by_key(|(start, end, _, _)| (*start, *end));
        let mut paint: Vec<Option<Token>> = vec![None; bytes.len()];
        for (start, end, _, token) in found {
            paint[start - bytes.start..end - bytes.start].fill(Some(token));
        }
        lines
            .iter()
            .map(|line| {
                let mut runs: Vec<Mark> = Vec::new();
                let painted = &paint[line.start - bytes.start..line.end - bytes.start];
                for (at, token) in painted.iter().enumerate() {
                    let Some(token) = token else { continue };
                    match runs.last_mut() {
                        Some((run, was)) if run.end == at && was == token => run.end += 1,
                        _ => runs.push((at..at + 1, *token)),
                    }
                }
                // A node's ends are on characters; snapped anyway, so a cut never splits one.
                let shown = &self.text[line.clone()];
                crate::highlight::within(&runs, shown)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(lang: Lang, code: &str) -> Vec<Vec<(String, Token)>> {
        let tree = parse(lang, code, BUDGET).expect("parsed");
        let colours = Colours::new(lang, tree, code.to_string());
        let lines: Vec<&str> = code.lines().collect();
        (0..lines.len())
            .map(|at| {
                colours.line(at).into_iter().map(|(range, token)| (lines[at][range].to_string(), token)).collect()
            })
            .collect()
    }

    fn has(lang: Lang, code: &str, text: &str, token: Token) {
        let got: Vec<(String, Token)> = runs(lang, code).into_iter().flatten().collect();
        assert!(got.contains(&(text.to_string(), token)), "{lang:?}: {text:?} as {token:?} in {got:?}");
    }

    #[test]
    fn a_language_by_its_extension() {
        assert_eq!(
            (lang("rs"), lang("TSX"), lang("mjs"), lang("toml")),
            (Some(Lang::Rust), Some(Lang::Tsx), Some(Lang::JavaScript), Some(Lang::Toml))
        );
        assert_eq!((lang("md"), lang("go"), lang("")), (None, None, None));
    }

    #[test]
    fn a_capture_by_its_name() {
        let cases = [
            ("keyword.function", Some(Token::Keyword)),
            ("string.special.key", Some(Token::Key)),
            ("string.special", Some(Token::String)),
            ("function.method", Some(Token::Function)),
            ("type.builtin", Some(Token::Type)),
            ("constant.builtin", Some(Token::Number)),
            ("punctuation.bracket", Some(Token::Punctuation)),
            ("variable", None),
            ("operator", None),
        ];
        for (name, token) in cases {
            assert_eq!(super::token(name), token, "{name}");
        }
    }

    #[test]
    fn every_query_compiles() {
        for lang in LANGS {
            assert!(lang.query().is_some(), "{lang:?}");
        }
    }

    #[test]
    fn rust_and_typescript_spans_are_what_they_are() {
        let rust =
            "/// Sows.\npub fn sow(rows: u32) -> Packet {\n    let name = \"kale\";\n    Packet::new(name, 40)\n}";
        has(Lang::Rust, rust, "/// Sows.", Token::Comment);
        has(Lang::Rust, rust, "fn", Token::Keyword);
        has(Lang::Rust, rust, "sow", Token::Function);
        has(Lang::Rust, rust, "u32", Token::Type);
        has(Lang::Rust, rust, "\"kale\"", Token::String);
        has(Lang::Rust, rust, "40", Token::Number);
        has(Lang::Rust, rust, "{", Token::Punctuation);
        let ts = "interface Seed { name: string }\nexport function sow(seed: Seed): number {\n  return 3; // rows\n}";
        has(Lang::TypeScript, ts, "interface", Token::Keyword);
        has(Lang::TypeScript, ts, "Seed", Token::Type);
        has(Lang::TypeScript, ts, "string", Token::Type);
        has(Lang::TypeScript, ts, "sow", Token::Function);
        has(Lang::TypeScript, ts, "3", Token::Number);
        has(Lang::TypeScript, ts, "// rows", Token::Comment);
        has(Lang::Json, "{\"name\": \"kale\", \"ok\": true}", "\"name\"", Token::Key);
        has(Lang::Json, "{\"name\": \"kale\", \"ok\": true}", "\"kale\"", Token::String);
    }

    #[test]
    fn a_block_comment_over_lines_is_coloured_on_each_and_chunks_join() {
        let mut code = String::from("/* one\r\ntwo */ fn a() {}\n");
        code.push_str(&"fn b() {}\n".repeat(CHUNK * 2));
        let lines = runs(Lang::Rust, &code);
        assert_eq!(lines.len(), code.lines().count());
        assert_eq!(lines[0], [("/* one".to_string(), Token::Comment)]);
        assert_eq!(lines[1][0], ("two */".to_string(), Token::Comment));
        // Past the first chunk: coloured the same.
        assert_eq!(lines[CHUNK + 5], lines[2]);
    }

    #[test]
    fn a_line_longer_than_is_drawn_is_coloured_up_to_the_cut_and_the_next_whole() {
        let code = format!("[{}]\n[true]\n", "1, ".repeat(MAX_DRAWN));
        let tree = parse(Lang::Json, &code, BUDGET).expect("parsed");
        let colours = Colours::new(Lang::Json, tree, code);
        let end = colours.line(0).last().map(|(range, _)| range.end);
        assert!(end.is_some_and(|end| end > MAX_DRAWN - 4 && end <= MAX_DRAWN), "{end:?}");
        assert!(colours.line(1).contains(&(1..5, Token::Number)), "{:?}", colours.line(1));
    }

    #[test]
    fn a_large_file_or_a_slow_parse_falls_back() {
        let big = "fn a() {}\n".repeat(MAX_PARSE / 10 + 1);
        assert!(parse(Lang::Rust, &big, BUDGET).is_none(), "past MAX_PARSE");
        // A budget already spent: the parse stops at its first check.
        let long = "fn a() { let x = [1, 2, 3]; }\n".repeat(2_000);
        assert!(parse(Lang::Rust, &long, Duration::ZERO).is_none());
        assert!(parse(Lang::Rust, &long, BUDGET).is_some(), "60 KB parses within the budget, in a debug build too");
    }

    /// How long the largest file parsed takes in each language, against `BUDGET`: `cargo test
    /// --release -p ottospace --bin ottospace -- --ignored --nocapture timings`.
    #[test]
    #[ignore = "timings, by hand"]
    fn timings() {
        let cases: [(&str, Lang, &str, &str); 10] = [
            ("rust", Lang::Rust, "rs", include_str!("../tests/fixtures/code/seeds.rs")),
            ("typescript", Lang::TypeScript, "ts", include_str!("../tests/fixtures/code/seeds.ts")),
            (
                "tsx",
                Lang::Tsx,
                "tsx",
                "export function Row({ name }: { name: string }) {\n  return <li className=\"row\">{name}</li>;\n}\n",
            ),
            (
                "javascript",
                Lang::JavaScript,
                "js",
                "export function sow(seed, rows = 3) {\n  const beds = [1, 2, 3].map((n) => `${seed.name}-${n}`);\n  return { beds, rows }; // made up\n}\n",
            ),
            (
                "json",
                Lang::Json,
                "json",
                "{\"name\": \"kale\", \"rows\": [1, 2, 3], \"sown\": {\"indoors\": true, \"weeks\": 4.5}}\n",
            ),
            (
                "json, one line",
                Lang::Json,
                "json",
                "{\"name\": \"kale\", \"rows\": [1, 2, 3], \"sown\": {\"indoors\": true, \"weeks\": 4.5}} ",
            ),
            (
                "toml",
                Lang::Toml,
                "toml",
                "[[packet]]\nname = \"kale\" # made up\nrows = [1, 2, 3]\nsown = { indoors = true, weeks = 4.5 }\n\n",
            ),
            (
                "python",
                Lang::Python,
                "py",
                "class Packet:\n    \"\"\"Made up.\"\"\"\n\n    def sow(self, rows: int = 3) -> list[str]:\n        return [f\"{self.name}-{n}\" for n in range(rows)]  # beds\n\n",
            ),
            (
                "bash",
                Lang::Bash,
                "sh",
                "sow() {\n  for bed in \"$@\"; do\n    echo \"sown: ${bed:-none}\" | tr a-z A-Z # made up\n  done\n}\n[ -n \"$1\" ] && sow \"$1\"\n",
            ),
            ("rust, broken", Lang::Rust, "rs", "fn sow( { let x = [1, 2; \"kale\n"),
        ];
        println!("| file | KB | query made | parse | within {BUDGET:?} | a chunk | the lexer |");
        println!("|---|---|---|---|---|---|---|");
        for (name, lang, extension, piece) in cases {
            let text = piece.repeat(MAX_PARSE / piece.len());
            let start = Instant::now();
            assert!(lang.query().is_some());
            let made = start.elapsed();
            let start = Instant::now();
            let tree = parse(lang, &text, Duration::from_secs(120)).expect("parsed");
            let parsed = start.elapsed();
            let colours = Colours::new(lang, tree, text.clone());
            let start = Instant::now();
            let _ = colours.line(colours.lines.len() / 2);
            let chunk = start.elapsed();
            let start = Instant::now();
            let _ = crate::highlight::by_line(&text, &crate::highlight::marks(extension, &text));
            let (lexer, kb, within) = (start.elapsed(), text.len() / 1024, parsed <= BUDGET);
            println!("| {name} | {kb} | {made:.1?} | {parsed:.1?} | {within} | {chunk:.1?} | {lexer:.1?} |");
        }
    }

    /// Runs on character boundaries, in order, inside the line, whatever the text.
    #[test]
    fn runs_are_in_order_and_on_characters_whatever_the_text() {
        let pieces =
            ["\"", "'", "`", "/*", "*/", "//", "#", "{", "}", "é", "日", "\n", " ", "a", "1", ":", "=", "\\", "$("];
        let mut seed = 11u64;
        let mut next = |n: usize| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize % n
        };
        for lang in LANGS {
            for _ in 0..100 {
                let code: String = (0..next(40)).map(|_| pieces[next(pieces.len())]).collect();
                let Some(tree) = parse(lang, &code, BUDGET) else { continue };
                let colours = Colours::new(lang, tree, code.clone());
                for (at, line) in code.lines().enumerate() {
                    let mut end = 0;
                    for (range, _) in colours.line(at) {
                        assert!(range.start >= end && range.end > range.start && range.end <= line.len(), "{code:?}");
                        assert!(line.is_char_boundary(range.start) && line.is_char_boundary(range.end), "{code:?}");
                        end = range.end;
                    }
                }
            }
        }
    }
}
