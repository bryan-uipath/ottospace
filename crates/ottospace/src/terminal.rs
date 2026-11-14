//! Shell half of a terminal pane: a PTY running the user's shell, parsed into a grid by
//! `alacritty_terminal`, and the code that paints that grid.

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

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
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor};
use gpui::{
    App, Bounds, ContentMask, FontStyle, FontWeight, Hsla, Pixels, Size, TextRun, UnderlineStyle, Window, fill, font,
    point, px, rgb, size,
};
use ottospace_core::terminal::{
    CellPoint, Motion, Press, Proc, Processes, Scroll, SelectKind, Wheel, Work, running, saved_command,
    selection_holds, wheel_scrolls,
};
use ottospace_core::workbook::{self, History};
use ottospace_core::{clip, palette, settings, theme};
use ottospace_mac::process;
use ottospace_mac::time::utc_offset;

/// The font terminals draw with, once `choose_font` has run. Until then, the default.
static FONT: OnceLock<settings::Terminal> = OnceLock::new();

fn terminal_font() -> settings::Terminal {
    FONT.get().cloned().unwrap_or_default()
}

/// The terminal font's family: code in the drawer is drawn in it.
pub(crate) fn font_family() -> gpui::SharedString {
    terminal_font().font.into()
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
    /// How often the user scrolled the history: the scrollbar shows on a change.
    scrolls: AtomicU32,
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
    /// A workbook's command, to type on the prompt once the shell is quiet there; never run.
    typed: std::cell::RefCell<Option<Typed>>,
    /// That command until the user types here: saving again keeps it, though nothing runs it yet.
    offered: std::cell::RefCell<Option<String>>,
    /// The history it opened with and its time zone's offset: saved again once, not read off the view.
    opened: Option<(History, i64)>,
    /// The user typed here: only then is what this run printed saved.
    touched: std::cell::Cell<bool>,
    /// The selection's text as last made: output that changes it clears the selection.
    kept: std::cell::RefCell<Option<String>>,
}

/// A command waiting for the shell's prompt, and when the terminal's output last came.
struct Typed {
    text: String,
    heard: Option<Instant>,
}

/// The shell is at its prompt once it printed, was quiet this long, and nothing runs in front of it.
const QUIET: Duration = Duration::from_millis(300);

impl Terminal {
    /// Starts the user's default shell (or `$OTTOSPACE_SHELL`) in `cwd`, with `vars` added to its
    /// environment, e.g. `[("OTTOSPACE_PANE", "3")]`. Without `cwd`: the current directory, or
    /// the home directory when that is `/` (an app started from Finder or the Dock). `history`:
    /// a workbook's, shown above the prompt (written to the view, not the shell), its command
    /// typed on the prompt once the shell is there, not run.
    pub fn spawn(vars: &[(&str, String)], cwd: Option<&str>, history: Option<&History>) -> io::Result<Self> {
        // `$OTTOSPACE_SHELL` replaces the login shell. Tests use it: a login shell with a heavy
        // profile can take a minute to reach a prompt on a busy machine.
        let shell = std::env::var("OTTOSPACE_SHELL").ok().map(|program| tty::Shell::new(program, Vec::new()));
        Self::start(shell, vars, cwd, history)
    }

    /// Runs `program` with `args` through the user's login shell, for its `PATH`:
    /// `$SHELL -lic 'exec "$@"' ottospace-agent <program> <args…>`. The script is fixed text;
    /// the program and each argument are positional parameters, never read by the shell.
    /// With `$OTTOSPACE_SHELL` (tests), that shell runs it without reading a profile.
    pub fn run(program: &str, args: &[String], vars: &[(&str, String)], cwd: Option<&str>) -> io::Result<Self> {
        let mut argv = vec![program.to_string()];
        argv.extend(args.iter().cloned());
        Self::script("exec \"$@\"", "ottospace-agent", argv, vars, cwd)
    }

    /// Runs the user's editor on `file`, as `run` runs a program: `$VISUAL`, else `$EDITOR`,
    /// else `vi`, as the login shell sets them. The shell splits the variable itself, so
    /// `code -w` works (zsh is told to split as POSIX shells do, and none expands a glob).
    pub fn edit(file: &str, vars: &[(&str, String)]) -> io::Result<Self> {
        const EDIT: &str = "if [ -n \"$ZSH_VERSION\" ]; then setopt shwordsplit noglob; else set -f; fi; \
                            exec ${VISUAL:-${EDITOR:-vi}} \"$1\"";
        Self::script(EDIT, "ottospace-settings", vec![file.to_string()], vars, None)
    }

    /// `$SHELL -lic <script> <name> <args…>`: the script is fixed text, the arguments its
    /// positional parameters. With `$OTTOSPACE_SHELL` (tests), that shell with no profile.
    fn script(
        script: &str,
        name: &str,
        args: Vec<String>,
        vars: &[(&str, String)],
        cwd: Option<&str>,
    ) -> io::Result<Self> {
        let (shell, flags) = match std::env::var("OTTOSPACE_SHELL") {
            Ok(test) => (test, "-c"),
            Err(_) => (login_shell(), "-lic"),
        };
        let mut argv = vec![flags.to_string(), script.to_string(), name.to_string()];
        argv.extend(args);
        Self::start(Some(tty::Shell::new(shell, argv)), vars, cwd, None)
    }

    fn start(
        shell: Option<tty::Shell>,
        vars: &[(&str, String)],
        cwd: Option<&str>,
        history: Option<&History>,
    ) -> io::Result<Self> {
        let grid = Grid { cols: 80, rows: 24 };
        let shared = Arc::new(Shared::default());
        let listener = Listener(shared.clone());
        let mut env = HashMap::from([("TERM".to_string(), "xterm-256color".to_string())]);
        env.extend(vars.iter().map(|(name, value)| (name.to_string(), value.clone())));
        let at_root = std::env::current_dir().is_ok_and(|dir| dir.parent().is_none());
        // A `cwd` that is gone starts in the user's folder: never in `/`, nor where ottospace started.
        let gone = cwd.is_some_and(|dir| !std::path::Path::new(dir).is_dir());
        let home = std::env::var_os("HOME").filter(|_| at_root || gone).map(PathBuf::from);
        let working_directory = cwd.map(PathBuf::from).filter(|dir| dir.is_dir()).or(home).filter(|dir| dir.is_dir());
        let options = tty::Options { shell, env, working_directory, ..Default::default() };
        let pty = tty::new(&options, window_size(grid, size(px(8.), px(terminal_font().line_height()))), 0)?;
        let (child, master) = (pty.child().id(), pty.file().try_clone().ok());
        let term = Arc::new(FairMutex::new(Term::new(Config::default(), &grid, listener.clone())));
        // Before the shell can print: the history comes first, its prompt below it.
        let opened = history.map(|history| (history.clone(), utc_offset(history.saved as i64 / 1000)));
        if let Some((history, offset)) = &opened {
            Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new()
                .advance(&mut *term.lock(), &workbook::history_view(history, *offset));
        }
        let offered = history.and_then(|history| history.kept(0).command);
        let typed = offered.clone().map(|text| Typed { text, heard: None });
        let event_loop = EventLoop::new(term.clone(), listener, pty, false, false)?;
        let input = event_loop.channel();
        let _ = shared.input.set(input.clone());
        event_loop.spawn();
        let (typed, offered) = (typed.into(), offered.into());
        let (touched, kept) = (Default::default(), Default::default());
        Ok(Self {
            term,
            shared,
            input,
            grid,
            wheel: Wheel::default(),
            child,
            pty: master,
            typed,
            offered,
            opened,
            touched,
            kept,
        })
    }

    /// The history to save: what it opened with, once, and this run's text if the user typed here
    /// (`workbook::next_history`). The command aside.
    pub fn history(&self, now: u64, most: usize) -> History {
        let text = if most == 0 { String::new() } else { self.text(true) };
        let (opened, offset) = self.opened.as_ref().map_or((None, 0), |(history, offset)| (Some(history), *offset));
        workbook::next_history(opened, offset, &text, self.touched.get(), now, most)
    }

    /// Types a workbook's command on the prompt, without ↵, once the shell has printed, gone
    /// quiet, and nothing runs in front of it (a startup program asking a question is not typed
    /// into). `output`: new output came since the last call. Keys the user typed first win: it
    /// is dropped. No prompt, no typing: there is no deadline.
    fn type_waiting(&self, output: bool) {
        let mut waiting = self.typed.borrow_mut();
        let Some(typed) = waiting.as_mut() else { return };
        if output {
            typed.heard = Some(Instant::now());
        }
        let quiet = typed.heard.is_some_and(|heard| heard.elapsed() >= QUIET);
        if let Some(typed) = waiting.take_if(|_| quiet && self.running().is_none()) {
            let _ = self.input.send(Msg::Input(typed.text.into_bytes().into()));
        }
    }

    /// The command line of the program in front of the shell, e.g. "npm run dev". `None` at a
    /// prompt, for a job in the background, or when it does not read. The process group's
    /// leader's arguments as it was started: of `a | b`, `a` alone.
    pub fn foreground_command(&self) -> Option<String> {
        let work = self.running()?;
        if work.background {
            return None;
        }
        let foreground = self.pty.as_ref().and_then(|pty| process::foreground(pty.as_fd()))?;
        // As the shell started it: `argv[0]` is the name as typed, `sleep` and not `/bin/sleep`.
        saved_command(&process::args(foreground)?)
    }

    /// The workbook's command this terminal opened with, while the user has typed nothing here.
    pub fn offered(&self) -> Option<String> {
        self.offered.borrow().clone()
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
    /// A command still waiting to be typed is not: the user's own keys came first.
    pub fn write(&self, bytes: Vec<u8>) {
        self.typed.take();
        self.offered.take();
        self.touched.set(true);
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
        self.shared.scrolls.fetch_add(1, Relaxed);
    }

    pub fn scrolls(&self) -> u32 {
        self.shared.scrolls.load(Relaxed)
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

    /// Starts a selection at a visible cell, replacing any other, or with `Press::Extend` moves
    /// the far end of the one there. A click that never drags leaves an empty selection, which
    /// selects nothing.
    pub fn select_start(&self, at: CellPoint, press: Press) {
        press_at(&mut self.term.lock(), at, press);
        self.selection_changed();
    }

    /// Moves the selection's far end to a visible cell, as a drag does.
    pub fn select_to(&self, at: CellPoint) {
        extend_to(&mut self.term.lock(), at);
        self.selection_changed();
    }

    /// A drag held past the top or bottom edge: the view scrolls `lines`, and the selection
    /// follows to the cell at `at`, now on another line of the history.
    pub fn select_scrolling(&self, lines: i32, at: CellPoint) {
        self.term.lock().scroll_display(GridScroll::Delta(lines));
        self.shared.scrolls.fetch_add(1, Relaxed);
        self.select_to(at);
    }

    pub fn clear_selection(&self) {
        self.term.lock().selection = None;
        self.selection_changed();
    }

    /// What ⌘C copies (`clip::copy_text`), if anything is selected.
    pub fn selected(&self) -> Option<String> {
        selected_text(&self.term.lock())
    }

    /// The selection's first and last cell as (line, column), line 0 the top of the screen and
    /// negative in history.
    pub fn selection_range(&self) -> Option<[(i32, usize); 2]> {
        let term = self.term.lock();
        let range = term.selection.as_ref()?.to_range(&term)?;
        Some([(range.start.line.0, range.start.column.0), (range.end.line.0, range.end.column.0)])
    }

    /// Notes the selection's text: output written over it later clears it (`selection_holds`).
    fn selection_changed(&self) {
        *self.kept.borrow_mut() = self.selected();
        self.shared.dirty.store(true, Relaxed);
    }

    /// After new output: a selection whose text changed under it goes. One that scrolled with
    /// its text stays; one scrolled out of the history is already gone.
    fn check_selection(&self) {
        let Some(kept) = self.kept.borrow().clone() else { return };
        let mut term = self.term.lock();
        if !selection_holds(&kept, selected_text(&term).as_deref()) {
            term.selection = None;
            drop(term);
            self.kept.take();
        }
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
        drop(term);
        self.selection_changed();
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
        drop(term);
        self.selection_changed();
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
        drop(term);
        self.selection_changed();
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
        self.write(clip::paste_bytes(text, bracketed));
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

    /// True once since new output arrived. Also types a workbook's command once the prompt is there.
    pub fn take_dirty(&self) -> bool {
        let dirty = self.shared.dirty.swap(false, Relaxed);
        self.type_waiting(dirty);
        if dirty {
            self.check_selection();
        }
        dirty
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
        self.screen().lines()
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

impl Screen {
    /// The rows as plain text, trailing spaces removed.
    pub fn lines(&self) -> Vec<String> {
        let text = |row: &Vec<Span>| row.iter().map(|s| s.text.as_str()).collect::<String>().trim_end().to_string();
        self.rows.iter().map(text).collect()
    }

    /// The rows as a picture of a pane `size` points large, `scale` pixels a point: a PNG.
    /// Plain data in and out: for a thread of its own.
    pub fn picture(&self, size: [f64; 2], cell: [f64; 2], scale: f64) -> Option<Vec<u8>> {
        use ottospace_mac::image::{Grid, Run, grid_png};
        let run = |span: &Span| Run { text: span.text.clone(), fg: span.fg, bg: span.bg, bold: span.bold };
        let rows: Vec<Vec<Run>> = self.rows.iter().map(|row| row.iter().map(run).collect()).collect();
        let font = terminal_font();
        let (font_size, background) = (f64::from(font.font_size), palette::BACKGROUND);
        grid_png(&Grid { rows: &rows, size, cell, font: &font.font, font_size, background, scale })
    }
}

impl Terminal {
    /// Sends `signal` to the program it runs (SIGHUP to hang up, SIGKILL to end it), keeping the
    /// terminal to see when the program is gone (`exited`).
    pub fn signal(&self, signal: i32) {
        if !self.exited() {
            // SAFETY: plain integers; the child is ours and not reaped while `exited` is false.
            unsafe { libc::kill(self.child as i32, signal) };
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.input.send(Msg::Shutdown);
    }
}

/// A visible cell as a point in the grid, history included, and the side of the cell.
fn grid_point<T>(term: &Term<T>, at: CellPoint) -> (Point, Side) {
    let line = Line(at.row as i32 - term.grid().display_offset() as i32);
    (Point::new(line, Column(at.col)), if at.right { Side::Right } else { Side::Left })
}

/// A press at a visible cell: a new selection of its kind, or the far end of the one there.
fn press_at<T>(term: &mut Term<T>, at: CellPoint, press: Press) {
    let kind = match press {
        Press::Extend if term.selection.is_some() => return extend_to(term, at),
        Press::Extend => SelectKind::Simple,
        Press::Start(kind) => kind,
    };
    let (point, side) = grid_point(term, at);
    let ty = match kind {
        SelectKind::Simple => SelectionType::Simple,
        SelectKind::Word => SelectionType::Semantic,
        SelectKind::Line => SelectionType::Lines,
    };
    term.selection = Some(Selection::new(ty, point, side));
}

fn extend_to<T>(term: &mut Term<T>, at: CellPoint) {
    let (point, side) = grid_point(term, at);
    if let Some(selection) = term.selection.as_mut() {
        selection.update(point, side);
    }
}

/// The selection's text as ⌘C copies it. A wrapped line comes back as one.
fn selected_text<T>(term: &Term<T>) -> Option<String> {
    term.selection_to_string().map(|raw| clip::copy_text(&raw)).filter(|text| !text.is_empty())
}

/// The user's `$SHELL` if it reads `exec "$@"` as a POSIX shell does, else zsh: e.g. fish
/// names its arguments otherwise.
pub(crate) fn login_shell() -> String {
    let shell = std::env::var("SHELL").unwrap_or_default();
    let name = shell.rsplit('/').next().unwrap_or_default();
    let posix = shell.starts_with('/') && matches!(name, "zsh" | "bash" | "sh" | "ksh" | "dash");
    if posix { shell } else { "/bin/zsh".into() }
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

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::vte::ansi::StdSyncHandler;

    /// A grid `cols` wide and `rows` tall that printed `text`.
    fn term(cols: usize, rows: usize, text: &str) -> Term<VoidListener> {
        let mut term = Term::new(Config::default(), &Grid { cols, rows }, VoidListener);
        print(&mut term, text);
        term
    }

    fn print(term: &mut Term<VoidListener>, text: &str) {
        Processor::<StdSyncHandler>::new().advance(term, text.as_bytes());
    }

    fn at(row: usize, col: usize) -> CellPoint {
        CellPoint { row, col, right: false }
    }

    /// The far half of a cell: a drag to it takes the cell.
    fn past(row: usize, col: usize) -> CellPoint {
        CellPoint { row, col, right: true }
    }

    /// A press at `from` and a drag to `to`: what ⌘C would copy.
    fn drag(term: &mut Term<VoidListener>, from: CellPoint, to: CellPoint) -> Option<String> {
        press_at(term, from, Press::Start(SelectKind::Simple));
        extend_to(term, to);
        selected_text(term)
    }

    #[test]
    fn a_wrapped_line_copies_as_one_and_real_newlines_stay() {
        // 10 columns: the first line wraps onto the second row.
        let mut wrapped = term(10, 4, "abcdefghijklmno\r\nnext\r\n");
        assert_eq!(drag(&mut wrapped, at(0, 0), past(1, 4)).as_deref(), Some("abcdefghijklmno"));
        assert_eq!(drag(&mut wrapped, at(0, 0), past(2, 9)).as_deref(), Some("abcdefghijklmno\nnext"));
        // Each line loses its trailing spaces.
        let mut spaced = term(20, 4, "one   \r\ntwo\r\n");
        assert_eq!(drag(&mut spaced, at(0, 0), past(1, 19)).as_deref(), Some("one\ntwo"));
        assert_eq!(drag(&mut spaced, at(0, 3), past(0, 10)), None, "only spaces: nothing to copy");
    }

    #[test]
    fn clicks_select_a_word_or_a_line_and_shift_extends() {
        let mut t = term(30, 3, "alpha beta gamma\r\n");
        press_at(&mut t, at(0, 7), Press::Start(SelectKind::Word));
        assert_eq!(selected_text(&t).as_deref(), Some("beta"));
        press_at(&mut t, at(0, 7), Press::Start(SelectKind::Line));
        assert_eq!(selected_text(&t).as_deref(), Some("alpha beta gamma"), "no newline at its end");
        // A click, then ⇧-clicks further on: from the click to each.
        press_at(&mut t, at(0, 0), Press::Start(SelectKind::Simple));
        press_at(&mut t, past(0, 4), Press::Extend);
        assert_eq!(selected_text(&t).as_deref(), Some("alpha"));
        press_at(&mut t, past(0, 9), Press::Extend);
        assert_eq!(selected_text(&t).as_deref(), Some("alpha beta"));
        // ⇧-click with none starts one, which selects nothing yet.
        t.selection = None;
        press_at(&mut t, at(0, 3), Press::Extend);
        assert_eq!((t.selection.is_some(), selected_text(&t)), (true, None));
    }

    #[test]
    fn a_selection_reaches_into_the_history_and_follows_its_text() {
        // 3 rows: l1 and l2 scrolled into the history.
        let mut t = term(10, 3, "l1\r\nl2\r\nl3\r\nl4\r\nl5");
        t.scroll_display(GridScroll::Delta(2));
        press_at(&mut t, at(0, 0), Press::Start(SelectKind::Simple));
        t.scroll_display(GridScroll::Bottom);
        extend_to(&mut t, past(2, 1));
        assert_eq!(selected_text(&t).as_deref(), Some("l1\nl2\nl3\nl4\nl5"));
        // New output scrolls the screen: the selection goes with its text.
        press_at(&mut t, at(2, 0), Press::Start(SelectKind::Word));
        print(&mut t, "\r\nl6\r\nl7");
        assert_eq!(selected_text(&t).as_deref(), Some("l5"));
        // Its line erased: it goes.
        let mut t = term(10, 3, "keep\r\n");
        press_at(&mut t, at(0, 0), Press::Start(SelectKind::Word));
        print(&mut t, "\x1b[1;1H\x1b[2K");
        assert_eq!(selected_text(&t), None);
    }
}
