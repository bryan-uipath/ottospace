//! Terminal pane rules: scrolling back, mouse and keyboard selection, paste. The shell applies
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

/// Whether the wheel scrolls our history. Not while the program reads the mouse or uses the
/// alternate screen: the wheel is the program's there.
pub fn wheel_scrolls(mouse_mode: bool, alt_screen: bool) -> bool {
    !mouse_mode && !alt_screen
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
#[derive(Clone, Copy, Debug, PartialEq)]
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

/// Bytes for pasting `text`. With bracketed paste the program sees it as one paste; ESC is
/// removed so the text cannot end the bracket early. Without it, newlines become ↵ (CR).
pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let body: String = text.chars().filter(|c| *c != '\x1b').collect();
        return format!("\x1b[200~{body}\x1b[201~").into_bytes();
    }
    text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
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

    #[test]
    fn the_wheel_belongs_to_programs_that_read_the_mouse_or_use_the_alternate_screen() {
        assert!(wheel_scrolls(false, false));
        assert!(!wheel_scrolls(true, false));
        assert!(!wheel_scrolls(false, true));
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
    fn paste_brackets_when_asked_and_otherwise_sends_returns() {
        assert_eq!(paste("a\nb", true), b"\x1b[200~a\nb\x1b[201~".to_vec());
        assert_eq!(paste("x\x1b[201~rm", true), b"\x1b[200~x[201~rm\x1b[201~".to_vec(), "no early end");
        assert_eq!(paste("a\nb\r\nc", false), b"a\rb\rc".to_vec());
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
