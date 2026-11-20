//! A code file's colours from its tree-sitter grammar: parsed once off the main thread within a
//! budget, then only the lines drawn are coloured, a chunk at a time, kept. A language with no
//! grammar here, a file too large or a parse past its budget: `highlight`'s lexer, as before.
//! Markdown, HTML and markdown's fences hold other languages: each parsed as its own layer.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::{ControlFlow, Range};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ottospace_core::viewer::{GRAMMARS, Lang, lang_named};
use tree_sitter::{Language, Node, ParseOptions, Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::highlight::{Mark, Token};
use crate::viewer::MAX_DRAWN;

/// The most bytes parsed: past it, the lexer.
pub const MAX_PARSE: usize = 2 * 1024 * 1024;
/// How long a parse may take before it is dropped: for the lexer, or a language inside, plain.
pub const BUDGET: Duration = Duration::from_millis(500);
/// Lines coloured together: the lines drawn and those near them.
const CHUNK: usize = 128;
/// How deep languages nest: markdown, its paragraph, an HTML tag in it, that tag's script.
const MAX_DEPTH: usize = 3;

fn grammar(lang: Lang) -> Language {
    match lang {
        Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
        Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Lang::Json => tree_sitter_json::LANGUAGE.into(),
        Lang::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
        Lang::Python => tree_sitter_python::LANGUAGE.into(),
        Lang::Bash => tree_sitter_bash::LANGUAGE.into(),
        Lang::Go => tree_sitter_go::LANGUAGE.into(),
        Lang::C => tree_sitter_c::LANGUAGE.into(),
        Lang::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        Lang::Java => tree_sitter_java::LANGUAGE.into(),
        Lang::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
        Lang::Ruby => tree_sitter_ruby::LANGUAGE.into(),
        Lang::Html => tree_sitter_html::LANGUAGE.into(),
        Lang::Css => tree_sitter_css::LANGUAGE.into(),
        Lang::Yaml => tree_sitter_yaml::LANGUAGE.into(),
        Lang::Sql => tree_sitter_sequel::LANGUAGE.into(),
        Lang::Swift => tree_sitter_swift::LANGUAGE.into(),
        Lang::Kotlin => tree_sitter_kotlin_sg::LANGUAGE.into(),
        Lang::Php => tree_sitter_php::LANGUAGE_PHP.into(),
        Lang::PhpOnly => tree_sitter_php::LANGUAGE_PHP_ONLY.into(),
        Lang::Lua => tree_sitter_lua::LANGUAGE.into(),
        Lang::Make => tree_sitter_make::LANGUAGE.into(),
        Lang::Xml => tree_sitter_xml::LANGUAGE_XML.into(),
        Lang::Markdown => tree_sitter_md::LANGUAGE.into(),
        Lang::MarkdownInline => tree_sitter_md::INLINE_LANGUAGE.into(),
    }
}

/// Its highlights query, its own patterns first. TypeScript's adds to JavaScript's; JSX's
/// elements are JavaScript's and TSX's; C++'s adds to C's.
fn highlights(lang: Lang) -> String {
    let (script, jsx) = (tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_javascript::JSX_HIGHLIGHT_QUERY);
    let typed = tree_sitter_typescript::HIGHLIGHTS_QUERY;
    match lang {
        Lang::Rust => tree_sitter_rust::HIGHLIGHTS_QUERY.into(),
        Lang::TypeScript => format!("{typed}\n{script}"),
        Lang::Tsx => format!("{typed}\n{jsx}\n{script}"),
        Lang::JavaScript => format!("{jsx}\n{script}"),
        Lang::Json => tree_sitter_json::HIGHLIGHTS_QUERY.into(),
        Lang::Toml => tree_sitter_toml_ng::HIGHLIGHTS_QUERY.into(),
        Lang::Python => tree_sitter_python::HIGHLIGHTS_QUERY.into(),
        Lang::Bash => tree_sitter_bash::HIGHLIGHT_QUERY.into(),
        Lang::Go => tree_sitter_go::HIGHLIGHTS_QUERY.into(),
        Lang::C => tree_sitter_c::HIGHLIGHT_QUERY.into(),
        Lang::Cpp => format!("{}\n{}", tree_sitter_cpp::HIGHLIGHT_QUERY, tree_sitter_c::HIGHLIGHT_QUERY),
        Lang::Java => tree_sitter_java::HIGHLIGHTS_QUERY.into(),
        Lang::CSharp => tree_sitter_c_sharp::HIGHLIGHTS_QUERY.into(),
        Lang::Ruby => tree_sitter_ruby::HIGHLIGHTS_QUERY.into(),
        Lang::Html => tree_sitter_html::HIGHLIGHTS_QUERY.into(),
        Lang::Css => tree_sitter_css::HIGHLIGHTS_QUERY.into(),
        Lang::Yaml => tree_sitter_yaml::HIGHLIGHTS_QUERY.into(),
        Lang::Sql => tree_sitter_sequel::HIGHLIGHTS_QUERY.into(),
        Lang::Swift => tree_sitter_swift::HIGHLIGHTS_QUERY.into(),
        Lang::Kotlin => tree_sitter_kotlin_sg::HIGHLIGHTS_QUERY.into(),
        Lang::Php | Lang::PhpOnly => tree_sitter_php::HIGHLIGHTS_QUERY.into(),
        Lang::Lua => tree_sitter_lua::HIGHLIGHTS_QUERY.into(),
        Lang::Make => tree_sitter_make::HIGHLIGHTS_QUERY.into(),
        Lang::Xml => tree_sitter_xml::XML_HIGHLIGHT_QUERY.into(),
        Lang::Markdown => tree_sitter_md::HIGHLIGHT_QUERY_BLOCK.into(),
        Lang::MarkdownInline => tree_sitter_md::HIGHLIGHT_QUERY_INLINE.into(),
    }
}

/// Where it holds other languages: markdown's paragraphs, fences and front matter; HTML's
/// `<script>` and `<style>`. No other's: only these are drawn layered.
fn injections(lang: Lang) -> Option<&'static str> {
    match lang {
        Lang::Markdown => Some(tree_sitter_md::INJECTION_QUERY_BLOCK),
        Lang::MarkdownInline => Some(tree_sitter_md::INJECTION_QUERY_INLINE),
        Lang::Html => Some(tree_sitter_html::INJECTIONS_QUERY),
        _ => None,
    }
}

/// True if of two patterns for one node the later wins: queries written for Neovim (`(string_scalar)
/// @string` first, a key's after). Of the tree-sitter org's, the first wins.
fn later_wins(lang: Lang) -> bool {
    matches!(
        lang,
        Lang::Yaml
            | Lang::Sql
            | Lang::Swift
            | Lang::Kotlin
            | Lang::Lua
            | Lang::Make
            | Lang::Xml
            | Lang::Markdown
            | Lang::MarkdownInline
    )
}

/// A language's queries, and each highlights capture's token by index.
struct Made {
    highlights: Query,
    tokens: Vec<Option<Token>>,
    injections: Option<Query>,
}

/// Its queries, made once, on its first use (a read thread, or a fence's block). `None`: they do
/// not compile.
fn made(lang: Lang) -> Option<&'static Made> {
    static MADE: [OnceLock<Option<Made>>; GRAMMARS.len()] = [const { OnceLock::new() }; GRAMMARS.len()];
    let made = MADE[lang as usize].get_or_init(|| {
        let language = grammar(lang);
        let mut highlights = Query::new(&language, &highlights(lang)).ok()?;
        // A predicate tree-sitter does not test (`#lua-match?`, `#is-not? local`) would let its
        // pattern match anything: Ruby's every variable a method.
        for pattern in 0..highlights.pattern_count() {
            if !highlights.general_predicates(pattern).is_empty() || !highlights.property_predicates(pattern).is_empty()
            {
                highlights.disable_pattern(pattern);
            }
        }
        let tokens = highlights.capture_names().iter().map(|name| token(name)).collect();
        let injections = match injections(lang) {
            Some(source) => {
                let mut query = Query::new(&language, source).ok()?;
                // What lies between a document's first two rules is not front matter: `minus_metadata` is.
                for pattern in 0..query.pattern_count() {
                    let text = &source[query.start_byte_for_pattern(pattern)..query.end_byte_for_pattern(pattern)];
                    if text.contains("thematic_break") {
                        query.disable_pattern(pattern);
                    }
                }
                Some(query)
            }
            None => None,
        };
        Some(Made { highlights, tokens, injections })
    });
    made.as_ref()
}

/// What a highlights capture is drawn as, by its name's most specific known part: `keyword`,
/// `string.special.key` (a key), `function.method`. `None`: plain (`variable`, `operator`).
/// One table for every grammar, nvim's names (`conditional`, `text.title`) too.
pub fn token(capture: &str) -> Option<Token> {
    let first = capture.split('.').next().unwrap_or(capture);
    Some(match (first, capture) {
        (_, "string.special.key") | ("property" | "field", _) => Token::Key,
        // Markdown's: a heading, code, a link, emphasis.
        (_, "text.title" | "markup.heading") => Token::Keyword,
        (_, "text.literal" | "markup.raw") => Token::String,
        (_, "text.uri" | "text.reference" | "markup.link") => Token::Function,
        (_, "text.emphasis" | "text.strong") => Token::Type,
        (
            "keyword" | "label" | "conditional" | "repeat" | "exception" | "include" | "storageclass" | "preproc"
            | "tag" | "import" | "charset" | "keyframes" | "media" | "supports",
            _,
        ) => Token::Keyword,
        ("string" | "escape" | "character", _) => Token::String,
        ("comment", _) => Token::Comment,
        // Rust's numbers are `constant.builtin`, as JSON's `true`.
        ("number" | "float" | "constant" | "boolean", _) => Token::Number,
        ("type" | "constructor" | "attribute" | "module" | "namespace", _) => Token::Type,
        ("function" | "method", _) => Token::Function,
        ("punctuation" | "delimiter", _) => Token::Punctuation,
        _ => return None,
    })
}

/// `text` parsed as `lang`, if within `MAX_PARSE` and `budget`. The languages inside it are parsed
/// as their lines are first drawn (`Colours::span`).
pub fn parse(lang: Lang, text: &str, budget: Duration) -> Option<Tree> {
    if text.len() > MAX_PARSE {
        return None;
    }
    made(lang)?;
    let mut parser = Parser::new();
    parser.set_language(&grammar(lang)).ok()?;
    parse_with(&mut parser, text, budget)
}

/// `text` parsed by `parser` over its included ranges, if within `budget`.
fn parse_with(parser: &mut Parser, text: &str, budget: Duration) -> Option<Tree> {
    let start = Instant::now();
    let mut past = |_: &tree_sitter::ParseState| {
        if start.elapsed() > budget { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
    };
    let bytes = text.as_bytes();
    let mut read = |at: usize, _| &bytes[at.min(bytes.len())..];
    parser.parse_with_options(&mut read, None, Some(ParseOptions::new().progress_callback(&mut past)))
}

/// The languages inside `tree` over `bytes` by `lang`'s injections query, each with its ranges: the content
/// node less its named children (a quote's `>` inside a fence). A language with no grammar: plain.
fn inside(lang: Lang, tree: &Tree, text: &str, bytes: Range<usize>) -> Vec<(Lang, Vec<tree_sitter::Range>)> {
    let Some(query) = made(lang).and_then(|made| made.injections.as_ref()) else { return Vec::new() };
    let content = query.capture_index_for_name("injection.content");
    let named = query.capture_index_for_name("injection.language");
    let mut found = Vec::new();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(bytes);
    let mut matches = cursor.matches(query, tree.root_node(), text.as_bytes());
    while let Some(matched) = matches.next() {
        let by_node = matched.captures().iter().find(|capture| Some(capture.index) == named);
        let name = by_node.and_then(|capture| text.get(capture.node.byte_range())).or_else(|| {
            let set = query.property_settings(matched.pattern_index);
            set.iter().find(|set| &*set.key == "injection.language").and_then(|set| set.value.as_deref())
        });
        let Some(inner) = name.and_then(lang_named) else { continue };
        for capture in matched.captures().iter().filter(|capture| Some(capture.index) == content) {
            let inner = untagged(inner, text.get(capture.node.byte_range()).unwrap_or_default());
            if made(inner).is_none() {
                continue;
            }
            let ranges = ranges(capture.node);
            if !unclosed(capture.node) && !ranges.is_empty() {
                found.push((inner, ranges));
            }
        }
    }
    found
}

/// A fence never closed runs to its container's end: its language is not painted over the rest.
fn unclosed(node: Node) -> bool {
    node.parent().filter(|fence| fence.kind() == "fenced_code_block").is_some_and(|fence| {
        let mut cursor = fence.walk();
        fence.children(&mut cursor).filter(|n| n.kind() == "fenced_code_block_delimiter").count() < 2
    })
}

/// PHP with no `<?php` (`echo "a";`, a fence's usual): the grammar of PHP alone, else it is text.
fn untagged(lang: Lang, code: &str) -> Lang {
    if lang == Lang::Php && !code.trim_start().starts_with("<?") { Lang::PhpOnly } else { lang }
}

/// `node`'s text less its named children's, as ranges.
fn ranges(node: Node) -> Vec<tree_sitter::Range> {
    let mut found = Vec::new();
    let (mut byte, mut point) = (node.start_byte(), node.start_position());
    let mut cursor = node.walk();
    let children =
        node.named_children(&mut cursor).map(|child| (child.start_byte(), child.start_position(), Some(child)));
    for (start, at, child) in children.chain([(node.end_byte(), node.end_position(), None)]) {
        if start > byte {
            found.push(tree_sitter::Range { start_byte: byte, end_byte: start, start_point: point, end_point: at });
        }
        if let Some(child) = child {
            (byte, point) = (child.end_byte(), child.end_position());
        }
    }
    found
}

/// A fenced block's runs, by byte in `code`: by the grammar `label` names if it parses within the
/// budget, else `highlight`'s lexer, as before (plain for a label it does not know).
pub fn fence(label: &str, code: &str) -> Vec<Mark> {
    let lang = lang_named(label).map(|lang| untagged(lang, code));
    let parsed = lang.and_then(|lang| Some((lang, parse(lang, code, BUDGET)?)));
    match parsed {
        Some((lang, tree)) => Colours::new(lang, tree, code.to_string()).all(),
        None => crate::highlight::marks(label, code),
    }
}

/// A file's tree, the languages inside it as drawn, and its lines' colours as they are drawn.
pub struct Colours {
    lang: Lang,
    tree: Tree,
    text: String,
    /// Each line's bytes in `text`, without its newline: as `str::lines` splits it, cut as drawn.
    lines: Vec<Range<usize>>,
    chunks: RefCell<HashMap<usize, Vec<Vec<Mark>>>>,
    /// Each language inside it parsed so far, by its first byte: `None` past the budget (plain).
    inner: RefCell<HashMap<(Lang, usize), Option<Tree>>>,
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
        Colours { lang, tree, text, lines, chunks: RefCell::default(), inner: RefCell::default() }
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

    /// Every line's runs, by byte in the text.
    fn all(&self) -> Vec<Mark> {
        let lines = self.lines.iter().enumerate();
        let by_text = |(at, line): (usize, &Range<usize>)| {
            let start = line.start;
            self.line(at).into_iter().map(move |(run, token)| (run.start + start..run.end + start, token))
        };
        lines.flat_map(by_text).collect()
    }

    /// The runs of lines `range`, queried together up to a line that was cut: its rest is skipped.
    fn colour(&self, range: Range<usize>) -> Vec<Vec<Mark>> {
        let lines = self.lines.get(range).unwrap_or_default();
        lines.chunk_by(|line, next| next.start - line.end <= 2).flat_map(|lines| self.span(lines)).collect()
    }

    /// The trees over `bytes`: its own, then each language inside one after it, `MAX_DEPTH` deep.
    fn layers(&self, bytes: Range<usize>) -> Vec<(Lang, Tree)> {
        let mut layers = vec![(self.lang, self.tree.clone(), 0)];
        let mut at = 0;
        while let Some((lang, tree, depth)) = layers.get(at).cloned() {
            at += 1;
            if depth == MAX_DEPTH {
                continue;
            }
            for (inner, ranges) in inside(lang, &tree, &self.text, bytes.clone()) {
                if let Some(tree) = self.inner(inner, &ranges) {
                    layers.push((inner, tree, depth + 1));
                }
            }
        }
        layers.into_iter().map(|(lang, tree, _)| (lang, tree)).collect()
    }

    /// `lang` parsed over `ranges`, once: a fence over two chunks is parsed for the first.
    fn inner(&self, lang: Lang, ranges: &[tree_sitter::Range]) -> Option<Tree> {
        let key = (lang, ranges.first()?.start_byte);
        if let Some(tree) = self.inner.borrow().get(&key) {
            return tree.clone();
        }
        // A parser of its own: one stopped by its budget would resume where it stopped.
        let mut parser = Parser::new();
        let set = parser.set_language(&grammar(lang)).is_ok() && parser.set_included_ranges(ranges).is_ok();
        let tree = if set { parse_with(&mut parser, &self.text, BUDGET) } else { None };
        self.inner.borrow_mut().insert(key, tree.clone());
        tree
    }

    /// The runs of `lines`, one after another in the text. A node inside another is drawn as its
    /// own; of two captures of one node, the query's first; a language inside another, over it.
    fn span(&self, lines: &[Range<usize>]) -> Vec<Vec<Mark>> {
        let (Some(first), Some(last)) = (lines.first(), lines.last()) else { return Vec::new() };
        let bytes = first.start..last.end;
        let mut paint: Vec<Option<Token>> = vec![None; bytes.len()];
        for (lang, tree) in self.layers(bytes.clone()) {
            let Some(Made { highlights: query, tokens, .. }) = made(lang) else { continue };
            // Code inside a string (`${name}`), a fence's code: plain, then its own colours.
            let plain = [query.capture_index_for_name("embedded"), query.capture_index_for_name("none")];
            let mut found: Vec<(usize, usize, usize, Option<Token>)> = Vec::new();
            let mut cursor = QueryCursor::new();
            cursor.set_byte_range(bytes.clone());
            let mut captures = cursor.captures(query, tree.root_node(), self.text.as_bytes());
            while let Some((matched, index)) = captures.next() {
                let capture = matched.captures()[*index];
                let token = tokens.get(capture.index as usize).copied().flatten();
                if token.is_some() || plain.contains(&Some(capture.index)) {
                    let node = capture.node.byte_range();
                    found.push((node.start, node.end, matched.pattern_index, token));
                }
            }
            // Outer before inner, so the inner is painted over it; one node's winning pattern first.
            // By the nodes' own ends: cut to these lines first, an outer and its inner could be one.
            let later = later_wins(lang);
            found.sort_by_key(|(start, end, pattern, _)| {
                (*start, std::cmp::Reverse(*end), if later { usize::MAX - pattern } else { *pattern })
            });
            found.dedup_by_key(|(start, end, _, _)| (*start, *end));
            // Only within the layer's ranges: a comment over two quoted lines leaves the `>` between.
            let ranges = tree.included_ranges();
            for (start, end, _, token) in found {
                for range in &ranges {
                    let start = start.max(bytes.start).max(range.start_byte);
                    let end = end.min(bytes.end).min(range.end_byte);
                    if start < end {
                        paint[start - bytes.start..end - bytes.start].fill(token);
                    }
                }
            }
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

    /// Each language's made-up file, its extension for the lexer, and a keyword, a string and a
    /// comment in it.
    const FIXTURES: [(Lang, &str, &str, &str, &str, &str); 17] = [
        (
            Lang::Go,
            "go",
            include_str!("../tests/fixtures/code/seeds.go"),
            "func",
            "\"kale\"",
            "// Packet is a packet of seeds on the shelf.",
        ),
        (
            Lang::C,
            "c",
            include_str!("../tests/fixtures/code/seeds.c"),
            "return",
            "\"%s %d\\n\"",
            "// Prints a packet and its rows.",
        ),
        (
            Lang::Cpp,
            "cpp",
            include_str!("../tests/fixtures/code/seeds.cpp"),
            "class",
            "\"kale\"",
            "// A made-up seed shelf.",
        ),
        (
            Lang::Java,
            "java",
            include_str!("../tests/fixtures/code/Seeds.java"),
            "public",
            "\"kale\"",
            "// Each packet's name.",
        ),
        (
            Lang::CSharp,
            "cs",
            include_str!("../tests/fixtures/code/Seeds.cs"),
            "namespace",
            "\"kale\"",
            "// A made-up seed shelf.",
        ),
        (
            Lang::Ruby,
            "rb",
            include_str!("../tests/fixtures/code/seeds.rb"),
            "def",
            "\"kale\"",
            "# A made-up seed shelf.",
        ),
        (
            Lang::Html,
            "html",
            include_str!("../tests/fixtures/code/seeds.html"),
            "ul",
            "shelf",
            "<!-- A made-up seed shelf. -->",
        ),
        (
            Lang::Css,
            "css",
            include_str!("../tests/fixtures/code/seeds.css"),
            "@media",
            "\"Menlo\"",
            "/* A made-up seed shelf. */",
        ),
        (
            Lang::Yaml,
            "yml",
            include_str!("../tests/fixtures/code/seeds.yml"),
            "sown",
            "\"basil\"",
            "# A made-up seed shelf.",
        ),
        (
            Lang::Sql,
            "sql",
            include_str!("../tests/fixtures/code/seeds.sql"),
            "SELECT",
            "'kale'",
            "-- A made-up seed shelf.",
        ),
        (
            Lang::Swift,
            "swift",
            include_str!("../tests/fixtures/code/seeds.swift"),
            "guard",
            "\"kale\"",
            "// A made-up seed shelf.",
        ),
        (
            Lang::Kotlin,
            "kt",
            include_str!("../tests/fixtures/code/seeds.kt"),
            "fun",
            "\"kale\"",
            "// A made-up seed shelf.",
        ),
        (
            Lang::Php,
            "php",
            include_str!("../tests/fixtures/code/seeds.php"),
            "function",
            "'kale'",
            "// A made-up seed shelf.",
        ),
        (
            Lang::Lua,
            "lua",
            include_str!("../tests/fixtures/code/seeds.lua"),
            "local",
            "\"kale\"",
            "-- A made-up seed shelf.",
        ),
        (
            Lang::Make,
            "mk",
            include_str!("../tests/fixtures/code/Makefile"),
            "ifeq",
            "\"no rows\"",
            "# A made-up seed shelf.",
        ),
        (
            Lang::Xml,
            "xml",
            include_str!("../tests/fixtures/code/seeds.xml"),
            "shelf",
            "kale",
            "<!-- A made-up seed shelf. -->",
        ),
        (Lang::Markdown, "md", include_str!("../tests/fixtures/code/seeds.md"), "fn", "\"kale\"", "// Sow a row."),
    ];

    #[test]
    fn every_fixture_parses_whole_and_has_its_colours() {
        for (lang, _, code, keyword, string, comment) in FIXTURES {
            let tree = parse(lang, code, BUDGET).expect("parsed");
            let colours = Colours::new(lang, tree.clone(), code.to_string());
            colours.all();
            let inner =
                colours.inner.borrow().iter().map(|((lang, _), tree)| (*lang, tree.clone())).collect::<Vec<_>>();
            for (inner, tree) in [(lang, Some(tree))].into_iter().chain(inner) {
                let tree = tree.expect("parsed");
                assert!(!tree.root_node().has_error(), "{lang:?}: {inner:?} has an error: {}", tree.root_node());
            }
            has(lang, code, keyword, Token::Keyword);
            has(lang, code, string, Token::String);
            has(lang, code, comment, Token::Comment);
        }
    }

    /// Markdown's source: its own marks, and each fence's and its front matter's language.
    #[test]
    fn markdown_source_is_coloured_with_its_fences_and_front_matter() {
        let md = include_str!("../tests/fixtures/code/seeds.md");
        has(Lang::Markdown, md, "Seed shelf", Token::Keyword);
        has(Lang::Markdown, md, "seeds.rs", Token::String);
        has(Lang::Markdown, md, "made-up", Token::Type);
        has(Lang::Markdown, md, "https://example.com/guide", Token::Function);
        has(Lang::Markdown, md, "- ", Token::Punctuation);
        has(Lang::Markdown, md, "```", Token::Punctuation);
        has(Lang::Markdown, md, "title", Token::Key);
        has(Lang::Markdown, md, "# made up", Token::Comment);
        has(Lang::Markdown, md, "u32", Token::Type);
        // A fence of no language known: its code plain, not one string.
        let lines = runs(Lang::Markdown, "```brainfuck\n+[-->+]\n```\n");
        assert!(lines[1].is_empty(), "{:?}", lines[1]);
        // A fence never closed: the rest stays markdown, not a Rust string to the end.
        let lines = runs(Lang::Markdown, "```rust\nlet s = \"open\n\n# Heading\n");
        assert!(!lines[1].iter().any(|(_, token)| *token == Token::Keyword), "{:?}", lines[1]);
        assert!(lines[3].iter().all(|(_, token)| *token != Token::String), "{:?}", lines[3]);
        // A quoted fence: a comment over two lines leaves the `>` between them a quote's.
        let lines = runs(Lang::Markdown, "> ```rust\n> /* one\n> two */\n> ```\n");
        assert_eq!(lines[2][0], ("> ".to_string(), Token::Punctuation), "{:?}", lines[2]);
        assert!(lines[2].contains(&("two */".to_string(), Token::Comment)), "{:?}", lines[2]);
        // Between two rules at the top is no front matter: the heading stays a heading.
        has(Lang::Markdown, "***\n\n# Plan\n\n***\n", "Plan", Token::Keyword);
    }

    #[test]
    fn a_fence_is_coloured_by_the_grammar_its_label_names() {
        let code = "// Sow.\nfn sow() -> &'static str { \"kale\" }";
        for label in ["rust", "rs", "RUST"] {
            let marks = fence(label, code);
            let got: Vec<(&str, Token)> = marks.iter().map(|(range, token)| (&code[range.clone()], *token)).collect();
            for want in [("// Sow.", Token::Comment), ("fn", Token::Keyword), ("\"kale\"", Token::String)] {
                assert!(got.contains(&want), "{label}: {want:?} in {got:?}");
            }
        }
        assert_eq!(fence("brainfuck", code), Vec::new(), "a language unknown: plain");
        // PHP with no `<?php`, and with it.
        for php in ["echo \"kale\";", "<?php echo \"kale\";"] {
            let at = php.find('"').unwrap_or_default();
            assert!(fence("php", php).contains(&(at..at + 6, Token::String)), "{php}: {:?}", fence("php", php));
        }
        // One a grammar does not have: the lexer's, as before.
        assert_eq!(fence("diff", "+ a"), crate::highlight::marks("diff", "+ a"));
    }

    /// A markdown file of many fences parses within the budget: the languages inside are parsed as
    /// their lines are drawn, a chunk's only.
    #[test]
    fn many_fences_parse_within_the_budget_and_a_chunk_parses_its_own() {
        let block =
            "Some *made-up* text.\n\n```rust\nfn sow() -> u32 { 3 }\n```\n\n```ts\nconst a = \"kale\";\n```\n\n";
        let md = block.repeat(500);
        let start = Instant::now();
        let tree = parse(Lang::Markdown, &md, BUDGET).expect("within the budget");
        assert!(start.elapsed() <= BUDGET, "{:?}", start.elapsed());
        let colours = Colours::new(Lang::Markdown, tree, md);
        // `fn sow`, in the 11th chunk: its Rust coloured.
        assert!(colours.line(CHUNK * 10 + 3).contains(&(0..2, Token::Keyword)), "{:?}", colours.line(CHUNK * 10 + 3));
        // 128 lines of 10-line blocks: 13 or 14 blocks, 3 languages inside each.
        assert!((36..=45).contains(&colours.inner.borrow().len()), "{}", colours.inner.borrow().len());
    }

    /// A pattern whose predicate tree-sitter does not test is left out: Ruby's `#is-not? local`
    /// would make every variable a method.
    #[test]
    fn a_ruby_variable_is_no_method() {
        let got: Vec<(String, Token)> = runs(Lang::Ruby, "rows = 1\nrows + 1\n").into_iter().flatten().collect();
        assert!(!got.iter().any(|(text, _)| text == "rows"), "{got:?}");
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
            ("conditional", Some(Token::Keyword)),
            ("tag", Some(Token::Keyword)),
            ("float", Some(Token::Number)),
            ("method.call", Some(Token::Function)),
            ("text.title", Some(Token::Keyword)),
            ("text.literal", Some(Token::String)),
            ("text.uri", Some(Token::Function)),
            ("none", None),
            ("variable", None),
            ("operator", None),
        ];
        for (name, token) in cases {
            assert_eq!(super::token(name), token, "{name}");
        }
    }

    #[test]
    fn every_query_compiles() {
        for grammar in &GRAMMARS {
            assert!(made(grammar.lang).is_some(), "{}", grammar.name);
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
    fn code_inside_a_string_is_plain_and_a_string_over_chunks_stays_one() {
        let js: Vec<(String, Token)> =
            runs(Lang::JavaScript, "const s = `sown ${name} today`;").into_iter().flatten().collect();
        assert!(js.contains(&("`sown ".to_string(), Token::String)), "{js:?}");
        assert!(js.iter().all(|(text, token)| !text.contains("name") || *token != Token::String), "{js:?}");
        // TOML's pair is captured too (a key), around its string: the string wins in every chunk.
        let toml = format!("note = \"\"\"\n{}\"\"\"\n", "made up\n".repeat(CHUNK * 3));
        let lines = runs(Lang::Toml, &toml);
        assert_eq!(lines[CHUNK + 5], [("made up".to_string(), Token::String)]);
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
        let mut cases: Vec<(&str, Lang, &str, &str)> = vec![
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
        // Each fixture of `FIXTURES` repeated: markdown's last, its fences in languages made above.
        cases
            .extend(FIXTURES.iter().map(|(lang, extension, text, ..)| (lang.grammar().name, *lang, *extension, *text)));
        println!("| file | KB | query made | parse | within {BUDGET:?} | a chunk | the lexer |");
        println!("|---|---|---|---|---|---|---|");
        for (name, lang, extension, piece) in cases {
            // XML has one root: the rest inside it. PHP opens once.
            let (head, piece, tail) = match (lang, piece.split_once("<shelf"), piece.strip_prefix("<?php\n")) {
                (Lang::Xml, Some((head, _)), _) => (format!("{head}<shelf>"), &piece[head.len()..], "</shelf>"),
                (Lang::Php, _, Some(rest)) => ("<?php\n".to_string(), rest, ""),
                _ => (String::new(), piece, ""),
            };
            let text = format!("{head}{}{tail}", piece.repeat((MAX_PARSE - head.len() - tail.len()) / piece.len()));
            let start = Instant::now();
            assert!(made(lang).is_some() && (lang != Lang::Markdown || made(Lang::MarkdownInline).is_some()));
            let made = start.elapsed();
            let start = Instant::now();
            let tree = parse(lang, &text, Duration::from_secs(120)).expect("parsed");
            let parsed = start.elapsed();
            let colours = Colours::new(lang, tree.clone(), text.clone());
            let start = Instant::now();
            let _ = colours.line(colours.lines.len() / 2);
            let chunk = start.elapsed();
            // A chunk's languages inside, parsed with it.
            let inner = colours.inner.borrow().len();
            let errors = if tree.root_node().has_error() { " (errors)" } else { "" };
            let name =
                if inner > 0 { format!("{name}{errors}, {inner} inside a chunk") } else { format!("{name}{errors}") };
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
        for lang in GRAMMARS.iter().map(|grammar| grammar.lang) {
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
