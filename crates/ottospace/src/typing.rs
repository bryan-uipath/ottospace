//! Developer ▸ "Time Typing in This Terminal": made-up keys written to a throwaway `cat` in a new
//! pane, never the user's shell or anything else, each timed from the key to the frame that paints
//! its echo (`ottospace_core::typing`). A test run also times a full-screen program, and a busy pane
//! beside. The keys are written to that terminal as its key handler writes a key's bytes; GPUI's
//! dispatch of a real key to that handler (one call on the main thread) is not in the times.

use std::time::{Duration, Instant};

use gpui::{Context, Window};
use ottospace_core::layout::Axis;
use ottospace_core::model::{Action, Kind, Origin, PaneId};
use ottospace_core::typing::{self, APP_BUDGET_MS, Key, Typed, key_char};

use crate::Workspace;
use crate::terminal::{Stamps, Terminal};

/// Keys a run times, one each `GAP`: longer than a key's way to the screen, so each is its own.
pub(crate) const KEYS: usize = 250;
const GAP: Duration = Duration::from_millis(60);
/// The program's start, before the first key.
const START: Duration = Duration::from_millis(800);
/// An Enter to `cat` after this many keys keeps its line short; it is not timed.
const LINE: usize = 40;
/// A run this old is over, its task gone (its window closed): a new one replaces it.
const STALE: Duration = Duration::from_secs(60);

/// What a run's pane runs: `cat` (its echo is the tty's, as at a shell's prompt), a full-screen
/// program that draws every row again for each key (`ottospace typing-echo`), or made-up output
/// without end for the busy pane beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Program {
    Cat,
    Full,
    Busy,
}

impl Program {
    pub(crate) fn start(self) -> std::io::Result<Terminal> {
        match self {
            Program::Cat => Terminal::exec("/bin/cat", Vec::new()),
            Program::Full => {
                let me = std::env::current_exe()?.to_string_lossy().into_owned();
                Terminal::exec(&me, vec!["typing-echo".into()])
            }
            Program::Busy => Terminal::exec("/usr/bin/yes", vec!["made-up output line 0123456789 abcdefghij".into()]),
        }
    }
}

/// A run in flight: its pane, the busy pane beside it, the pane focused before, the keys timed.
pub(crate) struct Typing {
    pane: PaneId,
    busy: Option<PaneId>,
    back: Option<PaneId>,
    full: bool,
    started: Instant,
    keys: Vec<Key>,
}

/// A run's numbers with what it ran: `typing::Typed` and the display's refresh.
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct Timed {
    #[serde(flatten)]
    pub(crate) typed: Typed,
    pub(crate) full: bool,
    pub(crate) busy: bool,
    pub(crate) done: bool,
    /// The window's display: how often it shows a frame (Hz), 0 unknown.
    pub(crate) hz: f64,
}

impl Workspace {
    /// Starts a run: the user's own (the Developer command; a test run's `typing_time`). `full`: a
    /// full-screen program in place of `cat`; `busy`: a pane printing without end beside it.
    pub(crate) fn typing_start(
        &mut self,
        origin: Origin,
        (full, busy): (bool, bool),
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if origin != Origin::User || self.typing.as_ref().is_some_and(|t| t.started.elapsed() < STALE) {
            return;
        }
        if let Some(stale) = self.typing.take() {
            self.typing_close(&stale, window, cx);
        }
        // Split from a pane that holds something: an empty one would be filled in place.
        let back = self.model.focused.filter(|pane| self.model.pane(*pane).is_some_and(|p| p.kind != Kind::Empty));
        let program = if full { Program::Full } else { Program::Cat };
        let Some(pane) = back.and_then(|_| self.typing_pane(program, Axis::Right, origin, window, cx)) else {
            self.told = Some(("Focus a terminal or an app pane to time typing beside it".into(), Instant::now()));
            return cx.notify();
        };
        let busy = busy.then(|| self.typing_pane(Program::Busy, Axis::Down, origin, window, cx)).flatten();
        let started = Instant::now();
        self.typing = Some(Typing { pane, busy, back, full, started, keys: Vec::new() });
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(START).await;
            for i in 0..KEYS {
                if !this.update(cx, |this, _| this.typing_key(pane, i)).unwrap_or(false) {
                    break;
                }
                cx.background_executor().timer(GAP).await;
                if !this.update(cx, |this, _| this.typing_took(pane)).unwrap_or(false) {
                    break;
                }
                if !full && (i + 1) % LINE == 0 {
                    let _ = this.update(cx, |this, _| this.terminals.get(&pane).map(|t| t.write(b"\r".to_vec())));
                    cx.background_executor().timer(GAP).await;
                }
            }
            let _ = this.update_in(cx, |this, window, cx| this.typing_end(pane, window, cx));
        })
        .detach();
        cx.notify();
    }

    /// A pane split from the focused one, running `program`.
    fn typing_pane(
        &mut self,
        program: Program,
        axis: Axis,
        origin: Origin,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        let was = self.model.focused;
        self.typing_program = Some(program);
        self.dispatch(Action::Split { axis, kind: Kind::Terminal }, origin, window, cx);
        self.typing_program = None;
        self.model.focused.filter(|pane| Some(*pane) != was && self.terminals.contains_key(pane))
    }

    /// The `i`th key written to the run's own terminal, its timing begun; false once the run is not
    /// this one or its pane is gone.
    fn typing_key(&mut self, pane: PaneId, i: usize) -> bool {
        let ours = self.typing.as_ref().is_some_and(|t| t.pane == pane);
        let Some(terminal) = self.terminals.get(&pane).filter(|_| ours) else { return false };
        terminal.time_key(Instant::now());
        terminal.write(key_char(i).to_string().into_bytes());
        true
    }

    /// The timed key's stages; false once the run is not this one.
    fn typing_took(&mut self, pane: PaneId) -> bool {
        let Some(stamps) = self.terminals.get(&pane).map(Terminal::stamps) else { return false };
        let Some(typing) = self.typing.as_mut().filter(|t| t.pane == pane) else { return false };
        typing.keys.extend(key_of(&stamps));
        true
    }

    /// The run of `pane` ends (its keys, Esc, or its pane gone): its panes closed, the focus back,
    /// its numbers in a toast, the event log and a test run's state.
    fn typing_end(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(typing) = self.typing.take_if(|t| t.pane == pane) else { return };
        self.typing_close(&typing, window, cx);
        let hz = window.display(cx).map_or(0., |display| ottospace_mac::display::refresh_hz(display.id().into()));
        let typed = typing::summary(&typing.keys);
        let done = typing.keys.len() == KEYS;
        let text = format!(
            "Typing{}: {} keys, key to screen p50 {:.1} ms, p99 {:.1}; the app's own share p50 {:.1}, p99 {:.1} (budget {APP_BUDGET_MS}); display {hz:.0} Hz{}",
            if done { "" } else { " stopped" },
            typed.keys,
            typed.total.p50,
            typed.total.p99,
            typed.app.p50,
            typed.app.p99,
            if typed.missed > 0 { format!("; {} never drawn", typed.missed) } else { String::new() },
        );
        let timed = Timed { typed, full: typing.full, busy: typing.busy.is_some(), done, hz };
        crate::event_log::event("typing_time", serde_json::to_value(&timed).unwrap_or_default());
        self.told = Some((text, Instant::now()));
        self.typing_timed = Some(timed);
        cx.notify();
    }

    /// A run's panes closed, the focus back if one of them still has it. A pane the core will not
    /// close now (the key reference is over it): its program ends, and an ended terminal's pane
    /// closes at the tick once it may.
    fn typing_close(&mut self, typing: &Typing, window: &Window, cx: &mut Context<Self>) {
        let ours = [Some(typing.pane), typing.busy];
        let had_focus = ours.contains(&self.model.focused);
        for pane in ours.into_iter().flatten() {
            if self.model.pane(pane).is_none() {
                continue;
            }
            self.dispatch(Action::Close { pane: Some(pane) }, Origin::User, window, cx);
            if self.model.pane(pane).is_some()
                && let Some(terminal) = self.terminals.get(&pane)
            {
                terminal.signal(libc::SIGTERM);
            }
        }
        if let Some(back) = typing.back.filter(|back| had_focus && self.model.pane(*back).is_some()) {
            self.dispatch(Action::Focus { pane: back }, Origin::User, window, cx);
        }
    }

    /// Esc during a run stops it.
    pub(crate) fn typing_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(pane) = self.typing.as_ref().map(|t| t.pane).filter(|_| self.input == Origin::User) else {
            return false;
        };
        self.typing_end(pane, window, cx);
        true
    }

    /// For a test run's state: whether a run is on, and the last one's numbers.
    pub(crate) fn typing_snapshot(&self) -> serde_json::Value {
        serde_json::json!({ "running": self.typing.is_some(), "result": self.typing_timed })
    }
}

/// A key's stages in µs from its event; none for a key never written.
fn key_of(s: &Stamps) -> Option<Key> {
    let key = s.key?;
    let us = |at: Option<Instant>| at.map(|at| at.saturating_duration_since(key).as_micros() as u64);
    s.write?;
    Some(Key { write: us(s.write), echo: us(s.echo.map(|(at, _)| at)), taken: us(s.taken), painted: us(s.painted) })
}

/// `ottospace typing-echo`: a made-up full-screen program for a timed run. Raw mode, the
/// alternate screen; each key read draws every row again, the keys so far on the last. Ends at
/// end of input (its pane closed) or ⌃D.
pub(crate) fn echo_main() -> ! {
    use std::io::{Read, Write};
    let fd = libc::STDIN_FILENO;
    // SAFETY: a termios of our own stdin, read and set.
    let mut raw: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut raw) } == 0 {
        unsafe { libc::cfmakeraw(&mut raw) };
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) };
    }
    let size = || {
        // SAFETY: TIOCGWINSZ fills a winsize of ours.
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
        (ws.ws_row.max(1) as usize, ws.ws_col.max(1) as usize)
    };
    let (mut input, mut out) = (std::io::stdin().lock(), std::io::stdout().lock());
    let _ = out.write_all(b"\x1b[?1049h");
    let (mut typed, mut byte) = (String::new(), [0u8]);
    loop {
        let (rows, cols) = size();
        // The typed row first: the batch that brings a key's echo holds it; then every other row.
        let tail: String =
            typed.chars().rev().take(cols.saturating_sub(1)).collect::<Vec<_>>().into_iter().rev().collect();
        let mut screen = format!("\x1b[{rows};1H\x1b[2K{tail}\x1b[H");
        for row in 0..rows.saturating_sub(1) {
            let line: String = (0..cols).map(|c| key_char(row * 7 + c)).collect();
            screen.push_str(&line);
            screen.push_str("\r\n");
        }
        screen.push_str(&format!("\x1b[{rows};{}H", tail.chars().count() + 1));
        let _ = out.write_all(screen.as_bytes());
        let _ = out.flush();
        match input.read(&mut byte) {
            Ok(1) if byte[0] != 4 => typed.push(byte[0] as char),
            _ => break,
        }
    }
    let _ = out.write_all(b"\x1b[?1049l");
    std::process::exit(0)
}
