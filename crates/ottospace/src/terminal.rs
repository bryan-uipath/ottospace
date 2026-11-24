//! Shell half of a terminal pane: a PTY running the user's shell, parsed into a grid by
//! `alacritty_terminal`, and the code that paints that grid.

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Acquire, Ordering::Relaxed, Ordering::Release};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll as GridScroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Osc52, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vi_mode::ViMotion;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor, Rgb};
use gpui::{
    Bounds, ContentMask, FontFeatures, FontStyle, FontWeight, Hsla, Pixels, Size, StrikethroughStyle, TextRun,
    UnderlineStyle, Window, fill, font, point, px, rgb, size,
};
use ottospace_core::keys::{self, Kitty, Mods};
use ottospace_core::term_links::{self, Link};
use ottospace_core::terminal::{
    CellPoint, Motion, MouseMode, Pointer, Press, Proc, Processes, Scroll, SelectKind, Wheel, WheelGoes, Work,
    mouse_report, running, saved_command, selection_holds, utf8_locale, wheel_goes,
};
use ottospace_core::workbook::{self, History};
use ottospace_core::{cell_art, clip, palette, settings, theme};
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
        settings::Terminal { font: settings::Terminal::default().font, ..wanted.clone() }
    };
    let _ = FONT.set(font);
    (!installed).then(|| format!("settings.json: the font \"{}\" is not installed; using Menlo", wanted.font))
}

/// The font terminals draw with, for the state dump: `{"name":"Menlo","size":13.0}`.
pub fn font_in_use() -> serde_json::Value {
    let font = terminal_font();
    serde_json::json!({ "name": font.font, "size": font.font_size })
}

/// Size of one character cell: the font's advance, rounded to whole device pixels (up by default,
/// as Ghostty does: every column's glyph then lands on the pixel grid alike), by the line height.
pub fn cell_size(window: &Window) -> Size<Pixels> {
    let (text, chosen) = (window.text_system(), terminal_font());
    let size_px = px(chosen.font_size);
    let width = text.em_advance(text.resolve_font(&font(chosen.font.clone())), size_px).unwrap_or(size_px * 0.6);
    let scale = window.scale_factor();
    size(px(chosen.cell_pixels(f32::from(width) * scale) / scale), px(chosen.line_height()))
}

/// `v` on the nearest whole device pixel at `scale`: 12.6 → 13 at 1x, 12.6 → 12.5 at 2x.
fn on_pixel(v: Pixels, scale: f32) -> Pixels {
    px((f32::from(v) * scale).round() / scale)
}

/// Where a terminal's grid starts in its `bounds` at `scale`: on the next whole device pixel (never
/// outside them), so each whole-pixel cell is on the pixel grid. Painting and the pointer share it.
pub fn grid_origin(origin: gpui::Point<Pixels>, scale: f32) -> gpui::Point<Pixels> {
    origin.map(|v| px((f32::from(v) * scale).ceil() / scale))
}

/// State the PTY reader thread shares with the UI thread.
#[derive(Default)]
struct Shared {
    dirty: AtomicBool,
    exited: AtomicBool,
    /// How often the user scrolled the history: the scrollbar shows on a change.
    scrolls: AtomicU32,
    input: OnceLock<EventLoopSender>,
    /// How the program ended: its exit code, or `None` for a signal. Set before `exited`.
    status: OnceLock<Option<i32>>,
    /// The text area's width and height in points as drawn, for a program that asks (CSI 14 t).
    area: std::sync::Mutex<Option<[f32; 2]>>,
    /// The text the program last asked to put on the clipboard (OSC 52), for the core to hold.
    copy: std::sync::Mutex<Option<String>>,
    /// Output batches parsed so far: a screen made after the `n`th shows at least that output.
    seq: AtomicU64,
    /// A timed key's way to the screen (`typing`), while one is timed: each stage stamped once.
    timing: AtomicBool,
    stamps: std::sync::Mutex<Stamps>,
}

/// When a timed key reached each stage: its key event, its bytes sent to the pty, its echo read
/// and parsed (with that batch's `seq`), the main thread taking the output, and a frame painting it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Stamps {
    pub key: Option<Instant>,
    pub write: Option<Instant>,
    pub echo: Option<(Instant, u64)>,
    pub taken: Option<Instant>,
    pub painted: Option<Instant>,
}

impl Shared {
    /// Stamps a stage of the key being timed, if one is: `stage` sets it once.
    fn stamp(&self, stage: impl FnOnce(&mut Stamps)) {
        if self.timing.load(Relaxed) {
            stage(&mut self.stamps.lock().unwrap_or_else(|e| e.into_inner()));
        }
    }

    /// New output parsed into the grid.
    fn output(&self) {
        let seq = self.seq.fetch_add(1, Release) + 1;
        self.stamp(|s| {
            if s.write.is_some() && s.echo.is_none() {
                s.echo = Some((Instant::now(), seq));
            }
        });
        self.set_dirty();
    }

    /// A frame painted the screen made at `seq`.
    fn painted(&self, seq: u64) {
        self.stamp(|s| {
            if s.echo.is_some_and(|(_, echo)| seq >= echo) && s.painted.is_none() {
                s.painted = Some(Instant::now());
            }
        });
    }

    /// New output, or a scroll: taken at once, and drawn on the display's next frame (`wake::draw`).
    fn set_dirty(&self) {
        self.dirty.store(true, Relaxed);
        ottospace_mac::wake::draw();
    }

    fn set_exited(&self) {
        // Release: whoever sees `exited` sees the status.
        self.exited.store(true, Release);
        ottospace_mac::wake::post();
    }
}

#[derive(Clone)]
struct Listener(Arc<Shared>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::Wakeup => self.0.output(),
            // Answers the terminal owes the program, e.g. a cursor position report.
            Event::PtyWrite(text) => {
                if let Some(input) = self.0.input.get() {
                    let _ = input.send(Msg::Input(text.into_bytes().into()));
                }
            }
            Event::ColorRequest(..) | Event::TextAreaSizeRequest(_) => {
                let area = *self.0.area.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(text) = answer(&event, area) {
                    self.send_event(Event::PtyWrite(text))
                }
            }
            // Never copied here: the user answers in the pane's header (`copy_ask`).
            Event::ClipboardStore(_, mut text) => {
                // Past the cap the ask is refused: only enough is kept to say so (`copy_ask::MOST`).
                let mut cut = ottospace_core::copy_ask::MOST + 1;
                while cut < text.len() && !text.is_char_boundary(cut) {
                    cut += 1;
                }
                text.truncate(cut);
                *self.0.copy.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
                self.0.set_dirty()
            }
            Event::ChildExit(status) => {
                let _ = self.0.status.set(status.code());
                self.0.set_exited()
            }
            Event::Exit => self.0.set_exited(),
            _ => {}
        }
    }
}

/// The answer to a program's question about the terminal: a colour (OSC 10/11/12/4, e.g. the
/// background, to pick a light or dark theme) as drawn, or the text area in points (CSI 14 t).
/// Never the clipboard: OSC 52 reads are refused (`Config::osc52`).
fn answer(event: &Event, area: Option<[f32; 2]>) -> Option<String> {
    match event {
        Event::ColorRequest(index, reply) => {
            let c = palette::queried(*index);
            Some(reply(Rgb { r: (c >> 16) as u8, g: (c >> 8) as u8, b: c as u8 }))
        }
        // One cell the size of the whole area: the reply multiplies cells by their size, and a
        // cell's points are fractional (7.83 in Menlo 13), the PTY's whole.
        Event::TextAreaSizeRequest(reply) => area.map(|[w, h]| {
            reply(WindowSize { num_lines: 1, num_cols: 1, cell_width: w.round() as u16, cell_height: h.round() as u16 })
        }),
        _ => None,
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

/// A run of cells on one row that share a style. Dim text is already in `fg`.
#[derive(serde::Serialize)]
pub struct Span {
    text: String,
    /// Its first column, and the columns it covers.
    #[serde(skip)]
    col: usize,
    #[serde(skip)]
    cells: usize,
    /// One character the font may not have (`⎿`, `日`, `█`): drawn by itself, centred in its cells.
    #[serde(skip)]
    alone: bool,
    fg: u32,
    bg: Option<u32>,
    bold: bool,
    italic: bool,
    strike: bool,
    underline: Option<Underline>,
}

/// A line under text: wavy for a curly one (SGR 4:3), else straight (double, dotted and dashed
/// too); in its own colour (SGR 58) or the text's.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
struct Underline {
    curly: bool,
    color: Option<u32>,
}

impl Span {
    /// Anything but plain text on the default background.
    fn styled(&self) -> bool {
        self.fg != palette::FOREGROUND
            || self.bg.is_some()
            || self.bold
            || self.italic
            || self.strike
            || self.underline.is_some()
    }
}

/// What to paint: styled rows, and the cursor's (row, column) when it is visible.
pub struct Screen {
    rows: Vec<Vec<Span>>,
    /// Row, column and the shape the program asked for (DECSCUSR): block, underline or beam.
    cursor: Option<(usize, usize, CursorShape)>,
    /// The output it shows (`Shared::seq`), and its terminal while a key is timed: painting it stamps.
    seq: u64,
    timed: Option<Arc<Shared>>,
}

pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    shared: Arc<Shared>,
    input: EventLoopSender,
    grid: Grid,
    wheel: Wheel,
    /// Where the wheel's part line was kept for.
    wheel_to: Option<WheelGoes>,
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

    /// Runs an agent through the user's login shell, for its `PATH` and aliases:
    /// `$SHELL -lic <script> ottospace-agent <argv…>`, as `agents::shell_run` makes them. The
    /// script is fixed text; each argument is a positional parameter, never read by the shell.
    /// With `$OTTOSPACE_SHELL` (tests), that shell runs it without reading a profile.
    pub fn run(script: &str, argv: Vec<String>, vars: &[(&str, String)], cwd: Option<&str>) -> io::Result<Self> {
        Self::script(script, "ottospace-agent", argv, vars, cwd)
    }

    /// Runs the user's editor on `file`, as `run` runs a program: `$VISUAL`, else `$EDITOR`,
    /// else `vi`, as the login shell sets them. The shell splits the variable itself, so
    /// `code -w` works (zsh is told to split as POSIX shells do, and none expands a glob).
    /// Open Settings and ⌃G's message both run it.
    pub fn edit(file: &str, vars: &[(&str, String)]) -> io::Result<Self> {
        const EDIT: &str = "if [ -n \"$ZSH_VERSION\" ]; then setopt shwordsplit noglob; else set -f; fi; \
                            exec ${VISUAL:-${EDITOR:-vi}} \"$1\"";
        Self::script(EDIT, "ottospace-editor", vec![file.to_string()], vars, None)
    }

    /// Runs `program` with `args` directly, no shell: a timed run's throwaway `cat` (`typing`).
    pub(crate) fn exec(program: &str, args: Vec<String>) -> io::Result<Self> {
        Self::start(Some(tty::Shell::new(program.into(), args)), &[], None, None)
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
        let mut env = program_env();
        env.extend(vars.iter().map(|(name, value)| (name.to_string(), value.clone())));
        let at_root = std::env::current_dir().is_ok_and(|dir| dir.parent().is_none());
        // A `cwd` that is gone starts in the user's folder: never in `/`, nor where ottospace started.
        let gone = cwd.is_some_and(|dir| !std::path::Path::new(dir).is_dir());
        let home = std::env::var_os("HOME").filter(|_| at_root || gone).map(PathBuf::from);
        let working_directory = cwd.map(PathBuf::from).filter(|dir| dir.is_dir()).or(home).filter(|dir| dir.is_dir());
        let options = tty::Options { shell, env, working_directory, ..Default::default() };
        let first = window_size(grid, size(px(8.), px(terminal_font().line_height())));
        let area = [grid.cols as f32 * 8., grid.rows as f32 * terminal_font().line_height()];
        *shared.area.lock().unwrap_or_else(|e| e.into_inner()) = Some(area);
        let pty = tty::new(&options, first, 0)?;
        let (child, master) = (pty.child().id(), pty.file().try_clone().ok());
        // The kitty keyboard protocol for a program that asks (Claude Code does): ⇧↵ is not ↵.
        // OSC 52: a program may ask to set the clipboard (held for the user), never to read it.
        let config = Config { kitty_keyboard: true, osc52: Osc52::OnlyCopy, ..Config::default() };
        let term = Arc::new(FairMutex::new(Term::new(config, &grid, listener.clone())));
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
            wheel_to: None,
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

    /// The process in front of the shell, `None` at a prompt: a suspended job behind it is not.
    pub fn front_pid(&self) -> Option<i32> {
        let foreground = self.pty.as_ref().and_then(|pty| process::foreground(pty.as_fd()))?;
        (foreground != self.child as i32).then_some(foreground)
    }

    /// The shell's own directory, whatever program is in front of it.
    pub fn shell_cwd(&self) -> Option<String> {
        process::cwd(self.child as i32)
    }

    /// Sends bytes to the shell, as if typed. Typing while scrolled back returns to the bottom.
    /// A command still waiting to be typed is not: the user's own keys came first.
    pub fn write(&self, bytes: Vec<u8>) {
        self.typed.take();
        self.offered.take();
        self.touched.set(true);
        self.scroll(Scroll::Bottom);
        // Before the send: the pty's thread may write and read the echo back before this one runs on.
        self.shared.stamp(|s| {
            s.write.get_or_insert_with(Instant::now);
        });
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
        self.shared.set_dirty();
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
            shared.set_dirty();
        }
    }

    /// A wheel or trackpad move of `dy` pixels over cell `at`: the history scrolls, or the program
    /// gets it as mouse reports or, on the alternate screen, as ↑ and ↓ (`wheel_goes`).
    pub fn wheel(&mut self, dy: f32, line_height: f32, at: CellPoint, mods: Mods) {
        let mode = self.mouse_mode();
        let Some(goes) = wheel_goes(mode, mods.shift) else { return };
        // A part line kept for one place is not spent in another (⇧ pressed mid-scroll).
        if self.wheel_to.replace(goes) != Some(goes) {
            self.wheel = Wheel::default();
        }
        let lines = self.wheel.lines(dy, line_height);
        let up = lines > 0;
        let each = match goes {
            _ if lines == 0 => return,
            WheelGoes::History => return self.scroll(Scroll::Lines(lines)),
            WheelGoes::Reports => mouse_report(Pointer::Wheel { up }, at.col, at.row, mods, mode),
            // Plain arrows: the kitty protocol keeps them as they were.
            WheelGoes::Arrows => {
                let key = if up { "up" } else { "down" };
                keys::encode(key, None, Mods::default(), self.app_cursor(), Kitty::default())
            }
        };
        if let Some(each) = each {
            (0..lines.unsigned_abs()).for_each(|_| self.send(each.clone()));
        }
    }

    /// What the program asked of the mouse and the wheel.
    pub fn mouse_mode(&self) -> MouseMode {
        let mode = *self.term.lock().mode();
        MouseMode {
            clicks: mode.contains(TermMode::MOUSE_REPORT_CLICK),
            drags: mode.contains(TermMode::MOUSE_DRAG),
            motion: mode.contains(TermMode::MOUSE_MOTION),
            sgr: mode.contains(TermMode::SGR_MOUSE),
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
        }
    }

    /// Tells a program that reads the mouse what the pointer did at `at`, by the modes it had
    /// (`mode`, read once for the event); false if it took nothing.
    pub fn report(&self, pointer: Pointer, at: CellPoint, mods: Mods, mode: MouseMode) -> bool {
        mouse_report(pointer, at.col, at.row, mods, mode).map(|bytes| self.send(bytes)).is_some()
    }

    /// Bytes for the program that are not typing: a report, the wheel's arrows.
    fn send(&self, bytes: Vec<u8>) {
        let _ = self.input.send(Msg::Input(bytes.into()));
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

    /// Something is selected, without building its text: for each frame and key.
    pub fn has_selection(&self) -> bool {
        self.term.lock().selection.as_ref().is_some_and(|selection| !selection.is_empty())
    }

    /// What ⌘C copies (`clip::copy_text`), if anything is selected.
    pub fn selected(&self) -> Option<String> {
        selected_text(&self.term.lock())
    }

    /// What ⌘C would copy, for a client: `[hidden]` if a row it takes holds a live pass's cell.
    pub fn selected_shown(&self) -> Option<String> {
        selected_shown(&self.term.lock())
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
        self.shared.set_dirty();
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
        let drawn = [grid.cols as f32 * f32::from(cell.width), grid.rows as f32 * f32::from(cell.height)];
        // The same grid of other cells (the window moved to another scale) is told too.
        let same = grid == self.grid && *self.shared.area.lock().unwrap_or_else(|e| e.into_inner()) == Some(drawn);
        if same || grid.cols < 2 || grid.rows < 1 {
            return;
        }
        self.grid = grid;
        // Under the grid's lock: a program's questions in one read see one size, cells and points.
        let mut term = self.term.lock();
        *self.shared.area.lock().unwrap_or_else(|e| e.into_inner()) = Some(drawn);
        let _ = self.input.send(Msg::Resize(window_size(grid, cell)));
        term.resize(grid);
    }

    /// True once since new output arrived. Also types a workbook's command once the prompt is there.
    pub fn take_dirty(&self) -> bool {
        let dirty = self.shared.dirty.swap(false, Relaxed);
        if dirty {
            self.shared.stamp(|s| {
                if s.echo.is_some() {
                    s.taken.get_or_insert_with(Instant::now);
                }
            });
        }
        self.type_waiting(dirty);
        if dirty {
            self.check_selection();
        }
        dirty
    }

    /// Its program got (`on`) or lost the focus: `CSI I` or `CSI O`, if it asked (DECSET 1004).
    pub fn focus(&self, on: bool) {
        if self.term.lock().mode().contains(TermMode::FOCUS_IN_OUT) {
            let _ = self.input.send(Msg::Input(if on { &b"\x1b[I"[..] } else { &b"\x1b[O"[..] }.into()));
        }
    }

    /// The program's latest ask to set the clipboard, once.
    pub fn take_copy_ask(&self) -> Option<String> {
        self.shared.copy.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    pub fn exited(&self) -> bool {
        self.shared.exited.load(Acquire)
    }

    /// The name of the program the terminal started, as it runs now (`nvim` once the shell
    /// exec'd it). `None` once it has exited.
    pub fn program(&self) -> Option<String> {
        (!self.exited()).then(|| process::name(self.child as i32)).flatten()
    }

    /// True once the program has exited with status 0. A signal, or no status heard, is not.
    pub fn exited_ok(&self) -> bool {
        self.exited() && self.shared.status.get() == Some(&Some(0))
    }

    /// True while the program asked for application-cursor arrow keys.
    pub fn app_cursor(&self) -> bool {
        self.term.lock().mode().contains(TermMode::APP_CURSOR)
    }

    /// What the program asked of the kitty keyboard protocol.
    pub fn kitty(&self) -> Kitty {
        let mode = *self.term.lock().mode();
        Kitty {
            disambiguate: mode.contains(TermMode::DISAMBIGUATE_ESC_CODES),
            alternates: mode.contains(TermMode::REPORT_ALTERNATE_KEYS),
        }
    }

    pub fn grid_size(&self) -> (usize, usize) {
        (self.grid.cols, self.grid.rows)
    }

    /// Visible rows as plain text, trailing spaces removed. A live pass reads `[hidden]`: the
    /// lines the screen's edges cut are searched whole (`pass::hide_rows`).
    pub fn lines(&self) -> Vec<String> {
        lines_of(&self.term.lock())
    }

    /// The bottom screen as plain text, with everything scrolled off above it if `history`.
    /// A wrapped line is one line. Trailing spaces, and blank lines at the end, are removed.
    /// The alternate screen (vim, less) has no history. A live pass reads `[hidden]`, in every
    /// client's read and in the saved history.
    pub fn text(&self, history: bool) -> String {
        text_of(&self.term.lock(), history)
    }

    /// The link on visible cell `at` (`term_links::link_at`), its cells by screen row; those off
    /// screen are left out.
    pub fn link_at(&self, at: CellPoint) -> Option<Link> {
        let term = self.term.lock();
        // A cell kept from before a resize may be off the grid now.
        if at.row >= term.screen_lines() || at.col >= term.columns() {
            return None;
        }
        let offset = term.grid().display_offset() as i32;
        let line = Line(at.row as i32 - offset);
        // Its line's rows, as far as a URL may reach.
        let reach = term_links::reach(term.columns()) as i32;
        let (mut from, mut to) = (line, line);
        while from.0 > line.0 - reach && from > term.topmost_line() && wraps(&term, from - 1) {
            from -= 1;
        }
        while to.0 < line.0 + reach && to < term.bottommost_line() && wraps(&term, to) {
            to += 1;
        }
        let rows: Vec<_> = (from.0..=to.0).map(|l| link_row(&term, Line(l))).collect();
        let mut link = term_links::link_at(&rows, (line.0 - from.0) as usize, at.col)?;
        let shown = |r: usize| usize::try_from(from.0 + r as i32 + offset).ok().filter(|row| *row < self.grid.rows);
        link.cells = link.cells.into_iter().filter_map(|(r, a, b)| Some((shown(r)?, a, b))).collect();
        Some(link)
    }

    pub fn screen(&self) -> Screen {
        // Read before the grid: the screen shows at least this much output.
        let seq = self.shared.seq.load(Acquire);
        let timed = self.shared.timing.load(Relaxed).then(|| self.shared.clone());
        Screen { seq, timed, ..screen_of(&self.term.lock(), self.grid.rows) }
    }

    /// Times the next key, its event at `key`: each stage is stamped as it comes (`stamps`).
    pub(crate) fn time_key(&self, key: Instant) {
        *self.shared.stamps.lock().unwrap_or_else(|e| e.into_inner()) = Stamps { key: Some(key), ..Stamps::default() };
        self.shared.timing.store(true, Relaxed);
    }

    /// The timed key's stamps so far; ends the timing.
    pub(crate) fn stamps(&self) -> Stamps {
        self.shared.timing.store(false, Relaxed);
        *self.shared.stamps.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Spans with a style, for a test run's state: `{"row":0,"text":"dim","fg":..}`.
    /// A row that hides a live pass (`lines`) gives none: its spans would hold the digits.
    pub fn styled(&self) -> Vec<serde_json::Value> {
        let (screen, lines) = (self.screen(), self.lines());
        let whole = |row: usize, spans: &Vec<Span>| {
            let text: String = spans.iter().map(|s| s.text.as_str()).collect();
            lines.get(row).is_some_and(|line| text.trim_end() == line)
        };
        let rows = screen.rows.iter().enumerate().filter(|(row, spans)| whole(*row, spans));
        let spans = rows.flat_map(|(row, spans)| spans.iter().filter(|s| s.styled()).map(move |s| (row, s)));
        let json = |(row, span): (usize, &Span)| {
            let mut value = serde_json::to_value(span).unwrap_or_default();
            value["row"] = row.into();
            value
        };
        spans.map(json).collect()
    }
}

/// `Terminal::screen`: the `rows` on show, styled, and the cursor.
fn screen_of<T: EventListener>(term: &Term<T>, rows: usize) -> Screen {
    let content = term.renderable_content();
    let mut lines: Vec<Vec<Span>> = (0..rows).map(|_| Vec::new()).collect();
    let offset = content.display_offset as i32;
    let selection = content.selection;
    for cell in content.display_iter {
        let Some(row) = usize::try_from(cell.point.line.0 + offset).ok().and_then(|r| lines.get_mut(r)) else {
            continue;
        };
        if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue; // second half of a wide character
        }
        let (mut fg, mut bg) = (colour(cell.fg), colour(cell.bg));
        if cell.flags.contains(Flags::INVERSE) {
            (fg, bg) = (bg, fg);
        }
        if cell.flags.contains(Flags::DIM) {
            fg = palette::dim(fg, bg);
        }
        let selected = selection.is_some_and(|s| s.contains(cell.point));
        let bg = if selected { Some(theme::SELECTION) } else { (bg != palette::BACKGROUND).then_some(bg) };
        let bold = cell.flags.contains(Flags::BOLD);
        let italic = cell.flags.contains(Flags::ITALIC);
        let strike = cell.flags.contains(Flags::STRIKEOUT);
        let underline = cell.flags.intersects(Flags::ALL_UNDERLINES).then(|| Underline {
            curly: cell.flags.contains(Flags::UNDERCURL),
            color: cell.underline_color().map(colour),
        });
        let hidden = cell.c == '\t' || cell.flags.contains(Flags::HIDDEN);
        let c = if hidden { ' ' } else { cell.c };
        // Its zero-width characters: a combining accent, an emoji's joiner or presentation selector.
        // A cell with an underline colour has an empty list: none.
        let zero_width = cell.zerowidth().filter(|z| !hidden && !z.is_empty());
        let (col, wide) = (cell.point.column.0, cell.flags.contains(Flags::WIDE_CHAR));
        let alone = !c.is_ascii() || wide || zero_width.is_some();
        match row.last_mut() {
            // The next column of a run of plain characters in one style (an erased wide
            // character's half leaves a gap: a new run after it).
            Some(s)
                if !alone
                    && !s.alone
                    && s.col + s.cells == col
                    && (s.fg, s.bg, s.bold, s.italic, s.strike, s.underline)
                        == (fg, bg, bold, italic, strike, underline) =>
            {
                s.text.push(c);
                s.cells += 1;
            }
            _ => {
                let mut text = c.to_string();
                text.extend(zero_width.into_iter().flatten());
                let cells = if wide { 2 } else { 1 };
                row.push(Span { text, col, cells, alone, fg, bg, bold, italic, strike, underline })
            }
        }
    }
    let at = content.cursor.point;
    let shape = content.cursor.shape;
    let cursor = (shape != CursorShape::Hidden)
        .then(|| usize::try_from(at.line.0 + offset).ok().map(|row| (row, at.column.0, shape)))
        .flatten()
        .filter(|(row, ..)| *row < rows); // scrolled back past it
    Screen { rows: lines, cursor, seq: 0, timed: None }
}

impl Screen {
    /// The rows as a picture of a pane `size` points large, `scale` pixels a point: a PNG.
    /// Plain data in and out: for a thread of its own.
    pub fn picture(&self, size: [f64; 2], cell: [f64; 2], scale: f64) -> Option<Vec<u8>> {
        use ottospace_mac::image::{Grid, Run, grid_png};
        // The picture counts a cell a character: a gap before a span is spaces, a wide character
        // has a space after it, and zero-width characters are left out.
        let runs = |row: &Vec<Span>| {
            let mut runs = Vec::new();
            let mut at = 0;
            for span in row {
                let gap = span.col.saturating_sub(at);
                if gap > 0 {
                    runs.push(Run { text: " ".repeat(gap), fg: palette::FOREGROUND, bg: None, bold: false });
                }
                let text = match span.alone {
                    true => span.text.chars().take(1).chain(" ".repeat(span.cells - 1).chars()).collect(),
                    false => span.text.clone(),
                };
                runs.push(Run { text, fg: span.fg, bg: span.bg, bold: span.bold });
                at = span.col + span.cells;
            }
            runs
        };
        let rows: Vec<Vec<Run>> = self.rows.iter().map(runs).collect();
        let font = terminal_font();
        let (font_size, background) = (f64::from(font.font_size), palette::BACKGROUND);
        grid_png(&Grid { rows: &rows, size, cell, font: &font.font, font_size, background, scale })
    }
}

/// The most rows searched past an edge: a pass printed a character a row, blank rows between,
/// is one in pieces (not hidden), and a read stays cheap.
const MOST_PAST: i32 = 256;

/// The last row searched past `edge`, going `step` (-1 up, 1 down): rows until they hold 32
/// characters other than a row's edges (`pass::edge`: a box's borders count none), and one more.
/// A pass the edge cuts is then searched whole however few columns its rows use (a narrow pane, a
/// TUI's indent or column, a narrow box). To the grid's end at most.
fn past<T>(term: &Term<T>, edge: Line, step: i32) -> Line {
    let (top, bottom) = (term.topmost_line(), term.bottommost_line());
    let (mut line, mut held) = (edge, 0);
    for _ in 0..MOST_PAST {
        let next = line + step;
        if next < top || next > bottom {
            break;
        }
        line = next;
        if held >= 32 {
            break;
        }
        held += rows_of(term, line, line)
            .iter()
            .map(|(row, _)| row.chars().filter(|c| !crate::pass::edge(*c)).count())
            .sum::<usize>();
    }
    line
}

/// The rows a read from `top` to `bottom` searches: the lines its edges cut, and `past` them.
fn searched<T>(term: &Term<T>, top: Line, bottom: Line) -> (Line, Line) {
    (line_start(term, past(term, top, -1)), line_end(term, past(term, bottom, 1)))
}

/// `Terminal::lines`: the rows on show, a live pass hidden in the rows around them too.
fn lines_of<T>(term: &Term<T>) -> Vec<String> {
    let offset = term.grid().display_offset() as i32;
    let top = Line(-offset);
    let bottom = Line(term.screen_lines() as i32 - 1 - offset);
    let (from, to) = searched(term, top, bottom);
    let rows = cut_history(term, from, rows_of(term, from, to)).into_iter().map(|(row, _)| row).collect();
    let shown = crate::pass::hide_rows(rows).into_iter().skip((top.0 - from.0) as usize).take(term.screen_lines());
    shown.map(|row| row.trim_end().to_string()).collect()
}

/// `Terminal::text`, a live pass hidden also in the rows above the screen's top.
fn text_of<T>(term: &Term<T>, history: bool) -> String {
    let top = if history { term.topmost_line() } else { Line(0) };
    let (from, to) = searched(term, top, term.bottommost_line());
    let (rows, wraps): (Vec<String>, Vec<bool>) = cut_history(term, from, rows_of(term, from, to)).into_iter().unzip();
    let skip = (top.0 - from.0) as usize;
    let mut text = String::new();
    for (row, wraps) in crate::pass::hide_rows(rows).into_iter().zip(wraps).skip(skip) {
        text.push_str(&row);
        if !wraps {
            text.push('\n');
        }
    }
    text.truncate(text.trim_end().len());
    text
}

/// `Terminal::selected_shown`. Whole rows decide, by `lines_of`'s search: what a part of a pass
/// would hide never does, so a selection says nothing of a guess (e.g. 31 digits of 32).
fn selected_shown<T>(term: &Term<T>) -> Option<String> {
    let text = selected_text(term)?;
    let range = term.selection.as_ref()?.to_range(term)?;
    let (from, to) = searched(term, range.start.line, range.end.line);
    let rows = rows_of(term, from, to);
    let raw: Vec<String> = rows.iter().map(|(row, _)| row.clone()).collect();
    let shown = crate::pass::hide_rows(cut_history(term, from, rows).into_iter().map(|(row, _)| row).collect());
    let (skip, take) = ((range.start.line.0 - from.0) as usize, (range.end.line.0 - range.start.line.0 + 1) as usize);
    let touched = raw.iter().zip(&shown).skip(skip).take(take).any(|(raw, shown)| raw != shown);
    Some(if touched { crate::pass::HIDDEN.to_string() } else { text })
}

/// Whether grid row `line` wraps into the next.
fn wraps<T>(term: &Term<T>, line: Line) -> bool {
    term.grid()[line][Column(term.columns() - 1)].flags.contains(Flags::WRAPLINE)
}

/// Grid row `line` as `term_links` reads it: each column's text (a wide character's second half
/// empty, hidden text and a tab a space), and its OSC 8 target (empty: too long to open).
fn link_row<T>(term: &Term<T>, line: Line) -> term_links::Row {
    let row = &term.grid()[line];
    let spacer = Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER;
    let (cells, targets) = (0..term.columns())
        .map(|col| {
            let cell = &row[Column(col)];
            let text = match cell.flags {
                f if f.intersects(spacer) => String::new(),
                f if f.contains(Flags::HIDDEN) || cell.c == '\t' => " ".into(),
                _ => std::iter::once(cell.c).chain(cell.zerowidth().into_iter().flatten().copied()).collect(),
            };
            let uri = cell.hyperlink().map(|link| link.uri().to_string());
            (text, uri.map(|uri| if uri.len() > ottospace_core::links::MAX_LINK { String::new() } else { uri }))
        })
        .unzip();
    term_links::Row { cells, targets, wraps: wraps(term, line) }
}

/// The first row of the line `line` is part of.
fn line_start<T>(term: &Term<T>, mut line: Line) -> Line {
    while line > term.topmost_line() && wraps(term, line - 1) {
        line -= 1;
    }
    line
}

/// The last row of the line `line` is part of.
fn line_end<T>(term: &Term<T>, mut line: Line) -> Line {
    while line < term.bottommost_line() && wraps(term, line) {
        line += 1;
    }
    line
}

/// Grid rows `from..=to` as plain text, each with whether it wraps into the next. A wide
/// character's spacer is left out; a row that does not wrap loses its trailing spaces.
fn rows_of<T>(term: &Term<T>, from: Line, to: Line) -> Vec<(String, bool)> {
    let row = |line: i32| {
        let row = &term.grid()[Line(line)];
        let spacer = Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER;
        let cells = (0..term.columns()).map(|col| &row[Column(col)]).filter(|cell| !cell.flags.intersects(spacer));
        let mut text = String::new();
        for cell in cells {
            if cell.c == '\t' || cell.flags.contains(Flags::HIDDEN) {
                text.push(' ');
            } else {
                // Its zero-width characters too (a combining accent, an emoji's joiner).
                text.push(cell.c);
                text.extend(cell.zerowidth().into_iter().flatten());
            }
        }
        let wraps = wraps(term, Line(line));
        if !wraps {
            text.truncate(text.trim_end().len());
        }
        (text, wraps)
    };
    (from.0..=to.0).map(row).collect()
}

/// Rows read from the top of a full history: what was dropped above may have cut a pass, so a
/// run of 8 or more hex digits starting there is hidden, whatever its digits
/// (`pass::hide_digits_at_top`: a rule on any digits says nothing of a pass's).
fn cut_history<T>(term: &Term<T>, from: Line, rows: Vec<(String, bool)>) -> Vec<(String, bool)> {
    let full = term.grid().history_size() >= Config::default().scrolling_history;
    // No pass live: nothing to hide (e.g. a commit hash at the top of a user's saved history).
    if !full || from != term.topmost_line() || !crate::pass::any_live() {
        return rows;
    }
    let (text, wraps): (Vec<String>, Vec<bool>) = rows.into_iter().unzip();
    crate::pass::hide_digits_at_top(text).into_iter().zip(wraps).collect()
}

impl Terminal {
    /// Sends `signal` to the program it runs (SIGHUP to hang up, SIGKILL to end it), keeping the
    /// terminal to see when the program is gone (`exited`).
    /// The process group in front, if not the program's own: a job of its shell, as an agent
    /// started by `agent_commands`' text, whose shell does not exec it.
    pub fn job(&self) -> Option<i32> {
        let group = self.pty.as_ref().and_then(|pty| process::foreground(pty.as_fd()))?;
        (group != self.child as i32).then_some(group)
    }

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

/// What a program reads to pick its features: 24-bit colour (`COLORTERM`), and who we are. Set
/// over what ottospace inherited, e.g. another terminal's `TERM_PROGRAM` when started from one.
/// And a UTF-8 `LANG` when ottospace has no locale of its own (started from the Dock).
fn program_env() -> HashMap<String, String> {
    let vars = [
        ("TERM", "xterm-256color"),
        ("COLORTERM", "truecolor"),
        ("TERM_PROGRAM", "ottospace"),
        ("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION")),
    ];
    let mut env: HashMap<String, String> =
        vars.into_iter().map(|(name, value)| (name.to_string(), value.to_string())).collect();
    let set = ["LANG", "LC_ALL"].iter().any(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty()));
    let exists = |locale: &str| std::path::Path::new("/usr/share/locale").join(locale).is_dir();
    // The system's locale is read only when it is needed.
    let system = (!set).then(ottospace_mac::locale::identifier).flatten();
    if let Some(lang) = utf8_locale(set, system.as_deref(), exists) {
        env.insert("LANG".into(), lang);
    }
    env
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

/// A wheel's move in points: up and down, or with ⇧ the sideways move macOS makes of a mouse
/// wheel's turn then.
pub(crate) fn wheel_dy(moved: gpui::Point<Pixels>, shift: bool) -> f32 {
    f32::from(if shift && moved.y == px(0.) { moved.x } else { moved.y })
}

/// A key for a terminal's program, as a pane's, the drawer agent's and the editor's go: a scroll
/// key scrolls its history, else it is encoded and written. A ⌘ shortcut encodes to nothing.
pub fn send_key<T: 'static>(terminal: &Terminal, event: &gpui::KeyDownEvent, cx: &mut gpui::Context<T>) {
    let (key, mods) = (&event.keystroke, crate::mods(event.keystroke.modifiers));
    if let Some(scroll) = ottospace_core::terminal::scroll_key(&key.key, mods, terminal.alt_screen()) {
        terminal.scroll(scroll);
        cx.notify();
        return cx.stop_propagation();
    }
    let (typed, kitty) = (key.key_char.as_deref(), terminal.kitty());
    if let Some(bytes) = ottospace_core::keys::encode(&key.key, typed, mods, terminal.app_cursor(), kitty) {
        terminal.write(bytes);
        cx.stop_propagation();
    }
}

/// Paints the grid from the top-left of `bounds`, one shaped line per row.
pub fn paint(
    screen: &Screen,
    bounds: Bounds<Pixels>,
    cell: Size<Pixels>,
    focused: bool,
    link: &[(usize, usize, usize)],
    window: &mut Window,
) {
    paint_scaled(screen, bounds, (cell, 1.), focused, link, window)
}

/// The grid as blocks, no glyph shaped: each run of text a bar of its colour, half a row high, and
/// each background as it is. A zoomed-out tile too small to read (`zoom`): cheap at any size.
pub fn paint_blocks(screen: &Screen, bounds: Bounds<Pixels>, cell: Size<Pixels>, window: &mut Window) {
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        for (r, spans) in screen.rows.iter().enumerate() {
            let y = bounds.origin.y + cell.height * r as f32;
            for s in spans {
                let x = bounds.origin.x + cell.width * s.col as f32;
                let width = cell.width * s.cells as f32;
                if let Some(bg) = s.bg {
                    window.paint_quad(fill(Bounds { origin: point(x, y), size: size(width, cell.height) }, rgb(bg)));
                }
                // The cells its text covers, its trailing spaces left out (a wide glyph is two).
                let trailing = s.text.len() - s.text.trim_end().len();
                let len = s.cells.saturating_sub(trailing);
                if len > 0 && !s.text.trim().is_empty() {
                    let bar = Bounds {
                        origin: point(x, y + cell.height * 0.25),
                        size: size(cell.width * len as f32, cell.height * 0.5),
                    };
                    window.paint_quad(fill(bar, Hsla::from(rgb(s.fg)).opacity(0.55)));
                }
            }
        }
    });
}

/// `paint`, every cell and glyph `scale` times its size: a zoomed-out tile's terminal (`zoom`).
pub fn paint_scaled(
    screen: &Screen,
    bounds: Bounds<Pixels>,
    (cell, scale): (Size<Pixels>, f32),
    focused: bool,
    link: &[(usize, usize, usize)],
    window: &mut Window,
) {
    let chosen = terminal_font();
    let cell = size(cell.width * scale, cell.height * scale);
    // The grid's origin on the pixel grid, so each cell's (whole-pixel wide) is too.
    let device = window.scale_factor();
    let bounds = Bounds { origin: grid_origin(bounds.origin, device), ..bounds };
    let mut art = 0;
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        for (r, spans) in screen.rows.iter().enumerate() {
            let y = bounds.origin.y + cell.height * r as f32;
            let at = |s: &Span| point(bounds.origin.x + cell.width * s.col as f32, y);
            // Backgrounds first, a cell's full width and height: no seams between rows.
            for s in spans {
                if let Some(bg) = s.bg {
                    let size = size(cell.width * s.cells as f32, cell.height);
                    window.paint_quad(fill(Bounds { origin: at(s), size }, Hsla::from(rgb(bg))));
                }
            }
            for s in spans.iter().filter(|s| !s.text.trim().is_empty() || s.underline.is_some() || s.strike) {
                // One character alone, no mark on it: a box-drawing or block one is drawn as shapes.
                let mut chars = s.text.chars();
                let single = chars.next().filter(|_| s.alone && chars.next().is_none());
                let (w, h) = (f32::from(cell.width), f32::from(cell.height));
                if let Some(pieces) = single.and_then(|c| cell_art::pieces(c, w, h)) {
                    paint_art(&pieces, at(s), rgb(s.fg).into(), window);
                    // Its underline and strikethrough, straight, across the cell.
                    let line = |y: f32, color: u32| {
                        fill(
                            Bounds { origin: at(s) + point(px(0.), px(y)), size: size(cell.width, px(1.)) },
                            Hsla::from(rgb(color)),
                        )
                    };
                    if let Some(underline) = s.underline {
                        window.paint_quad(line(h - 2., underline.color.unwrap_or(s.fg)));
                    }
                    if s.strike {
                        window.paint_quad(line((h / 2.).floor(), s.fg));
                    }
                    art += 1;
                    continue;
                }
                let mut font = font(chosen.font.clone());
                // No ligature: `fi` must stay two glyphs on two columns.
                font.features =
                    FontFeatures(Arc::new(["calt", "liga", "clig", "dlig"].map(|tag| (tag.into(), 0)).into()));
                if s.bold {
                    font.weight = FontWeight::BOLD;
                }
                if s.italic {
                    font.style = FontStyle::Italic;
                }
                let color: Hsla = rgb(s.fg).into();
                let text = s.text.clone().into();
                let run = TextRun {
                    len: s.text.len(),
                    font,
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line = window.text_system().shape_line(text, px(chosen.font_size * scale), &[run], None);
                // One character alone: centred in its cells, whatever font fell back for it (`⎿` is
                // 13 points wide in Hiragino).
                let mut origin = at(s);
                if s.alone {
                    origin.x = on_pixel(origin.x + (cell.width * s.cells as f32 - line.width) / 2., device);
                }
                // The baseline on a whole pixel (where GPUI would put it: `(height - ascent - descent) / 2
                // + ascent` down): a fractional one rasterises every row at a quarter-pixel offset.
                let baseline =
                    on_pixel(origin.y + (cell.height - line.ascent - line.descent) / 2. + line.ascent, device);
                for run in &line.runs {
                    for glyph in &run.glyphs {
                        // A plain run is ASCII, a byte a column: each glyph on its own column's whole
                        // pixel, never where the font's advance would carry it (GPUI's forced width
                        // moves a glyph only once it is a point off).
                        let x = if s.alone {
                            origin.x + glyph.position.x
                        } else {
                            origin.x + cell.width * glyph.index as f32
                        };
                        let at = point(x, baseline);
                        let _ = match glyph.is_emoji {
                            true => window.paint_emoji(at, run.font_id, glyph.id, line.font_size),
                            false => window.paint_glyph(at, run.font_id, glyph.id, line.font_size, color),
                        };
                    }
                }
                // Its underline and strikethrough across its cells, where GPUI puts them.
                let width = if s.alone { line.width } else { cell.width * s.cells as f32 };
                if let Some(u) = s.underline {
                    let color = Some(u.color.map_or(color, |c| rgb(c).into()));
                    let style = UnderlineStyle { thickness: px(1.), color, wavy: u.curly };
                    window.paint_underline(point(origin.x, baseline + line.descent * 0.618), width, &style);
                }
                if s.strike {
                    let style = StrikethroughStyle { thickness: px(1.), color: Some(color) };
                    let y = origin.y + (line.ascent * 0.5 + (baseline - origin.y)) * 0.5;
                    window.paint_strikethrough(point(origin.x, y), width, &style);
                }
            }
        }
        // A link under the pointer while ⌘ is held: underlined, in the text's colour.
        for (row, from, to) in link {
            let origin = bounds.origin + point(cell.width * *from as f32, cell.height * (*row as f32 + 1.) - px(2.));
            let line = size(cell.width * (to + 1 - from) as f32, px(1.));
            window.paint_quad(fill(Bounds { origin, size: line }, Hsla::from(rgb(palette::FOREGROUND))));
        }
        if let Some((row, col, shape)) = screen.cursor {
            let origin = bounds.origin + point(cell.width * col as f32, cell.height * row as f32);
            let color: Hsla = rgb(palette::FOREGROUND).into();
            // The shape the program asked for when this pane has the keyboard, a faint block when not.
            let (at, size) = match shape {
                CursorShape::Beam if focused => (origin, size(px(2.), cell.height)),
                CursorShape::Underline if focused => {
                    (origin + point(px(0.), cell.height - px(2.)), size(cell.width, px(2.)))
                }
                _ => (origin, cell),
            };
            let alpha = match shape {
                _ if !focused => 0.2,
                CursorShape::HollowBlock => 0.3,
                _ => 0.6,
            };
            window.paint_quad(fill(Bounds { origin: at, size }, color.opacity(alpha)));
        }
    });
    if let Some(shared) = &screen.timed {
        shared.painted(screen.seq);
    }
    // Counted for a test run's state only: once a frame.
    if art > 0 && *TEST_RUN.get_or_init(|| std::env::var_os("OTTOSPACE_E2E").is_some()) {
        ART_CELLS.fetch_add(art, Relaxed);
    }
}

static TEST_RUN: OnceLock<bool> = OnceLock::new();

/// Box-drawing and block cells drawn as shapes since start (`counts.cell_art`, test runs).
static ART_CELLS: AtomicU64 = AtomicU64::new(0);

pub fn art_cells() -> u64 {
    ART_CELLS.load(Relaxed)
}

/// A cell's shapes (`cell_art::pieces`) from its top-left `origin`, in `color`.
fn paint_art(pieces: &[cell_art::Piece], origin: gpui::Point<Pixels>, color: Hsla, window: &mut Window) {
    let at = |[x, y]: [f32; 2]| origin + point(px(x), px(y));
    for piece in pieces {
        match *piece {
            cell_art::Piece::Rect { x, y, w, h, alpha } => {
                window.paint_quad(fill(Bounds { origin: at([x, y]), size: size(px(w), px(h)) }, color.opacity(alpha)))
            }
            cell_art::Piece::Arc { from, to, corner, width } => {
                let mut path = gpui::PathBuilder::stroke(px(width));
                path.move_to(at(from));
                path.curve_to(at(to), at(corner));
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            }
            cell_art::Piece::Line { from, to, width } => {
                let mut path = gpui::PathBuilder::stroke(px(width));
                path.move_to(at(from));
                path.line_to(at(to));
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            }
        }
    }
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

    /// A pass that wraps across the screen's edge: no digit of it in the rows on show, the text
    /// of the screen or the history. Its line is searched whole, wherever the edge cuts it.
    #[test]
    fn a_live_pass_the_screens_edge_cuts_shows_no_digit() {
        let dir = crate::pass::minted_dir("edge-cuts");
        let live = crate::pass::Live::mint_in(dir.as_ref()).unwrap();
        let hex = live.pass().hex();
        // 20 wide: "PASS=" and 15 digits on one row, 17 on the next; then 3 rows under it.
        let mut term = term(20, 3, &format!("PASS={hex}\r\none\r\ntwo\r\nend"));
        let digits = |text: &str| hex.as_bytes().windows(4).any(|w| text.contains(std::str::from_utf8(w).unwrap()));
        // Its tail is the top row on show: the screen's top cuts its line.
        term.scroll_display(GridScroll::Delta(1));
        let shown = lines_of(&term);
        assert_eq!(shown, ["", "one", "two"], "its tail goes with the hidden head");
        assert!(!digits(&text_of(&term, false)), "{}", text_of(&term, false));
        // Its head is the bottom row on show: the screen's bottom cuts it.
        term.scroll_display(GridScroll::Delta(1));
        assert_eq!(lines_of(&term), ["PASS=[hidden]", "", "one"]);
        assert!(text_of(&term, true).starts_with("PASS=[hidden]\none"), "{}", text_of(&term, true));
        // Broken by the program's own line break, then by a cursor move on the alternate screen
        // (a TUI): no wrap flag, one pass still. The screen's edge between the halves too.
        for opening in ["", "\x1b[?1049h"] {
            let split = format!("{opening}PASS={}\r\n{}\r\nend", &hex[..16], &hex[16..]);
            let mut term = super::tests::term(40, 3, &split);
            assert_eq!(lines_of(&term), ["PASS=[hidden]", "", "end"], "{opening:?}");
            assert!(!digits(&text_of(&term, true)));
            let moved = format!("\x1b[2J\x1b[1;1HPASS={}\x1b[2;3H{}", &hex[..20], &hex[20..]);
            print(&mut term, &moved);
            assert!(!lines_of(&term).iter().any(|row| digits(row)), "{:?}", lines_of(&term));
        }
        let mut short = super::tests::term(40, 2, &format!("PASS={}\r\n{}\r\none\r\ntwo", &hex[..16], &hex[16..]));
        short.scroll_display(GridScroll::Delta(1));
        assert_eq!(lines_of(&short), ["", "one"], "the top edge between its halves");
        // 4 wide, the program's own breaks: eight rows of 4 digits, the top edge on the last one.
        let rows: Vec<&str> = (0..8).map(|n| &hex[n * 4..n * 4 + 4]).collect();
        let mut narrow = super::tests::term(4, 2, &format!("{}\r\none\r\ntwo", rows.join("\r\n")));
        narrow.scroll_display(GridScroll::Delta(1));
        assert_eq!(lines_of(&narrow), ["", "one"], "all eight rows searched, though the edge cuts them");
        // 40 wide, but each row indented 36: 4 digits a row, as in a TUI's column. The rows a
        // pass could use are searched, not those its width would fill.
        let indent = " ".repeat(36);
        let indented: Vec<String> = rows.iter().map(|row| format!("{indent}{row}")).collect();
        let mut column = super::tests::term(40, 2, &format!("{}\r\none\r\ntwo", indented.join("\r\n")));
        column.scroll_display(GridScroll::Delta(1));
        assert_eq!(lines_of(&column), ["", "one"], "the tail row of an indented pass");
        column.scroll_display(GridScroll::Delta(8));
        let head = format!("{indent}[hidden]");
        assert_eq!(lines_of(&column), [head.as_str(), ""], "its head on show, the bottom edge cutting the rest");
        // In a box 6 wide, 2 digits a row: its borders do not shrink the rows searched.
        let boxed: Vec<String> =
            rows.iter().flat_map(|row| [&row[..2], &row[2..]]).map(|d| format!("\u{2502} {d} \u{2502}")).collect();
        let mut boxed = super::tests::term(6, 2, &format!("{}\r\none", boxed.join("\r\n")));
        assert_eq!(lines_of(&boxed), ["\u{2502}  \u{2502}", "one"], "the tail row of a boxed pass");
        boxed.scroll_display(GridScroll::Delta(1));
        assert_eq!(lines_of(&boxed), ["\u{2502}  \u{2502}", "\u{2502}  \u{2502}"]);
        // A selection of part of it, here the 16 digits on the top row, reads `[hidden]` in the
        // state; one of other rows reads as it is.
        assert_eq!(drag(&mut short, at(0, 0), past(0, 15)).as_deref(), Some(&hex[16..]));
        assert_eq!(selected_shown(&short).as_deref(), Some(crate::pass::HIDDEN));
        drag(&mut short, at(1, 0), past(1, 2));
        assert_eq!(selected_shown(&short).as_deref(), Some("one"));
        // No oracle: part of the pass a client typed at the prompt, the bottom edge right after
        // it, reads as it is, as wrong digits do; the edge cuts no pass there.
        let wrong = "0123456789abcdef0123456789abcdef";
        for part in [&hex[..31], &wrong[..31], &hex[1..]] {
            let typed = super::tests::term(40, 3, &format!("one\r\n$ echo {part}"));
            assert_eq!(lines_of(&typed), ["one", &format!("$ echo {part}"), ""]);
            assert_eq!(text_of(&typed, true), format!("one\n$ echo {part}"));
        }
        // Gone, the pass shows again: it grants nothing.
        drop(live);
        assert_eq!(lines_of(&short)[0], &hex[16..]);
    }

    /// Programs see 24-bit colour and ottospace's name, whatever terminal started ottospace.
    #[test]
    fn programs_are_told_of_24_bit_colour_and_ottospace() {
        let env = program_env();
        assert_eq!(env["COLORTERM"], "truecolor");
        assert_eq!((env["TERM"].as_str(), env["TERM_PROGRAM"].as_str()), ("xterm-256color", "ottospace"));
        assert_eq!(env["TERM_PROGRAM_VERSION"], env!("CARGO_PKG_VERSION"));
    }

    /// A program asking for the background, the text's colour, a palette entry or the text area's
    /// size in points is answered as drawn.
    #[test]
    fn colour_and_size_questions_are_answered() {
        #[derive(Clone, Default)]
        struct Heard(Arc<std::sync::Mutex<Vec<Event>>>);
        impl EventListener for Heard {
            fn send_event(&self, event: Event) {
                self.0.lock().unwrap().push(event);
            }
        }
        let heard = Heard::default();
        let mut term = Term::new(Config::default(), &Grid { cols: 80, rows: 24 }, heard.clone());
        Processor::<StdSyncHandler>::new().advance(&mut term, b"\x1b]11;?\x1b\\\x1b]10;?\x07\x1b]4;1;?\x07\x1b[14t");
        let area = Some([80. * 8., 24. * 18.]);
        let answers: Vec<Option<String>> = heard.0.lock().unwrap().iter().map(|e| answer(e, area)).collect();
        let ansi_red = format!("{:06x}", palette::xterm(1));
        let red = format!("{0}{0}/{1}{1}/{2}{2}", &ansi_red[..2], &ansi_red[2..4], &ansi_red[4..]);
        assert_eq!(
            answers,
            [
                Some("\x1b]11;rgb:0909/0909/0b0b\x1b\\".to_string()),
                Some("\x1b]10;rgb:fafa/fafa/fafa\x07".to_string()),
                Some(format!("\x1b]4;1;rgb:{red}\x07")),
                Some("\x1b[4;432;640t".to_string()),
            ]
        );
        assert_eq!(answer(&Event::TextAreaSizeRequest(Arc::new(|_| String::new())), None), None, "no size yet");
        // 100 columns of Menlo 13 (7.83 points a cell): the width as drawn, not 100 × 7.
        let reply = Arc::new(|size: WindowSize| {
            format!("{}x{}", size.num_cols * size.cell_width, size.num_lines * size.cell_height)
        });
        assert_eq!(answer(&Event::TextAreaSizeRequest(reply), Some([783.2, 432.])).as_deref(), Some("783x432"));
    }

    /// Spans of row 0 as (text, fg, strike, underline).
    fn spans(term: &Term<VoidListener>) -> Vec<(String, u32, bool, Option<Underline>)> {
        let screen = screen_of(term, term.screen_lines());
        screen.rows[0].iter().map(|s| (s.text.clone(), s.fg, s.strike, s.underline)).collect()
    }

    /// The styles the grid keeps are drawn: dim, strikethrough, a curly underline in its own colour.
    #[test]
    fn dim_strikethrough_and_underline_styles_are_drawn() {
        let fg = palette::FOREGROUND;
        let plain = |text: &str| (text.to_string(), fg, false, None);
        let styled = term(40, 2, "a\x1b[2mdim\x1b[0m\x1b[9mx\x1b[0m\x1b[4:3;58:2::255:0:0mcu\x1b[4:2;59md\x1b[0m.");
        assert_eq!(
            spans(&styled),
            [
                plain("a"),
                ("dim".into(), palette::dim(fg, palette::BACKGROUND), false, None),
                ("x".into(), fg, true, None),
                ("cu".into(), fg, false, Some(Underline { curly: true, color: Some(0xff0000) })), // one run
                ("d".into(), fg, false, Some(Underline { curly: false, color: None })),
                plain(&format!(".{}", " ".repeat(31))), // the rest of the 40 columns
            ]
        );
        // Dim reversed text dims toward its new background.
        assert_eq!(spans(&term(4, 1, "\x1b[2;7mr"))[0].1, palette::dim(palette::BACKGROUND, fg));
    }

    /// A combining accent and an emoji's presentation selector are drawn with their cell.
    #[test]
    fn zero_width_characters_are_drawn_with_their_cell() {
        let term = term(10, 1, "cafe\u{301}\u{2764}\u{fe0f}");
        let row: String = spans(&term).into_iter().map(|span| span.0).collect();
        assert!(row.starts_with("cafe\u{301}\u{2764}\u{fe0f}"), "{row:?}");
    }

    /// Plain characters run together; one the font may not have stands alone on its columns, so
    /// the text after it keeps its own (`⎿` from a fallback font is wider than a cell).
    #[test]
    fn a_character_the_font_may_lack_is_placed_alone_on_its_columns() {
        let placed = term(10, 1, "a\u{23bf}b\u{65e5}c\u{2502}");
        let screen = screen_of(&placed, 1);
        let spans: Vec<(&str, usize, usize, bool)> =
            screen.rows[0].iter().map(|s| (s.text.as_str(), s.col, s.cells, s.alone)).collect();
        assert_eq!(
            spans,
            [
                ("a", 0, 1, false),
                ("\u{23bf}", 1, 1, true),
                ("b", 2, 1, false),
                ("\u{65e5}", 3, 2, true),
                ("c", 5, 1, false),
                ("\u{2502}", 6, 1, true),
                ("   ", 7, 3, false),
            ]
        );
        // The first half of a wide character erased: the letter after it keeps its column.
        let mut erased = term(10, 1, "a\u{65e5}b");
        print(&mut erased, "\r\x1b[2G\x1b[X");
        let row = &screen_of(&erased, 1).rows[0];
        let b = row.iter().find(|s| s.text.starts_with('b')).map(|s| s.col);
        assert_eq!(b, Some(3), "{:?}", row.iter().map(|s| (&s.text, s.col)).collect::<Vec<_>>());
    }

    /// A program that pushes the kitty keyboard protocol is told it is on, and its flags kept.
    #[test]
    fn the_kitty_keyboard_protocol_is_on_for_a_program_that_asks() {
        let mut t =
            Term::new(Config { kitty_keyboard: true, ..Config::default() }, &Grid { cols: 10, rows: 2 }, VoidListener);
        print(&mut t, "\x1b[>5u");
        assert!(t.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_ALTERNATE_KEYS));
        print(&mut t, "\x1b[<u");
        assert!(!t.mode().intersects(TermMode::KITTY_KEYBOARD_PROTOCOL));
    }

    /// What clients read keeps a cell's zero-width characters: a combining accent, a joiner.
    #[test]
    fn zero_width_characters_are_read_with_their_cell() {
        let term = term(20, 2, "cafe\u{301} a\u{200d}b");
        assert_eq!(lines_of(&term)[0], "cafe\u{301} a\u{200d}b");
        assert_eq!(text_of(&term, false), "cafe\u{301} a\u{200d}b");
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
