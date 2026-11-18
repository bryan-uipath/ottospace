//! A reply's markdown as display blocks, for the drawer to lay out with GPUI elements. Parsed
//! by pulldown-cmark into a small tree; never HTML: raw HTML is drawn as the text it is. A link
//! is a link only if `links::target` accepts it. An image is never fetched: a link, and a local
//! file in the repository that the shell may read itself. Text is cleaned of terminal escapes
//! and hidden marks, and capped. Bare addresses are links, Slack's `<@U…>` a mention, fenced
//! code coloured (`highlight`). Pure.

use ottospace_core::ax::truncate as cap;
use ottospace_core::links::{self, Target};
use ottospace_core::transcript::{clean, cut as head};
use std::collections::{HashMap, HashSet};

use pulldown_cmark::{Alignment, BlockQuoteKind, CodeBlockKind, Event, Options, Parser, Tag, TagEnd, TextMergeStream};

use crate::highlight::{self, Mark};

/// The most blocks one reply is drawn as; the rest is counted.
pub const MAX_BLOCKS: usize = 300;
/// The most characters of one drawn line.
pub const MAX_LINE: usize = 2000;
/// The most lines of a code block drawn; Copy takes all of it.
pub const MAX_CODE_LINES: usize = 400;
/// How much of a text is drawn: a reply's (`CHAT`) or a viewed file's (`FILE`), which is longer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    pub blocks: usize,
    pub code_lines: usize,
}

pub const CHAT: Caps = Caps { blocks: MAX_BLOCKS, code_lines: MAX_CODE_LINES };
/// A file the viewer reads is at most 5 MB: this draws all of most, and stops before a stall.
pub const FILE: Caps = Caps { blocks: 20_000, code_lines: 20_000 };
/// Containers nested deeper than this are drawn flat.
const MAX_DEPTH: usize = 12;
/// The image files a thumbnail is drawn for.
const IMAGES: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];
/// A table column's widest, in characters: a longer cell wraps.
pub const MAX_COLUMN: usize = 40;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub strong: bool,
    pub emphasis: bool,
    pub code: bool,
    pub strike: bool,
    /// A Slack mention (`<@U0123>` drawn `@U0123`).
    pub mention: bool,
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
    /// A place in the same text it goes to: a heading's slug (`#tables`), or `^name`, a footnote.
    pub anchor: Option<String>,
}

/// A GitHub callout: `> [!NOTE]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Callout {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// A list item: its blocks, and with a task box (`- [x]`), whether it is done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

/// A footnote at the foot: its number, its name as written (`[^one]`), its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub n: usize,
    pub name: String,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Para(Vec<Span>),
    /// `level` 1 to 6; `slug`, what a `#link` to it says (`slug`), unique in the text.
    Heading {
        level: u8,
        slug: String,
        spans: Vec<Span>,
    },
    /// `label`: the fence's language or file; none for an indented or unlabelled block.
    /// `shown`: at most `MAX_CODE_LINES` lines; `more`: the lines left out. `copy`: the whole
    /// block. `marks`: `shown`'s coloured runs.
    Code {
        label: Option<String>,
        shown: String,
        copy: String,
        more: usize,
        marks: Vec<Mark>,
    },
    Quote {
        callout: Option<Callout>,
        blocks: Vec<Block>,
    },
    /// `start`: the first number of an ordered list.
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    /// Rows of cells; the first row is the header. Each column's alignment and width in
    /// characters (`column_widths`).
    Table {
        align: Vec<Align>,
        widths: Vec<usize>,
        rows: Vec<Vec<Vec<Span>>>,
    },
    /// A local image file in the repository: the shell may draw it as a thumbnail, with `alt`
    /// as its words if it cannot.
    Image {
        path: String,
        alt: String,
    },
    Rule,
    /// The footnotes, at the end, by number.
    Footnotes(Vec<Note>),
    /// What the caps left out, said in fixed words.
    Cut {
        blocks: usize,
    },
}

/// What is being built: a container of blocks, or a run of spans.
enum Frame {
    Root(Vec<Block>),
    Quote(Option<Callout>, Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    /// A list item: its task box, its blocks, and the spans of a tight item not yet a paragraph.
    Item(Option<bool>, Vec<Block>, Vec<Span>),
    Para(Vec<Span>),
    /// A block of raw HTML: its lines, drawn as the text they are.
    Html(Vec<Span>),
    Heading(u8, Vec<Span>),
    Code {
        label: Option<String>,
        text: String,
    },
    /// A footnote's definition, by name.
    Note(String, Vec<Block>),
    Table {
        align: Vec<Align>,
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
    anchor: Option<String>,
    raw: String,
    text: String,
}

struct Builder<'a> {
    repo: Option<&'a str>,
    caps: Caps,
    frames: Vec<Frame>,
    style: Style,
    links: Vec<Open>,
    /// An image's alt text being read, and its destination.
    image: Option<(String, String)>,
    /// Local images seen in the block being read, and their alt text: drawn after it.
    pictures: Vec<(String, String)>,
    blocks: usize,
    cut: usize,
    /// Characters on the drawn line so far (`budget`).
    line: usize,
    /// Each heading slug given, and each base's last suffix: the next alike is `-1`, `-2` (GitHub's).
    slugs: HashSet<String>,
    suffixes: HashMap<String, usize>,
    /// Footnotes by name: their numbers, in the order first referred to; their text once defined.
    numbers: HashMap<String, usize>,
    notes: Vec<(String, Vec<Block>)>,
}

/// `text`, a reply's markdown, as blocks. `repo`: the task's repository, whose files may be links.
pub fn parse(text: &str, repo: Option<&str>) -> Vec<Block> {
    parse_with(text, repo, CHAT)
}

/// `parse`, drawing as much as `caps` says: a viewed file's folder is its `repo`.
pub fn parse_with(text: &str, repo: Option<&str>, caps: Caps) -> Vec<Block> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM;
    let mut b = Builder {
        repo,
        caps,
        frames: vec![Frame::Root(Vec::new())],
        style: Style::default(),
        links: Vec::new(),
        image: None,
        pictures: Vec::new(),
        blocks: 0,
        cut: 0,
        line: 0,
        slugs: HashSet::new(),
        suffixes: HashMap::new(),
        numbers: HashMap::new(),
        notes: Vec::new(),
    };
    // Merged: a bare address or a mention is found whole.
    for event in TextMergeStream::new(Parser::new_ext(text, options)) {
        b.event(event);
    }
    // What a broken stream left open is closed as it stands.
    while b.frames.len() > 1 {
        b.close();
    }
    let notes = b.footnotes();
    let Some(Frame::Root(mut blocks)) = b.frames.pop() else { return Vec::new() };
    blocks.extend(notes);
    if b.cut > 0 {
        blocks.push(Block::Cut { blocks: b.cut });
    }
    blocks
}

impl Builder<'_> {
    fn depth(&self) -> usize {
        self.frames
            .iter()
            .filter(|frame| matches!(frame, Frame::Quote(..) | Frame::List { .. } | Frame::Item(..)))
            .count()
    }

    /// A container, or `Flat` past `MAX_DEPTH`.
    fn open(&mut self, frame: Frame) {
        let deep =
            self.depth() >= MAX_DEPTH && matches!(frame, Frame::Quote(..) | Frame::List { .. } | Frame::Item(..));
        self.frames.push(if deep { Frame::Flat } else { frame });
    }

    /// The footnotes defined, at the foot by number: referred to first, then the rest as defined.
    fn footnotes(&mut self) -> Option<Block> {
        let notes = std::mem::take(&mut self.notes);
        if notes.is_empty() || !self.room() {
            return None;
        }
        let mut notes: Vec<Note> = notes
            .into_iter()
            .map(|(name, blocks)| {
                let next = self.numbers.len() + 1;
                let n = *self.numbers.entry(name.clone()).or_insert(next);
                Note { n, name, blocks }
            })
            .collect();
        notes.sort_by_key(|note| note.n);
        Some(Block::Footnotes(notes))
    }

    /// Footnote `name`'s number: the next one the first time it is referred to.
    fn number(&mut self, name: &str) -> usize {
        let next = self.numbers.len() + 1;
        *self.numbers.entry(name.to_string()).or_insert(next)
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
            // Slack's `<!here>` reads as HTML to the parser.
            Event::InlineHtml(text) if mention(&text).is_some_and(|(len, _)| len == text.len()) => {
                let name = mention(&text).map(|(_, name)| name).unwrap_or_default();
                self.span(name, Style { mention: true, ..self.style })
            }
            // An HTML block's lines past its first count as blocks: a `<pre>` of a million lines is cut.
            Event::Html(_)
                if matches!(self.frames.last(), Some(Frame::Html(spans)) if !spans.is_empty())
                    && !self.room_but_one() => {}
            // Raw HTML is the text it is: never a tag, nor an address or a mention.
            Event::Html(text) | Event::InlineHtml(text) => self.span(clean(&text), self.style),
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.text("\n"),
            Event::Rule => self.block(Block::Rule),
            Event::TaskListMarker(done) => {
                if let Some(Frame::Item(task, ..)) =
                    self.frames.iter_mut().rev().find(|frame| matches!(frame, Frame::Item(..)))
                {
                    *task = Some(done);
                }
            }
            Event::FootnoteReference(name) => {
                let name = note_name(&name);
                let n = self.number(&name);
                let text = self.budget(&format!("[{n}]"));
                let anchor = Some(format!("^{name}")).filter(|_| name.len() <= MAX_ANCHOR);
                self.push_span(Span { text, style: self.style, link: None, anchor });
            }
            Event::InlineMath(text) | Event::DisplayMath(text) => self.text(&text),
        }
    }

    fn start(&mut self, tag: Tag) {
        if matches!(tag, Tag::Paragraph | Tag::Heading { .. } | Tag::Item | Tag::TableCell | Tag::HtmlBlock) {
            self.line = 0;
        }
        match tag {
            Tag::Paragraph => self.frames.push(Frame::Para(Vec::new())),
            Tag::HtmlBlock => self.frames.push(Frame::Html(Vec::new())),
            Tag::Heading { level, .. } => self.frames.push(Frame::Heading(level as u8, Vec::new())),
            Tag::BlockQuote(kind) => self.open(Frame::Quote(kind.map(callout), Vec::new())),
            Tag::FootnoteDefinition(name) => self.frames.push(Frame::Note(note_name(&name), Vec::new())),
            Tag::CodeBlock(kind) => {
                let label = match kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().map(|word| cap(&clean(word), 40)),
                    CodeBlockKind::Indented => None,
                };
                self.frames.push(Frame::Code { label: label.filter(|label| !label.is_empty()), text: String::new() });
            }
            Tag::List(start) => self.open(Frame::List { start, items: Vec::new() }),
            Tag::Item => self.open(Frame::Item(None, Vec::new(), Vec::new())),
            Tag::Table(align) => {
                let align = align.iter().map(|a| match a {
                    Alignment::Center => Align::Center,
                    Alignment::Right => Align::Right,
                    Alignment::Left | Alignment::None => Align::Left,
                });
                let align = align.collect();
                self.frames.push(Frame::Table { align, rows: Vec::new(), row: Vec::new(), cell: Vec::new() })
            }
            Tag::Emphasis => self.style.emphasis = true,
            Tag::Strong => self.style.strong = true,
            Tag::Strikethrough => self.style.strike = true,
            Tag::Link { dest_url, .. } => {
                let raw = dest_url.to_string();
                let link = links::target(&raw, self.repo).map(|target| Link { target, raw: clean(&raw) });
                // `#tables`: a heading in this text.
                let anchor =
                    raw.strip_prefix('#').filter(|name| !name.is_empty() && name.len() <= MAX_ANCHOR).map(clean);
                self.links.push(Open { link, anchor, raw, text: String::new() });
            }
            Tag::Image { dest_url, .. } => self.image = Some((dest_url.to_string(), String::new())),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph
            | TagEnd::HtmlBlock
            | TagEnd::Heading(_)
            | TagEnd::BlockQuote(_)
            | TagEnd::CodeBlock
            | TagEnd::List(_)
            | TagEnd::Item
            | TagEnd::FootnoteDefinition
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
                    self.push_span(Span { text: real, style: self.style, link: None, anchor: None });
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
                let shown = if alt.trim().is_empty() { "image".to_string() } else { alt.clone() };
                if let (true, Some(Target::File(path))) = (picture, &target) {
                    self.pictures.push((path.clone(), cap(&shown, MAX_LINE)));
                    // A picture drawn is not named again; inside a link, its alt text is the link's.
                    if self.links.is_empty() {
                        return;
                    }
                }
                // An image inside a link is its text: alt text that names an address is checked too.
                if let Some(open) = self.links.last_mut() {
                    open.text.push_str(&alt);
                }
                // Never loaded from where it says: a link to it, under its alt text.
                let link = target.map(|target| Link { target, raw: clean(&raw) });
                let text = self.budget(&format!("[image: {shown}]"));
                self.push_span(Span { text, style: self.style, link, anchor: None });
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
            Frame::Quote(callout, blocks) => self.block(Block::Quote { callout, blocks }),
            Frame::List { start, items } => self.block(Block::List { start, items }),
            Frame::Item(task, mut blocks, spans) => {
                if spans.iter().any(|span| !span.text.trim().is_empty()) && self.room() {
                    blocks.push(Block::Para(spans));
                }
                // A tight item's pictures stay in it.
                for (path, alt) in std::mem::take(&mut self.pictures) {
                    if self.room() {
                        blocks.push(Block::Image { path, alt });
                    }
                }
                match self.frames.last_mut() {
                    Some(Frame::List { items, .. }) => items.push(Item { task, blocks }),
                    _ => blocks.into_iter().for_each(|block| self.block(block)),
                }
            }
            // A paragraph that held only a picture drawn below it is not drawn empty.
            Frame::Para(spans) if spans.iter().all(|span| span.text.trim().is_empty()) => self.block_pictures(),
            Frame::Para(spans) => self.block(Block::Para(spans)),
            // Its lines' breaks, less the last: no blank line under it.
            Frame::Html(mut spans) => {
                while let Some(last) = spans.last_mut() {
                    let kept = last.text.trim_end().len();
                    last.text.truncate(kept);
                    if !last.text.is_empty() {
                        break;
                    }
                    spans.pop();
                }
                if !spans.is_empty() {
                    self.block(Block::Para(spans));
                }
            }
            Frame::Heading(level, spans) => {
                let slug = self.unique(slug(&spans.iter().map(|span| span.text.as_str()).collect::<String>()));
                self.block(Block::Heading { level, slug, spans })
            }
            Frame::Note(name, blocks) => self.notes.push((name, blocks)),
            Frame::Code { label, text } => {
                let copy = text.strip_suffix('\n').unwrap_or(&text).to_string();
                let lines: Vec<&str> = copy.split('\n').collect();
                let most = self.caps.code_lines;
                let shown: Vec<String> = lines.iter().take(most).map(|line| cap(line, MAX_LINE)).collect();
                let more = lines.len().saturating_sub(most);
                let shown = shown.join("\n");
                let marks = label.as_deref().map(|label| highlight::marks(label, &shown)).unwrap_or_default();
                self.block(Block::Code { label, shown, copy, more, marks });
            }
            Frame::Table { align, mut rows, row, cell } => {
                let mut row = row;
                if !cell.is_empty() {
                    row.push(cell);
                }
                if !row.is_empty() {
                    rows.push(row);
                }
                let widths = column_widths(&rows);
                self.block(Block::Table { align, widths, rows });
            }
            Frame::Flat => {}
        }
    }

    /// `slug` once: a second alike is `slug-1`, then `slug-2`, past any already given (`# A-1`).
    fn unique(&mut self, slug: String) -> String {
        let mut n = self.suffixes.get(&slug).copied().unwrap_or(0);
        let mut out = slug.clone();
        while self.slugs.contains(&out) {
            n += 1;
            out = format!("{slug}-{n}");
        }
        self.suffixes.insert(slug, n);
        self.slugs.insert(out.clone());
        out
    }

    /// The pictures named in a block not drawn, alone.
    fn block_pictures(&mut self) {
        for (path, alt) in std::mem::take(&mut self.pictures) {
            if self.room() {
                self.add(Block::Image { path, alt });
            }
        }
    }

    /// Adds a block to the innermost container, then any pictures it named. Past `MAX_BLOCKS`,
    /// counted and left out.
    fn block(&mut self, block: Block) {
        // A tight list item's text, made a paragraph here, counts with the block after it.
        if self.room() {
            self.add(block);
        }
        self.block_pictures();
    }

    /// `block` into the innermost container, already counted.
    fn add(&mut self, block: Block) {
        match self.frames.iter_mut().rev().find(|frame| !matches!(frame, Frame::Flat)) {
            Some(Frame::Root(blocks) | Frame::Quote(_, blocks) | Frame::Note(_, blocks)) => blocks.push(block),
            Some(Frame::Item(_, blocks, spans)) => {
                if !spans.is_empty() {
                    blocks.push(Block::Para(std::mem::take(spans)));
                }
                blocks.push(block);
            }
            Some(Frame::List { items, .. }) => items.push(Item { task: None, blocks: vec![block] }),
            // A block inside a run of text (a rule in a cell): its text, if it has any.
            Some(_) | None => {}
        }
    }

    /// As `room`, keeping one block for the frame being read (an HTML block's line).
    fn room_but_one(&mut self) -> bool {
        if self.blocks + 1 < self.caps.blocks {
            self.room()
        } else {
            self.cut += 1;
            false
        }
    }

    /// True if one more block may be drawn; else it is counted as cut.
    fn room(&mut self) -> bool {
        let room = self.blocks < self.caps.blocks;
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
        // Inside a link its text is the link's.
        if !self.links.is_empty() {
            return self.span(text, self.style);
        }
        for piece in pieces(&text) {
            match piece {
                Piece::Text(text) => self.span(text.to_string(), self.style),
                Piece::Url(url) => {
                    let link = links::target(url, None).map(|target| Link { target, raw: url.to_string() });
                    let text = self.budget(url);
                    self.push_span(Span { text, style: self.style, link, anchor: None });
                }
                Piece::Mention(name) => self.span(name, Style { mention: true, ..self.style }),
            }
        }
    }

    fn span(&mut self, text: String, style: Style) {
        let text = self.budget(&text);
        if let Some(open) = self.links.last_mut() {
            open.text.push_str(&text);
        }
        let link = self.links.last().and_then(|open| open.link.clone());
        let anchor = self.links.last().and_then(|open| open.anchor.clone());
        self.push_span(Span { text, style, link, anchor });
    }

    fn push_span(&mut self, span: Span) {
        match self.frames.iter_mut().rev().find(|frame| !matches!(frame, Frame::Flat)) {
            Some(Frame::Para(spans) | Frame::Html(spans) | Frame::Heading(_, spans) | Frame::Item(_, _, spans)) => {
                spans.push(span)
            }
            Some(Frame::Table { cell, .. }) => cell.push(span),
            Some(Frame::Code { text, .. }) => text.push_str(&span.text),
            // Text straight in a container (raw HTML at the top): a paragraph of its own.
            Some(_) | None => self.block(Block::Para(vec![span])),
        }
    }
}

/// The blocks as plain text, for Copy.
pub fn plain(blocks: &[Block]) -> String {
    let spans = |spans: &[Span]| spans.iter().map(|span| span.text.as_str()).collect::<String>();
    let mut parts = Vec::new();
    for block in blocks {
        parts.push(match block {
            Block::Para(s) | Block::Heading { spans: s, .. } => spans(s),
            Block::Code { copy, .. } => copy.clone(),
            Block::Quote { blocks, .. } => {
                plain(blocks).lines().map(|line| format!("> {line}")).collect::<Vec<_>>().join("\n")
            }
            Block::List { start, items } => items
                .iter()
                .enumerate()
                .map(|(at, item)| {
                    let task = match item.task {
                        Some(true) => "[x] ",
                        Some(false) => "[ ] ",
                        None => "",
                    };
                    match start {
                        Some(n) => format!("{}. {task}{}", n + at as u64, plain(&item.blocks)),
                        None => format!("- {task}{}", plain(&item.blocks)),
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Block::Table { rows, .. } => rows
                .iter()
                .map(|row| row.iter().map(|cell| spans(cell)).collect::<Vec<_>>().join("\t"))
                .collect::<Vec<_>>()
                .join("\n"),
            Block::Image { alt, .. } => format!("[image: {alt}]"),
            Block::Rule => "---".into(),
            Block::Footnotes(notes) => {
                notes.iter().map(|note| format!("[{}] {}", note.n, plain(&note.blocks))).collect::<Vec<_>>().join("\n")
            }
            Block::Cut { blocks } => format!("[{blocks} more blocks]"),
        });
    }
    parts.join("\n\n")
}

/// A block's kind, as the state counts them: a quote with a callout is a `callout`.
pub fn kind(block: &Block) -> &'static str {
    match block {
        Block::Para(_) => "para",
        Block::Heading { .. } => "heading",
        Block::Code { .. } => "code",
        Block::Quote { callout: None, .. } => "quote",
        Block::Quote { callout: Some(_), .. } => "callout",
        Block::List { .. } => "list",
        Block::Table { .. } => "table",
        Block::Image { .. } => "image",
        Block::Rule => "rule",
        Block::Footnotes(_) => "footnotes",
        Block::Cut { .. } => "cut",
    }
}

/// The blocks a block holds: a quote's, each list item's, each footnote's.
pub fn inner(block: &Block) -> Vec<&[Block]> {
    match block {
        Block::Quote { blocks, .. } => vec![blocks],
        Block::List { items, .. } => items.iter().map(|item| item.blocks.as_slice()).collect(),
        Block::Footnotes(notes) => notes.iter().map(|note| note.blocks.as_slice()).collect(),
        _ => Vec::new(),
    }
}

/// The runs of text a block draws itself (not those of the blocks it holds).
fn own_spans(block: &Block) -> Vec<&[Span]> {
    match block {
        Block::Para(spans) | Block::Heading { spans, .. } => vec![spans],
        Block::Table { rows, .. } => rows.iter().flatten().map(Vec::as_slice).collect(),
        _ => Vec::new(),
    }
}

/// Every link the blocks hold, in order.
pub fn links_of(blocks: &[Block]) -> Vec<&Link> {
    let mut out = Vec::new();
    for block in blocks {
        own_spans(block).into_iter().flatten().for_each(|span| out.extend(span.link.as_ref()));
        inner(block).into_iter().for_each(|blocks| out.extend(links_of(blocks)));
    }
    out
}

/// The top-level block a `#anchor` goes to: the one holding the heading of that slug (any case),
/// or for `^name` the footnotes, if that footnote is among them.
pub fn anchor_at(blocks: &[Block], anchor: &str) -> Option<usize> {
    fn holds(block: &Block, anchor: &str) -> bool {
        match block {
            Block::Heading { slug, .. } => slug.eq_ignore_ascii_case(anchor),
            Block::Footnotes(notes) => {
                anchor.strip_prefix('^').is_some_and(|name| notes.iter().any(|n| n.name == name))
            }
            _ => inner(block).into_iter().flatten().any(|block| holds(block, anchor)),
        }
    }
    blocks.iter().position(|block| holds(block, anchor))
}

/// A heading's slug as GitHub makes it: lower case, spaces as `-`, other punctuation dropped.
/// "Quotes and callouts" → `quotes-and-callouts`.
pub fn slug(text: &str) -> String {
    let text = text.trim().to_lowercase();
    text.chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            _ if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .collect()
}

/// Each column's width in characters: its widest cell, at least 3, at most `MAX_COLUMN`.
pub fn column_widths(rows: &[Vec<Vec<Span>>]) -> Vec<usize> {
    let mut widths: Vec<usize> = Vec::new();
    for row in rows {
        for (at, cell) in row.iter().enumerate() {
            let chars = cell.iter().map(|span| span.text.chars().count()).sum::<usize>();
            if widths.len() <= at {
                widths.resize(at + 1, 3);
            }
            widths[at] = widths[at].max(chars.clamp(3, MAX_COLUMN));
        }
    }
    widths
}

fn callout(kind: BlockQuoteKind) -> Callout {
    match kind {
        BlockQuoteKind::Note => Callout::Note,
        BlockQuoteKind::Tip => Callout::Tip,
        BlockQuoteKind::Important => Callout::Important,
        BlockQuoteKind::Warning => Callout::Warning,
        BlockQuoteKind::Caution => Callout::Caution,
    }
}

/// A run of text, as written, an address that opens (`https://…`, not in a link), or a Slack
/// mention drawn as its name.
#[derive(Debug, PartialEq, Eq)]
enum Piece<'a> {
    Text(&'a str),
    Url(&'a str),
    Mention(String),
}

/// `text` in pieces: its bare web addresses and Slack mentions (`<@U0123>`, `<@U0123|ana>`,
/// `<#C0123|general>`, `<!here>`) apart from the rest. An address ends at a space or `<>"`, less
/// the punctuation after it (`see https://e.com/a.` ends before the `.`) and a `)` it did not open.
fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let (mut from, mut at) = (0, 0);
    while at < text.len() {
        let rest = &text[at..];
        // As GitHub: an address after a space or `(*_~`, a mention after anything but a letter.
        let prev = text[..at].chars().next_back();
        let found = if rest.starts_with('<') && !prev.is_some_and(char::is_alphanumeric) {
            mention(rest).map(|(len, name)| (len, Piece::Mention(name)))
        } else if prev.is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '*' | '_' | '~')) {
            url(rest).map(|len| (len, Piece::Url(&rest[..len])))
        } else {
            None
        };
        match found {
            Some((len, piece)) => {
                if from < at {
                    out.push(Piece::Text(&text[from..at]));
                }
                out.push(piece);
                at += len;
                from = at;
            }
            None => at += rest.chars().next().map_or(1, char::len_utf8),
        }
    }
    if from < text.len() {
        out.push(Piece::Text(&text[from..]));
    }
    out
}

/// Footnote labels as the parser matches them: in any case (`[^A]` is `[^a]`).
fn note_name(name: &str) -> String {
    clean(name).to_lowercase()
}

/// The longest `#anchor` or footnote name kept: each span of a link's text holds a copy.
const MAX_ANCHOR: usize = 256;

/// The longest bare address read (`links::target` opens none longer), and the longest mention.
const MAX_URL: usize = 2048;
const MAX_MENTION: usize = 120;

/// The length of the web address `text` starts with, if it is one that opens.
fn url(text: &str) -> Option<usize> {
    let lower = text.get(..8)?.to_ascii_lowercase();
    if !lower.starts_with("https://") && !lower.starts_with("http://") {
        return None;
    }
    // Read no further than the longest address that opens: a long run is never scanned twice.
    let scan = head(text, MAX_URL + 1);
    // No end within the window, and more after it: longer than any that opens.
    let mut end = match scan.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"')) {
        Some(end) => end,
        None if scan.len() == text.len() => text.len(),
        None => return None,
    };
    if end > MAX_URL {
        return None;
    }
    // Brackets counted once, then as each is trimmed.
    let (opened, mut closed) = (text[..end].matches('(').count(), text[..end].matches(')').count());
    loop {
        let last = text[..end].chars().next_back()?;
        let unopened = last == ')' && opened < closed;
        if matches!(last, '.' | ',' | ':' | ';' | '!' | '?' | '\'' | '*' | '_' | '~') || unopened {
            end -= last.len_utf8();
            closed -= usize::from(last == ')');
        } else {
            break;
        }
    }
    let url = &text[..end];
    matches!(links::target(url, None), Some(Target::Web(_))).then_some(end)
}

/// A Slack mention `text` starts with: its length, and the name it is drawn as.
fn mention(text: &str) -> Option<(usize, String)> {
    let close = head(text, MAX_MENTION).find('>')?;
    let body = &text[1..close];
    let (id, label) = body.split_once('|').map_or((body, None), |(id, label)| (id, Some(label)));
    let sigil = id.chars().next()?;
    let id = &id[sigil.len_utf8()..];
    let label = label.filter(|label| !label.is_empty() && !label.contains(['<', '\n']));
    let name = match sigil {
        '@' | '#' if !id.is_empty() && id.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) => {
            format!("{}{}", if sigil == '#' { '#' } else { '@' }, label.unwrap_or(id))
        }
        '!' if matches!(id, "here" | "channel" | "everyone") => format!("@{id}"),
        _ => return None,
    };
    Some((close + 1, name))
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
        assert!(
            matches!(&blocks[0], Block::Heading { level: 1, slug, spans } if texts(spans) == "Title" && slug == "title")
        );
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
                more: 0,
                marks: vec![(0..2, highlight::Token::Keyword)],
            }
        );
        assert!(matches!(&blocks[3], Block::Quote { callout: None, blocks } if plain(blocks) == "quoted"));
        assert!(matches!(&blocks[4], Block::List { start: None, items } if items.len() == 2));
        assert!(matches!(&blocks[5], Block::List { start: Some(1), .. }));
        assert!(matches!(&blocks[6], Block::Table { rows, .. } if rows.len() == 2 && texts(&rows[1][1]) == "2"));
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
        assert!(blocks.contains(&Block::Image { path: "/w/app/docs/shot.png".into(), alt: "shot".into() }));
        assert!(!blocks.iter().any(|b| matches!(b, Block::Image { path, .. } if path.contains("evil"))));
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
                blocks.iter().map(|b| 1 + inner(b).into_iter().map(all).sum::<usize>()).sum()
            }
            all(blocks)
        };
        assert!(count(&parse(&"- x\n".repeat(MAX_BLOCKS * 10), None)) <= MAX_BLOCKS * 2 + 2);
        assert!(count(&parse(&"![x](a.png)\n\n".repeat(MAX_BLOCKS * 2), REPO)) <= MAX_BLOCKS * 2 + 2);
        let deep = format!("{}x", "> ".repeat(500));
        assert!(!parse(&deep, None).is_empty());
    }

    #[test]
    fn headings_have_levels_and_unique_slugs_a_link_finds() {
        let text =
            "# Top\n\n## Quotes and callouts!\n\n> ### Inside\n\n## Top\n\n[go](#quotes-and-callouts) [up](#TOP-1)";
        let blocks = parse(text, None);
        let heads: Vec<(u8, &str)> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Heading { level, slug, .. } => Some((*level, slug.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(heads, [(1, "top"), (2, "quotes-and-callouts"), (2, "top-1")]);
        // A suffix already taken by a heading of its own is passed over.
        let slugs: Vec<String> = parse("# A\n\n# A-1\n\n# A\n\n# A", None)
            .into_iter()
            .filter_map(|b| match b {
                Block::Heading { slug, .. } => Some(slug),
                _ => None,
            })
            .collect();
        assert_eq!(slugs, ["a", "a-1", "a-2", "a-3"]);
        let Block::Para(spans) = &blocks[4] else { panic!("{blocks:#?}") };
        let anchors: Vec<_> = spans.iter().filter_map(|s| s.anchor.as_deref()).collect();
        assert_eq!(anchors, ["quotes-and-callouts", "TOP-1"]);
        assert!(spans.iter().all(|s| s.link.is_none()), "an anchor opens nothing");
        assert_eq!(anchor_at(&blocks, "quotes-and-callouts"), Some(1));
        assert_eq!(anchor_at(&blocks, "TOP-1"), Some(3));
        // A heading in a quote: the quote is where the view goes.
        assert_eq!(anchor_at(&blocks, "inside"), Some(2));
        assert_eq!(anchor_at(&blocks, "nowhere"), None);
    }

    #[test]
    fn tasks_tables_callouts_and_footnotes_have_their_structure() {
        let text = "- [x] done\n- [ ] not\n  - [x] inside\n- plain\n\n\
                    | L | C | R |\n|:--|:-:|--:|\n| a | longer cell | 1 |\n\n\
                    > [!WARNING]\n> careful\n\n\
                    Two notes.[^b][^a] Again.[^b]\n\n[^a]: First defined.\n[^b]: Second defined.\n[^c]: Never used.\n";
        let blocks = parse(text, None);
        let Block::List { items, .. } = &blocks[0] else { panic!("{blocks:#?}") };
        assert_eq!(items.iter().map(|item| item.task).collect::<Vec<_>>(), [Some(true), Some(false), None]);
        let Block::List { items: nested, .. } = &items[1].blocks[1] else { panic!("{items:#?}") };
        assert_eq!(nested[0].task, Some(true));
        assert!(plain(&blocks[..1]).starts_with("- [x] done"));
        let Block::Table { align, widths, .. } = &blocks[1] else { panic!() };
        assert_eq!(align, &[Align::Left, Align::Center, Align::Right]);
        assert_eq!(widths, &[3, 11, 3]);
        assert!(
            matches!(&blocks[2], Block::Quote { callout: Some(Callout::Warning), blocks } if plain(blocks) == "careful")
        );
        // Numbered as referred to; the one never referred to after; at the foot.
        let Block::Para(spans) = &blocks[3] else { panic!() };
        assert_eq!(texts(spans), "Two notes.[1][2] Again.[1]");
        assert_eq!(spans[1].anchor.as_deref(), Some("^b"));
        let Some(Block::Footnotes(notes)) = blocks.last() else { panic!("{blocks:#?}") };
        let got: Vec<_> = notes.iter().map(|n| (n.n, n.name.as_str(), plain(&n.blocks))).collect();
        assert_eq!(
            got,
            [(1, "b", "Second defined.".into()), (2, "a", "First defined.".into()), (3, "c", "Never used.".into())]
        );
        assert_eq!(anchor_at(&blocks, "^a"), Some(blocks.len() - 1));
        // Labels in any case are one footnote, as the parser matches them.
        let blocks = parse("See [^Note].\n\n[^note]: Details.", None);
        let Some(Block::Footnotes(notes)) = blocks.last() else { panic!("{blocks:#?}") };
        assert_eq!((plain(&blocks[..1]).as_str(), notes[0].n), ("See [1].", 1));
        assert_eq!(anchor_at(&blocks, "^note"), Some(1));
        assert_eq!(
            column_widths(&[
                vec![vec![]],
                vec![vec![Span { text: "x".repeat(99), style: Style::default(), link: None, anchor: None }]]
            ]),
            [MAX_COLUMN]
        );
    }

    #[test]
    fn bare_addresses_are_links_and_slack_mentions_names() {
        let text = "See https://example.com/bare. And (https://e.com/a_(b)) or https://e.com/x, \
                    `https://code.com` [https://text.com](https://text.com) <@U0123>, <@U0456|ana> <#C1|general> <!here> \
                    <@lower> a@https://no.com";
        let blocks = parse(text, None);
        let Block::Para(spans) = &blocks[0] else { panic!() };
        let raws: Vec<&str> = spans.iter().filter_map(|s| s.link.as_ref()).map(|l| l.raw.as_str()).collect();
        assert_eq!(raws, ["https://example.com/bare", "https://e.com/a_(b)", "https://e.com/x", "https://text.com"]);
        let mentions: Vec<&str> = spans.iter().filter(|s| s.style.mention).map(|s| s.text.as_str()).collect();
        assert_eq!(mentions, ["@U0123", "@ana", "#general", "@here"]);
        let all = texts(spans);
        assert!(all.contains("bare. And (") && all.contains("<@lower>") && all.contains("a@https://no.com"), "{all}");
        assert!(spans.iter().any(|s| s.style.code && s.text == "https://code.com" && s.link.is_none()));
        // Raw HTML is never split: an address in it is text.
        let html = parse("<div>https://e.com</div>", None);
        assert!(links_of(&html).is_empty());
    }

    /// Hostile runs are read within a window, never to the end of the text from each place in it.
    #[test]
    fn mentions_and_addresses_are_read_within_a_bound() {
        // A mention's `>` past `MAX_MENTION` is not looked for: none, however long the text.
        let far = format!("<@U{}>", "A".repeat(MAX_MENTION));
        assert_eq!(mention(&far), None);
        assert_eq!(mention(&format!("<@U{}>", "A".repeat(MAX_MENTION - 5))).map(|(len, _)| len), Some(MAX_MENTION - 1));
        let lt = "<".repeat(200_000);
        assert_eq!(plain(&parse_with(&lt, None, FILE)).chars().count(), MAX_LINE + 1, "its line, capped");
        // An address longer than `MAX_URL` is text; one of brackets is trimmed by counts kept.
        assert_eq!(url(&format!("https://e.com/{}", "a".repeat(MAX_URL))), None);
        // Longer than the window by a character that straddles its edge: not cut short there.
        assert_eq!(url(&format!("https://e.com/{}é", "a".repeat(MAX_URL - 14))), None);
        let long_anchor = format!("[x](#{})", "a".repeat(MAX_ANCHOR + 1));
        let Block::Para(spans) = &parse(&long_anchor, None)[0] else { panic!() };
        assert!(spans.iter().all(|s| s.anchor.is_none()));
        let brackets = format!("https://e.com/{}", ")".repeat(MAX_URL - 14));
        assert_eq!(url(&brackets), Some("https://e.com/".len()));
        let opens = format!("x https://e.com/{}", "(".repeat(100_000));
        assert!(links_of(&parse_with(&opens, None, FILE)).is_empty());
        // `é` across the window's edge cuts it at a character.
        assert_eq!(mention(&format!("<@U{}é>", "A".repeat(MAX_MENTION - 4))), None);
    }

    #[test]
    fn a_picture_drawn_is_not_named_too_and_html_leaves_no_blank_line() {
        let blocks = parse("Before.\n\n![shot](docs/shot.png)\n\nAfter.", REPO);
        assert_eq!(blocks.len(), 3, "{blocks:#?}");
        assert_eq!(blocks[1], Block::Image { path: "/w/app/docs/shot.png".into(), alt: "shot".into() });
        assert_eq!(plain(&blocks), "Before.\n\n[image: shot]\n\nAfter.", "Copy names it once");
        // Inside text it is drawn under it, its words gone from the line.
        let blocks = parse("See ![shot](docs/shot.png) here.", REPO);
        assert!(matches!(&blocks[..], [Block::Para(s), Block::Image { .. }] if texts(s) == "See  here."));
        let blocks = parse("<details><summary>Open</summary>\n\nInside.\n\n</details>\n\nAfter.", None);
        let paras: Vec<String> = blocks.iter().map(|b| plain(std::slice::from_ref(b))).collect();
        assert_eq!(paras, ["<details><summary>Open</summary>", "Inside.", "</details>", "After."]);
        let blocks = parse("<div>\na\n</div>\n", None);
        assert_eq!(plain(&blocks), "<div>\na\n</div>");
        // A tight item's picture stays in its item; an item of only a picture draws no empty line.
        let blocks = parse("- ![a](a.png)\n- b\n  - c\n\nAfter.", REPO);
        let Block::List { items, .. } = &blocks[0] else { panic!("{blocks:#?}") };
        assert_eq!(items[0].blocks, [Block::Image { path: "/w/app/a.png".into(), alt: "a".into() }]);
        assert!(!blocks.iter().any(|b| matches!(b, Block::Image { .. })), "none after the list: {blocks:#?}");
        // An HTML block's lines are counted: a long `<pre>` is cut at the caps.
        let pre = format!("<pre>\n{}</pre>\n", "x\n".repeat(MAX_BLOCKS * 10));
        let Block::Para(spans) = &parse(&pre, None)[0] else { panic!() };
        assert!(texts(spans).lines().count() <= MAX_BLOCKS + 1);
        // A block with no language has no label, and no colours.
        let blocks = parse("    indented\n\n```\nplain\n```", None);
        assert!(blocks.iter().all(|b| matches!(b, Block::Code { label: None, marks, .. } if marks.is_empty())));
    }

    /// The fixture the viewer's pictures are taken of (`tests/fixtures/markdown`): every kind of
    /// block it holds is drawn, each fence's language named, its picture beside it local.
    #[test]
    fn the_fixture_gives_every_kind_of_block() {
        let text = include_str!("../tests/fixtures/markdown/fixture.md");
        let blocks = parse_with(text, Some("/fixture"), FILE);
        let mut labels = Vec::new();
        fn walk<'a>(blocks: &'a [Block], labels: &mut Vec<&'a str>, kinds: &mut Vec<&'static str>) {
            for block in blocks {
                if let Block::Code { label, .. } = block {
                    labels.extend(label.as_deref());
                }
                inner(block).into_iter().for_each(|blocks| walk(blocks, labels, kinds));
                kinds.push(kind(block));
            }
        }
        let mut kinds = Vec::new();
        walk(&blocks, &mut labels, &mut kinds);
        let count = |kind: &str| kinds.iter().filter(|k| **k == kind).count();
        let langs = ["rust", "ts", "js", "python", "sh", "json", "yaml", "toml", "go", "swift", "sql", "diff"];
        assert_eq!(labels, langs);
        assert_eq!((count("table"), count("rule"), count("cut")), (2, 2, 0));
        assert!(count("heading") >= 10 && count("list") >= 6, "{kinds:?}");
        // The quote and the one inside it; `[!NOTE]`, `[!WARNING]` and `[!TIP]`; both footnotes, at the foot.
        assert_eq!((count("quote"), count("callout")), (2, 3));
        let Some(Block::Footnotes(notes)) = blocks.last() else { panic!("{kinds:?}") };
        assert_eq!(
            notes.iter().map(|note| (note.n, note.name.as_str())).collect::<Vec<_>>(),
            [(1, "one"), (2, "long")]
        );
        assert!(blocks.contains(&Block::Image { path: "/fixture/checker.png".into(), alt: "checker".into() }));
        let web = links_of(&blocks).iter().filter(|link| matches!(link.target, Target::Web(_))).count();
        assert!(web >= 5, "{web} web links");
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
                own_spans(block).into_iter().for_each(|s| spans.extend(s.iter().cloned()));
                inner(block).into_iter().for_each(|blocks| walk(blocks, spans));
                match block {
                    Block::Code { shown, label, marks, .. } => {
                        assert!(marks.iter().all(|(range, _)| shown.get(range.clone()).is_some()));
                        let text = format!("{}{shown}", label.clone().unwrap_or_default());
                        spans.push(Span { text, style: Style::default(), link: None, anchor: None });
                    }
                    Block::Image { path, alt } => {
                        assert!(path.starts_with("/w/app/"), "{path}");
                        spans.push(Span { text: alt.clone(), style: Style::default(), link: None, anchor: None });
                    }
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
