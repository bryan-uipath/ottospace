//! Shell half of a terminal pane: a PTY running the user's shell, parsed into a grid by
//! `alacritty_terminal`, and the code that paints that grid.

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, OnceLock};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll as GridScroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vi_mode::ViMotion;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};
use gpui::{
    App, Bounds, ContentMask, FontStyle, FontWeight, Hsla, Pixels, Size, TextRun, UnderlineStyle, Window, fill, font,
    point, px, rgb, size,
};
use ottospace_core::terminal::{
    CellPoint, Motion, Proc, Processes, Scroll, SelectKind, Wheel, Work, paste, running, wheel_scrolls,
};
use ottospace_core::{palette, settings, theme};
use ottospace_mac::process;

/// The font terminals draw with, once `choose_font` has run. Until then, the default.
static FONT: OnceLock<settings::Terminal> = OnceLock::new();

fn terminal_font() -> settings::Terminal {
    FONT.get().cloned().unwrap_or_default()
}

/// Takes the terminal font from the settings. A font that is not installed would be replaced by
/// some other one, perhaps not monospaced: the default is used, and the reply says so.
pub fn choose_font(window: &Window) -> Option<String> {
    let wanted = &crate::config::get().settings.terminal;
    let installed = window.text_system().all_font_names().contains(&wanted.font);
    let font = if installed {
        wanted.clone()
    } else {
        settings::Terminal { font_size: wanted.font_size, ..Default::default() }
    };
    let _ = FONT.set(font);
    (!installed).then(|| format!("settings.json: the font \"{}\" is not installed; using Menlo", wanted.font))
}

/// The font terminals draw with, for the state dump: `{"name":"Menlo","size":13.0}`.
pub fn font_in_use() -> serde_json::Value {
    let font = terminal_font();
    serde_json::json!({ "name": font.font, "size": font.font_size })
}

/// Size of one character cell: the font's advance by the line height.
pub fn cell_size(window: &Window) -> Size<Pixels> {
    let (text, chosen) = (window.text_system(), terminal_font());
    let size_px = px(chosen.font_size);
    let width = text.em_advance(text.resolve_font(&font(chosen.font.clone())), size_px).unwrap_or(size_px * 0.6);
    size(width, px(chosen.line_height()))
}

/// State the PTY reader thread shares with the UI thread.
#[derive(Default)]
struct Shared {
    dirty: AtomicBool,
    exited: AtomicBool,
    input: OnceLock<EventLoopSender>,
}

#[derive(Clone)]
struct Listener(Arc<Shared>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::Wakeup => self.0.dirty.store(true, Relaxed),
            // Answers the terminal owes the program, e.g. a cursor position report.
            Event::PtyWrite(text) => {
                if let Some(input) = self.0.input.get() {
                    let _ = input.send(Msg::Input(text.into_bytes().into()));
                }
            }
            Event::Exit | Event::ChildExit(_) => self.0.exited.store(true, Relaxed),
            _ => {}
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Grid {
    cols: usize,
    rows: usize,
}

impl Dimensions for Grid {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// A run of cells on one row that share a style.
pub struct Span {
    text: String,
    fg: u32,
    bg: Option<u32>,
    bold: bool,
    italic: bool,
    underline: bool,
}

/// What to paint: styled rows, and the cursor's (row, column) when it is visible.
pub struct Screen {
    rows: Vec<Vec<Span>>,
    cursor: Option<(usize, usize)>,
}

pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    shared: Arc<Shared>,
    input: EventLoopSender,
    grid: Grid,
    wheel: Wheel,
    /// The process started in the PTY (the shell, or `login` wrapping it), and the PTY's master
    /// side: to ask what runs besides the shell.
    child: u32,
    pty: Option<File>,
}

impl Terminal {
    /// Starts the user's default shell (or `$OTTOSPACE_SHELL`) in `cwd`, with `vars` added to its
    /// environment, e.g. `[("OTTOSPACE_PANE", "3")]`. Without `cwd`: the current directory, or
    /// the home directory when that is `/` (an app started from Finder or the Dock).
    pub fn spawn(vars: &[(&str, String)], cwd: Option<&str>) -> io::Result<Self> {
        let grid = Grid { cols: 80, rows: 24 };
        let shared = Arc::new(Shared::default());
        let listener = Listener(shared.clone());
        let mut env = HashMap::from([("TERM".to_string(), "xterm-256color".to_string())]);
        env.extend(vars.iter().map(|(name, value)| (name.to_string(), value.clone())));
        // `$OTTOSPACE_SHELL` replaces the login shell. Tests use it: a login shell with a heavy
        // profile can take a minute to reach a prompt on a busy machine.
        let shell = std::env::var("OTTOSPACE_SHELL").ok().map(|program| tty::Shell::new(program, Vec::new()));
        let at_root = std::env::current_dir().is_ok_and(|dir| dir.parent().is_none());
        let home = std::env::var_os("HOME").filter(|_| at_root).map(PathBuf::from);
        // A `cwd` that is gone falls back too: never to `/`.
        let working_directory = cwd.map(PathBuf::from).filter(|dir| dir.is_dir()).or(home).filter(|dir| dir.is_dir());
        let options = tty::Options { shell, env, working_directory, ..Default::default() };
        let pty = tty::new(&options, window_size(grid, size(px(8.), px(terminal_font().line_height()))), 0)?;
        let (child, master) = (pty.child().id(), pty.file().try_clone().ok());
        let term = Arc::new(FairMutex::new(Term::new(Config::default(), &grid, listener.clone())));
        let event_loop = EventLoop::new(term.clone(), listener, pty, false, false)?;
        let input = event_loop.channel();
        let _ = shared.input.set(input.clone());
        event_loop.spawn();
        Ok(Self { term, shared, input, grid, wheel: Wheel::default(), child, pty: master })
    }

    /// What quitting would end here: the program in front of the shell, or a job behind it.
    /// `None`: the shell is at its prompt with no jobs, or gone. The core decides (`running`).
    pub fn running(&self) -> Option<Work> {
        if self.shared.exited.load(Relaxed) {
            return None;
        }
        let proc = |pid| Proc { pid, name: process::name(pid) };
        let foreground = self.pty.as_ref().and_then(|pty| process::foreground(pty.as_fd()));
        running(&Processes {
            child: proc(self.child as i32),
            foreground,
            leader: foreground.and_then(process::name),
            leader_parent: foreground.and_then(process::parent),
            children: foreground.map(process::children).unwrap_or_default().into_iter().map(proc).collect(),
        })
    }

    /// The directory the shell is in now: that of the process in front (at a prompt, the shell
    /// itself). `None`: it could not be read, e.g. the shell is gone.
    pub fn cwd(&self) -> Option<String> {
        let foreground = self.pty.as_ref().and_then(|pty| process::foreground(pty.as_fd()));
        foreground.and_then(process::cwd).or_else(|| process::cwd(self.child as i32))
    }

    /// Sends bytes to the shell, as if typed. Typing while scrolled back returns to the bottom.
    pub fn write(&self, bytes: Vec<u8>) {
        self.scroll(Scroll::Bottom);
        let _ = self.input.send(Msg::Input(bytes.into()));
    }

    /// Moves the view through the scrollback.
    pub fn scroll(&self, scroll: Scroll) {
        let mut term = self.term.lock();
        let scroll = match scroll {
            Scroll::Lines(n) => GridScroll::Delta(n),
            Scroll::PageUp => GridScroll::PageUp,
            Scroll::PageDown => GridScroll::PageDown,
            Scroll::Top => GridScroll::Top,
            Scroll::Bottom if term.grid().display_offset() == 0 => return,
            Scroll::Bottom => GridScroll::Bottom,
        };
        term.scroll_display(scroll);
        self.shared.dirty.store(true, Relaxed);
    }

    /// Scrolls to a number of lines back from the bottom: the scrollbar's drag and paging.
    pub fn scroller(&self) -> impl Fn(usize) + 'static {
        let (term, shared) = (self.term.clone(), self.shared.clone());
        move |back| {
            let mut term = term.lock();
            let delta = back as i64 - term.grid().display_offset() as i64;
            term.scroll_display(GridScroll::Delta(delta as i32));
            shared.dirty.store(true, Relaxed);
        }
    }

    /// A wheel or trackpad move of `dy` pixels. Ignored while the program owns the wheel.
    pub fn wheel(&mut self, dy: f32, line_height: f32) {
        let mode = *self.term.lock().mode();
        if !wheel_scrolls(mode.intersects(TermMode::MOUSE_MODE), mode.contains(TermMode::ALT_SCREEN)) {
            return;
        }
        let lines = self.wheel.lines(dy, line_height);
        if lines != 0 {
            self.scroll(Scroll::Lines(lines));
        }
    }

    /// True while the program shows the alternate screen (vim, less), which has no history.
    pub fn alt_screen(&self) -> bool {
        self.term.lock().mode().contains(TermMode::ALT_SCREEN)
    }

    /// Lines scrolled back from the bottom, and lines of history kept.
    pub fn scrollback(&self) -> (usize, usize) {
        let term = self.term.lock();
        (term.grid().display_offset(), term.grid().history_size())
    }

    /// Starts a selection at a visible cell, replacing any other. A click that never drags leaves
    /// an empty selection, which selects nothing.
    pub fn select_start(&self, at: CellPoint, kind: SelectKind) {
        let mut term = self.term.lock();
        let (point, side) = grid_point(&term, at);
        let ty = match kind {
            SelectKind::Simple => SelectionType::Simple,
            SelectKind::Word => SelectionType::Semantic,
            SelectKind::Line => SelectionType::Lines,
        };
        term.selection = Some(Selection::new(ty, point, side));
        self.shared.dirty.store(true, Relaxed);
    }

    /// Moves the selection's far end to a visible cell, as a drag does.
    pub fn select_to(&self, at: CellPoint) {
        let mut term = self.term.lock();
        let (point, side) = grid_point(&term, at);
        if let Some(selection) = term.selection.as_mut() {
            selection.update(point, side);
            self.shared.dirty.store(true, Relaxed);
        }
    }

    /// The selected text, if anything is selected.
    pub fn selected(&self) -> Option<String> {
        self.term.lock().selection_to_string().filter(|s| !s.is_empty())
    }

    /// Turns visual mode (alacritty's vi mode) on, with its cursor at the terminal cursor, or off.
    /// Either way the selection clears, so `v` starts a fresh one; leaving returns to the bottom.
    pub fn set_visual(&self, on: bool) {
        let mut term = self.term.lock();
        if term.mode().contains(TermMode::VI) != on {
            term.toggle_vi_mode();
        }
        term.selection = None;
        if !on {
            term.scroll_display(GridScroll::Bottom);
        }
        self.shared.dirty.store(true, Relaxed);
    }

    /// Moves the visual-mode cursor, scrolling to keep it in view. A selection follows it.
    pub fn visual_move(&self, motion: Motion) {
        let motion = match motion {
            Motion::Left => ViMotion::Left,
            Motion::Down => ViMotion::Down,
            Motion::Up => ViMotion::Up,
            Motion::Right => ViMotion::Right,
            Motion::WordRight => ViMotion::SemanticRight,
            Motion::WordLeft => ViMotion::SemanticLeft,
            Motion::WordEnd => ViMotion::SemanticRightEnd,
            Motion::LineStart => ViMotion::First,
            Motion::LineEnd => ViMotion::Last,
            Motion::FirstOccupied => ViMotion::FirstOccupied,
            Motion::Top => ViMotion::High,
            Motion::Middle => ViMotion::Middle,
            Motion::Bottom => ViMotion::Low,
        };
        let mut term = self.term.lock();
        term.vi_motion(motion);
        let point = term.vi_mode_cursor.point;
        term.scroll_to_point(point);
        self.shared.dirty.store(true, Relaxed);
    }

    /// Starts a selection at the visual-mode cursor (whole lines with `lines`), or clears it.
    pub fn visual_select(&self, lines: bool) {
        let mut term = self.term.lock();
        if term.selection.take().is_none() {
            let ty = if lines { SelectionType::Lines } else { SelectionType::Simple };
            let mut selection = Selection::new(ty, term.vi_mode_cursor.point, Side::Left);
            // Covers the cell under the cursor, so the selection is not empty and follows moves.
            selection.include_all();
            term.selection = Some(selection);
        }
        self.shared.dirty.store(true, Relaxed);
    }

    /// The visual-mode cursor as (line, column), line 0 the top of the screen and negative in
    /// history; `None` outside visual mode.
    pub fn visual_cursor(&self) -> Option<(i32, usize)> {
        let term = self.term.lock();
        let point = term.vi_mode_cursor.point;
        term.mode().contains(TermMode::VI).then_some((point.line.0, point.column.0))
    }

    /// Pastes `text`, bracketed if the program asked for it.
    pub fn paste(&self, text: &str) {
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        self.write(paste(text, bracketed));
    }

    /// Resizes the grid to fill `area`, if that changes its size.
    pub fn fit(&mut self, area: Size<Pixels>, cell: Size<Pixels>) {
        let grid = Grid { cols: (area.width / cell.width) as usize, rows: (area.height / cell.height) as usize };
        if grid == self.grid || grid.cols < 2 || grid.rows < 1 {
            return;
        }
        self.grid = grid;
        let _ = self.input.send(Msg::Resize(window_size(grid, cell)));
        self.term.lock().resize(grid);
    }

    /// True once since new output arrived.
    pub fn take_dirty(&self) -> bool {
        self.shared.dirty.swap(false, Relaxed)
    }

    pub fn exited(&self) -> bool {
        self.shared.exited.load(Relaxed)
    }

    /// True while the program asked for application-cursor arrow keys.
    pub fn app_cursor(&self) -> bool {
        self.term.lock().mode().contains(TermMode::APP_CURSOR)
    }

    pub fn grid_size(&self) -> (usize, usize) {
        (self.grid.cols, self.grid.rows)
    }

    /// Visible rows as plain text, trailing spaces removed.
    pub fn lines(&self) -> Vec<String> {
        let screen = self.screen();
        let text = |row: &Vec<Span>| row.iter().map(|s| s.text.as_str()).collect::<String>().trim_end().to_string();
        screen.rows.iter().map(text).collect()
    }

    /// The bottom screen as plain text, with everything scrolled off above it if `history`.
    /// A wrapped line is one line. Trailing spaces, and blank lines at the end, are removed.
    /// The alternate screen (vim, less) has no history.
    pub fn text(&self, history: bool) -> String {
        let term = self.term.lock();
        let top = if history { term.topmost_line() } else { Line(0) };
        let (start, end) = (Point::new(top, Column(0)), Point::new(term.bottommost_line(), term.last_column()));
        let mut text = term.bounds_to_string(start, end);
        text.truncate(text.trim_end().len());
        text
    }

    pub fn screen(&self) -> Screen {
        let term = self.term.lock();
        let content = term.renderable_content();
        let mut rows: Vec<Vec<Span>> = (0..self.grid.rows).map(|_| Vec::new()).collect();
        let offset = content.display_offset as i32;
        let selection = content.selection;
        for cell in content.display_iter {
            let Some(row) = usize::try_from(cell.point.line.0 + offset).ok().and_then(|r| rows.get_mut(r)) else {
                continue;
            };
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue; // second half of a wide character
            }
            let (mut fg, mut bg) = (colour(cell.fg), colour(cell.bg));
            if cell.flags.contains(Flags::INVERSE) {
                (fg, bg) = (bg, fg);
            }
            let selected = selection.is_some_and(|s| s.contains(cell.point));
            let bg = if selected { Some(theme::SELECTION) } else { (bg != palette::BACKGROUND).then_some(bg) };
            let bold = cell.flags.contains(Flags::BOLD);
            let italic = cell.flags.contains(Flags::ITALIC);
            let underline = cell.flags.intersects(Flags::ALL_UNDERLINES);
            let c = if cell.c == '\t' || cell.flags.contains(Flags::HIDDEN) { ' ' } else { cell.c };
            match row.last_mut() {
                Some(s) if (s.fg, s.bg, s.bold, s.italic, s.underline) == (fg, bg, bold, italic, underline) => {
                    s.text.push(c)
                }
                _ => row.push(Span { text: c.to_string(), fg, bg, bold, italic, underline }),
            }
        }
        let at = content.cursor.point;
        let cursor = (content.cursor.shape != CursorShape::Hidden)
            .then(|| usize::try_from(at.line.0 + offset).ok().map(|row| (row, at.column.0)))
            .flatten()
            .filter(|(row, _)| *row < self.grid.rows); // scrolled back past it
        Screen { rows, cursor }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.input.send(Msg::Shutdown);
    }
}

/// A visible cell as a point in the grid, history included, and the side of the cell.
fn grid_point(term: &Term<Listener>, at: CellPoint) -> (Point, Side) {
    let line = Line(at.row as i32 - term.grid().display_offset() as i32);
    (Point::new(line, Column(at.col)), if at.right { Side::Right } else { Side::Left })
}

fn window_size(grid: Grid, cell: Size<Pixels>) -> WindowSize {
    WindowSize {
        num_lines: grid.rows as u16,
        num_cols: grid.cols as u16,
        cell_width: f32::from(cell.width) as u16,
        cell_height: f32::from(cell.height) as u16,
    }
}

fn colour(color: Color) -> u32 {
    match color {
        Color::Spec(c) => (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32,
        Color::Indexed(i) => palette::xterm(i),
        Color::Named(NamedColor::Background) => palette::BACKGROUND,
        // The 16 ANSI names are 0-15. The rest (foreground, cursor, dim) draw as foreground.
        Color::Named(name) => {
            u8::try_from(name as usize).ok().filter(|i| *i < 16).map_or(palette::FOREGROUND, palette::xterm)
        }
    }
}

/// Paints the grid from the top-left of `bounds`, one shaped line per row.
pub fn paint(
    screen: &Screen,
    bounds: Bounds<Pixels>,
    cell: Size<Pixels>,
    focused: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let chosen = terminal_font();
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        for (r, spans) in screen.rows.iter().enumerate() {
            let text: String = spans.iter().map(|s| s.text.as_str()).collect();
            if text.trim_end().is_empty() && spans.iter().all(|s| s.bg.is_none()) {
                continue;
            }
            let runs: Vec<TextRun> = spans
                .iter()
                .map(|s| {
                    let mut font = font(chosen.font.clone());
                    if s.bold {
                        font.weight = FontWeight::BOLD;
                    }
                    if s.italic {
                        font.style = FontStyle::Italic;
                    }
                    TextRun {
                        len: s.text.len(),
                        font,
                        color: rgb(s.fg).into(),
                        background_color: s.bg.map(|bg| rgb(bg).into()),
                        underline: s.underline.then(|| UnderlineStyle { thickness: px(1.), color: None, wavy: false }),
                        strikethrough: None,
                    }
                })
                .collect();
            let line = window.text_system().shape_line(text.into(), px(chosen.font_size), &runs, None);
            let origin = bounds.origin + point(px(0.), cell.height * r as f32);
            let _ = line.paint_background(origin, cell.height, window, cx);
            let _ = line.paint(origin, cell.height, window, cx);
        }
        if let Some((row, col)) = screen.cursor {
            let origin = bounds.origin + point(cell.width * col as f32, cell.height * row as f32);
            let color: Hsla = rgb(palette::FOREGROUND).into();
            // Solid block when this pane has the keyboard, faint when it does not.
            window.paint_quad(fill(Bounds { origin, size: cell }, color.opacity(if focused { 0.6 } else { 0.2 })));
        }
    });
}
