//! Terminal pane rules: scrolling back, mouse and keyboard selection (paste is `clip`). The shell applies
//! them to its `alacritty_terminal` grid.

use crate::keys::Mods;

/// A move of the view through the scrollback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scroll {
    /// Positive: back into history.
    Lines(i32),
    PageUp,
    PageDown,
    Top,
    Bottom,
}

/// ⇧PageUp / ⇧PageDown / ⇧Home / ⇧End scroll the history. On the alternate screen (vim, less)
/// there is no history, so the key goes to the program.
pub fn scroll_key(key: &str, mods: Mods, alt_screen: bool) -> Option<Scroll> {
    if alt_screen || !mods.shift || mods.ctrl || mods.alt || mods.cmd {
        return None;
    }
    match key {
        "pageup" => Some(Scroll::PageUp),
        "pagedown" => Some(Scroll::PageDown),
        "home" => Some(Scroll::Top),
        "end" => Some(Scroll::Bottom),
        _ => None,
    }
}

/// `LANG` for a terminal's programs when ottospace has neither `LANG` nor `LC_ALL` (started from
/// the Dock): the system's locale as UTF-8 if `exists` knows it, e.g. `de_CH@rg=chzzzz` →
/// `de_CH.UTF-8`, `zh-Hans_CN` → `zh_CN.UTF-8`, `ca_ES_VALENCIA` → `ca_ES.UTF-8`, else `en_US.UTF-8`. `None`: one is set, keep it.
pub fn utf8_locale(set: bool, system: Option<&str>, exists: impl Fn(&str) -> bool) -> Option<String> {
    if set {
        return None;
    }
    // `language[-Script]_REGION[_VARIANT][@keywords]`, e.g. `ca_ES_VALENCIA`.
    let id = system.unwrap_or_default().split('@').next().unwrap_or_default();
    let mut parts = id.split('_');
    let language = parts.next().unwrap_or_default().split('-').next().unwrap_or_default();
    let region = parts.next();
    let wanted = region.map(|region| format!("{language}_{region}.UTF-8"));
    Some(wanted.filter(|locale| !language.is_empty() && exists(locale)).unwrap_or_else(|| "en_US.UTF-8".into()))
}

/// What a program asked of the mouse and the wheel (DECSET 1000, 1002, 1003, 1006, 1007).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MouseMode {
    /// 1000: presses and releases.
    pub clicks: bool,
    /// 1002: moves while a button is down too.
    pub drags: bool,
    /// 1003: every move.
    pub motion: bool,
    /// 1006: reports as `CSI < b ; x ; y M`, not the old bytes.
    pub sgr: bool,
    pub alt_screen: bool,
    /// 1007: on the alternate screen, the wheel as arrow keys (on unless the program turns it off).
    pub alternate_scroll: bool,
    /// A program runs in front of the shell. At a prompt the modes are a program's that exited
    /// without turning them off: they ask for nothing.
    pub program: bool,
}

impl MouseMode {
    /// The program reads the mouse.
    pub fn reads(&self) -> bool {
        self.program && (self.clicks || self.drags || self.motion)
    }
}

/// Where the wheel goes: ottospace's history, the program's mouse, or the program as ↑ and ↓.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WheelGoes {
    History,
    Reports,
    Arrows,
}

/// Where the wheel goes. ⇧ held: ottospace's history, whatever the program asked (the way out
/// other terminals give). Else the program's while it reads the mouse (Claude Code's and Codex's
/// full-screen views scroll themselves); on the alternate screen (less, man) ↑ and ↓ unless the
/// program turned that off, then nothing; else the history. At a prompt, modes left on by a
/// program that exited ask for nothing.
pub fn wheel_goes(mode: MouseMode, shift: bool) -> Option<WheelGoes> {
    match (mode.reads(), mode.alt_screen && mode.program) {
        _ if shift => Some(WheelGoes::History),
        (true, _) => Some(WheelGoes::Reports),
        (false, true) => mode.alternate_scroll.then_some(WheelGoes::Arrows),
        (false, false) => Some(WheelGoes::History),
    }
}

/// A press goes to a program that reads the mouse, unless ⇧ or ⌥ is held: then it selects, as in
/// Terminal and iTerm. ⌘-click (open a path) is ottospace's either way.
pub fn press_is_the_programs(mode: MouseMode, mods: Mods) -> bool {
    mode.reads() && !mods.shift && !mods.alt && !mods.cmd
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

/// What the pointer did over a cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pointer {
    Press(Button),
    Release(Button),
    /// A move, with the button held if any.
    Move(Option<Button>),
    /// One line of the wheel: up (back) or down.
    Wheel {
        up: bool,
    },
}

/// The report of `pointer` at cell (`col`, `row`), from 0, for the program's `mode`; `None` when the
/// mode does not take it (a move without 1002 or 1003). e.g. a left press at (0, 0) with 1006:
/// `CSI < 0 ; 1 ; 1 M`, its release `... m`. Modifiers: ⇧ 4, ⌥ 8, ⌃ 16.
pub fn mouse_report(pointer: Pointer, col: usize, row: usize, mods: Mods, mode: MouseMode) -> Option<Vec<u8>> {
    if !mode.reads() {
        return None;
    }
    let code = |button: Button| match button {
        Button::Left => 0,
        Button::Middle => 1,
        Button::Right => 2,
    };
    let (button, release) = match pointer {
        Pointer::Press(button) => (code(button), false),
        Pointer::Release(button) => (code(button), true),
        Pointer::Move(Some(button)) if mode.drags || mode.motion => (code(button) + 32, false),
        Pointer::Move(None) if mode.motion => (3 + 32, false),
        Pointer::Move(_) => return None,
        Pointer::Wheel { up } => (if up { 64 } else { 65 }, false),
    };
    let button = button + 4 * mods.shift as u32 + 8 * mods.alt as u32 + 16 * mods.ctrl as u32;
    let (x, y) = (col + 1, row + 1);
    if mode.sgr {
        let end = if release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{button};{x};{y}{end}").into_bytes());
    }
    // The old form: a byte each, 32 on; a release has no button of its own (3). Past 223 it cannot say.
    let button = if release { 3 + (button & !3) } else { button };
    let byte = |n: usize| u8::try_from(n + 32).ok();
    Some(vec![0x1b, b'[', b'M', byte(button as usize)?, byte(x)?, byte(y)?])
}

/// Turns trackpad pixels into whole lines, keeping the remainder for the next event.
#[derive(Clone, Copy, Debug, Default)]
pub struct Wheel {
    rest: f32,
}

impl Wheel {
    /// `dy` is the event's vertical delta in pixels; positive scrolls back into history.
    pub fn lines(&mut self, dy: f32, line_height: f32) -> i32 {
        // Bounded: alacritty adds the count to its offset as an i32, which must not overflow.
        const MAX: f32 = 100_000.;
        if line_height <= 0. || !dy.is_finite() {
            return 0;
        }
        let total = self.rest + dy;
        let lines = (total / line_height).trunc();
        if lines.abs() > MAX {
            self.rest = 0.;
            return (lines.signum() * MAX) as i32;
        }
        self.rest = total - lines * line_height;
        lines as i32
    }
}

/// A point on the visible grid: row from the top, column, and whether it is the cell's right half.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CellPoint {
    pub row: usize,
    pub col: usize,
    pub right: bool,
}

/// The cell under `(x, y)`, measured from the grid's top-left in points. Outside the grid it
/// clamps to the nearest cell, so a drag past an edge selects to that edge.
pub fn cell_at(x: f32, y: f32, cell: (f32, f32), grid: (usize, usize)) -> CellPoint {
    let ((w, h), (cols, rows)) = (cell, grid);
    if w <= 0. || h <= 0. || cols == 0 || rows == 0 {
        return CellPoint { row: 0, col: 0, right: false };
    }
    let col = (x / w).floor();
    let row = (y / h).floor().clamp(0., (rows - 1) as f32) as usize;
    // Past the right edge is the last cell's right half; before the left edge, the first's left.
    if col >= cols as f32 {
        return CellPoint { row, col: cols - 1, right: true };
    }
    if col < 0. {
        return CellPoint { row, col: 0, right: false };
    }
    CellPoint { row, col: col as usize, right: x - col * w >= w / 2. }
}

/// What a click selects, by click count: 1 starts a plain drag selection, 2 a word, 3 a line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SelectKind {
    Simple,
    Word,
    Line,
}

pub fn select_kind(clicks: usize) -> SelectKind {
    match clicks {
        0 | 1 => SelectKind::Simple,
        2 => SelectKind::Word,
        _ => SelectKind::Line,
    }
}

/// What a left press on a terminal does to its selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Press {
    /// Starts one, replacing any other: a plain click selects nothing until it drags.
    Start(SelectKind),
    /// ⇧: moves the far end of the one there to the press. With none, starts one.
    Extend,
}

pub fn press(clicks: usize, shift: bool) -> Press {
    if shift { Press::Extend } else { Press::Start(select_kind(clicks)) }
}

/// Lines to scroll while a selection is dragged past the top (positive: back into history) or
/// the bottom of a grid `height` points tall, `y` from its top: one line a step at the edge,
/// more the farther out, at most 10. Inside the grid, none.
pub fn edge_scroll(y: f32, height: f32, line: f32) -> i32 {
    if line <= 0. || !y.is_finite() {
        return 0;
    }
    let past = if y < 0. { -y } else { y - height };
    let lines = (1. + past / (2. * line)).min(10.) as i32;
    match (y < 0., y > height) {
        (true, _) => lines,
        (_, true) => -lines,
        _ => 0,
    }
}

/// Esc with nothing held clears a terminal's selection, and the program does not get it.
pub fn clears_selection(key: &str, mods: Mods, selected: bool) -> bool {
    selected && key == "escape" && mods == Mods::default()
}

/// A selection stays while its text is what was selected: output that scrolls it keeps it,
/// output written over it (`kept` is no longer its text) clears it.
pub fn selection_holds(kept: &str, now: Option<&str>) -> bool {
    now == Some(kept)
}

/// A cursor move in visual mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Motion {
    Left,
    Down,
    Up,
    Right,
    WordRight,
    WordLeft,
    WordEnd,
    LineStart,
    LineEnd,
    FirstOccupied,
    Top,
    Middle,
    Bottom,
}

/// What a key does in visual mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VisualKey {
    Move(Motion),
    /// `v`: start a selection at the cursor, or clear it.
    Select,
    /// `V`: start a line selection, or clear it.
    SelectLines,
    /// `y` or ↵: copy the selection and leave.
    Copy,
    Exit,
}

/// Visual-mode keys, vi style. `typed` is the key's text, so ⇧H arrives as "H".
pub fn visual_key(key: &str, typed: Option<&str>) -> Option<VisualKey> {
    use Motion::*;
    let action = match key {
        "escape" => VisualKey::Exit,
        "enter" => VisualKey::Copy,
        "left" => VisualKey::Move(Left),
        "down" => VisualKey::Move(Down),
        "up" => VisualKey::Move(Up),
        "right" => VisualKey::Move(Right),
        _ => VisualKey::Move(match typed? {
            "h" => Left,
            "j" => Down,
            "k" => Up,
            "l" => Right,
            "w" => WordRight,
            "b" => WordLeft,
            "e" => WordEnd,
            "0" => LineStart,
            "$" => LineEnd,
            "^" => FirstOccupied,
            "H" => Top,
            "M" => Middle,
            "L" => Bottom,
            "v" => return Some(VisualKey::Select),
            "V" => return Some(VisualKey::SelectLines),
            "y" => return Some(VisualKey::Copy),
            _ => return None,
        }),
    };
    Some(action)
}

/// A process, as the shell crate reads it. `name`: e.g. "zsh"; `None`: it could not be read.
#[derive(Clone, Debug, PartialEq)]
pub struct Proc {
    pub pid: i32,
    pub name: Option<String>,
}

/// A terminal's processes, read by the shell crate when the user asks to quit.
#[derive(Clone, Debug, PartialEq)]
pub struct Processes {
    /// The process the terminal started: the shell, or `login` wrapping it (the default on macOS).
    pub child: Proc,
    /// The PTY's foreground process group. `None`: it could not be read.
    pub foreground: Option<i32>,
    /// The foreground group's leader: its name and its parent.
    pub leader: Option<String>,
    pub leader_parent: Option<i32>,
    /// The leader's children. When the leader is the shell, these are its jobs.
    pub children: Vec<Proc>,
}

/// What quitting would end in a terminal.
#[derive(Clone, Debug, PartialEq)]
pub struct Work {
    /// e.g. "cargo". `None`: no name could be read.
    pub program: Option<String>,
    /// A background or stopped job of the shell, not the program in front.
    pub background: bool,
}

/// Arguments as one line a shell reads back as the same arguments: each that holds anything
/// but letters, digits and `_@%+=:,./-` is quoted. e.g. `["grep", "a b", "it's"]` →
/// `grep 'a b' 'it'\''s'`.
pub fn command_line(args: &[String]) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c);
    let quoted = |arg: &String| match !arg.is_empty() && arg.chars().all(plain) {
        true => arg.clone(),
        false => format!("'{}'", arg.replace('\'', "'\\''")),
    };
    args.iter().map(quoted).collect::<Vec<_>>().join(" ")
}

/// A program's arguments as a workbook keeps its command, or none. A program that set its own
/// title (Node, npm: `process.title`) leaves `argv[0]` holding the title and the rest blank, e.g.
/// `["npm run dev", "", ""]`: an `argv[0]` with a space in it is no command. Blank arguments at
/// the end are such leftovers too, and go.
pub fn saved_command(args: &[String]) -> Option<String> {
    let used = args.iter().rposition(|arg| !arg.is_empty()).map_or(0, |last| last + 1);
    let args = &args[..used];
    let name = args.first().filter(|name| !name.chars().any(char::is_whitespace))?;
    (!name.is_empty()).then(|| command_line(args))
}

/// What a terminal runs besides its shell; `None` at an idle prompt.
///
/// The shell is the foreground group's leader when that is the child itself, or a child of the
/// child `login`. In front: idle unless it has children (`sleep 100 &`, a stopped vim), which
/// quitting ends too. Anything else in front is a program. Unknown counts: better to ask than
/// to lose work.
pub fn running(p: &Processes) -> Option<Work> {
    let Some(foreground) = p.foreground else {
        return Some(Work { program: None, background: false });
    };
    let wrapped = p.child.name.as_deref() == Some("login") && p.leader_parent == Some(p.child.pid);
    if foreground != p.child.pid && !wrapped {
        return Some(Work { program: p.leader.clone(), background: false });
    }
    let job = p.children.first()?;
    Some(Work { program: job.name.clone(), background: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminal_runs_something_unless_its_shell_is_in_front_with_no_jobs() {
        let proc = |pid, name: &str| Proc { pid, name: Some(name.into()) };
        let work = |program: &str, background| Some(Work { program: Some(program.into()), background });
        let unnamed = Some(Work { program: None, background: false });
        // (child, foreground, leader, leader's parent, leader's children) -> what runs
        type Case = (&'static str, Proc, Option<i32>, Option<&'static str>, Option<i32>, Vec<Proc>, Option<Work>);
        let cases: Vec<Case> = vec![
            ("direct shell, idle", proc(40, "sh"), Some(40), Some("sh"), Some(1), vec![], None),
            ("login-wrapped shell, idle", proc(40, "login"), Some(41), Some("zsh"), Some(40), vec![], None),
            ("program in front", proc(40, "sh"), Some(50), Some("cargo"), Some(40), vec![], work("cargo", false)),
            (
                "program, login-wrapped",
                proc(40, "login"),
                Some(50),
                Some("cargo"),
                Some(41),
                vec![],
                work("cargo", false),
            ),
            // A child of the child is the shell only when the child is login.
            ("child of a shell", proc(40, "zsh"), Some(50), Some("vim"), Some(40), vec![], work("vim", false)),
            (
                "background job",
                proc(40, "sh"),
                Some(40),
                Some("sh"),
                Some(1),
                vec![proc(60, "sleep")],
                work("sleep", true),
            ),
            (
                "stopped job, login-wrapped",
                proc(40, "login"),
                Some(41),
                Some("zsh"),
                Some(40),
                vec![proc(60, "vim"), proc(61, "sleep")],
                work("vim", true),
            ),
            // The shell has not taken the terminal yet: it is login's child, behind login.
            (
                "login in front",
                proc(40, "login"),
                Some(40),
                Some("login"),
                Some(1),
                vec![proc(41, "zsh")],
                work("zsh", true),
            ),
            ("foreground unreadable", proc(40, "sh"), None, None, None, vec![], unnamed.clone()),
            ("leader unreadable", proc(40, "login"), Some(41), None, None, vec![], unnamed),
            (
                "child unnamed",
                Proc { pid: 40, name: None },
                Some(41),
                Some("zsh"),
                Some(40),
                vec![],
                work("zsh", false),
            ),
        ];
        for (case, child, foreground, leader, leader_parent, children, expected) in cases {
            let processes = Processes { child, foreground, leader: leader.map(String::from), leader_parent, children };
            assert_eq!(running(&processes), expected, "{case}");
        }
    }

    #[test]
    fn a_command_line_quotes_what_a_shell_would_split_or_expand() {
        let line = |args: &[&str]| command_line(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>());
        assert_eq!(line(&["sleep", "293"]), "sleep 293");
        assert_eq!(line(&["grep", "a b", "it's", "$HOME", ""]), "grep 'a b' 'it'\\''s' '$HOME' ''");
        // A title a program set over its arguments is no command; blanks it left at the end go.
        let saved = |args: &[&str]| saved_command(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>());
        assert_eq!(saved(&["npm run dev", "", "", ""]), None);
        assert_eq!(saved(&["claude", "", ""]).as_deref(), Some("claude"));
        assert_eq!(saved(&["grep", "", "x"]).as_deref(), Some("grep '' x"));
        assert_eq!((saved(&[]), saved(&["", "x"])), (None, None));
    }

    const SHIFT: Mods = Mods { ctrl: false, alt: false, shift: true, cmd: false };

    #[test]
    fn shift_page_keys_scroll_except_on_the_alternate_screen() {
        assert_eq!(scroll_key("pageup", SHIFT, false), Some(Scroll::PageUp));
        assert_eq!(scroll_key("pagedown", SHIFT, false), Some(Scroll::PageDown));
        assert_eq!(scroll_key("home", SHIFT, false), Some(Scroll::Top));
        assert_eq!(scroll_key("end", SHIFT, false), Some(Scroll::Bottom));
        assert_eq!(scroll_key("pageup", Mods::default(), false), None, "plain PageUp is the program's");
        assert_eq!(scroll_key("pageup", Mods { ctrl: true, ..SHIFT }, false), None);
        assert_eq!(scroll_key("pageup", SHIFT, true), None, "less and vim get it");
        assert_eq!(scroll_key("a", SHIFT, false), None);
    }

    const CLICKS: MouseMode = MouseMode {
        clicks: true,
        drags: false,
        motion: false,
        sgr: true,
        alt_screen: false,
        alternate_scroll: true,
        program: true,
    };

    #[test]
    fn a_terminal_gets_a_utf8_locale_only_when_none_is_set() {
        let known = |locale: &str| ["en_US.UTF-8", "de_CH.UTF-8", "zh_CN.UTF-8", "ca_ES.UTF-8"].contains(&locale);
        assert_eq!(utf8_locale(true, Some("de_CH"), known), None, "the user's own LANG or LC_ALL stays");
        assert_eq!(utf8_locale(false, Some("de_CH@rg=chzzzz"), known).as_deref(), Some("de_CH.UTF-8"));
        assert_eq!(utf8_locale(false, Some("zh-Hans_CN"), known).as_deref(), Some("zh_CN.UTF-8"));
        assert_eq!(utf8_locale(false, Some("ca_ES_VALENCIA"), known).as_deref(), Some("ca_ES.UTF-8"));
        for unknown in [Some("fr_BE"), Some("en"), Some(""), None] {
            assert_eq!(utf8_locale(false, unknown, known).as_deref(), Some("en_US.UTF-8"), "{unknown:?}");
        }
    }

    #[test]
    fn the_wheel_belongs_to_programs_that_read_the_mouse_or_use_the_alternate_screen() {
        let plain = MouseMode { alternate_scroll: true, program: true, ..MouseMode::default() };
        assert_eq!(wheel_goes(plain, false), Some(WheelGoes::History), "a shell's prompt: the history");
        assert_eq!(wheel_goes(CLICKS, false), Some(WheelGoes::Reports), "Claude Code's full-screen view");
        assert_eq!(wheel_goes(MouseMode { alt_screen: true, ..CLICKS }, false), Some(WheelGoes::Reports));
        // less and man read no mouse: on the alternate screen the wheel is ↑ and ↓, unless turned off.
        assert_eq!(wheel_goes(MouseMode { alt_screen: true, ..plain }, false), Some(WheelGoes::Arrows));
        let off = MouseMode { alt_screen: true, alternate_scroll: false, ..plain };
        assert_eq!(wheel_goes(off, false), None);
    }

    /// ⇧ and the wheel scroll ottospace's history whatever the program asked.
    #[test]
    fn shift_and_the_wheel_always_scroll_the_history() {
        let alt = MouseMode { alt_screen: true, ..CLICKS };
        for mode in [CLICKS, alt, MouseMode { clicks: false, ..alt }, MouseMode::default()] {
            assert_eq!(wheel_goes(mode, true), Some(WheelGoes::History), "{mode:?}");
        }
    }

    /// A program that exited with the mouse or the alternate screen on leaves the modes at the
    /// prompt: there they ask for nothing, so the wheel scrolls the history and a click selects.
    #[test]
    fn modes_left_on_at_a_prompt_ask_for_nothing() {
        let left = MouseMode { alt_screen: true, program: false, ..CLICKS };
        assert_eq!(wheel_goes(left, false), Some(WheelGoes::History));
        assert!(!press_is_the_programs(left, Mods::default()));
        assert_eq!(mouse_report(Pointer::Wheel { up: true }, 0, 0, Mods::default(), left), None);
    }

    #[test]
    fn a_press_is_the_programs_unless_shift_or_option_is_held() {
        assert!(press_is_the_programs(CLICKS, Mods::default()));
        assert!(press_is_the_programs(CLICKS, Mods { ctrl: true, ..Mods::default() }));
        for held in [Mods { shift: true, ..Mods::default() }, Mods { alt: true, ..Mods::default() }] {
            assert!(!press_is_the_programs(CLICKS, held), "{held:?} selects");
        }
        assert!(!press_is_the_programs(CLICKS, Mods { cmd: true, ..Mods::default() }), "⌘-click opens a path");
        assert!(!press_is_the_programs(MouseMode::default(), Mods::default()), "a shell's prompt: it selects");
    }

    #[test]
    fn mouse_reports_say_the_button_the_cell_and_the_modifiers() {
        let none = Mods::default();
        let report = |pointer, col, row, mods, mode| mouse_report(pointer, col, row, mods, mode).map(String::from_utf8);
        let sgr = |text: &str| Some(Ok(text.to_string()));
        assert_eq!(report(Pointer::Press(Button::Left), 0, 0, none, CLICKS), sgr("\x1b[<0;1;1M"));
        assert_eq!(report(Pointer::Release(Button::Left), 4, 2, none, CLICKS), sgr("\x1b[<0;5;3m"));
        assert_eq!(
            report(Pointer::Press(Button::Right), 0, 0, Mods { ctrl: true, ..none }, CLICKS),
            sgr("\x1b[<18;1;1M")
        );
        assert_eq!(report(Pointer::Wheel { up: true }, 1, 1, none, CLICKS), sgr("\x1b[<64;2;2M"));
        assert_eq!(report(Pointer::Wheel { up: false }, 1, 1, none, CLICKS), sgr("\x1b[<65;2;2M"));
        // Moves only where the program asked: with a button under 1002, any under 1003.
        assert_eq!(report(Pointer::Move(Some(Button::Left)), 0, 0, none, CLICKS), None);
        let drags = MouseMode { drags: true, ..CLICKS };
        assert_eq!(report(Pointer::Move(Some(Button::Left)), 2, 0, none, drags), sgr("\x1b[<32;3;1M"));
        assert_eq!(report(Pointer::Move(None), 2, 0, none, drags), None);
        assert_eq!(report(Pointer::Move(None), 2, 0, none, MouseMode { motion: true, ..CLICKS }), sgr("\x1b[<35;3;1M"));
        // The old form: bytes, 32 on; a release is button 3; too far right for a byte, none.
        let old = MouseMode { sgr: false, ..CLICKS };
        assert_eq!(mouse_report(Pointer::Press(Button::Left), 0, 0, none, old), Some(b"\x1b[M !!".to_vec()));
        assert_eq!(mouse_report(Pointer::Release(Button::Left), 0, 0, none, old), Some(b"\x1b[M#!!".to_vec()));
        assert_eq!(mouse_report(Pointer::Press(Button::Left), 300, 0, none, old), None);
        assert_eq!(mouse_report(Pointer::Press(Button::Left), 0, 0, none, MouseMode::default()), None);
    }

    #[test]
    fn wheel_keeps_partial_lines_for_the_next_event() {
        let mut wheel = Wheel::default();
        assert_eq!(wheel.lines(10., 18.), 0);
        assert_eq!(wheel.lines(10., 18.), 1); // 20px: one line, 2px left
        assert_eq!(wheel.lines(34., 18.), 2); // 36px
        assert_eq!(wheel.lines(-18., 18.), -1);
        assert_eq!(wheel.lines(5., 0.), 0);
        // A huge or broken delta (e.g. from the socket) stays bounded and leaves the wheel working.
        assert_eq!(wheel.lines(f32::INFINITY, 18.), 0);
        assert_eq!(wheel.lines(1e30, 18.), 100_000);
        assert_eq!(wheel.lines(-1e30, 18.), -100_000);
        assert_eq!(wheel.lines(18., 18.), 1);
    }

    #[test]
    fn positions_map_to_cells_and_clamp_at_the_edges() {
        let (cell, grid) = ((8., 18.), (80, 24));
        assert_eq!(cell_at(0., 0., cell, grid), CellPoint { row: 0, col: 0, right: false });
        assert_eq!(cell_at(13., 20., cell, grid), CellPoint { row: 1, col: 1, right: true });
        assert_eq!(cell_at(11.9, 35.9, cell, grid), CellPoint { row: 1, col: 1, right: false });
        assert_eq!(cell_at(-5., -5., cell, grid), CellPoint { row: 0, col: 0, right: false });
        assert_eq!(cell_at(9999., 9999., cell, grid), CellPoint { row: 23, col: 79, right: true });
        assert_eq!(cell_at(5., 5., cell, (0, 0)), CellPoint { row: 0, col: 0, right: false });
    }

    #[test]
    fn click_count_picks_the_selection_kind() {
        assert_eq!(select_kind(1), SelectKind::Simple);
        assert_eq!(select_kind(2), SelectKind::Word);
        assert_eq!(select_kind(3), SelectKind::Line);
        assert_eq!(select_kind(4), SelectKind::Line);
    }

    #[test]
    fn shift_extends_a_selection_and_clicks_pick_its_kind() {
        assert_eq!(press(1, false), Press::Start(SelectKind::Simple));
        assert_eq!(press(2, false), Press::Start(SelectKind::Word));
        assert_eq!(press(3, false), Press::Start(SelectKind::Line));
        assert_eq!((press(1, true), press(2, true)), (Press::Extend, Press::Extend));
    }

    #[test]
    fn a_drag_past_an_edge_scrolls_faster_the_farther_out() {
        let (height, line) = (180., 18.);
        assert_eq!(edge_scroll(90., height, line), 0);
        assert_eq!((edge_scroll(0., height, line), edge_scroll(height, height, line)), (0, 0));
        assert_eq!(edge_scroll(-1., height, line), 1, "above: back into history");
        assert_eq!(edge_scroll(height + 1., height, line), -1, "below: toward the bottom");
        assert_eq!(edge_scroll(-72., height, line), 3);
        assert_eq!(edge_scroll(-9_999., height, line), 10);
        assert_eq!((edge_scroll(f32::NAN, height, line), edge_scroll(-5., height, 0.)), (0, 0));
    }

    #[test]
    fn plain_esc_clears_a_selection() {
        assert!(clears_selection("escape", Mods::default(), true));
        assert!(!clears_selection("escape", Mods::default(), false), "with none, the program gets it");
        assert!(!clears_selection("escape", SHIFT, true));
        assert!(!clears_selection("a", Mods::default(), true));
    }

    #[test]
    fn a_selection_holds_while_its_text_does() {
        assert!(selection_holds("beta", Some("beta")));
        assert!(!selection_holds("beta", Some("bet")), "written over");
        assert!(!selection_holds("beta", None), "scrolled out of the history, or cleared");
    }

    #[test]
    fn visual_keys_are_vi_keys() {
        assert_eq!(visual_key("h", Some("h")), Some(VisualKey::Move(Motion::Left)));
        assert_eq!(visual_key("down", None), Some(VisualKey::Move(Motion::Down)));
        assert_eq!(visual_key("h", Some("H")), Some(VisualKey::Move(Motion::Top)));
        assert_eq!(visual_key("4", Some("$")), Some(VisualKey::Move(Motion::LineEnd)));
        assert_eq!(visual_key("v", Some("v")), Some(VisualKey::Select));
        assert_eq!(visual_key("v", Some("V")), Some(VisualKey::SelectLines));
        assert_eq!(visual_key("y", Some("y")), Some(VisualKey::Copy));
        assert_eq!(visual_key("enter", Some("\n")), Some(VisualKey::Copy));
        assert_eq!(visual_key("escape", None), Some(VisualKey::Exit));
        assert_eq!(visual_key("q", Some("q")), None);
    }
}
