//! Random actions against the model, with what must hold after every one of them (`check`,
//! `Run::step`). Seeded: a run is the same on every machine. A failure prints its seed and the
//! least steps that still fail, as a test to paste under "Regressions" below.
//!
//! `OTTOSPACE_PROP_SEED=<n>` runs that one sequence again.

use super::*;
use crate::ax::{Frame, Source};
use crate::calendar::Event;
use crate::compose::{Known, Names, PathFound};
use crate::fuzz::{self, Rng};
use crate::layout_file;
use crate::marks::{Mark, PaneRecord, RecordKind, TabRecord};
use crate::progress;
use crate::repos::{MAX_DIRS, RepoDir, RepoDirs, Worktree};
use crate::sessions::{Link, Seen};
use crate::settings::ListChange;
use crate::sneak_apps;
use crate::transcript::{Event as Said, Outcome, Text, View};
use crate::workbook::{self, History, Workbook, WorkbookId};

const WORDS: &[&str] = &["fix", "zoom", "flicker", "deploy", "x"];
const REPOS: &[&str] = &["app", "canvas", "My Project", "2048", "Ottospace"];
const DIRS: &[&str] = &["/code/app", "/code/canvas", "/code/app-2", "/tmp", "/Users/me/My Notes", "relative"];
const AGENTS: &[&str] = &["claude", "codex", "Claude Code", "claude-opus-5-5", "codex-gpt-6-astra"];
const SKILLS: &[&str] = &["code-review", "deploy"];
const APPS: &[&str] = &["Slack", "Safari", "Finder", "Sketch", "Slides", ""];
const UIDS: &[&str] = &["u1", "u2", "u3", "u4", ""];
/// A workbook's id is its file's name: three that may be one, and some that may not.
const BOOK_UIDS: &[&str] = &["00000000000000a1", "00000000000000a2", "00000000000000a3", "../../x", "A1", ""];
/// The last one a session a runner can go on with: its id is a UUID.
const SESSIONS: &[&str] = &["s1", "s2", "s3", "8f14e45f-ceea-467f-a8d3-0242ac130004"];
/// In every line of a terminal's saved history, and nowhere else.
const SAVED: &str = "SAVEDHISTORY";
const CONTROLS: &[&str] = &["rail:apps", "pane:1", "close:1", "tab:1", "tab-close:1", "mark-remove:1", "c0", "c1"];

/// Arguments for an action: from the model as it is half the time, else anything.
struct Gen<'a> {
    rng: &'a mut Rng,
    m: &'a Model,
}

impl Gen<'_> {
    fn stray(&mut self) -> u32 {
        let next = self.m.next_id + 1;
        self.rng.pick(&[0, 1, 2, 3, 7, next, u32::MAX])
    }

    /// One of `real`, half the time and if there is one; else `stray`.
    fn one_of<T: Clone>(&mut self, real: &[T], stray: impl FnOnce(&mut Self) -> T) -> T {
        if !real.is_empty() && self.rng.coin() { self.rng.pick(real) } else { stray(self) }
    }

    fn pane(&mut self) -> PaneId {
        let real: Vec<PaneId> = self.m.panes.iter().map(|p| p.id).collect();
        self.one_of(&real, Self::stray)
    }

    fn tab_id(&mut self) -> u32 {
        let real: Vec<u32> = self.m.tabs.iter().map(|t| t.id).collect();
        self.one_of(&real, Self::stray)
    }

    fn index(&mut self, len: usize) -> usize {
        let real: Vec<usize> = (0..len).collect();
        self.one_of(&real, |g| g.rng.pick(&[0, len, 99, usize::MAX]))
    }

    fn workbook(&mut self) -> WorkbookId {
        let real: Vec<WorkbookId> = self.m.workbooks.list.iter().map(|book| book.id).collect();
        self.one_of(&real, Self::stray)
    }

    /// A place among a workbook's tabs, or none: the whole workbook.
    fn book_tab(&mut self) -> Option<usize> {
        self.rng.coin().then(|| self.rng.pick(&[0, 1, 2, 5, usize::MAX]))
    }

    /// A workbook as a file may hold it: sound or not, with keys this build does not know.
    fn kept_workbook(&mut self) -> Workbook {
        let panes = vec![self.record(1), self.record(2)];
        let split =
            Node::Split { axis: self.axis(), ratio: 0.5, a: Box::new(Node::Leaf(1)), b: Box::new(Node::Leaf(2)) };
        let layout = self.rng.pick(&[Some(Node::Leaf(1)), Some(split), Some(Node::Leaf(9)), None]);
        let tab =
            TabRecord { title: self.name(WORDS), task: None, focused: Some(1), layout, panes, extra: self.extra() };
        Workbook {
            id: 0,
            version: self.rng.pick(&[workbook::VERSION, 1, 3]),
            uid: self.rng.pick(BOOK_UIDS).to_string(),
            name: self.name(WORDS),
            tabs: (0..self.rng.below(3)).map(|_| tab.clone()).collect(),
            live: Vec::new(),
            extra: self.extra(),
        }
    }

    /// A session's id as the shell makes one, or text that is none.
    fn session_id(&mut self) -> String {
        self.rng.pick(&["8f14e45f-ceea-467f-a8d3-0242ac130003", "--resume", ""]).to_string()
    }

    fn task(&mut self) -> TaskId {
        let real: Vec<TaskId> = self.m.tasks.list.iter().map(|t| t.id).collect();
        self.one_of(&real, Self::stray)
    }

    /// A runner's event: turns, text, calls, requests (cut or not) and their end, an exit.
    fn event(&mut self) -> Said {
        let text = Text::new(&self.name(WORDS));
        let id = self.rng.pick(&["a", "b"]).to_string();
        let (tool, label) = ("Bash".to_string(), self.name(WORDS));
        match self.rng.below(14) {
            12 => Said::MessageElsewhere,
            13 => Said::WatchFromStart,
            0 | 1 => Said::TurnStarted,
            2 => Said::TextDelta { text },
            3 => Said::Text { text },
            4 => {
                let input = serde_json::json!({"file_path": "/w/a", "old_string": "x", "new_string": text.text()});
                let diff = self.rng.coin().then(|| crate::diff::claude("Edit", &input)).flatten();
                Said::ToolStart { id, sub: self.rng.coin(), tool, label, detail: text, diff }
            }
            5 => Said::ToolEnd { id, failed: self.rng.coin() },
            6 | 7 => {
                let (ticket, cut) = (self.rng.below(3) as u64, self.rng.coin());
                Said::Permission { ticket, tool, label, detail: text, len: 9, cut, claim: Text::default() }
            }
            8 => Said::PermissionClosed { ticket: self.rng.below(3) as u64 },
            9 => Said::TurnFinished {
                outcome: self.rng.pick(&[
                    Outcome::Completed,
                    Outcome::Interrupted,
                    Outcome::Failed { kind: "error".into() },
                ]),
            },
            10 => Said::Usage { context: Some(10), window: None, cost_micro_usd: None },
            _ => Said::Exited { code: None },
        }
    }

    /// A card's ticket: mostly the one shown, else another (one withdrawn, or none).
    fn card(&mut self) -> u64 {
        let front = self.m.shown_chat().and_then(|chat| chat.transcript.cards.front().map(|card| card.ticket));
        match front.filter(|_| !self.rng.one_in(4)) {
            Some(ticket) => ticket,
            None => self.rng.below(3) as u64,
        }
    }

    /// A conversation's subject: a task's, or a session's.
    fn subject(&mut self) -> Subject {
        if self.rng.coin() {
            return Subject::Task(self.task());
        }
        let (runner, id) = self.session();
        Subject::Session { runner, id }
    }

    /// The bundle of an app of the pool, or a path that is no app's.
    fn app_path(&mut self) -> String {
        let name = self.name(APPS);
        if self.rng.one_in(6) { "/tmp/Evil.app".to_string() } else { format!("/Applications/{name}.app") }
    }

    fn delta(&mut self) -> i32 {
        self.rng.pick(&[1, -1, 1, -1, 2, -3, 0, i32::MAX, i32::MIN])
    }

    fn time(&mut self) -> u64 {
        let now = self.m.now;
        let later = [1, tasks::NOTICE_MS, 60_000, 200_000_000].map(|ms| now.saturating_add(ms));
        self.rng.pick(&[0, 1, now, later[0], later[1], later[2], later[3], u64::MAX])
    }

    fn text(&mut self) -> String {
        fuzz::hostile(self.rng)
    }

    /// A meeting near the model's time, with an allowed link, one that is not, or none.
    fn meeting(&mut self) -> Event {
        let now = self.m.now;
        let near = [60_000, 600_000].map(|ms| now.saturating_add(ms));
        let start = self.rng.pick(&[now, now.saturating_sub(60_000), near[0], near[1], 0, u64::MAX / 2]);
        let link = self.rng.pick(&[None, Some("https://example.zoom.us/j/1"), Some("http://example.com")]);
        Event {
            id: self.rng.pick(&[1, 2, 3]),
            start,
            end: start.saturating_add(self.rng.pick(&[0, 60_000, 1_800_000])),
            all_day: self.rng.one_in(5),
            title: self.text(),
            location: self.text(),
            link: link.map(String::from),
        }
    }

    fn meeting_id(&mut self) -> u64 {
        let real: Vec<u64> = self.m.calendar.events.iter().map(|e| e.id).collect();
        self.one_of(&real, |g| g.rng.pick(&[0, 9, u64::MAX]))
    }

    /// A plain name from `pool`, else hostile text.
    fn name(&mut self, pool: &[&str]) -> String {
        if self.rng.one_in(4) { self.text() } else { self.rng.pick(pool).to_string() }
    }

    /// A runner as the wire names it, or now and then any text.
    fn runner(&mut self) -> String {
        self.name(&RunnerId::ALL.map(RunnerId::name))
    }

    fn names(&mut self, pool: &[&str], most: usize) -> Vec<String> {
        (0..self.rng.below(most + 1)).map(|_| self.name(pool)).collect()
    }

    fn axis(&mut self) -> Axis {
        self.rng.pick(&[Axis::Right, Axis::Down])
    }

    fn dir(&mut self) -> Dir {
        self.rng.pick(&[Dir::Left, Dir::Right, Dir::Up, Dir::Down])
    }

    fn state(&mut self) -> State {
        use State::*;
        self.rng.pick(&[Captured, Triaged, Running, NeedsYou, Review, Stale, Parked, Done, Cleaned])
    }

    fn kind(&mut self) -> Kind {
        match self.rng.below(4) {
            0 => Kind::Empty,
            1 | 2 => Kind::Terminal,
            _ => Kind::App {
                window: self.rng.below(5) as u32,
                title: self.name(&["general", "w1"]),
                bundle: self.rng.maybe(|rng| rng.pick(&["com.slack", "com.safari"]).to_string()),
                name: self.name(APPS),
            },
        }
    }

    /// A size in points: a likely one, else zero, negative or not a number at all.
    fn size(&mut self) -> [f32; 2] {
        let likely = [400., 406., 520., 600., 638., 232.];
        let odd = [0., 1., -5., 1e9, f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
        [0, 1].map(|_| if self.rng.one_in(4) { self.rng.pick(&odd) } else { self.rng.pick(&likely) })
    }

    /// A line for the new-task box: words and tags, known and not.
    fn line(&mut self) -> String {
        let word = |g: &mut Self| match g.rng.below(8) {
            0 => format!("#{}", g.name(REPOS)),
            1 => format!(">{}", g.name(REPOS)),
            2 => format!("@{}", g.name(AGENTS)),
            3 => format!("${}", g.name(SKILLS)),
            4 => g.name(&["/src/export", "~/notes", "/src/My\\ Dir/", "p1", "P9", "p0"]),
            _ => g.name(WORDS),
        };
        let words: Vec<String> = (0..self.rng.below(6)).map(|_| word(self)).collect();
        words.join(self.rng.pick(&[" ", " ", "  ", "\t"]))
    }

    fn known(&mut self, line: &str) -> Known {
        let found =
            |words: Vec<String>| PathFound { resolved: format!("/code/app{}", words[0]), typed: words[0].clone() };
        let paths =
            if self.rng.coin() { compose::path_words(line).into_iter().map(found).collect() } else { Vec::new() };
        let names = Names { agents: self.names(AGENTS, 3), skills: self.names(SKILLS, 3) };
        Known { names, paths, entries: self.names(&["/src/", "/src/export", "~/My Notes/"], 3) }
    }

    fn repos(&mut self) -> Vec<Repo> {
        let branch = |g: &mut Self| g.rng.maybe(|rng| rng.pick(&["main", "fix/zoom"]).to_string());
        let worktree = |g: &mut Self| Worktree { name: g.name(REPOS), path: g.name(DIRS), branch: branch(g) };
        let repo = |g: &mut Self| Repo {
            name: g.name(REPOS),
            path: g.name(DIRS),
            branch: branch(g),
            worktrees: (0..g.rng.below(3)).map(|_| worktree(g)).collect(),
        };
        (0..self.rng.below(4)).map(|_| repo(self)).collect()
    }

    fn repo_dirs(&mut self) -> RepoDirs {
        let dir = |g: &mut Self| g.name(DIRS);
        let dirs = (0..self.rng.below(3)).map(|_| dir(self)).map(|path| RepoDir { text: path.clone(), path }).collect();
        RepoDirs { dirs, fixed: self.rng.below(4) == 0 }
    }

    fn repo_path(&mut self) -> String {
        let real: Vec<String> = self.m.repo_names().map(|(_, path)| path.to_string()).collect();
        self.one_of(&real, |g| g.name(DIRS))
    }

    fn assign(&mut self) -> Assign {
        let most = if self.rng.one_in(8) { 30 } else { 2 };
        let paths = self.names(DIRS, most);
        let resolved = if self.rng.coin() { paths.clone() } else { self.names(DIRS, 2) };
        Assign { agent: self.rng.coin().then(|| self.name(AGENTS)), paths, resolved, skills: self.names(SKILLS, 3) }
    }

    fn link(&mut self) -> Link {
        Link { runner: self.runner(), id: self.name(SESSIONS), state: self.rng.coin().then(|| self.state()) }
    }

    fn kept_task(&mut self) -> Task {
        let id = self.task();
        Task {
            id: self.rng.pick(&[id, id, tasks::MAX_ID, tasks::MAX_ID + 1]),
            title: self.name(WORDS),
            state: self.state(),
            priority: self.rng.pick(&[0, 1, 3, 9, 200]),
            snoozed_until: self.rng.coin().then(|| self.time()),
            dismissed: self.rng.one_in(4),
            project: self.rng.coin().then(|| self.name(REPOS)),
            repo: self.rng.coin().then(|| self.name(DIRS)),
            agent: self.rng.coin().then(|| self.name(AGENTS)),
            paths: self.names(DIRS, 2),
            resolved: self.names(DIRS, 2),
            skills: self.names(SKILLS, 2),
            created: self.rng.coin().then(|| self.time()),
            session: self.rng.one_in(4).then(|| self.link()),
            refs: Vec::new(),
            title_generated: self.rng.one_in(4),
            started_at: self.rng.coin().then(|| self.time()),
            finished_at: self.rng.one_in(4).then(|| self.time()),
            work: (0..self.rng.below(3)).map(|_| self.work()).collect(),
            colour: self.rng.coin().then(|| self.rng.pick(&[0, 3, 7, 8, 255])),
            extra: if self.rng.one_in(4) {
                [("tabs".to_string(), serde_json::json!([1]))].into()
            } else {
                Default::default()
            },
            materials: self.names(
                &[
                    "1-k3f9a2c41b7d/1.json",
                    "2-k3f9a2c41b7d/1.json",
                    "3-k3f9a2c41b7d/2.json",
                    "1-w0rdw0rdw0rd/2.json",
                    "1/1.json",
                    "../1.json",
                    "/etc/passwd",
                    "1-k3f9a2c41b7d/x.json",
                ],
                3,
            ),
        }
    }

    /// What a session did, for a task's session half the time.
    fn work(&mut self) -> progress::Work {
        let (runner, session) = self.session();
        let tokens = |g: &mut Gen| progress::Tokens {
            model: g.name(&["claude-opus-5-5", "claude-haiku-5", "gpt-6-astra", "not a model"]),
            sub: g.rng.coin(),
            input: g.rng.pick(&[0, 1, 40_000, u64::MAX]),
            output: g.rng.pick(&[0, 7, u64::MAX]),
            cached: g.rng.pick(&[0, 30_000]),
        };
        let mut clock = progress::Clock::default();
        for _ in 0..self.rng.below(4) {
            let (at, day) = (self.time(), self.time());
            clock.tick(at, day);
        }
        progress::Work {
            runner,
            session,
            clock,
            tokens: (0..self.rng.below(4)).map(|_| tokens(self)).collect(),
            context: self.rng.coin().then(|| self.time()),
            window: self.rng.coin().then(|| self.time()),
        }
    }

    fn found(&mut self) -> Found {
        Found {
            runner: self.runner(),
            id: self.name(SESSIONS),
            cwd: self.repo_path(),
            repo: self.repo_path(),
            started: self.rng.coin().then(|| self.time()),
            last_activity: self.rng.coin().then(|| self.time()),
            active: self.rng.coin(),
            station: self.rng.coin().then(|| self.name(&["shell", "files"])),
            label: self.rng.coin().then(|| self.name(&["cargo test"])),
            title: self.rng.coin().then(|| self.name(&["Fix the form", "#tag >repo @agent p0\nnext line\u{202e}"])),
            model: self.rng.coin().then(|| self.name(&["claude-opus-4-7", "gpt-6", "not a model"])),
            held: self.rng.coin(),
        }
    }

    /// A session by (runner, id): a listed one half the time.
    fn session(&mut self) -> (String, String) {
        let real: Vec<(String, String)> = self.m.found.iter().map(|s| (s.runner.clone(), s.id.clone())).collect();
        self.one_of(&real, |g| (g.runner(), g.name(SESSIONS)))
    }

    /// Controls a frame drew. A few; one time in 40, hundreds: three-letter labels.
    fn targets(&mut self) -> Vec<Target> {
        let n = if self.rng.one_in(40) { 400 } else { self.rng.below(7) };
        let place = |g: &mut Self| g.rng.pick(&[0., 20., 30., 150., -4., f64::NAN, f64::INFINITY]);
        let target = |g: &mut Self, i: usize| Target {
            frame: Frame { x: place(g), y: place(g), w: place(g), h: place(g) },
            source: match g.rng.below(8) {
                0 => Source::App { pane: g.pane(), path: vec![0, i], role: "AXButton".into(), at: Frame::default() },
                1 | 2 => Source::Own { id: format!("c{i}") },
                _ => Source::Own { id: g.name(CONTROLS) },
            },
        };
        (0..n).map(|i| target(self, i)).collect()
    }

    fn extra(&mut self) -> Extra {
        let value = |rng: &mut Rng| match rng.below(3) {
            0 => serde_json::json!(1),
            1 => serde_json::json!("v"),
            _ => serde_json::json!({"a": [1, null], "b": {"c": true}}),
        };
        let keys = self.rng.some(2, |rng| rng.pick(&["future", "x", "z"]).to_string());
        keys.into_iter().map(|key| (key, value(self.rng))).collect()
    }

    fn record(&mut self, id: u32) -> PaneRecord {
        let kind = match self.rng.below(4) {
            0 => RecordKind::Empty,
            1 | 2 => RecordKind::Terminal,
            _ => RecordKind::App {
                bundle: self.rng.maybe(|rng| rng.pick(&["com.slack", "com.safari"]).to_string()),
                name: self.name(APPS),
                title: self.name(&["general", "w1"]),
                extra: self.extra(),
            },
        };
        PaneRecord { id, kind, cwd: self.rng.coin().then(|| self.name(DIRS)), extra: self.extra() }
    }

    /// A tree over `leaves`, with ratios a hand-written file may hold. Not NaN: JSON has none.
    fn tree(&mut self, leaves: &[u32]) -> Node {
        if leaves.len() == 1 {
            return Node::Leaf(leaves[0]);
        }
        let (a, b) = leaves.split_at(1 + self.rng.below(leaves.len() - 1));
        let ratio = self.rng.pick(&[0.5, 0.5, 0.3, 0.05, 2.0, -1.0, 0.0]);
        Node::Split { axis: self.axis(), ratio, a: Box::new(self.tree(a)), b: Box::new(self.tree(b)) }
    }

    /// A mark as a file may hold it: mostly sound, sometimes a layout that names what it does not list.
    fn kept_mark(&mut self) -> Mark {
        let target = if self.rng.coin() {
            let id = self.rng.below(3) as u32;
            marks::Target::Pane(self.record(id))
        } else {
            let listed: Vec<u32> = (1..=self.rng.below(5) as u32).collect();
            let shown = if self.rng.one_in(4) { self.rng.some(3, |rng| rng.below(4) as u32) } else { listed.clone() };
            marks::Target::Tab(TabRecord {
                title: self.name(&["Tab 1", "app"]),
                task: self.rng.one_in(3).then(|| self.task()),
                focused: self.rng.one_in(3).then(|| self.rng.below(5) as u32),
                layout: (!shown.is_empty()).then(|| self.tree(&shown)),
                panes: listed.into_iter().map(|id| self.record(id)).collect(),
                extra: self.extra(),
            })
        };
        let key = self.rng.maybe(|rng| rng.pick(&['a', 'b', 'Z', '1', 'é']));
        Mark { id: 0, uid: self.name(UIDS), name: self.name(WORDS), key, target, live: None, extra: self.extra() }
    }

    fn mark_ref(&mut self) -> MarkRef {
        let list = &self.m.marks.list;
        let ids: Vec<MarkId> = list.iter().map(|mark| mark.id).collect();
        let names: Vec<String> = list.iter().map(|mark| mark.name.clone()).collect();
        let keys: Vec<char> = list.iter().filter_map(|mark| mark.key).collect();
        match self.rng.below(4) {
            0 => MarkRef::Id(self.one_of(&ids, Self::stray)),
            1 => MarkRef::Name(self.one_of(&names, |g| g.name(WORDS))),
            2 => MarkRef::Key(self.one_of(&keys, |g| g.rng.pick(&['a', 'b', 'c', 'Z', '1', 'é']))),
            _ => MarkRef::Place(self.index(list.len())),
        }
    }

    fn mark_of(&mut self) -> MarkOf {
        if self.rng.coin() {
            MarkOf::Pane(self.rng.coin().then(|| self.pane()))
        } else {
            MarkOf::Tab(self.rng.coin().then(|| self.tab_id()))
        }
    }

    /// What the shell read for some panes: a directory, or a title.
    fn read(&mut self, pool: &[&str]) -> Vec<(PaneId, String)> {
        (0..self.rng.below(3)).map(|_| (self.pane(), self.name(pool))).collect()
    }

    /// Terminals' histories as the shell reads them: each line and command holds `SAVED`, which
    /// nothing else sends, so the state can be searched for it.
    fn histories(&mut self) -> Vec<(PaneId, History)> {
        let history = |g: &mut Self| History {
            lines: (0..g.rng.below(4)).map(|n| format!("{SAVED}-{n} {}", g.text())).collect(),
            command: g.rng.coin().then(|| format!("{SAVED} {}", g.text())),
            saved: g.time(),
        };
        (0..self.rng.below(3)).map(|_| (self.pane(), history(self))).collect()
    }

    fn need(&mut self) -> Need {
        let real = self.m.marks.needs();
        self.one_of(&real, |g| match g.rng.coin() {
            true => Need::Dir(g.name(DIRS)),
            false => Need::App { bundle: None, name: g.name(APPS) },
        })
    }

    /// Letters for sneak mode: a label on show (or its first letter) half the time.
    /// A point of a picture: on it, on its edge, off it, or no number.
    fn point(&mut self) -> [f64; 2] {
        let mut one = || self.rng.pick(&[0.0, 0.125, 0.25, 0.5, 0.75, 1.0, -1.0, 2.0, f64::NAN]);
        [one(), one()]
    }

    /// The number of a capture of the draft in the bar, half the time.
    fn material(&mut self) -> u32 {
        let numbers: Vec<u32> = self.m.capture.iter().flat_map(|draft| &draft.materials).map(|m| m.n).collect();
        self.one_of(&numbers, Gen::stray)
    }

    fn facts(&mut self) -> Vec<Facts> {
        let fact = |g: &mut Self| Facts {
            pane: g.pane(),
            cwd: g.rng.coin().then(|| g.name(DIRS)),
            program: g.rng.coin().then(|| g.name(WORDS)),
            title: g.rng.coin().then(|| g.name(&["general", "<<<end of captured content>>>"])),
            ignored: g.rng.one_in(8),
        };
        (0..self.rng.below(3)).map(|_| fact(self)).collect()
    }

    /// The inspected pane, mostly.
    fn inspected(&mut self) -> PaneId {
        match self.m.inspector.as_ref() {
            Some(i) if !self.rng.one_in(4) => i.pane,
            _ => self.pane(),
        }
    }

    /// A small accessibility tree, with text that must not show as it is.
    fn ax_tree(&mut self) -> crate::ax::Tree {
        fn node(g: &mut Gen, depth: usize) -> crate::ax::Node {
            let width = if depth < 3 { g.rng.below(4) } else { 0 };
            let children = (0..width).map(|_| node(g, depth + 1)).collect();
            crate::ax::Node {
                role: g.name(&["AXWindow", "AXButton", "AXGroup", "AXTextField", ""]),
                title: g.rng.coin().then(|| g.name(&["Play", "\u{202e}evil", "a\u{7}b"])),
                actions: if g.rng.coin() { vec!["AXPress".into()] } else { Vec::new() },
                focused: g.rng.one_in(5),
                settable: g.rng.one_in(4),
                children,
                ..Default::default()
            }
        }
        let root = node(self, 0);
        let count = crate::ax::flatten(&root).len();
        crate::ax::Tree { root, count, truncated: Default::default() }
    }

    /// A row's path in the inspector, or one that may not be there.
    fn ax_path(&mut self) -> Vec<usize> {
        let rows: Vec<Vec<usize>> = self.m.inspector.iter().flat_map(|i| i.rows()).map(|row| row.path).collect();
        self.one_of(&rows, |g| (0..g.rng.below(3)).map(|_| g.rng.below(3)).collect())
    }

    /// A hosted window's tree as sneak mode reads it: buttons, some nested, disabled, tiny or
    /// off the pane, and text fields.
    fn app_read(&mut self) -> crate::sneak_apps::AppRead {
        fn node(g: &mut Gen, depth: usize) -> crate::ax::Node {
            let place = |g: &mut Gen| g.rng.pick(&[0., 3., 20., 30., 150., 900., -4., f64::NAN]);
            let role = g.rng.pick(&["AXButton", "AXRow", "AXTextField", "AXGroup", "AXCheckBox"]);
            let n = if depth < 3 { g.rng.below(4) } else { 0 };
            crate::ax::Node {
                role: role.into(),
                actions: if g.rng.coin() { vec!["AXPress".into()] } else { vec![] },
                enabled: g.rng.pick(&[None, Some(true), Some(false)]),
                frame: (!g.rng.one_in(8)).then(|| Frame { x: place(g), y: place(g), w: place(g), h: place(g) }),
                children: (0..n).map(|_| node(g, depth + 1)).collect(),
                ..Default::default()
            }
        }
        let root = node(self, 0);
        let crop = self.rng.pick(&[0., 28.]);
        let window = Frame { x: 0., y: 0., w: 400., h: 300. };
        crate::sneak_apps::AppRead { window, crop, content: Frame { x: 50., y: 30., w: 300., h: 200. }, root }
    }

    fn letters(&mut self) -> String {
        // After a first letter, often the rest of a label it starts.
        let typed = self.m.sneak.as_ref().map_or(String::new(), |s| s.typed.clone());
        let rests: Vec<String> = self
            .m
            .sneak
            .iter()
            .flat_map(|s| s.targets.iter())
            .filter_map(|t| t.label.strip_prefix(typed.as_str()).filter(|_| !typed.is_empty()).map(str::to_string))
            .collect();
        if !rests.is_empty() && self.rng.coin() {
            return self.one_of(&rests, |g| g.name(&["a"]));
        }
        let labels: Vec<String> = self.m.sneak.iter().flat_map(|s| s.targets.iter().map(|t| t.label.clone())).collect();
        let label = self.one_of(&labels, |g| g.name(&["a", "s", "x", "1", "ö", "AS"]));
        if self.rng.one_in(3) { label.chars().take(1).collect() } else { label }
    }
}

type Make = fn(&mut Gen) -> Action;

/// One maker for each action. `the_makers_make_every_action` fails when a variant has none.
const MAKERS: &[Make] = &[
    |g| Action::Split { axis: g.axis(), kind: g.kind() },
    |g| Action::Open { kind: g.kind(), axis: g.axis(), beside: g.rng.coin().then(|| g.pane()) },
    |g| Action::Adopt { kind: g.kind(), beside: g.rng.coin().then(|| g.pane()) },
    |g| Action::Close { pane: g.rng.coin().then(|| g.pane()) },
    |g| Action::Focus { pane: g.pane() },
    |g| Action::FocusDir { dir: g.dir() },
    |g| Action::FocusCycle { delta: g.delta() },
    |g| Action::Resize { dir: g.dir() },
    |g| Action::SwapDir { dir: g.dir() },
    |g| Action::MovePane { pane: g.rng.coin().then(|| g.pane()), tab: g.index(g.m.tabs.len()) },
    |_| Action::Equalize,
    |g| {
        let zone = g.rng.pick(&[Zone::Left, Zone::Right, Zone::Top, Zone::Bottom, Zone::Center]);
        Action::DropPane { source: g.pane(), target: g.pane(), zone }
    },
    |g| {
        // As the shell measures: a window some points off its pane. Else any sizes.
        let content = g.size();
        let (outer, window) = match g.rng.coin() {
            true => ([content[0] + 6., content[1] + 38.], [content[0] + g.rng.pick(&[-160., 0., 120.]), content[1]]),
            false => (g.size(), g.size()),
        };
        Action::PaneWants { pane: g.pane(), content, outer, window }
    },
    |g| Action::Fit { pane: g.rng.coin().then(|| g.pane()) },
    |g| Action::Enter { pane: g.pane() },
    |_| Action::Leave,
    |g| {
        let section = g.rng.pick(&[Section::Apps, Section::Workbook, Section::Tasks, Section::Repos, Section::Marks]);
        Action::ToggleSection { section }
    },
    |_| Action::CloseSidebar,
    |g| {
        let item = |g: &mut Gen| match g.rng.coin() {
            true => PickItem::Terminal,
            false if g.rng.one_in(4) => PickItem::Closed { name: g.name(APPS), path: g.name(DIRS) },
            false => PickItem::App { name: g.name(APPS) },
        };
        let items = (0..g.rng.below(5)).map(|_| item(g)).collect();
        Action::OpenPicker { axis: g.axis(), new_tab: g.rng.coin(), items }
    },
    |_| Action::NewTab,
    |_| Action::NewEmptyTab,
    |g| Action::SelectTab { index: g.index(g.m.tabs.len()) },
    |g| Action::CycleTab { delta: g.delta() },
    |g| Action::CloseTab { index: g.rng.coin().then(|| g.index(g.m.tabs.len())) },
    |g| Action::PickerQuery { query: g.name(&["s", "S", "sl", "term", "Slack"]) },
    |g| Action::PickerMove { delta: g.delta() },
    |g| {
        // Edits of every kind, from the field's own test: offsets inside a character and past
        // the end too. One time in 4 a line with tags, for `ComposeSubmit` to read.
        let len = g.m.open_field().map_or(0, |(_, field)| field.text().len());
        let edit = crate::edit::fuzz::Rng(g.rng.next() | 1).edit(len);
        Action::FieldEdit { edit: if g.rng.one_in(4) { Edit::Insert(g.line()) } else { edit } }
    },
    |g| {
        let app = |g: &mut Gen| {
            let name = g.name(APPS);
            Installed { path: format!("/Applications/{name}.app"), name }
        };
        let apps: Vec<Installed> = (0..g.rng.below(5)).map(|_| app(g)).collect();
        let running = apps.iter().filter(|_| g.rng.one_in(3)).map(|app| app.path.clone()).collect();
        Action::SetInstalled { apps, running }
    },
    |g| Action::PickerChoose { index: g.rng.coin().then(|| g.index(3)) },
    |_| Action::ClosePicker,
    |_| Action::PaletteOpen,
    |_| Action::PaletteTasks,
    |g| Action::PaletteMove { delta: g.delta() },
    |g| Action::PaletteChoose { index: g.rng.coin().then(|| g.index(50)) },
    |_| Action::ClosePalette,
    |g| Action::InspectorOpen { pane: g.pane(), app: g.name(&["Spotify", "\u{202e}evil", "a\u{7}b", ""]) },
    |g| Action::InspectorRead {
        pane: g.inspected(),
        tree: g.ax_tree(),
        window: Frame { x: 0., y: 0., w: 800., h: 600. },
        millis: 3.0,
    },
    |g| Action::InspectorFailed { pane: g.inspected(), why: g.name(&["the window is gone", "\u{7}"]) },
    |_| Action::InspectorClose,
    |g| Action::InspectorFold { path: g.ax_path() },
    |g| Action::InspectorSelect { path: g.rng.coin().then(|| g.ax_path()) },
    |g| Action::InspectorMove { delta: g.delta() },
    |g| Action::InspectorFilter { on: g.rng.coin() },
    |_| Action::ToggleKeys,
    |_| Action::CloseKeys,
    |_| Action::ToggleVisual,
    |_| Action::LeaveVisual,
    |_| Action::ToggleAxOverlay,
    |g| Action::SetRepos { repos: g.repos() },
    |g| Action::SetRepoDirs { dirs: g.repo_dirs() },
    |g| Action::RepoAdd { dir: g.name(DIRS), found: g.repos() },
    |g| Action::RepoRemove { path: g.name(DIRS) },
    |g| Action::OpenRepo { path: g.repo_path(), axis: g.axis() },
    |g| {
        let tasks = (0..g.rng.below(4)).map(|_| g.kept_task()).collect();
        Action::RestoreTasks { tasks, floor: g.rng.pick(&[0, 5, tasks::MAX_ID, u32::MAX]) }
    },
    |g| Action::TaskAdd {
        title: g.name(WORDS),
        priority: g.rng.pick(&[0, 1, 3, 9, 200]),
        state: g.state(),
        tab: g.rng.coin().then(|| g.tab_id()),
        assign: g.assign(),
    },
    |g| {
        let line = g.m.compose_line().cloned().filter(|_| g.rng.coin()).unwrap_or_else(|| g.line());
        let known = g.known(&line);
        // For the path at the cursor, or (an answer that is late) for another.
        let typing =
            g.m.compose_typing().filter(|_| g.rng.coin()).or_else(|| g.rng.one_in(4).then(|| "/src/".to_string()));
        Action::SetKnown {
            line,
            typing,
            names: g.rng.coin().then_some(known.names),
            paths: known.paths,
            entries: known.entries,
        }
    },
    |g| Action::TaskSetState { id: g.task(), state: g.state() },
    |g| Action::TaskSnooze { id: g.task(), until: g.time() },
    |g| Action::TaskHold { id: g.task() },
    |g| Action::TaskDismiss { id: g.task() },
    |_| Action::TaskUndo,
    |g| Action::RowSnooze { id: g.task(), until: g.time() },
    |g| Action::RowHold { id: g.task() },
    |g| Action::RowDismiss { id: g.task() },
    |g| Action::TaskGo { id: g.rng.coin().then(|| g.task()) },
    |g| Action::NewTabOf { task: g.rng.coin().then(|| g.task()) },
    |g| {
        let at = if g.rng.coin() { TabAt::End(g.rng.coin().then(|| g.task())) } else { TabAt::Before(g.tab_id()) };
        Action::TabPlace { tab: g.rng.coin().then(|| g.tab_id()), at }
    },
    |g| Action::SelectVisibleTab { index: g.index(g.m.tabs.len()) },
    |g| Action::Tick { now: g.time() },
    |g| Action::OpenCompose { text: g.line() },
    |_| Action::ComposeComplete,
    |g| Action::ComposeSubmit { open: g.rng.coin() },
    |_| Action::CloseCompose,
    |_| Action::ComposeEscape,
    |g| Action::ComposeMove { delta: g.delta() },
    |g| Action::ComposeAccept { index: g.index(4) },
    |g| {
        // Paths of every kind: a space, a narrow no-break space, a backslash, a line break,
        // one too long, one that is no path.
        const PATHS: &[&str] = &[
            "/code/app",
            "/Users/me/My Notes/a b.md",
            "/tmp/Shot 9.41\u{202f}AM.png",
            "/a\\b",
            "/a\nb",
            "relative",
            "/",
        ];
        let mut paths: Vec<String> = (0..g.rng.below(3)).map(|_| g.name(PATHS)).collect();
        if g.rng.one_in(8) {
            paths.push(format!("/{}", "x".repeat(tasks::MAX_TAG + 1)));
        }
        Action::ComposeAttach { paths, home: g.rng.coin().then(|| "/Users/me".to_string()), unreadable: g.rng.below(2) }
    },
    |g| Action::ComposeRef { target: if g.rng.coin() { Pointed::Pane(g.pane()) } else { Pointed::Tab(g.tab_id()) } },
    |g| {
        let mark = g.rng.pick(&['>', '@', '#']);
        let name = g.rng.coin().then(|| g.name(&["app", "claude", "codex-gpt-6-astra", "ana", "--yolo"]));
        Action::ComposePick { mark, name }
    },
    |g| {
        let info = |g: &mut Gen| TerminalInfo {
            pane: g.pane(),
            cwd: g.rng.coin().then(|| g.name(DIRS)),
            program: g.rng.coin().then(|| g.name(&["vim", "cargo", ""])),
        };
        let terminals = (0..g.rng.below(3)).map(|_| info(g)).collect();
        Action::SetTerminals { terminals, home: g.rng.coin().then(|| "/code".to_string()) }
    },
    |g| {
        let line = g.line();
        Action::TaskFromLine { known: g.known(&line), line, open: g.rng.coin() }
    },
    |g| {
        use Heading::*;
        Action::ToggleGroup { heading: g.rng.pick(&[NeedsYou, Running, Later, Done, Sessions]) }
    },
    |g| Action::TasksView { view: g.rng.pick(&[TasksView::State, TasksView::Project]) },
    |g| {
        let named: Vec<Option<String>> = g.m.tasks.list.iter().map(|t| t.project.clone()).collect();
        let project =
            if named.is_empty() || g.rng.one_in(4) { g.rng.coin().then(|| g.name(REPOS)) } else { g.rng.pick(&named) };
        Action::ToggleProject { project, done: g.rng.coin() }
    },
    |g| Action::SessionWork { work: g.work() },
    |g| Action::SetSessions { found: (0..g.rng.below(5)).map(|_| g.found()).collect(), day: g.time() },
    |g| Action::SessionQuiet { ms: g.rng.pick(&[0, 1, sessions::QUIET_MS, u64::MAX]) },
    |g| {
        let seen = |g: &mut Gen| {
            let (runner, id) = g.session();
            Seen { runner, id, last_activity: g.time(), active: g.rng.coin() }
        };
        Action::SessionsSeen { seen: (0..g.rng.below(4)).map(|_| seen(g)).collect() }
    },
    |g| {
        let (runner, id) = g.session();
        Action::SessionAdopt { runner, id }
    },
    |g| {
        let (runner, id) = g.session();
        Action::SessionHide { runner, id }
    },
    |g| {
        let (runner, id) = g.session();
        Action::SessionClick { runner, id }
    },
    |g| Action::SessionsRange { range: g.rng.pick(&sessions::Range::EVERY) },
    |g| Action::SetApps { names: g.names(APPS, 4) },
    |_| Action::FocusList,
    |_| Action::LeaveList,
    |g| Action::ListMove { delta: g.delta() },
    |g| Action::ListSection { delta: g.delta() },
    |g| Action::ListChoose { axis: g.axis() },
    |g| Action::ListSnooze { until: g.time() },
    |_| Action::ListHold,
    |_| Action::ListDismiss,
    |g| Action::Sneak { targets: g.targets() },
    |g| Action::SneakEnter { targets: g.targets() },
    |g| {
        // The same controls, less some: what a frame after the labels went up draws.
        let labelled: Vec<Target> = g.m.sneak.iter().flat_map(|s| s.targets.iter().map(|t| t.target.clone())).collect();
        let still: Vec<Target> = labelled.into_iter().filter(|_| !g.rng.one_in(4)).collect();
        Action::SneakDrawn { targets: if g.rng.coin() { still } else { g.targets() } }
    },
    |g| Action::SneakType { text: g.letters() },
    |_| Action::SneakBackspace,
    |_| Action::LeaveSneak,
    |g| {
        let pane = match (g.m.focused, g.rng.coin()) {
            (Some(focused), true) => focused,
            _ => g.pane(),
        };
        Action::SneakApp { pane, read: (!g.rng.one_in(5)).then(|| g.app_read()) }
    },
    |g| Action::AppWheel { pane: g.pane(), now: g.time() },
    |g| {
        let scrolled = g.rng.coin().then(|| g.time());
        Action::PeekCheck { now: g.time(), over: g.rng.coin(), pressed: g.rng.one_in(4), scrolled }
    },
    |g| {
        let one = |g: &mut Gen| Running {
            pane: g.pane(),
            program: g.rng.coin().then(|| g.name(&["cargo"])),
            background: g.rng.coin(),
        };
        Action::QuitAsked { running: (0..g.rng.below(3)).map(|_| one(g)).collect() }
    },
    |_| Action::QuitConfirm,
    |_| Action::QuitCancel,
    |_| Action::Quit,
    // The Apps section: what the settings, the Dock and the scan say, and the user's pins.
    |g| {
        let entry = |g: &mut Gen| {
            let name = g.name(APPS);
            let found =
                g.rng.coin().then(|| Installed { path: format!("/Applications/{name}.app"), name: name.clone() });
            crate::apps::Entry { text: name, app: found }
        };
        let pinned = (0..g.rng.below(3)).map(|_| entry(g)).collect();
        let hidden = (0..g.rng.below(2)).map(|_| entry(g)).collect();
        Action::SetAppSettings { pinned, hidden, suggest: !g.rng.one_in(4) }
    },
    |g| {
        let app = |g: &mut Gen| {
            let name = g.name(APPS);
            Installed { path: format!("/Applications/{name}.app"), name }
        };
        Action::SetDock { apps: (0..g.rng.below(4)).map(|_| app(g)).collect() }
    },
    |g| Action::AppPin { path: g.app_path() },
    |g| Action::AppHide { path: g.app_path() },
    |_| Action::AppsUndo,
    |_| Action::ListPin,
    |g| Action::AppsToggle { group: if g.rng.coin() { crate::apps::Group::More } else { crate::apps::Group::All } },
    |g| Action::AppsFilter { on: g.rng.coin() },
    |g| {
        // As the shell does: only what `marks::parse` lets through.
        let kept: Vec<Mark> = (0..g.rng.below(4)).map(|_| g.kept_mark()).collect();
        let loaded = marks::parse(&marks::write(&kept, &g.extra())).unwrap_or_default();
        Action::RestoreMarks { marks: loaded.marks, extra: loaded.extra }
    },
    |g| Action::Titles { titles: g.read(&["general", "w1"]) },
    |g| {
        let waiting: Vec<PaneId> = g.m.marks.unfilled().iter().map(|empty| empty.pane).collect();
        Action::Unfilled { pane: g.one_of(&waiting, Gen::pane), why: g.name(&["Slack is not running"]) }
    },
    |_| Action::MarkUndo,
    |_| Action::ListUndo,
    |g| Action::MarkAdd {
        of: g.mark_of(),
        name: g.rng.coin().then(|| g.name(WORDS)),
        key: g.rng.maybe(|rng| rng.pick(&['a', 'b', 'c', 'Z', '1', 'é'])),
        uid: g.name(UIDS),
        cwds: g.read(DIRS),
    },
    |g| Action::MarkToggle { of: g.mark_of(), uid: g.name(UIDS), cwds: g.read(DIRS) },
    |g| Action::MarkRemove { mark: g.mark_ref() },
    |g| Action::MarkRename { mark: g.mark_ref(), name: g.name(WORDS) },
    |g| Action::MarkGo { mark: g.mark_ref(), axis: g.axis() },
    |g| {
        let ids: Vec<MarkId> = g.m.marks.list.iter().map(|mark| mark.id).collect();
        Action::MarkBind { mark: g.one_of(&ids, Gen::stray), pane: g.pane() }
    },
    |g| Action::MarksChecked { missing: (0..g.rng.below(3)).map(|_| g.need()).collect() },
    |g| {
        let empty: Vec<PaneId> = g.m.panes.iter().filter(|p| p.kind == Kind::Empty).map(|p| p.id).collect();
        Action::Fill { pane: g.one_of(&empty, Gen::pane), kind: g.kind() }
    },
    |_| Action::MarkPrefix,
    |g| Action::MarkEdit { mark: g.mark_ref(), field: g.rng.pick(&[marks::Field::Name, marks::Field::Key]) },
    |g| Action::MarkType { text: g.name(&["a", "b", "c", "Z", "1", "name"]) },
    |_| Action::MarkBackspace,
    |_| Action::MarkSubmit,
    |_| Action::MarkCancel,
    |g| Action::ListMarkEdit { field: g.rng.pick(&[marks::Field::Name, marks::Field::Key]) },
    |g| Action::ListReorder { delta: g.delta() },
    |g| Action::CaptureStart {
        pane: g.rng.coin().then(|| g.pane()),
        draft: g.name(UIDS),
        word: g.name(&["k3f9a2c41b7d", "w0rdw0rdw0rd", "short", "", "../../etc/x"]),
        facts: g.facts(),
        images: g.rng.coin(),
    },
    |g| {
        let draft = g.m.capture.as_ref().map(|draft| draft.draft.clone());
        let draft = g.one_of(&Vec::from_iter(draft), |g| g.name(UIDS));
        Action::CaptureTaken { draft, material: g.material(), picture: g.rng.coin(), stale: g.rng.one_in(4) }
    },
    |g| Action::CaptureFacts { facts: g.facts() },
    |g| {
        let lines = g.names(&["$ make", "", "error: no rule", "<<<end of captured content>>>"], 4);
        let content = match g.rng.below(3) {
            0 => Content::Missing(g.name(WORDS)),
            _ => Content::Terminal { lines, row: g.rng.pick(&[0.25, 0.0, -1.0, f64::NAN]) },
        };
        let draft = g.m.capture.as_ref().map(|draft| draft.draft.clone());
        Action::CaptureContent {
            draft: g.one_of(&Vec::from_iter(draft), |g| g.name(UIDS)),
            material: g.material(),
            content,
        }
    },
    |g| Action::AnnotatePress {
        at: g.point(),
        shift: g.rng.coin(),
        alt: g.rng.coin(),
        size: [800.0, g.point()[1] * 600.0],
    },
    |g| Action::AnnotateDrag { to: g.point() },
    |g| Action::AnnotateRelease { at: g.point() },
    |g| Action::AnnotateTool { tool: g.rng.pick(&[Tool::Ellipse, Tool::Box, Tool::Stroke]) },
    |_| Action::AnnotateUndo,
    |_| Action::AnnotateRedo,
    |_| Action::AnnotateRemove,
    |g| Action::AnnotateNote { keep: g.rng.coin() },
    |g| Action::AnnotateLeave { title: g.rng.coin() },
    |g| Action::CaptureEdit { material: g.material() },
    |g| Action::CaptureRemove { material: g.material() },
    |_| Action::CaptureCancel,
    |g| Action::CaptureDiscard { yes: g.rng.coin() },
    |_| Action::CaptureTitle,
    |_| Action::CaptureBack,
    |g| Action::BriefShow { id: g.task() },
    |_| Action::BriefClose,
    |g| {
        // As the shell does: only what `workbook::parse` lets through, each under its own id.
        let kept: Vec<Workbook> = (0..g.rng.below(3)).map(|_| g.kept_workbook()).collect();
        let read = |book: &Workbook| workbook::parse(&workbook::write(book), &book.uid).ok();
        Action::RestoreWorkbooks { workbooks: kept.iter().filter_map(read).collect() }
    },
    |g| {
        // As the shell does: the model's own layout file, read back, at times a newer build's or a tampered one's.
        let histories = g.histories();
        let mut layout = g.m.layout_file(&g.read(DIRS), &histories, g.rng.pick(&[0, 3, 500]));
        layout.version = g.rng.pick(&[layout_file::VERSION, layout_file::VERSION, 0, 2]);
        if let Some(saved) = layout.tabs.first_mut().filter(|_| g.rng.one_in(4)) {
            saved.tab.panes.iter_mut().for_each(|pane| pane.cwd = Some(g.name(DIRS)));
            saved.marks = vec![layout_file::MarkLink { id: g.name(UIDS), pane: g.rng.coin().then(|| g.stray()) }];
        }
        layout.selected = g.rng.coin().then(|| g.rng.pick(&[0, 1, 2, 99]));
        layout.drawer.task = g.rng.coin().then(|| g.task());
        let layout = layout_file::parse(&layout_file::write(&layout)).unwrap_or(layout);
        let gone = g.names(DIRS, 2);
        let home = g.rng.coin().then(|| g.name(DIRS));
        Action::RestoreLayout { layout: Box::new(layout), gone, home, launch: g.rng.coin() }
    },
    |_| Action::StartFresh,
    |g| Action::UnfilledOpen { pane: g.pane() },
    |g| Action::WorkbookSave { uid: g.rng.pick(BOOK_UIDS).to_string(), cwds: g.read(DIRS), histories: g.histories() },
    |g| Action::WorkbookAddTab { workbook: g.workbook(), cwds: g.read(DIRS), histories: g.histories() },
    |g| Action::WorkbookSaveAll {
        uid: g.rng.pick(BOOK_UIDS).to_string(),
        cwds: g.read(DIRS),
        histories: g.histories(),
    },
    |g| Action::WorkbookOpen { workbook: g.workbook(), tab: g.book_tab(), home: g.rng.coin().then(|| g.name(DIRS)) },
    |g| Action::WorkbookEdit { workbook: g.workbook(), tab: g.book_tab() },
    |g| Action::WorkbookRemove { workbook: g.workbook(), tab: g.book_tab() },
    |g| Action::WorkbookFold { workbook: g.workbook() },
    |_| Action::WorkbookUndo,
    |g| Action::SetAgents {
        claude: g.names(AGENTS, 2),
        codex: Vec::new(),
        default: g.rng.coin().then(|| g.name(AGENTS)),
        shows: g.rng.pick(&[View::Native, View::Native, View::Terminal]),
    },
    |g| {
        // Mostly the conversation on show, so its cards and turns come and go.
        let keep = !g.rng.one_in(4);
        let shown = g.m.talk.as_ref().map(|talk| talk.task.clone()).filter(|_| keep);
        let task = shown.unwrap_or_else(|| g.subject());
        let events = (0..g.rng.below(4)).map(|_| g.event()).collect();
        Action::AgentEvents { task, events, at: g.rng.pick(&[0, 1, 5_000]) }
    },
    |_| Action::TalkSend,
    |g| Action::TalkAnswer { allow: g.rng.coin(), ticket: g.card() },
    |g| Action::TalkExpand { ticket: g.card() },
    |_| Action::TalkDenyNote,
    |_| Action::TalkEscape,
    |_| Action::TalkInterrupt,
    |g| Action::TalkView { shows: g.rng.pick(&[View::Native, View::Terminal]) },
    |g| Action::TaskTalk { id: g.task() },
    |g| Action::TalkPick { id: g.subject() },
    |g| {
        let (runner, id) = g.session();
        Action::SessionOpen { runner, id }
    },
    |_| Action::ComposeEnter,
    |g| Action::AgentStarted {
        task: g.subject(),
        session: g.rng.coin().then(|| g.session_id()),
        at: g.rng.below(100) as u64,
    },
    |g| {
        let started = |g: &mut Gen| agents::Started {
            runner: g.rng.pick(&[RunnerId::Codex, RunnerId::Claude]),
            id: g.session_id(),
            cwd: g.rng.pick(&["/code/app", "/Users/me", "/elsewhere"]).to_string(),
            started: g.rng.below(200) as u64,
        };
        let found = (0..g.rng.below(3)).map(|_| started(g)).collect();
        Action::AgentSessions { found, home: Some("/Users/me".into()), now: g.rng.below(100_000) as u64 }
    },
    |g| Action::AgentExited { task: g.subject() },
    |g| {
        // A task's session half the time: what a resume waits on.
        let links: Vec<(String, String)> =
            g.m.tasks
                .list
                .iter()
                .filter_map(|t| t.session.as_ref())
                .map(|l| (l.runner.clone(), l.id.clone()))
                .collect();
        let (runner, id) = g.one_of(&links, Gen::session);
        let found = g.rng.coin().then(|| Found { runner: runner.clone(), id: id.clone(), ..g.found() });
        Action::SessionRead { runner, id, found }
    },
    |g| Action::SessionMissing { task: g.task(), id: g.session_id() },
    |g| Action::TaskStop { id: g.subject() },
    |_| Action::TalkToggle,
    |_| Action::TalkGo,
    |g| Action::TalkKeys { on: g.rng.coin() },
    |g| Action::TalkResize { width: g.rng.pick(&[0., 400., 9e9, f32::NAN]) },
    |_| Action::TalkWide,
    |g| Action::SetCalendar { on: g.rng.coin(), notify: g.rng.coin(), links_only: g.rng.coin() },
    |g| {
        let events = g.rng.coin().then(|| (0..g.rng.below(4)).map(|_| g.meeting()).collect());
        let day = g.m.now - g.m.now % 86_400_000;
        Action::CalendarRead { events, days: [0, 1, 2].map(|n: u64| day.saturating_add(n * 86_400_000)), at: g.time() }
    },
    |g| Action::CallSeen { in_call: g.rng.pick(&[None, Some(true), Some(false)]), at: g.time() },
    |g| Action::CalendarDismiss { id: g.meeting_id() },
    |g| Action::CalendarJoin { id: g.meeting_id() },
    |_| Action::CalendarTomorrow,
];

/// The way out of each box and mode. Picked one time in 8: with every action as likely as any
/// other, a run would sit behind the key reference for 40 steps.
const WAYS_OUT: &[Action] = &[
    Action::ClosePicker,
    Action::ClosePalette,
    Action::InspectorFilter { on: false },
    Action::CloseKeys,
    Action::LeaveVisual,
    Action::CloseCompose,
    Action::LeaveSneak,
    Action::QuitCancel,
    Action::Leave,
    Action::LeaveList,
    Action::MarkCancel,
    Action::AnnotateLeave { title: false },
];

fn random(rng: &mut Rng, m: &Model) -> Envelope {
    let action = match rng.below(8) {
        0 => rng.pick(WAYS_OUT),
        // Straight after a mark is removed: put it back, so `Run::step` can compare.
        1 if m.marks.removed().is_some() => Action::MarkUndo,
        _ => rng.pick(MAKERS)(&mut Gen { rng, m }),
    };
    action.sent_by(rng.pick(&[Origin::User, Origin::User, Origin::Socket, Origin::System]))
}

macro_rules! ensure {
    ($holds:expr, $($why:tt)+) => {
        if !$holds {
            return Err(format!($($why)+));
        }
    };
}

/// The moves that may change the tab on show: selecting, cycling or closing a tab, a new tab,
/// going to a task's tabs (`TaskGo`, `TalkGo`), the user's drag of a pane or a click into one,
/// and what goes to a mark, a workbook, a picked app or a capture's pane.
fn selects_tab(action: &Action, origin: Origin) -> bool {
    let _ = origin;
    matches!(
        action,
        Action::SelectTab { .. }
            | Action::SelectVisibleTab { .. }
            | Action::CycleTab { .. }
            | Action::CloseTab { .. }
            | Action::NewTab
            | Action::NewEmptyTab
            | Action::NewTabOf { .. }
            | Action::TaskGo { .. }
            | Action::TalkGo
            | Action::DropPane { .. }
            | Action::MovePane { .. }
            | Action::Focus { .. }
            | Action::Enter { .. }
            | Action::Open { .. }
            | Action::PickerChoose { .. }
            | Action::MarkGo { .. }
            | Action::WorkbookOpen { .. }
            | Action::ListChoose { .. }
            | Action::MarkType { .. }
            | Action::MarkSubmit
            | Action::CaptureBack
            | Action::RestoreLayout { .. }
    )
}

fn unique<T: PartialEq>(list: &[T]) -> bool {
    list.iter().enumerate().all(|(i, item)| !list[..i].contains(item))
}

/// What must hold of a model at rest, whatever was done to it.
/// A capture's drawings, a note still being typed among them as the note it becomes: when
/// annotate mode ends (its pane is gone or live), the note is kept, and that is no change.
fn drawn(material: &crate::capture::Material) -> Vec<crate::capture::Shape> {
    let marks = &material.marks;
    let typed = marks.note.as_ref().map(|(at, field)| (*at, field.text().trim().to_string()));
    let typed = typed.filter(|(_, text)| !text.is_empty()).map(|(at, text)| crate::capture::Shape::Note { at, text });
    marks.shapes.iter().cloned().chain(typed).collect()
}

fn check(m: &Model) -> Result<(), String> {
    // The Calendar is open, or has the rail's keyboard, only while a feed is set.
    let calendar_shown =
        m.sidebar == Some(Section::Calendar) || m.rail_focus == Some(zones::RailFocus::Left(Section::Calendar));
    ensure!(!calendar_shown || m.calendar.on, "the Calendar section with no feed");
    // The meeting banner is of an event that runs, with no call on.
    if let Some(event) = m.calendar.banner(m.now) {
        ensure!(calendar::stage(event, m.now) == calendar::Stage::Running, "a banner for an event that does not run");
        ensure!(m.calendar.in_call != Some(true) || m.calendar.in_other_call(event), "a banner in the meeting's call");
    }
    // Native conversations: live only while their agent runs; a card only while live; a
    // Deny's message only over a card; one transcript each.
    for chat in &m.chats {
        let (subject, t) = (&chat.subject, &chat.transcript);
        ensure!(!t.live || m.agent_runs(subject), "{subject:?} is live with no agent");
        ensure!(t.cards.is_empty() || t.live, "{subject:?} has a card and no agent");
        ensure!(!chat.deny || !t.cards.is_empty(), "{subject:?} writes a Deny with no card");
        // A watched transcript is read-only: no turn to stop.
        ensure!(t.live || t.turn.is_none(), "{subject:?} has a turn and no agent");
        ensure!(!chat.armed || t.turn.is_some(), "{subject:?} waits for a second Esc with no turn");
    }
    let chats: Vec<String> = m.chats.iter().map(|chat| format!("{:?}", chat.subject)).collect();
    ensure!(unique(&chats), "two transcripts for one conversation: {chats:?}");
    // Tabs, their trees and the panes.
    ensure!(!m.tabs.is_empty(), "no tabs");
    ensure!(m.active < m.tabs.len(), "tab {} is on show, of {}", m.active, m.tabs.len());
    let tab_ids: Vec<u32> = m.tabs.iter().map(|tab| tab.id).collect();
    ensure!(unique(&tab_ids), "two tabs share an id: {tab_ids:?}");
    let on_show = &m.tabs[m.active];
    ensure!(on_show.layout.is_none() && on_show.focused.is_none(), "the tab on show keeps a second tree or focus");
    // Each task's tabs side by side, groups in order of their first tab, no task's last; each
    // tab of a task there is, not dismissed.
    let groups = m.groups();
    let tasks_of: Vec<Option<TaskId>> = groups.iter().map(|group| group.task).collect();
    ensure!(unique(&tasks_of), "a task's tabs are apart: {tasks_of:?}");
    ensure!(groups.iter().rev().skip(1).all(|group| group.task.is_some()), "tabs of no task before a task's");
    for tab in &m.tabs {
        let live = tab.task.is_none_or(|id| m.tasks.get(id).is_some_and(|task| !task.dismissed));
        ensure!(live, "tab {} is of task {:?}, gone or dismissed", tab.id, tab.task);
    }
    let mut held: Vec<PaneId> = Vec::new();
    for (i, tab) in m.tabs.iter().enumerate() {
        let (tree, focused) = if i == m.active { (&m.layout, m.focused) } else { (&tab.layout, tab.focused) };
        let leaves = tree.as_ref().map(Node::leaves).unwrap_or_default();
        match focused {
            Some(pane) => ensure!(leaves.contains(&pane), "tab {} focuses pane {pane}, which it does not hold", tab.id),
            None => ensure!(leaves.is_empty(), "tab {} has panes and focuses none", tab.id),
        }
        if let Some(tree) = tree {
            tree.sound_ratios().map_err(|why| format!("tab {}: {why}", tab.id))?;
        }
        ensure!(
            tab.task.is_none_or(|id| m.tasks.get(id).is_some()),
            "tab {} is of task {:?}, which is gone",
            tab.id,
            tab.task
        );
        held.extend(leaves);
    }
    let mut ids: Vec<PaneId> = m.panes.iter().map(|p| p.id).collect();
    ensure!(unique(&ids), "two panes share an id: {ids:?}");
    ensure!(ids.iter().all(|id| (1..=m.next_id).contains(id)), "a pane's id is past the last one given: {ids:?}");
    ids.sort_unstable();
    held.sort_unstable();
    ensure!(ids == held, "panes {ids:?}, but the trees hold {held:?}");
    ensure!(m.tab_panes().concat().len() == ids.len(), "tab_panes does not list every pane once");
    let shown = m.layout.as_ref().map(Node::leaves).unwrap_or_default();
    for pane in &m.panes {
        ensure!(m.tab_of(pane.id).is_some(), "pane {} is in no tab", pane.id);
        if pane.fit.is_some() {
            ensure!(matches!(pane.kind, Kind::App { .. }), "pane {} is no app pane and has a fit", pane.id);
        }
        m.fit_refusal(Some(pane.id));
    }
    for (_, tab) in &m.last_tab {
        ensure!(tab_ids.contains(tab), "tab {tab} is remembered and gone");
    }
    // At most one peek, of a parked app pane on show, with no mode over the panes.
    if let Some(peek) = &m.peek {
        let app = m.pane(peek.pane).is_some_and(|p| matches!(p.kind, Kind::App { .. }));
        ensure!(
            app && shown.contains(&peek.pane) && m.live.is_none(),
            "a peek of pane {}, not parked or shown",
            peek.pane
        );
        let mode = matches!(m.mode(), Mode::Normal | Mode::List | Mode::Talk);
        ensure!(mode && !m.keys && !m.ax_overlay, "a peek under {:?}", m.mode());
    }

    // The drawer is on a task there is; the keyboard is in it only while it is open, its agent
    // runs, and no app, list or visual mode has the keyboard.
    let gone = |subject: &Subject| subject.task().is_some_and(|id| m.tasks.get(id).is_none());
    let dismissed = |subject: &Subject| subject.task().is_some_and(|id| m.tasks.get(id).is_some_and(|t| t.dismissed));
    if let Some(talk) = &m.talk {
        ensure!(!gone(&talk.task), "the drawer is on task {:?}, which is gone", talk.task);
        let free = m.live.is_none() && m.list.is_none() && m.visual.is_none() && m.annotated().is_none();
        ensure!(
            !talk.keys || (talk.open && m.talk_live(&talk.task) && free),
            "the keyboard is in a conversation that is closed, has no agent, or under another: {talk:?}"
        );
    }
    ensure!(m.mode() != Mode::Talk || m.talk.as_ref().is_some_and(|talk| talk.keys), "talk mode with no conversation");
    ensure!(unique(&m.agents), "an agent runs twice for one conversation: {:?}", m.agents);
    ensure!(!m.agents.iter().any(gone), "an agent runs for a task that is gone");
    // A session's conversation is never also a task's: adopting it moved it.
    let adopted = |subject: &Subject| match subject {
        Subject::Session { runner, id } => m.tasks.adopted(runner, id),
        Subject::Task(_) => false,
    };
    ensure!(!m.agents.iter().any(adopted), "an agent runs for a session a task owns: {:?}", m.agents);
    let width = m.talk_width;
    ensure!((talk::MIN_WIDTH..=talk::MAX_WIDTH).contains(&width), "the drawer is {width} wide");

    // Who has the keyboard, and that what has it is there.
    // The new-task line in the right bar is no box; its title or a picture may have the keyboard.
    let boxed = m.picker.is_some() || m.compose_modal() || m.compose_keys() || m.annotated().is_some();
    if let Some(live) = m.live {
        let app = m.pane(live).is_some_and(|p| matches!(p.kind, Kind::App { .. }));
        ensure!(app && shown.contains(&live), "live pane {live} is no app pane on show");
        ensure!(m.focused == Some(live), "pane {live} is live and {:?} is focused", m.focused);
        ensure!(m.mode() == Mode::App && !m.keys && m.visual.is_none(), "an app is live in {:?} mode", m.mode());
        ensure!(m.list.is_none(), "an app is live and the list has a selection");
    }
    if let Some(visual) = m.visual {
        let terminal = m.pane(visual).is_some_and(|p| p.kind == Kind::Terminal);
        ensure!(terminal && shown.contains(&visual), "visual mode is on pane {visual}, no terminal on show");
        ensure!(m.focused == Some(visual), "visual mode is on pane {visual} and {:?} is focused", m.focused);
        ensure!(!boxed, "visual mode is on under a box");
    }
    ensure!(!(m.picker.is_some() && m.compose_modal()), "the picker and the new-task box are both open");
    // The user's ⌘/ closes the picker. The new-task box it leaves open under the reference,
    // with its line kept and its field out of reach (below) until the reference closes.
    ensure!(!(m.keys && m.picker.is_some()), "the key reference is open over the picker");
    if let Some(picker) = &m.picker {
        let matches = picker.matches().len();
        ensure!(picker.selected < matches.max(1), "the picker selects {} of {matches}", picker.selected);
    }
    // The inspector is of a pane that is there; what it selects and folds is in its tree.
    if let Some(inspector) = &m.inspector {
        ensure!(m.pane(inspector.pane).is_some(), "the inspector is of pane {}, which is gone", inspector.pane);
        let rows = inspector.rows();
        let selected = inspector.selected.as_ref();
        ensure!(selected.is_none() || inspector.selected_node().is_some(), "the inspector selects {selected:?}");
        ensure!(!(inspector.filtering && m.live.is_some()), "the inspector's filter has the keyboard over a live app");
        // It covers the drawer: nothing there has the keyboard.
        let covered =
            m.talk.as_ref().is_some_and(|talk| talk.keys) || m.capture.as_ref().is_some_and(|draft| draft.typing);
        ensure!(!covered, "the drawer under the inspector has the keyboard");
        for row in &rows {
            let shown = format!("{}{}", row.role, row.text);
            ensure!(shown.chars().all(|c| !c.is_control() && c != '\u{202e}'), "a row shows {shown:?}");
        }
    }
    // The palette opens over nothing that has the keyboard, and nothing takes it from under it
    // but the quit question.
    if let Some(palette) = &m.palette {
        let matches = palette.count();
        ensure!(palette.selected < matches.max(1), "the palette selects {} of {matches}", palette.selected);
        ensure!(!boxed && !m.keys && m.visual.is_none() && m.live.is_none(), "the palette is open with {:?}", m.mode());
        ensure!(matches!(m.mode(), Mode::Palette | Mode::Confirm), "the palette is open in {:?} mode", m.mode());
    }
    // The text field is the picker's filter, else the new-task box's line; with neither open
    // there is none. It has the keyboard in their modes only, and not under the key reference.
    // A mark's name being typed is a text field too, before either.
    let renaming = m.mark_input.as_ref().and_then(MarkInput::text);
    // Else a note being typed on a capture, unless the draft's title has the keyboard; else the
    // Apps section's filter, while it has the keyboard and no box does.
    let note = m.note().filter(|_| !m.compose_keys());
    let apps = m.app_lists.filtering && !m.compose_modal() && !m.compose_keys();
    let whose = match (m.open_field(), renaming, &m.picker, note, &m.compose) {
        (Some(("mark" | "workbook", field)), Some(name), ..) => std::ptr::eq(field, name),
        (Some(("picker", field)), None, Some(picker), ..) => std::ptr::eq(field, &picker.query),
        (Some(("palette", field)), None, None, ..) => m.palette.as_ref().is_some_and(|p| std::ptr::eq(field, &p.query)),
        (Some(("inspector", field)), None, None, ..) => {
            m.inspector.as_ref().is_some_and(|i| i.filtering && std::ptr::eq(field, &i.filter))
        }
        (Some(("note", field)), None, None, Some(note), _) => std::ptr::eq(field, note),
        (Some(("apps", field)), None, None, None, _) => apps && std::ptr::eq(field, &m.app_lists.filter),
        (Some(("compose", field)), None, None, None, Some(line)) => !apps && std::ptr::eq(field, line),
        (None, None, None, None, None) => !apps,
        _ => false,
    };
    ensure!(whose, "the open text field is {:?}; it is not the open box's", m.open_field().map(|(of, _)| of));
    let has_keys = !m.keys
        && match m.mode() {
            Mode::Mark => renaming.is_some(),
            Mode::Pick | Mode::Palette | Mode::Inspect | Mode::Compose => true,
            Mode::Annotate => m.note().is_some(),
            Mode::List => m.app_lists.filtering,
            // A native conversation's composer, while no card has the keys.
            Mode::Talk => m.talk_typing(),
            _ => false,
        };
    ensure!(m.field().is_some() == has_keys, "a text field has the keyboard in {:?} mode", m.mode());
    // The new-task box: its dropdown, what its line points at, and what it says.
    let menu = m.dropdown();
    ensure!(menu.is_none() || m.field().is_some_and(|(of, _)| of == "compose"), "a dropdown with no line to finish");
    if let Some(menu) = &menu {
        let rows = menu.rows.len();
        ensure!(rows <= complete::MAX_ROWS, "the dropdown has {rows} rows");
        ensure!(menu.selected.is_none_or(|at| at < rows), "the dropdown highlights row {:?} of {rows}", menu.selected);
        ensure!(!menu.rows.is_empty() || menu.note.is_some(), "a dropdown with no row and nothing to say");
        let names: Vec<&String> = menu.rows.iter().map(|row| &row.name).collect();
        ensure!(unique(&names), "a name is on the dropdown twice: {names:?}");
        let line = m.compose_line().map(String::as_str).unwrap_or_default();
        ensure!(line.is_char_boundary(menu.start) && menu.start <= line.len(), "the dropdown hangs off the line");
    }
    let (aliases, targets): (Vec<&String>, Vec<(RefKind, Option<u32>, &Option<String>)>) =
        m.compose_refs.iter().map(|r| (&r.name, (r.kind, r.id, &r.bundle))).unzip();
    ensure!(
        unique(&aliases) && unique(&targets),
        "the line points at one thing twice, or two by one name: {aliases:?}"
    );
    let kept = !m.compose_refs.is_empty() || !m.compose_attached.is_empty() || m.compose_note.is_some();
    ensure!(m.compose.is_some() || !kept, "the box is closed and something of its line is kept");
    let offered: Vec<String> = m.ref_candidates().into_iter().map(|(r, _)| r.name).collect();
    ensure!(unique(&offered), "two things are offered by one name: {offered:?}");
    ensure!(m.compose_host().is_some() == m.compose.is_some(), "the box is open nowhere, or closed somewhere");
    if let Some((of, field)) = m.open_field() {
        let most = match of {
            "compose" => MAX_TITLE,
            "mark" | "workbook" => marks::MAX_NAME,
            "note" => capture::MAX_NOTE,
            _ => MAX_FILTER,
        };
        let text = field.text();
        ensure!(text.chars().count() <= most, "the {of} field holds {} characters", text.chars().count());
        ensure!(!text.chars().any(char::is_control), "the {of} field holds a control character: {text:?}");
        let (cursor, selection) = (field.cursor(), field.selection());
        ensure!(
            text.get(selection.clone()) == Some(field.selected()),
            "the {of} field selects {selection:?} of {text:?}"
        );
        ensure!(
            selection.contains(&cursor) || selection.end == cursor,
            "the {of} field's cursor is outside its selection"
        );
        ensure!(field.byte(field.utf16(cursor)) == cursor, "the {of} field's cursor is inside a character");
        ensure!(
            field.marked().is_none_or(|marked| text.get(marked).is_some()),
            "the {of} field marks no text of its own"
        );
    }
    if let Some(sneak) = &m.sneak {
        let labels: Vec<&str> = sneak.targets.iter().map(|t| t.label.as_str()).collect();
        ensure!(!labels.is_empty(), "sneak mode is on with no label");
        let prefix = |a: &&str, b: &&str| a.starts_with(*b);
        let free =
            labels.iter().enumerate().all(|(i, a)| labels.iter().enumerate().all(|(j, b)| i == j || !prefix(a, b)));
        ensure!(free, "a label starts another: {labels:?}");
        // An app's elements are labelled only in the one pane read, and the state counts them.
        let apps: Vec<_> = sneak.targets.iter().filter(|t| matches!(t.target.source, Source::App { .. })).collect();
        let pane = sneak.app.map(|a| a.pane);
        ensure!(
            apps.iter().all(|t| matches!(t.target.source, Source::App { pane: p, .. } if Some(p) == pane)),
            "a label is on an app's node of a pane not read: {pane:?}"
        );
        // One stray key never presses in another app: its labels have two letters at least.
        let short = apps.iter().find(|t| t.label.chars().count() < sneak_apps::MIN_LETTERS);
        ensure!(short.is_none(), "a one-letter label in an app: {short:?}");
        let counted = sneak.app.is_none_or(|a| a.controls == apps.len() && a.controls <= a.found.min(sneak_apps::CAP));
        ensure!(counted, "the app's count is off: {:?}", sneak.app);
        // Each app label names one element.
        let sources: Vec<String> = apps.iter().map(|t| format!("{:?}", t.target.source)).collect();
        ensure!(unique(&sources), "two labels on one element");
        let covered = boxed || m.keys || m.live.is_some() || m.visual.is_some() || m.mark_input.is_some();
        ensure!(!covered && !m.ax_overlay && m.confirm.is_none(), "sneak mode is on under something else");
        let _ = sneak.visible().count();
    }
    if let Some(confirm) = &m.confirm {
        let loses = match confirm.dismiss {
            Some(id) => m.agent_runs(&Subject::Task(id)),
            None => !confirm.running.is_empty() || m.quit_discards() > 0 || !m.agents.is_empty(),
        };
        ensure!(loses, "the quit question names no terminal, no draft holds a capture, and no agent runs");
        ensure!(
            confirm.running.iter().all(|r| m.pane(r.pane).is_some()),
            "the quit question names a pane that is gone"
        );
        ensure!(m.live.is_none(), "the quit question is up under a live app");
        let _ = (confirm.title(), confirm.lines());
    }
    let rows = m.list_rows();
    if let Some(at) = m.list {
        ensure!(m.sidebar.is_some(), "the list has a selection and the bar is closed");
        ensure!(at < rows.len().max(1), "the list selects row {at} of {}", rows.len());
    }
    match &m.mark_input {
        Some(MarkInput::Rename { mark, text }) => {
            ensure!(m.marks.get(*mark).is_some(), "a name is typed for mark {mark}, which is gone");
            ensure!(
                text.text().chars().count() <= marks::MAX_NAME,
                "a typed mark name has {} characters",
                text.text().chars().count()
            );
        }
        Some(MarkInput::SetKey { mark }) => {
            ensure!(m.marks.get(*mark).is_some(), "mark {mark} waits for a letter and is gone")
        }
        Some(MarkInput::Workbook { workbook, .. }) => {
            ensure!(m.workbooks.get(*workbook).is_some(), "a name is typed for workbook {workbook}, which is gone")
        }
        Some(MarkInput::Jump) | None => {}
    }
    if m.mark_input.is_some() {
        let covered = boxed || m.keys || m.live.is_some() || m.visual.is_some();
        ensure!(
            !covered && m.mode() == Mode::Mark,
            "a mark waits for input in {:?} mode, under something else",
            m.mode()
        );
    }

    // The draft in the right bar, and what it collected.
    ensure!(m.capture.is_none() || m.compose.is_some(), "a draft is in the bar with no title");
    ensure!((m.mode() == Mode::Annotate) <= m.annotated().is_some(), "annotate mode with no picture");
    if let Some(draft) = &m.capture {
        let numbers: Vec<u32> = draft.materials.iter().map(|material| material.n).collect();
        ensure!(unique(&numbers) && !numbers.contains(&0), "capture numbers {numbers:?}");
        ensure!(numbers.len() <= capture::MAX_MATERIALS, "the draft holds {} captures", numbers.len());
        ensure!(!draft.asked || !numbers.is_empty(), "an empty draft asks before it goes");
        ensure!(
            draft.annotating.is_none_or(|n| numbers.contains(&n)),
            "capture {:?} is annotated and gone",
            draft.annotating
        );
        if let Some(material) = m.annotated() {
            let pane = material.pane;
            ensure!(shown.contains(&pane) && m.live != Some(pane), "pane {pane} is annotated: not on show, or live");
        }
        for material in &draft.materials {
            let marks = &material.marks;
            ensure!(
                marks.shapes.len() <= capture::MAX_SHAPES,
                "capture {} holds {} shapes",
                material.n,
                marks.shapes.len()
            );
            ensure!(
                marks.selected.is_none_or(|at| at < marks.shapes.len()),
                "capture {} selects no shape of its own",
                material.n
            );
            let typed = marks.note.is_some() || marks.drawing();
            ensure!(
                !typed || draft.annotating == Some(material.n),
                "capture {} is drawn on and not annotated",
                material.n
            );
            for shape in &marks.shapes {
                let (region, points) = (
                    shape.region(),
                    matches!(shape, capture::Shape::Stroke { points } if points.len() > capture::MAX_POINTS + 1),
                );
                let on = |v: f64| (0.0..=1.0).contains(&v);
                let inside = on(region.x) && on(region.y) && on(region.x + region.w) && on(region.y + region.h);
                ensure!(inside && !points, "capture {} holds a shape off its picture: {shape:?}", material.n);
                ensure!(
                    shape
                        .note()
                        .is_none_or(|text| !text.trim().is_empty() && text.chars().count() <= capture::MAX_NOTE),
                    "a note: {shape:?}"
                );
            }
        }
    }
    ensure!(
        m.back_to.is_none_or(|pane| m.pane(pane).is_some()),
        "the way back leads to pane {:?}, which is gone",
        m.back_to
    );
    // The right rail: at most its cap, each cell a task there is, told apart by its badge; the
    // drawer's conversation is the one cell shown.
    let rail = m.rail();
    let mut badges: Vec<&str> = rail.iter().map(|cell| cell.mono.as_str()).collect();
    badges.sort_unstable();
    badges.dedup();
    ensure!(rail.len() <= talk::RAIL && badges.len() == rail.len(), "the rail: {rail:?}");
    ensure!(
        !rail.iter().any(|cell| gone(&cell.task) || dismissed(&cell.task)),
        "a rail cell for a task that is gone or dismissed"
    );
    let listed = |subject: &Subject| match subject {
        Subject::Task(_) => true,
        Subject::Session { runner, id } => m.session(runner, id).is_some() || m.agent_runs(subject),
    };
    ensure!(rail.iter().all(|cell| listed(&cell.task)), "a rail cell for a session not listed: {rail:?}");
    let open = m.talk.as_ref().filter(|talk| talk.open).map(|talk| &talk.task);
    ensure!(rail.iter().all(|cell| cell.shown == (open == Some(&cell.task))), "the rail shows another cell: {rail:?}");
    let shown =
        |id: TaskId| m.tab_task() == Some(id) || m.talk.as_ref().is_some_and(|talk| talk.task.task() == Some(id));
    let briefed = |id: TaskId| shown(id) && m.tasks.get(id).is_some_and(|task| !task.materials.is_empty());
    ensure!(m.brief.is_none_or(briefed), "the brief of task {:?} shows: not on show, or it has none", m.brief);
    for task in &m.tasks.list {
        let dir = task.materials_dir();
        let own = |file: &String| tasks::material_file(task.id, file).map(|(dir, _)| dir) == dir && dir.is_some();
        ensure!(
            task.materials.iter().all(own) && unique(&task.materials),
            "task {} names captures {:?}",
            task.id,
            task.materials
        );
    }

    // Tasks.
    let task_ids: Vec<TaskId> = m.tasks.list.iter().map(|t| t.id).collect();
    ensure!(unique(&task_ids) && !task_ids.contains(&0), "task ids {task_ids:?}");
    ensure!(m.tasks.list.iter().all(|t| (1..=9).contains(&t.priority)), "a task's priority is out of range");
    ensure!(unique(&m.collapsed), "a heading is collapsed twice");
    match m.undo_offer.map(|offer| offer.moved) {
        Some(Moved::MarkRemoved) => {
            ensure!(m.marks.removed().is_some(), "a removed mark is offered back and there is none")
        }
        Some(moved) if moved.of_apps() => {
            ensure!(m.app_lists.can_undo(), "a pin is offered back and there is none")
        }
        Some(_) => ensure!(m.tasks.can_undo(), "a task move is offered back and there is none"),
        None => {}
    }
    // The Apps section: an app is one bundle, listed once, and never while it runs. The filter
    // has the keyboard only in list mode there, with "All apps" unfolded.
    let bar = m.apps_bar();
    let listed: Vec<&String> = bar.not_running.iter().chain(&bar.all).map(|row| &row.path).collect();
    ensure!(unique(&listed), "an app is listed twice: {listed:?}");
    ensure!(listed.iter().all(|path| !m.installed.1.contains(path)), "a running app is listed as not running");
    ensure!(bar.all_open || bar.all.is_empty(), "apps show under a folded heading");
    let filtering = m.app_lists.filtering;
    ensure!(
        !filtering || (bar.all_open && m.list.is_some() && m.sidebar == Some(Section::Apps)),
        "the Apps filter has the keyboard out of its place: {:?} {:?}",
        m.sidebar,
        m.list
    );

    // Marks.
    let list = &m.marks.list;
    let mark_ids: Vec<MarkId> = list.iter().map(|mark| mark.id).collect();
    ensure!(unique(&mark_ids) && !mark_ids.contains(&0), "mark ids {mark_ids:?}");
    ensure!(m.marks.removed().is_none_or(|mark| !mark_ids.contains(&mark.id)), "the removed mark is also listed");
    let names: Vec<&str> = list.iter().map(|mark| mark.name.as_str()).collect();
    ensure!(unique(&names), "two marks share a name: {names:?}");
    let named = |name: &&str| {
        !name.is_empty() && name.chars().count() <= marks::MAX_NAME && !name.chars().any(char::is_control)
    };
    ensure!(names.iter().all(named), "a mark's name is empty, too long or not one line: {names:?}");
    let keys: Vec<char> = list.iter().filter_map(|mark| mark.key).collect();
    ensure!(unique(&keys) && keys.iter().all(char::is_ascii_lowercase), "mark letters {keys:?}");
    for mark in list {
        let stands_for = |pane: &Pane| match (&mark.target, &pane.kind) {
            (marks::Target::Pane(PaneRecord { kind: RecordKind::Terminal, .. }), Kind::Terminal) => true,
            (
                marks::Target::Pane(PaneRecord { kind: RecordKind::App { bundle, name, .. }, .. }),
                Kind::App { bundle: b, name: n, .. },
            ) => marks::same_app((bundle.as_deref(), name), (b.as_deref(), n)),
            _ => false,
        };
        match (&mark.target, mark.live) {
            (_, None) => {}
            (marks::Target::Tab(_), Some(tab)) => {
                ensure!(tab_ids.contains(&tab), "mark {:?} stands for tab {tab}, which is gone", mark.name)
            }
            (marks::Target::Pane(_), Some(pane)) => {
                ensure!(
                    m.pane(pane).is_some_and(stands_for),
                    "mark {:?} stands for pane {pane}, which is gone or of another kind",
                    mark.name
                )
            }
        }
        ensure!(
            m.mark_pane(mark).is_none_or(|pane| m.pane(pane).is_some()),
            "mark {:?} is open in a pane that is gone",
            mark.name
        );
    }

    // Workbooks: each is one file, named by its id, that reads back as it was written.
    let books = &m.workbooks.list;
    let book_ids: Vec<WorkbookId> = books.iter().map(|book| book.id).collect();
    ensure!(unique(&book_ids) && !book_ids.contains(&0), "workbook ids {book_ids:?}");
    let uids: Vec<&str> = books.iter().map(|book| book.uid.as_str()).collect();
    ensure!(unique(&uids) && uids.iter().all(|uid| workbook::valid_uid(uid)), "workbook file names {uids:?}");
    let names: Vec<&str> = books.iter().map(|book| book.name.as_str()).collect();
    ensure!(unique(&names) && names.iter().all(named), "workbook names {names:?}");
    let lives: Vec<u32> = books.iter().flat_map(|book| book.live.iter().flatten().copied()).collect();
    ensure!(unique(&lives) && lives.iter().all(|tab| tab_ids.contains(tab)), "workbooks stand for tabs {lives:?}");
    for book in books {
        ensure!(book.live.len() <= book.tabs.len(), "workbook {:?} stands for tabs it does not have", book.name);
        let tabs = book.tabs.len();
        ensure!((1..=workbook::MAX_TABS).contains(&tabs), "workbook {:?} has {tabs} tabs", book.name);
        let size = workbook::write(book).len();
        ensure!(size <= workbook::MAX_FILE_BYTES, "workbook {:?} is {size} bytes: too large to read back", book.name);
        let read = workbook::parse(&workbook::write(book), &book.uid);
        let read = read.map_err(|why| format!("workbook {:?} does not read back: {why}", book.name))?;
        ensure!(workbook::write(&read) == workbook::write(book), "workbook {:?} reads back changed", book.name);
    }
    let waiting: Vec<PaneId> = m.marks.unfilled().iter().map(|empty| empty.pane).collect();
    let empty = |id: &PaneId| m.pane(*id).is_some_and(|p| p.kind == Kind::Empty);
    ensure!(
        unique(&waiting) && waiting.iter().all(empty),
        "panes {waiting:?} wait to be filled; not each is one empty pane"
    );
    Ok(())
}

/// The keyboard is on one place there is: a pane on show, a row of the list, a rail's cell.
fn check_zones(m: &Model) -> Result<(), String> {
    use crate::model::zones::{RailFocus, Target};
    let leaves = m.layout.as_ref().map(Node::leaves).unwrap_or_default();
    match m.focus_target() {
        Target::Pane(pane) => ensure!(leaves.contains(&pane), "the keyboard is on pane {pane}, not on show"),
        Target::Right(at) => ensure!(at <= m.rail().len(), "right rail cell {at} of {}", m.rail().len()),
        Target::Nothing => {
            ensure!(leaves.is_empty() || m.mode() != Mode::Normal, "panes on show, and the keyboard on none")
        }
        _ => {}
    }
    if m.rail_focus.is_some() {
        ensure!(m.list.is_none() && m.mode() == Mode::List, "a rail and {:?} have the keyboard", m.mode());
    }
    if let Some(RailFocus::Right(at)) = m.rail_focus {
        ensure!(at <= m.rail().len(), "right rail place {at}");
    }
    Ok(())
}

/// The marks' file: written and read again, every mark is there as it was, and so are the keys
/// this build does not know.
/// Each view of the Tasks section lists every task of the inbox once, in the groups and order
/// it says; a project's bar counts its tasks; a task's file reads back as it was.
fn check_views(m: &Model) -> Result<(), String> {
    let mut inbox: Vec<TaskId> = m.tasks.inbox().iter().map(|(_, t)| t.id).collect();
    inbox.sort_unstable();
    let ids = |rows: &[ListRow]| {
        let mut ids: Vec<TaskId> =
            rows.iter().filter_map(|r| if let ListRow::Task(id) = r { Some(*id) } else { None }).collect();
        ids.sort_unstable();
        ids
    };
    for view in [TasksView::State, TasksView::Project] {
        // As if nothing were folded: every task shows.
        ensure!(ids(&m.task_list_rows(view, true)) == inbox, "the {view:?} view does not list each task once");
    }
    let groups = m.project_groups(0);
    let mut seen: Vec<&Option<String>> = Vec::new();
    for group in &groups {
        ensure!(!seen.contains(&&group.project), "project {:?} is two groups", group.project);
        seen.push(&group.project);
        let bar = group.bar;
        let open = bar.needs_you + bar.running + bar.later;
        ensure!(bar.done == group.done, "the bar of {:?} counts {} done of {}", group.project, bar.done, group.done);
        ensure!(
            group.folded || group.rows.len() == open,
            "{:?} lists {} open of {open}",
            group.project,
            group.rows.len()
        );
        let order: Vec<Heading> = group.rows.iter().map(|r| r.group.into()).collect();
        ensure!(order.is_sorted(), "{:?} is not by state: {order:?}", group.project);
        let shares = bar.shares();
        let sum: f32 = shares.iter().sum();
        ensure!(open + bar.done == 0 || (sum - 1.).abs() < 1e-4, "the bar's shares sum to {sum}");
    }
    ensure!(
        groups.iter().position(|g| g.project.is_none()).is_none_or(|at| at + 1 == groups.len()),
        "no project is not last"
    );
    for task in &m.tasks.list {
        let text = serde_json::to_string(&task.for_file()).map_err(|e| e.to_string())?;
        let back: Task =
            serde_json::from_str(&text).map_err(|e| format!("task {} does not read back: {e}", task.id))?;
        ensure!(back == task.for_file(), "task {} reads back changed", task.id);
        ensure!(task.work.len() <= progress::MAX_SESSIONS, "task {} keeps {} sessions", task.id, task.work.len());
    }
    Ok(())
}

fn check_file(m: &Model) -> Result<(), String> {
    let list = &m.marks.list;
    let loaded = marks::parse(&marks::write(list, &m.marks.extra))
        .map_err(|why| format!("marks.json does not read back: {why}"))?;
    ensure!(loaded.skipped.is_empty(), "a mark does not read back: {:?}", loaded.skipped);
    ensure!(loaded.extra == m.marks.extra, "marks.json loses the file's own keys");
    // As text: a ratio is kept to three places. A mark the shell gave no id yet gets one when read.
    let named = |(kept, read): (&Mark, &Mark)| {
        let uid = if kept.uid.is_empty() { read.uid.clone() } else { kept.uid.clone() };
        Mark { uid, ..kept.clone() }
    };
    let kept: Vec<Mark> = list.iter().zip(&loaded.marks).map(named).collect();
    let (kept, read) = (marks::write(&kept, &m.marks.extra), marks::write(&loaded.marks, &loaded.extra));
    ensure!(kept == read, "the marks read back changed: {kept} then {read}");
    Ok(())
}

/// The layout file: no terminal text with no lines to keep, never an app's title, and it reads
/// back as written. Restored into a fresh model with the same tasks, marks and workbooks, it is
/// the model's layout again: tabs, order, trees, ratios, focus, the scope and tab on show.
fn check_layout(m: &Model) -> Result<(), String> {
    let marked = |pane: &Pane| {
        let lines = vec![format!("{SAVED} line")];
        (pane.id, History { lines, command: Some(format!("echo {SAVED}")), saved: 1 })
    };
    let marked: Vec<(PaneId, History)> = m.panes.iter().map(marked).collect();
    let bare = layout_file::write(&m.layout_file(&[], &marked, 0));
    ensure!(!bare.contains(SAVED), "terminal text is kept with no lines to keep: {bare}");
    let file = m.layout_file(&[], &marked, 500);
    let records = || file.tabs.iter().flat_map(|saved| saved.tab.panes.iter());
    let titled = |pane: &PaneRecord| match &pane.kind {
        RecordKind::App { title, extra, .. } => !title.is_empty() || !extra.is_empty(),
        _ => false,
    };
    ensure!(!records().any(titled), "an app's title is kept in the layout file");
    let read = layout_file::parse(&layout_file::write(&file))
        .map_err(|why| format!("layout.json does not read back: {why:?}"))?;
    ensure!(layout_file::write(&read) == layout_file::write(&file), "layout.json read back changed");
    // Every tab is kept, or the comparison below says nothing.
    if file.tabs.len() != m.tabs.len() {
        return Ok(());
    }
    let mut fresh = Model::default();
    let floor = m.tasks.list.iter().map(|task| task.id).max().unwrap_or(0);
    let (on, notify, links_only) = (m.calendar.on, m.calendar.notify, m.calendar.links_only);
    for action in [
        // The settings are read before the layout, as at a start.
        Action::SetCalendar { on, notify, links_only },
        Action::RestoreTasks { tasks: m.tasks.list.clone(), floor },
        Action::RestoreWorkbooks { workbooks: m.workbooks.list.clone() },
        Action::RestoreMarks { marks: m.marks.list.clone(), extra: m.marks.extra.clone() },
        Action::RestoreLayout { layout: Box::new(read), gone: Vec::new(), home: None, launch: false },
    ] {
        fresh.update(action.sent_by(Origin::System));
    }
    // A task a file would not keep (an id past `tasks::MAX_ID`) is not one the comparison can use.
    let ids = |m: &Model| m.tasks.list.iter().map(|task| task.id).collect::<Vec<_>>();
    if ids(&fresh) != ids(m) {
        return Ok(());
    }
    // As text: a ratio is kept to three places. Each task's tabs together, no task's last.
    let mut was = m.layout_file(&[], &[], 0);
    layout_file::grouped(&mut was);
    let (was, now) = (layout_file::write(&was), layout_file::write(&fresh.layout_file(&[], &[], 0)));
    ensure!(was == now, "restored, the layout is another:\n{was}\nthen\n{now}");
    ensure!(fresh.tab_task() == m.tab_task(), "the tab on show is of another task");
    Ok(())
}

/// What the shell draws and sends: none of it may panic, the state is JSON, and what is kept
/// or read from disk reads back as written.
fn check_state(m: &Model) -> Result<(), String> {
    let state = serde_json::to_value(m).map_err(|why| format!("the state is not JSON: {why}"))?;
    ensure!(
        state.get("tabs").is_some_and(|tabs| tabs.as_array().is_some_and(|tabs| tabs.len() == m.tabs.len())),
        "tabs"
    );
    fn round_trips<T: Serialize + serde::de::DeserializeOwned + PartialEq>(what: &T) -> bool {
        let text = serde_json::to_string(what).unwrap_or_default();
        serde_json::from_str::<T>(&text).is_ok_and(|read| read == *what)
    }
    ensure!(round_trips(&m.tasks.list), "the tasks do not read back as written");
    ensure!(round_trips(&m.repos), "the repositories do not read back as written");
    ensure!(round_trips(&m.found), "the sessions do not read back as written");
    ensure!(m.panes.iter().all(|pane| round_trips(&pane.kind)), "a pane's kind does not read back as written");
    ensure!(round_trips(&(m.sidebar, m.collapsed.clone())), "the bar does not read back as written");
    // Other sessions: each row once and in range; under All every listed one, unless folded.
    let bar = m.sessions_bar();
    let ids: Vec<(String, String)> = bar.rows.iter().map(|row| (row.runner.clone(), row.id.clone())).collect();
    ensure!(unique(&ids), "a session is listed twice: {ids:?}");
    ensure!(bar.count <= bar.total && bar.range == m.sessions_range, "{} of {} in range", bar.count, bar.total);
    ensure!(bar.rows.len() == if bar.collapsed { 0 } else { bar.count }, "{} rows of {}", bar.rows.len(), bar.count);
    ensure!(bar.range != sessions::Range::All || bar.count == bar.total, "All lists {} of {}", bar.count, bar.total);
    let session = |runner: &str, id: &str| m.found.iter().find(|s| (s.runner.as_str(), s.id.as_str()) == (runner, id));
    for row in &bar.rows {
        let shown = session(&row.runner, &row.id).is_some_and(|s| s.in_range(bar.range, m.now, m.session_day));
        ensure!(shown, "{}:{} is listed out of range {:?}", row.runner, row.id, bar.range);
    }
    let _ = (m.task_sections(m.now), m.compose_spans(), m.home(&[]), m.mark_rows());
    let _ = (m.keyboard_terminal(), m.due(m.now), m.followed_sessions(), m.compose_read(), m.needs());
    let _ = (m.workbook_rows(), m.workbook_save_refusal(), m.workbook_of_tab(), m.workbooks.offered());
    let _: Vec<Vec<String>> = m.workbooks.list.iter().map(|book| m.workbook_gone(book.id, None)).collect();
    let _: Vec<Option<String>> = m.workbooks.list.iter().map(|book| m.workbook_add_refusal(book.id)).collect();
    let _ = m.workbook_save_all_refusal();
    // A terminal's saved history is kept, and never shown in the state: the rows count it only.
    let rows = serde_json::to_string(&m.workbook_rows()).unwrap_or_default();
    ensure!(!state.to_string().contains(SAVED) && !rows.contains(SAVED), "a saved history reached the state");
    let _ = (
        m.draft(m.compose_line().map(String::as_str).unwrap_or_default()),
        m.mark_refusal(MarkOf::Pane(None)),
        m.mark_of(MarkOf::Tab(None)),
    );
    let _: Vec<String> = m.tasks.list.iter().map(|task| m.task_tags(task)).collect();
    Ok(())
}

/// What the shell holds after the effects so far: the panes it opened, the window that is
/// live, the terminal with a visual cursor.
#[derive(Default)]
struct Shell {
    panes: Vec<PaneId>,
    live: Option<PaneId>,
    /// The window up for a scroll.
    peek: Option<PaneId>,
    visual: Option<PaneId>,
    /// What it keeps of the draft in the bar: (the draft, the capture's number).
    kept: Vec<(String, u32)>,
}

/// A model, the shell's view of it, and every step taken.
#[derive(Default)]
struct Run {
    m: Model,
    shell: Shell,
    steps: Vec<Envelope>,
    /// The marks before the last step, if all it did was take one off.
    before_removal: Option<Vec<Mark>>,
    /// Each mark's `live`, and whether it was a pane's id when it was set (else a tab's).
    live: Vec<(MarkId, u32, bool)>,
    /// Write the state and the marks' file out after every eighth step only: nine tenths of a
    /// run's time is spent there. A replay, and the long run, do it after every step.
    sparse: bool,
    /// The app elements labelled when nothing was last typed: a press must be of one of them,
    /// so every letter of its label was typed after the label was drawn.
    drawn_before: Vec<String>,
}

/// An app's label as `drawn_before` keeps it: the letters and the element, whatever its frame.
fn app_label(t: &crate::ax::Labelled) -> Option<String> {
    match &t.target.source {
        Source::App { pane, path, role, .. } => Some(format!("{pane} {path:?} {role}")),
        Source::Own { .. } => None,
    }
}

impl Run {
    /// Takes a step, then checks the model, the effects and what the step may have changed.
    fn step(&mut self, sent: Envelope) -> Result<(), String> {
        self.steps.push(sent.clone());
        let Envelope { action, origin } = sent.clone();
        if let Some(sneak) = self.m.sneak.as_ref().filter(|s| s.typed.is_empty()) {
            self.drawn_before = sneak.targets.iter().filter_map(app_label).collect();
        }
        let drawn_before = self.drawn_before.clone();
        let m = &mut self.m;
        let refused = m.refusal(&action, origin).map(|_| format!("{m:?}"));
        let awaiting = m.awaiting.clone();
        let switching = m.switching.clone();
        let chats_before = m.chats.clone();
        let drafts: Vec<(Subject, String)> =
            m.chats.iter().map(|chat| (chat.subject.clone(), chat.draft.text().to_string())).collect();
        let panes: Vec<PaneId> = m.panes.iter().map(|p| p.id).collect();
        let sneak = m.sneak.as_ref().map(|sneak| sneak.typed.clone());
        let labelled: Vec<String> =
            m.sneak.iter().flat_map(|s| s.targets.iter()).map(|t| format!("{:?}", t.target.source)).collect();
        // ottospace's own labels, which an app's tree arriving must not change.
        let own_labels = |m: &Model| -> Vec<(String, String)> {
            let own =
                m.sneak.iter().flat_map(|s| s.targets.iter()).filter(|t| matches!(t.target.source, Source::Own { .. }));
            own.map(|t| (t.label.clone(), format!("{:?}", t.target.source))).collect()
        };
        let own_before = own_labels(m);
        let typed_before = m.sneak.as_ref().map(|s| s.typed.clone());
        let asked: Option<Vec<PaneId>> =
            m.confirm.as_ref().map(|confirm| confirm.running.iter().map(|r| r.pane).collect());
        let (mark_input, marks, removed) = (m.mark_input.clone(), m.marks.list.clone(), m.marks.removed().cloned());
        let rows = m.picker.iter().flat_map(|picker| picker.matches());
        let offered: Vec<String> = m.apps.iter().cloned().chain(rows.map(|row| row.label().to_string())).collect();
        // What a pin, an unpin or a hide may name, and the apps the section's rows start.
        let apps_before = m.apps_bar();
        let app_rows: Vec<String> =
            apps_before.not_running.iter().chain(&apps_before.all).map(|row| row.name.clone()).collect();
        let offered: Vec<String> = offered.into_iter().chain(app_rows).collect();
        let chosen = m.picker.as_ref().and_then(|picker| picker.matches().get(picker.selected).copied().cloned());
        let field = (m.field().is_some(), m.open_field().map(|(_, field)| field.text().to_string()));
        let task_ids: Vec<TaskId> = m.tasks.list.iter().map(|task| task.id).collect();
        // ↵ on a highlighted row that would change the line takes the row: it files nothing.
        let takes = match (&action, m.dropdown().and_then(|menu| menu.selected)) {
            (Action::ComposeSubmit { open: false } | Action::ComposeEnter, Some(row)) => m.changes(row),
            _ => false,
        };
        // While the bar asks before discarding, ↵ is the answer: it files nothing.
        let answers = m.capture.as_ref().is_some_and(|draft| draft.asked);
        let submits = matches!(action, Action::ComposeSubmit { .. });
        let filing = m.compose_line().filter(|_| submits && !takes && !answers && refused.is_none());
        // A draft with captures is filed under a name of its own when none was typed.
        let named = m.capture_title();
        let filing = filing.map(|line| (line.clone(), m.draft(line)));
        // A row taken reads back as the tag it is, of that very name.
        let taken = match (&action, m.dropdown()) {
            (Action::ComposeAccept { index }, Some(menu)) => {
                menu.rows.get(*index).map(|row| (menu.mark, row.name.clone(), *index))
            }
            (Action::ComposeComplete, Some(menu)) => {
                let at = menu.selected.unwrap_or(0);
                menu.rows.get(at).map(|row| (menu.mark, row.name.clone(), at))
            }
            _ => None,
        };
        let taken = taken.filter(|(.., at)| m.taking(*at).is_some());
        // Esc or × on a draft with captures asks first: it never drops them at once.
        let cancels = matches!(action, Action::ComposeEscape | Action::CloseCompose | Action::CaptureCancel);
        let unasked =
            m.capture.as_ref().filter(|draft| cancels && !draft.asked).map(|draft| draft.materials.len()).unwrap_or(0);
        let drafted: Option<(Option<String>, Vec<Vec<crate::capture::Shape>>)> =
            m.capture.as_ref().map(|draft| (m.compose_line().cloned(), draft.materials.iter().map(drawn).collect()));
        let line_before = m.compose_line().cloned();
        let composing = m.composing();
        let palette_typing = m.field().is_some_and(|(of, _)| of == "palette");
        let tab_before = m.tabs[m.active].id;
        let tabs_before: Vec<u32> = m.tabs.iter().map(|tab| tab.id).collect();
        let talks = matches!(
            action,
            Action::TaskTalk { .. }
                | Action::SessionOpen { .. }
                | Action::SessionAdopt { .. }
                | Action::TalkPick { .. }
        );

        let peek_before = m.peek.as_ref().map(|peek| peek.pane);
        let effects = m.update(sent);
        check(m)?;
        // A peek that ends is parked again, made an entry, or its pane closed: never stranded.
        if let Some(pane) = peek_before.filter(|pane| m.peek.as_ref().map(|p| p.pane) != Some(*pane)) {
            let closed = |e: &Effect| matches!(e, Effect::Closed(p) if p.id == pane);
            let back = effects.iter().any(|e| [Effect::Parked(pane), Effect::Live(pane)].contains(e) || closed(e));
            ensure!(back, "the peek of pane {pane} ended with {effects:?}");
        }
        check_zones(m)?;
        if !self.sparse || self.steps.len().is_multiple_of(8) {
            check_state(m)?;
            check_file(m)?;
            check_views(m)?;
        }

        // A draft's title and drawings are the user's: nobody else changes them, and a line
        // someone else wrote is never a draft's title.
        if let (Some(before), Some(draft), false) = (&drafted, &m.capture, origin == Origin::User) {
            let now = (m.compose_line().cloned(), draft.materials.iter().map(drawn).collect());
            ensure!(*before == now, "a draft's title or drawings changed, and not by the user: {before:?}");
        }
        ensure!(m.capture.is_none() || !m.compose_theirs, "a draft's title holds what a client wrote");
        // Only a move to a tab changes the tab on show, or its closing.
        let tab_now = m.tabs[m.active].id;
        ensure!(
            tab_now == tab_before || !m.tabs.iter().any(|tab| tab.id == tab_before) || selects_tab(&action, origin),
            "{origin:?}'s {action:?} changed the tab on show from {tab_before} to {tab_now}"
        );
        // Starting, going on with, adopting or clicking a conversation makes, closes or shows no tab.
        let tabs_now: Vec<u32> = m.tabs.iter().map(|tab| tab.id).collect();
        let mut sorted = (tabs_before.clone(), tabs_now.clone());
        sorted.0.sort_unstable();
        sorted.1.sort_unstable();
        ensure!(!talks || (sorted.0 == sorted.1 && tab_now == tab_before), "{action:?} changed the tabs");
        let held = m.capture.as_ref().map_or(0, |draft| draft.materials.len());
        ensure!(unasked == 0 || held == unasked, "a draft's {unasked} captures went with no question");
        // Only the user starts a capture: a client's or a timer's is refused, whatever the mode.
        let foreign = matches!(action, Action::CaptureStart { .. }) && origin != Origin::User;
        ensure!(!foreign || refused.is_some(), "a capture that is not the user's was not refused");
        // Only the user opens the palette, types or moves in it, or chooses.
        let palette_own = matches!(
            action,
            Action::PaletteOpen
                | Action::PaletteTasks
                | Action::PaletteMove { .. }
                | Action::PaletteChoose { .. }
                | Action::ClosePalette
        ) || (palette_typing && matches!(action, Action::FieldEdit { .. }));
        ensure!(
            origin == Origin::User || !palette_own || refused.is_some(),
            "a client's {action:?} reached the palette"
        );
        // Only the user opens and moves in the inspector; no client fills it with a tree.
        let inspector_own = match &action {
            Action::InspectorRead { .. } | Action::InspectorFailed { .. } => origin == Origin::Socket,
            Action::InspectorOpen { .. }
            | Action::InspectorClose
            | Action::InspectorFold { .. }
            | Action::InspectorSelect { .. }
            | Action::InspectorMove { .. }
            | Action::InspectorFilter { .. } => origin != Origin::User,
            _ => false,
        };
        ensure!(!inspector_own || refused.is_some(), "a client's {action:?} reached the inspector");
        // A meeting's link opens only on the user's own Join, and only one the allow-function passes.
        for effect in &effects {
            if let Effect::OpenLink { url } = effect {
                let joins = matches!(action, Action::CalendarJoin { .. }) && origin == Origin::User;
                ensure!(joins && calendar::joinable(url), "{origin:?}'s {action:?} opened {url:?}");
            }
        }
        // A resume waits on a read only when the user asked for it.
        let waits_more = m.awaiting.iter().any(|subject| !awaiting.contains(subject));
        ensure!(origin == Origin::User || !waits_more, "{origin:?}'s {action:?} asked for a resume");
        // An action refused for who sent it changes nothing at all.
        if let Some(before) = &refused {
            ensure!(
                effects.is_empty() && format!("{m:?}") == *before,
                "a refused action changed the model, or had effects"
            );
        }
        let user = origin == Origin::User;
        // Only the shell's runner reports what an agent said.
        let reports = matches!(action, Action::AgentEvents { .. }) && origin != Origin::System;
        ensure!(!reports || m.chats == chats_before, "{origin:?} reported an agent's events");
        // A composer's draft is the user's words alone: it starts empty, and only the user changes it.
        for chat in m.chats.iter().filter(|_| !user) {
            let before = drafts.iter().find(|(subject, _)| *subject == chat.subject).map(|(_, text)| text.as_str());
            ensure!(before.unwrap_or("") == chat.draft.text(), "{origin:?} changed the draft of {:?}", chat.subject);
        }
        let sneak_own = matches!(
            action,
            Action::Sneak { .. }
                | Action::SneakEnter { .. }
                | Action::SneakDrawn { .. }
                | Action::SneakType { .. }
                | Action::SneakBackspace
                | Action::LeaveSneak
                | Action::SneakApp { .. }
                | Action::AppWheel { .. }
        );
        let mark_own = matches!(
            action,
            Action::MarkPrefix
                | Action::MarkEdit { .. }
                | Action::ListMarkEdit { .. }
                | Action::MarkType { .. }
                | Action::MarkBackspace
                | Action::MarkSubmit
                | Action::MarkCancel
                | Action::AppWheel { .. }
                // These start the typing of a workbook's name.
                | Action::WorkbookSave { .. }
                | Action::WorkbookSaveAll { .. }
                | Action::WorkbookEdit { .. }
        );
        // A name being typed is a text field: an edit of it is the wait's own, whoever sends it.
        let typing = mark_input.as_ref().is_some_and(|input| input.text().is_some());
        let mark_own = mark_own || (typing && matches!(action, Action::FieldEdit { .. }));
        // Only the user, or the mode's own keys, end sneak mode, the quit question or a mark's wait.
        // No letter but the user's is typed toward an app's label.
        if let Some(sneak) = m.sneak.as_ref().filter(|_| !user && matches!(action, Action::SneakType { .. })) {
            let app = sneak.targets.iter().filter(|t| matches!(t.target.source, Source::App { .. }));
            let toward = !sneak.typed.is_empty() && app.clone().any(|t| t.label.starts_with(&sneak.typed));
            ensure!(
                !toward || typed_before.as_deref() == Some(sneak.typed.as_str()),
                "{origin:?} typed toward an app's label"
            );
        }
        if matches!(action, Action::SneakApp { .. }) && m.sneak.is_some() {
            ensure!(own_labels(m) == own_before, "an app's tree changed ottospace's own labels");
        }
        if sneak.is_some() && !sneak_own {
            let now = m.sneak.as_ref().map(|sneak| sneak.typed.clone());
            ensure!(if user { now.is_none() } else { now == sneak }, "sneak mode: {sneak:?} before, {now:?} after");
        }
        if let Some(asked) = asked.filter(|_| !user && !matches!(action, Action::QuitCancel)) {
            let left = asked.iter().any(|pane| m.pane(*pane).is_some());
            ensure!(m.confirm.is_some() || !left, "the quit question went, and not by the user's answer");
            let quits = effects.contains(&Effect::Quit);
            ensure!(
                !quits || matches!(action, Action::QuitConfirm | Action::Quit),
                "a quit under the question, not by an answer"
            );
        }
        if mark_input.is_some() && !mark_own {
            let gone = matches!(&mark_input, Some(MarkInput::Rename { mark, .. } | MarkInput::SetKey { mark }) if m.marks.get(*mark).is_none());
            let gone = gone
                || matches!(&mark_input, Some(MarkInput::Workbook { workbook, tab, .. })
                    if m.workbooks.get(*workbook).is_none_or(|book| tab.is_some_and(|at| at >= book.tabs.len() || book.tabs.len() < 2)));
            let kept = if user { m.mark_input.is_none() } else { m.mark_input == mark_input || gone };
            ensure!(kept, "a mark's wait: {mark_input:?} before, {:?} after", m.mark_input);
        }

        // An edit goes to the field that has the keyboard, or nowhere.
        if matches!(action, Action::FieldEdit { .. }) && !field.0 {
            let now = m.open_field().map(|(_, field)| field.text().to_string());
            ensure!(now == field.1, "an edit changed a field with no keyboard: {:?} to {now:?}", field.1);
        }
        // The picker's rows change under the selection: it stays on its item while that is a row.
        if let (Action::SetInstalled { .. }, Some(chosen), Some(picker)) = (&action, &chosen, &m.picker) {
            let rows = picker.matches();
            let now = rows.get(picker.selected).copied();
            ensure!(!rows.contains(&chosen) || now == Some(chosen), "the picker was on {chosen:?} and is on {now:?}");
        }
        // What the new-task box files is what its line reads as: one task, or none and the box stays.
        if takes && refused.is_none() {
            ensure!(m.tasks.list.len() == task_ids.len() && m.compose.is_some(), "↵ on a row to take filed a task");
        }
        if let Some((mark, name, _)) = taken.filter(|_| refused.is_none()) {
            // Up to the word taken: of two `>repo` the last counts, and one may follow.
            let upto = m.compose.as_ref().map(|field| &field.text()[..field.cursor()]).unwrap_or_default();
            let draft = m.draft(upto);
            let reads = match mark {
                '#' => draft.project.as_ref() == Some(&name),
                '>' => draft.repo.as_ref() == Some(&name),
                '@' => draft.agent.as_ref() == Some(&name),
                '$' => draft.skills.iter().any(|skill| skill.eq_ignore_ascii_case(&name)),
                '^' => draft.refs.contains(&name),
                // A path is read once the shell has looked it up.
                _ => true,
            };
            ensure!(reads, "{mark}{name} was taken, and {:?} reads as {draft:?}", m.compose_line());
        }
        // What only the box's own line takes does nothing while the line has no keyboard.
        if matches!(action, Action::ComposeAttach { .. } | Action::ComposeRef { .. } | Action::ComposePick { .. })
            && !composing
        {
            ensure!(m.compose_line() == line_before.as_ref(), "the line changed with no keyboard: {line_before:?}");
        }
        if let Some((line, draft)) = filing {
            let filed: Vec<&Task> = m.tasks.list.iter().filter(|task| !task_ids.contains(&task.id)).collect();
            if let Some(named) = named.filter(|_| draft.title.is_empty()) {
                // Named by fixed words, and marked as not typed: nothing captured is in a title.
                let fixed = ["Capture from an app", "Capture from a terminal"].contains(&named.as_str());
                ensure!(
                    filed.len() == 1 && filed[0].title == named && filed[0].title_generated && fixed,
                    "a draft with captures and no title filed {filed:?}"
                );
            } else if draft.title.is_empty() || draft.title.chars().count() > MAX_TITLE {
                ensure!(
                    filed.is_empty() && m.compose_line() == Some(&line),
                    "a line with no title filed a task, or closed the box"
                );
            } else {
                ensure!(
                    filed.len() == 1 && m.compose.is_none(),
                    "{line:?} filed {} tasks; the box is {:?}",
                    filed.len(),
                    m.compose
                );
                ensure!(!filed[0].title_generated, "a title that was typed is marked as made up");
                let task = filed[0];
                let repo = draft.repo.as_deref().and_then(|name| m.repo_named(name));
                let read = (&draft.title, draft.priority.unwrap_or(3), &draft.project, &repo, &draft.agent);
                ensure!(
                    (&task.title, task.priority, &task.project, &task.repo, &task.agent) == read,
                    "{line:?} reads as {draft:?} and filed {task:?}"
                );
                let most = |names: &[String]| names.iter().take(tasks::MAX_TAGS).cloned().collect::<Vec<_>>();
                ensure!(
                    task.skills == most(&draft.skills) && task.paths == most(&draft.paths),
                    "{line:?} reads as {draft:?} and filed {task:?}"
                );
                ensure!(
                    task.state == State::Captured && task.resolved.len() == task.paths.len(),
                    "{line:?} filed {task:?}"
                );
            }
        }

        // The effects: each names what is there, and after them the shell holds what the model does.
        for effect in &effects {
            match effect {
                Effect::Opened(pane) => {
                    ensure!(m.pane(*pane).is_some(), "pane {pane} was opened and is not there");
                    self.shell.panes.retain(|id| id != pane);
                    self.shell.panes.push(*pane);
                }
                Effect::Closed(pane) => {
                    ensure!(
                        panes.contains(&pane.id) && m.pane(pane.id).is_none(),
                        "pane {} was closed: not there before, or still there",
                        pane.id
                    );
                    self.shell.panes.retain(|id| *id != pane.id);
                    self.shell.live = self.shell.live.filter(|live| *live != pane.id);
                    self.shell.visual = self.shell.visual.filter(|visual| *visual != pane.id);
                }
                // A history goes to a terminal that has just started, of the user's opening, and is clean.
                Effect::History { pane, history } => {
                    let opened = effects.contains(&Effect::Opened(*pane));
                    let terminal = m.pane(*pane).is_some_and(|p| p.kind == Kind::Terminal);
                    // Or the layout file's, read by the shell at start.
                    let restored = origin == Origin::System && matches!(action, Action::RestoreLayout { .. });
                    ensure!(
                        opened && terminal && (user || restored),
                        "a history for pane {pane}, not a terminal just opened by the user"
                    );
                    // Plain, and nothing credential-shaped left in it.
                    let plain = |text: &String| text.chars().all(crate::capture::shows);
                    let hidden = history.lines.iter().all(|line| plain(line) && crate::secrets::hide(line) == *line);
                    let command = history.command.as_ref().is_none_or(|c| plain(c) && !crate::secrets::holds_secret(c));
                    ensure!(hidden && command, "a history not cleaned: {history:?}");
                }
                Effect::Live(pane) => {
                    ensure!(self.shell.live.is_none(), "pane {pane} goes live over {:?}", self.shell.live);
                    // Only a peek of this pane may still be up: it becomes the entry.
                    let peek = self.shell.peek.take();
                    ensure!(peek.is_none_or(|p| p == *pane), "pane {pane} goes live under the peek of {peek:?}");
                    self.shell.live = Some(*pane);
                }
                Effect::Parked(pane) => {
                    let (live, peek) = (self.shell.live, self.shell.peek);
                    ensure!(
                        live == Some(*pane) || peek == Some(*pane),
                        "pane {pane} is parked: {live:?} live, {peek:?} up"
                    );
                    if live == Some(*pane) {
                        self.shell.live = None;
                    } else {
                        self.shell.peek = None;
                    }
                }
                Effect::Visual { pane, on: true } => {
                    ensure!(self.shell.visual.is_none(), "visual mode starts on {pane} over {:?}", self.shell.visual);
                    self.shell.visual = Some(*pane);
                }
                Effect::Visual { pane, on: false } => {
                    ensure!(
                        self.shell.visual == Some(*pane),
                        "visual mode ends on {pane}; it was on {:?}",
                        self.shell.visual
                    );
                    self.shell.visual = None;
                }
                Effect::HostApp { name, .. } | Effect::LaunchApp { name, .. } => {
                    ensure!(offered.contains(name), "an app that was not offered is hosted: {name:?}")
                }
                Effect::HostWindow { mark, into, .. } => {
                    ensure!(
                        mark.is_none_or(|id| m.marks.get(id).is_some()),
                        "a window is hosted for mark {mark:?}, which is gone"
                    );
                    let empty = |pane: PaneId| m.pane(pane).is_some_and(|p| p.kind == Kind::Empty);
                    ensure!(into.is_none_or(empty), "a window is hosted into {into:?}, no empty pane");
                }
                Effect::Activate { id } => {
                    let own = format!("{:?}", Source::Own { id: id.clone() });
                    ensure!(labelled.contains(&own), "a control with no label is activated: {id:?}");
                }
                Effect::StartAgent { task: subject, start, cwd, .. } => {
                    let gone = |subject: &Subject| subject.task().is_some_and(|id| m.tasks.get(id).is_none());
                    // An agent runs with the user's rights: no client's action starts one. A
                    // session read answers a resume the user asked for, and only that.
                    let asked = awaiting.contains(subject) && matches!(action, Action::SessionRead { .. });
                    // So does the end of an agent the user stopped to switch its view.
                    let switched = switching.contains(subject) && matches!(action, Action::AgentExited { .. });
                    let resumes = matches!(start, agents::Start::Resume { .. });
                    ensure!(user || ((asked || switched) && resumes), "{origin:?} started an agent for {subject:?}");
                    ensure!(!gone(subject) && m.agent_runs(subject), "an agent starts for nothing: {subject:?}");
                    // A session no task owns is gone on with only where it started, in a listed
                    // checkout, and only if nothing has it: not at work, held by no process.
                    if let Subject::Session { runner, id } = subject {
                        let found = m.found.iter().find(|s| (&s.runner, &s.id) == (runner, id));
                        let free = found.is_some_and(|s| !m.in_use(s) && Some(&s.cwd) == cwd.as_ref());
                        let listed =
                            cwd.as_ref().is_some_and(|cwd| m.repo_names().any(|(_, path)| sessions::below(cwd, path)));
                        ensure!(free && listed, "session {id} gone on with in {cwd:?}: at work, held, or outside");
                    }
                    let task = subject.task().unwrap_or_default();
                    let uuid = "8f14e45f-ceea-467f-a8d3-0242ac130003";
                    let command = match start {
                        agents::Start::New(launch) => launch.command(uuid, None),
                        agents::Start::Resume { runner, session } => agents::resume(*runner, session),
                    };
                    ensure!(command.is_some(), "an agent that cannot be started is asked for: {start:?}");
                    // Its prompt is the task's title alone: nothing captured from a pane or an app
                    // (a title nobody typed is fixed words), and never the brief's text.
                    if let agents::Start::New(launch) = start {
                        let title = m.tasks.get(task).map(|task| task.title.as_str());
                        ensure!(title == Some(launch.title.as_str()), "task {task} starts with {:?}", launch.title);
                    }
                }
                Effect::ReadSession { runner, id } => {
                    // Read for a resume the user asks for now: a task's session, not listed.
                    let waits = m.awaiting.iter().any(|s| m.session_of(s) == Some((runner.clone(), id.clone())));
                    let listed = m.found.iter().any(|s| (&s.runner, &s.id) == (runner, id));
                    ensure!(user && waits && !listed, "{origin:?} read {runner} {id}: no resume waits on it");
                }
                Effect::StopAgent { task } => {
                    ensure!(user && m.agent_runs(task), "{origin:?} stops the agent of {task:?}, or none runs");
                }
                // A message, an answer or an interrupt is the user's, to the conversation shown.
                Effect::AgentSend { task, .. } | Effect::AgentAnswer { task, .. } | Effect::AgentInterrupt { task } => {
                    let shown = m.talk.as_ref().is_some_and(|talk| talk.open && talk.task == *task);
                    ensure!(
                        user && shown && m.agent_runs(task),
                        "{origin:?} reached {task:?}: not the user, or not shown"
                    );
                }
                Effect::AgentMoved { from, to } => {
                    let task = to.task().and_then(|id| m.tasks.get(id));
                    ensure!(matches!(from, Subject::Session { .. }) && task.is_some(), "{from:?} moved to {to:?}");
                    ensure!(
                        m.agent_runs(to) && !m.agent_runs(from),
                        "{from:?} moved to {to:?}, and it runs for neither"
                    );
                }
                Effect::Peek(pane) => {
                    // Only the user's own wheel brings an app's window up, over its parked pane.
                    ensure!(
                        user && matches!(action, Action::AppWheel { .. }),
                        "{origin:?} brought an app's window up: {action:?}"
                    );
                    let (live, peek) = (self.shell.live, self.shell.peek);
                    ensure!(live.is_none() && peek.is_none(), "pane {pane} is peeked over {live:?} live, {peek:?} up");
                    self.shell.peek = Some(*pane);
                    ensure!(m.peek.as_ref().map(|p| p.pane) == Some(*pane), "pane {pane} peeked, the model says no");
                }
                Effect::PressApp { pane, path, role, at, .. } => {
                    // Only the user's own letter presses in another app, and only what was labelled.
                    ensure!(user, "{origin:?} pressed in an app: {path:?}");
                    ensure!(matches!(action, Action::SneakType { .. }), "{action:?} pressed in an app");
                    // A single keystroke, with nothing typed before it, never presses there.
                    let one = matches!(&action, Action::SneakType { text } if text.chars().count() == 1);
                    let fresh = typed_before.as_deref().is_none_or(str::is_empty);
                    ensure!(!(one && fresh), "a single keystroke pressed in an app: {path:?}");
                    let source = Source::App { pane: *pane, path: path.clone(), role: role.clone(), at: *at };
                    ensure!(labelled.contains(&format!("{source:?}")), "an element with no label is pressed: {path:?}");
                    let key = format!("{pane} {path:?} {role}");
                    ensure!(drawn_before.contains(&key), "a label drawn after its first letter pressed {key}");
                }
                Effect::SaveApps { changes } => {
                    // The settings file is the user's: no client's action writes it.
                    ensure!(user, "{origin:?} wrote the settings: {changes:?}");
                    ensure!(!changes.is_empty(), "the settings are written for no change");
                    let keys = changes.iter().all(|change| matches!(change.key, "apps" | "apps_hidden"));
                    ensure!(
                        keys && changes.iter().all(|change| !change.text.is_empty()),
                        "a change names no list or no app: {changes:?}"
                    );
                }
                Effect::SaveRepos { changes } => {
                    ensure!(user && !m.repo_dirs.fixed, "{origin:?} wrote the settings: {changes:?}");
                    let named = |change: &ListChange| change.key == "repos" && !change.text.is_empty();
                    ensure!(changes.len() == 1 && changes.iter().all(named), "not one directory: {changes:?}");
                    ensure!(m.repo_dirs.dirs.len() <= MAX_DIRS, "more directories than the settings may name");
                }
                Effect::Notice { text } => ensure!(user && !text.is_empty(), "{origin:?} is told {text:?}"),
                // The settings file is the user's.
                Effect::SaveTasksView { view } => {
                    ensure!(user && m.tasks_view == *view, "{origin:?} wrote tasks_view {view:?}")
                }
                Effect::SaveSessionsRange { range } => {
                    ensure!(user && m.sessions_range == *range, "{origin:?} wrote sessions_range {range:?}")
                }
                Effect::Run { command } => {
                    ensure!(user && matches!(action, Action::PaletteChoose { .. }), "{command:?} run by {action:?}")
                }
                // Only the user starts fresh; after that the layout is never told to the shell.
                Effect::ForgetLayout => {
                    ensure!(user && matches!(action, Action::StartFresh), "{origin:?} forgot the layout: {action:?}");
                    ensure!(!m.layout_kept, "the layout is forgotten and still kept");
                }
                // An alert comes with time passing, of an event that runs.
                Effect::MeetingAlert { started, .. } => {
                    ensure!(matches!(action, Action::Tick { .. }) && *started <= m.now, "an alert by {action:?}")
                }
                Effect::OpenLink { .. } => {}
                Effect::Quit => {
                    ensure!(
                        matches!(action, Action::QuitAsked { .. } | Action::QuitConfirm | Action::Quit),
                        "a quit by {action:?}"
                    )
                }
                Effect::Snapshot { draft, material, pane } => {
                    let empty = m.pane(*pane).is_none_or(|p| p.kind == Kind::Empty);
                    ensure!(!empty && m.live != Some(*pane), "a picture of pane {pane}: empty, gone or live");
                    let fresh = !self.shell.kept.contains(&(draft.clone(), *material));
                    ensure!(fresh, "capture {material} of {draft:?} is taken twice");
                    self.shell.kept.push((draft.clone(), *material));
                }
                Effect::CaptureDropped { draft, material } => {
                    let goes = |kept: &(String, u32)| kept.0 == *draft && material.is_none_or(|n| n == kept.1);
                    ensure!(
                        material.is_none() || self.shell.kept.iter().any(goes),
                        "a capture that was not kept is dropped"
                    );
                    self.shell.kept.retain(|kept| !goes(kept));
                }
                Effect::CaptureKept { draft, dir, brief } => {
                    let task = m.tasks.get(brief.task).ok_or("a brief for a task that is gone")?;
                    let mut numbers: Vec<u32> = brief.materials.iter().map(|kept| kept.n).collect();
                    let mut held: Vec<u32> =
                        self.shell.kept.iter().filter(|kept| kept.0 == *draft).map(|kept| kept.1).collect();
                    (numbers.sort_unstable(), held.sort_unstable());
                    ensure!(
                        numbers == held && !numbers.is_empty(),
                        "the brief keeps captures {numbers:?}; the shell holds {held:?}"
                    );
                    let own = dir.strip_prefix(&format!("{}-", task.id)).is_some_and(|word| !word.contains(['/', '.']));
                    ensure!(own && task.materials_dir() == Some(dir), "task {} keeps its captures in {dir:?}", task.id);
                    let files: Vec<String> =
                        brief.materials.iter().map(|kept| format!("{dir}/{}.json", kept.n)).collect();
                    ensure!(
                        task.materials == files,
                        "task {} names {:?}; its brief keeps {files:?}",
                        task.id,
                        task.materials
                    );
                    let text = capture::markdown(brief);
                    let last = capture::fence_lines(&brief.fence).1;
                    let ends = text.lines().filter(|line| **line == last).count();
                    ensure!(ends == brief.materials.len() + 1, "a quoted block of the brief ends twice, or not at all");
                    self.shell.kept.retain(|kept| kept.0 != *draft);
                }
            }
        }
        // What the shell keeps of a draft is what the draft holds: a draft that is gone leaves nothing.
        let mut holds: Vec<(String, u32)> = m
            .capture
            .iter()
            .flat_map(|draft| draft.materials.iter().map(|material| (draft.draft.clone(), material.n)))
            .collect();
        let mut kept = self.shell.kept.clone();
        (holds.sort(), kept.sort());
        ensure!(holds == kept, "the draft holds {holds:?}; the shell keeps {kept:?}");
        // The shell was told of every pane that shows something. (Not of an empty pane of a
        // tab built from a mark: it hears of that one when it is filled.)
        let unknown = |pane: &&Pane| pane.kind != Kind::Empty && !self.shell.panes.contains(&pane.id);
        let unknown: Vec<PaneId> = m.panes.iter().filter(unknown).map(|pane| pane.id).collect();
        ensure!(unknown.is_empty(), "the shell was never told panes {unknown:?} opened");
        let gone = |pane: &&PaneId| m.pane(**pane).is_none();
        ensure!(
            !self.shell.panes.iter().any(|pane| gone(&pane)),
            "the shell holds a pane the model closed without a word"
        );
        ensure!(self.shell.live == m.live, "the shell's live pane is {:?}; the model's {:?}", self.shell.live, m.live);
        let peek = m.peek.as_ref().map(|peek| peek.pane);
        ensure!(self.shell.peek == peek, "the shell's peek is {:?}; the model's {peek:?}", self.shell.peek);
        ensure!(
            self.shell.visual == m.visual,
            "the shell's visual pane is {:?}; the model's {:?}",
            self.shell.visual,
            m.visual
        );

        // An id a mark stands for is a pane's or a tab's for as long as the mark keeps it: it
        // is never read as the other, though the two count from 1 alike.
        let mut live = Vec::new();
        for mark in &m.marks.list {
            let Some(id) = mark.live else { continue };
            let pane = matches!(mark.target, marks::Target::Pane(_));
            let was = self.live.iter().find(|(of, kept, _)| (*of, *kept) == (mark.id, id)).map(|(.., was)| *was);
            let kinds = |pane: bool| if pane { "pane" } else { "tab" };
            ensure!(
                was.is_none_or(|was| was == pane),
                "mark {:?} kept {id}, a {}'s id, as a {}'s",
                mark.name,
                kinds(!pane),
                kinds(pane)
            );
            live.push((mark.id, id, pane));
        }
        self.live = live;

        // A mark taken off and put straight back is the mark it was, where it was.
        let undone = m.marks.removed().is_none() && removed.is_some();
        if let Some(before) = self.before_removal.take().filter(|_| undone && matches!(action, Action::MarkUndo)) {
            ensure!(m.marks.list == before, "a mark put back is not as it was: {:?}, before {before:?}", m.marks.list);
        }
        let took_one =
            m.marks.list.len() + 1 == marks.len() && m.marks.removed().is_some_and(|mark| marks.contains(mark));
        self.before_removal = took_one.then_some(marks);
        Ok(())
    }
}

/// The steps from a new model, each checked. The error names the step that failed.
fn replay(steps: &[Envelope]) -> Result<(), String> {
    let run = || {
        let mut run = Run::default();
        let step = |(at, sent): (usize, &Envelope)| run.step(sent.clone()).map_err(|why| format!("step {at}: {why}"));
        steps.iter().enumerate().try_for_each(step)
    };
    fuzz::caught(run).unwrap_or_else(Err)
}

/// Where a sequence starts: a workspace (`BASES`), then a mode (`MODES`).
type Setup = &'static [fn(&Model) -> Action];

fn terminal() -> Action {
    Action::Open { kind: Kind::Terminal, axis: Axis::Right, beside: None }
}

fn slack(window: u32) -> Kind {
    Kind::App { window, title: "general".into(), bundle: Some("com.slack".into()), name: "Slack".into() }
}

fn mark(of: MarkOf, key: char, uid: &str) -> Action {
    Action::MarkAdd { of, name: None, key: Some(key), uid: uid.into(), cwds: vec![(1, "/code/app".into())] }
}

const BASES: &[(&str, Setup)] = &[
    ("empty", &[]),
    // Two tabs, nested splits, an app pane.
    (
        "splits",
        &[
            |_| Action::Split { axis: Axis::Right, kind: Kind::Terminal },
            |_| Action::Split { axis: Axis::Down, kind: Kind::Terminal },
            |_| Action::Open { kind: app(1), axis: Axis::Right, beside: Some(1) },
            |_| Action::NewTab,
            |_| Action::Split { axis: Axis::Right, kind: Kind::Terminal },
            |_| Action::Split { axis: Axis::Down, kind: Kind::Empty },
            |_| Action::Split { axis: Axis::Right, kind: Kind::Terminal },
            |_| Action::Resize { dir: Dir::Left },
            |_| Action::SelectTab { index: 0 },
        ],
    ),
    // Repositories, tasks in several states, one open with a tab, sessions, the Tasks section.
    (
        "tasks",
        &[
            |_| {
                let worktree = Worktree { name: "app-2".into(), path: "/code/app-2".into(), branch: None };
                let app = Repo {
                    name: "app".into(),
                    path: "/code/app".into(),
                    branch: Some("main".into()),
                    worktrees: vec![worktree],
                };
                Action::SetRepos { repos: vec![app] }
            },
            |_| Action::Tick { now: 1_000_000 },
            |_| Action::TaskAdd {
                title: "a".into(),
                priority: 2,
                state: State::Running,
                tab: None,
                assign: Assign::default(),
            },
            |_| Action::TaskAdd {
                title: "b".into(),
                priority: 1,
                state: State::NeedsYou,
                tab: None,
                assign: Assign::default(),
            },
            |_| Action::TaskFromLine { line: "fix zoom #canvas >app p1".into(), known: Known::default(), open: false },
            |_| Action::NewTabOf { task: Some(3) },
            |_| Action::Split { axis: Axis::Down, kind: Kind::Terminal },
            |_| Action::TaskSnooze { id: 1, until: 2_000_000 },
            |_| Action::RowHold { id: 2 },
            |_| {
                let (runner, cwd) = ("claude".to_string(), "/code/app".to_string());
                let s = Found {
                    runner,
                    id: "s1".into(),
                    repo: cwd.clone(),
                    cwd,
                    started: Some(1),
                    last_activity: Some(999_000),
                    active: true,
                    station: None,
                    label: None,
                    title: None,
                    model: None,
                    held: false,
                };
                Action::SetSessions {
                    found: vec![s.clone(), Found { id: "s2".into(), active: false, last_activity: Some(5), ..s }],
                    day: 0,
                }
            },
            |_| Action::ToggleSection { section: Section::Tasks },
            |_| Action::SelectTab { index: 1 },
        ],
    ),
    // App panes: one measured, one in a background tab.
    (
        "hosted",
        &[
            |_| Action::Open { kind: app(1), axis: Axis::Right, beside: None },
            |_| Action::Split { axis: Axis::Right, kind: Kind::Terminal },
            |_| Action::Open { kind: slack(2), axis: Axis::Down, beside: Some(1) },
            |_| Action::PaneWants { pane: 1, content: [400., 600.], outer: [406., 638.], window: [520., 600.] },
            |_| Action::NewTab,
            |_| Action::Open { kind: app(3), axis: Axis::Right, beside: None },
            |_| Action::SelectTab { index: 0 },
        ],
    ),
    // Marks of a terminal, an app and two tabs: one tab closed and built again with a pane
    // that waits for its window, one mark just removed.
    (
        "marks",
        &[
            |_| terminal(),
            |_| mark(MarkOf::Pane(None), 'a', "u1"),
            |_| Action::Open { kind: slack(7), axis: Axis::Right, beside: None },
            |_| mark(MarkOf::Pane(None), 'b', "u2"),
            |_| mark(MarkOf::Tab(None), 'c', "u3"),
            |_| Action::NewTab,
            |_| terminal(),
            |_| mark(MarkOf::Tab(None), 'd', "u4"),
            |_| Action::CloseTab { index: Some(0) },
            |_| Action::MarkGo { mark: MarkRef::Key('c'), axis: Axis::Right },
            |_| Action::MarksChecked { missing: vec![Need::Dir("/gone".into())] },
            |_| Action::MarkRemove { mark: MarkRef::Key('d') },
        ],
    ),
];

const MODES: &[(Mode, Setup)] = &[
    (Mode::Normal, &[|_| Action::Leave]),
    (
        Mode::App,
        &[
            |_| Action::Open { kind: app(9), axis: Axis::Right, beside: None },
            |m| Action::Enter { pane: m.focused.unwrap_or(0) },
        ],
    ),
    (
        Mode::Pick,
        &[|_| {
            let items = vec![PickItem::Terminal, PickItem::App { name: "Slack".into() }];
            Action::OpenPicker { axis: Axis::Right, new_tab: false, items }
        }],
    ),
    (Mode::Visual, &[|_| terminal(), |_| Action::ToggleVisual]),
    (
        Mode::List,
        &[
            |_| Action::SetApps { names: vec!["Slack".into(), "Safari".into()] },
            |_| Action::Leave,
            |_| Action::FocusList,
        ],
    ),
    (Mode::Compose, &[|_| Action::OpenCompose { text: "fix zoom >ap".into() }]),
    (Mode::Palette, &[|_| Action::Leave, |_| Action::PaletteOpen]),
    (
        Mode::Inspect,
        &[
            |_| terminal(),
            |m| Action::InspectorOpen { pane: m.focused.unwrap_or(0), app: "Fixture".into() },
            |_| Action::InspectorFilter { on: true },
        ],
    ),
    (Mode::Sneak, &[|_| Action::Leave, |_| Action::Sneak { targets: controls(5) }]),
    (
        Mode::Confirm,
        &[
            |_| terminal(),
            |m| {
                let running =
                    vec![Running { pane: m.focused.unwrap_or(0), program: Some("cargo".into()), background: false }];
                Action::QuitAsked { running }
            },
        ],
    ),
    (Mode::Mark, &[|_| Action::Leave, |_| Action::MarkPrefix]),
    (
        Mode::Annotate,
        &[
            |_| terminal(),
            |_| Action::CaptureStart {
                pane: None,
                draft: "d1".into(),
                word: "k3f9a2c41b7d".into(),
                facts: Vec::new(),
                images: true,
            },
        ],
    ),
];

/// The base and the mode sequence `seed` starts from.
fn start(seed: u64) -> (&'static str, Mode, impl Iterator<Item = fn(&Model) -> Action>) {
    let (name, base) = BASES[seed as usize % BASES.len()];
    let (mode, setup) = MODES[seed as usize / BASES.len() % MODES.len()];
    (name, mode, base.iter().chain(setup).copied())
}

/// One sequence: `actions` random steps from the start `seed` names. `Err`: the report.
fn sequence(seed: u64, actions: usize, sparse: bool) -> Result<(), String> {
    let (mut rng, mut run) = (Rng::new(seed), Run { sparse, ..Run::default() });
    let (base, mode, setup) = start(seed);
    let outcome = fuzz::caught(|| {
        for make in setup {
            run.step(make(&run.m).sent_by(Origin::User))?;
        }
        (0..actions).try_for_each(|_| run.step(random(&mut rng, &run.m)))?;
        check_state(&run.m)?;
        check_layout(&run.m)?;
        check_file(&run.m)
    });
    let Err(why) = outcome.unwrap_or_else(Err) else { return Ok(()) };
    // The same failure, whatever its numbers and names: not another one met on the way down.
    let kind = |why: &str| {
        let why = why.split_once(": ").filter(|(step, _)| step.starts_with("step ")).map_or(why, |(_, why)| why);
        why.split('"').step_by(2).flat_map(str::chars).filter(|c| !c.is_ascii_digit()).collect::<String>()
    };
    // Replayed, every check runs after every step: a sparse run's failure may come second to
    // another. Then any failure is shrunk, and both are named.
    let replayed = replay(&run.steps).err();
    let other = replayed.as_ref().filter(|fails| kind(fails) != kind(&why));
    let same = |steps: &[Envelope]| replay(steps).is_err_and(|fails| other.is_some() || kind(&fails) == kind(&why));
    let least = if replayed.is_some() { fuzz::shrink(run.steps.clone(), same) } else { run.steps.clone() };
    let fails = match (replay(&least).err(), other) {
        (None, _) => "the replay does not fail: not shrunk".to_string(),
        (Some(fails), None) => fails,
        (Some(fails), Some(other)) => format!("{fails}; not the failure above: the replay fails first at {other}"),
    };
    let steps: String = least.iter().map(|sent| format!("            {},\n", code(sent))).collect();
    Err(format!(
        "seed {seed} (start: {base}, {mode:?} mode) failed at step {}: {why}\n\
         {} steps of {} still fail ({fails}). As a test:\n\n    \
         #[test]\n    fn regression() {{\n        replay(&[\n{steps}        ])\n        .unwrap();\n    }}\n",
        run.steps.len() - 1,
        least.len(),
        run.steps.len(),
    ))
}

fn explore(sequences: u64, actions: usize, sparse: bool) {
    let (first, count) = match std::env::var("OTTOSPACE_PROP_SEED").ok().and_then(|seed| seed.parse().ok()) {
        Some(seed) => (seed, 1),
        None => (0, sequences),
    };
    if let Some(report) = fuzz::each_seed(first, count, |seed| sequence(seed, actions, sparse).err()) {
        panic!("{report}");
    }
}

/// An enveloped action as Rust that builds it, from its `Debug` text: strings get `.into()`,
/// lists `vec!`, and each variant its enum's name, which the field it is in tells
/// (`ENUM_OF`). `UNKNOWN::` marks a variant in a field not listed there.
fn code(sent: &Envelope) -> String {
    const STRUCTS: &[&str] = &[
        "Repo",
        "RepoDir",
        "RepoDirs",
        "Worktree",
        "Task",
        "Assign",
        "Names",
        "PathFound",
        "Known",
        "Found",
        "Seen",
        "Link",
        "Target",
        "Frame",
        "Running",
        "Installed",
        "Entry",
        "TerminalInfo",
        "Facts",
        "Started",
        "History",
        "Tree",
        "Node",
        "Truncated",
        "AppRead",
        "Event",
    ];
    const ENUM_OF: &[(&str, &str)] = &[
        ("axis", "Axis"),
        ("at", "TabAt"),
        ("dir", "Dir"),
        ("zone", "Zone"),
        ("kind", "Kind"),
        ("state", "State"),
        ("heading", "Heading"),
        ("section", "Section"),
        ("source", "Source"),
        ("of", "MarkOf"),
        ("mark", "MarkRef"),
        ("field", "marks::Field"),
        ("edit", "Edit"),
        ("to", "Motion"),
        ("items", "PickItem"),
        ("group", "crate::apps::Group"),
        ("missing", "Need"),
        ("target", "Pointed"),
        ("tool", "Tool"),
        ("content", "Content"),
        ("view", "TasksView"),
        ("range", "sessions::Range"),
        ("runner", "RunnerId"),
        // A conversation's: `TalkPick { id: Task(1) }`.
        ("task", "Subject"),
        ("id", "Subject"),
        ("shows", "View"),
    ];
    // A mark is long, and half of it is JSON already: it is given as its file's text.
    if let Action::RestoreMarks { marks, extra } = &sent.action {
        let (marks, extra) =
            (serde_json::to_string(marks).unwrap_or_default(), serde_json::to_string(extra).unwrap_or_default());
        return format!("restore_marks({marks:?}, {extra:?}).sent_by(Origin::{:?})", sent.origin);
    }
    // So are kept tasks, and a session's work: both are read from files.
    if let Action::RestoreTasks { tasks, floor } = &sent.action {
        let tasks = serde_json::to_string(tasks).unwrap_or_default();
        return format!("restore_tasks({tasks:?}, {floor}).sent_by(Origin::{:?})", sent.origin);
    }
    if let Action::SessionWork { work } = &sent.action {
        let work = serde_json::to_string(work).unwrap_or_default();
        return format!("session_work({work:?}).sent_by(Origin::{:?})", sent.origin);
    }
    if let Action::RestoreLayout { layout, gone, home, launch } = &sent.action {
        let (layout, gone, home) = (
            serde_json::to_string(layout).unwrap_or_default(),
            serde_json::to_string(gone).unwrap_or_default(),
            serde_json::to_string(home).unwrap_or_default(),
        );
        return format!("restore_layout({layout:?}, {gone:?}, {home:?}, {launch}).sent_by(Origin::{:?})", sent.origin);
    }
    // An agent's events carry text whose `Debug` is its length: written as that many `x`.
    if let Action::AgentEvents { task, events, at } = &sent.action {
        let task = match task {
            Subject::Task(id) => format!("Subject::Task({id})"),
            Subject::Session { runner, id } => {
                format!("Subject::Session {{ runner: {runner:?}.into(), id: {id:?}.into() }}")
            }
        };
        let events: Vec<String> = events.iter().map(event_code).collect();
        let events = events.join(", ");
        return format!(
            "Action::AgentEvents {{ task: {task}, events: vec![{events}], at: {at} }}.sent_by(Origin::{:?})",
            sent.origin
        );
    }
    if let Action::RestoreWorkbooks { workbooks } = &sent.action {
        let workbooks = serde_json::to_string(workbooks).unwrap_or_default();
        return format!("restore_workbooks({workbooks:?}).sent_by(Origin::{:?})", sent.origin);
    }
    let text: Vec<char> = format!("{:?}", sent.action).chars().collect();
    let (mut out, mut at) = (String::new(), 0);
    // The field being read, and the fields of what encloses it.
    let (mut field, mut outer): (Option<String>, Vec<Option<String>>) = (None, Vec::new());
    while at < text.len() {
        let c = text[at];
        let quoted = |quote: char| {
            let mut end = at + 1;
            while text[end] != quote {
                end += if text[end] == '\\' { 2 } else { 1 };
            }
            end + 1
        };
        let word = |more: fn(char) -> bool| at + text[at..].iter().take_while(|c| more(**c)).count();
        let end = match c {
            '"' | '\'' => quoted(c),
            c if c.is_ascii_digit() => word(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+')),
            c if c.is_alphabetic() || c == '_' => word(|c| c.is_alphanumeric() || c == '_'),
            _ => at + 1,
        };
        let token: String = text[at..end].iter().collect();
        let is_field = text.get(end) == Some(&':');
        match c {
            '"' => out += &format!("{token}.into()"),
            '[' | '{' | '(' => {
                let sizes = matches!(field.as_deref(), Some("content" | "outer" | "window" | "at" | "to" | "size"));
                out += if c == '[' && !sizes { "vec![" } else { &token };
                outer.push(field.clone());
            }
            ']' | '}' | ')' => {
                field = outer.pop().flatten();
                out.push(c);
            }
            _ if is_field => {
                out += &token;
                field = Some(token);
            }
            _ if token == "NaN" => out += "f32::NAN as _",
            _ if token == "inf" => out += "f32::INFINITY as _",
            _ if c.is_uppercase()
                && !(STRUCTS.contains(&token.as_str()) && text.get(end + 1) == Some(&'{'))
                && !matches!(token.as_str(), "Some" | "None") =>
            {
                let named = field
                    .as_deref()
                    .map(|field| ENUM_OF.iter().find(|(of, _)| *of == field).map_or("UNKNOWN", |(_, name)| name));
                out += &format!("{}::{token}", named.unwrap_or("Action"));
            }
            _ => out += &token,
        }
        at = end;
    }
    format!("{out}.sent_by(Origin::{:?})", sent.origin)
}

/// One runner event as code, for `code`.
fn event_code(event: &Said) -> String {
    let text = |text: &Text| format!("Text::new(&\"x\".repeat({}))", text.len());
    match event {
        Said::Started { session, model, mode } => {
            let some = |of: &Option<String>| of.as_ref().map_or("None".into(), |s| format!("Some({s:?}.into())"));
            format!("Said::Started {{ session: {}, model: {}, mode: {} }}", some(session), some(model), some(mode))
        }
        Said::TurnStarted => "Said::TurnStarted".into(),
        Said::TextDelta { text: t } => format!("Said::TextDelta {{ text: {} }}", text(t)),
        Said::Text { text: t } => format!("Said::Text {{ text: {} }}", text(t)),
        Said::ToolStart { id, sub, tool, label, detail, diff } => format!(
            "Said::ToolStart {{ id: {id:?}.into(), sub: {sub}, tool: {tool:?}.into(), label: {label:?}.into(), detail: {}, \
             diff: {} }}",
            text(detail),
            match diff {
                Some(_) =>
                    "crate::diff::claude(\"Edit\", &serde_json::json!({\"file_path\": \"/w/a\", \
                            \"old_string\": \"x\", \"new_string\": \"y\"}))",
                None => "None",
            }
        ),
        Said::ToolEnd { id, failed } => format!("Said::ToolEnd {{ id: {id:?}.into(), failed: {failed} }}"),
        Said::Permission { ticket, tool, label, detail, len, cut, claim } => format!(
            "Said::Permission {{ ticket: {ticket}, tool: {tool:?}.into(), label: {label:?}.into(), detail: {}, \
             len: {len}, cut: {cut}, claim: {} }}",
            text(detail),
            text(claim)
        ),
        Said::PermissionClosed { ticket } => format!("Said::PermissionClosed {{ ticket: {ticket} }}"),
        Said::AutoDenied { tool, label, reason, detail } => format!(
            "Said::AutoDenied {{ tool: {tool:?}.into(), label: {label:?}.into(), reason: {reason:?}.into(), detail: {} }}",
            text(detail)
        ),
        Said::Usage { context, window, cost_micro_usd } => {
            format!("Said::Usage {{ context: {context:?}, window: {window:?}, cost_micro_usd: {cost_micro_usd:?} }}")
        }
        Said::Compaction => "Said::Compaction".into(),
        Said::TurnFinished { outcome } => {
            let outcome = match outcome {
                Outcome::Completed => "Outcome::Completed".into(),
                Outcome::Interrupted => "Outcome::Interrupted".into(),
                Outcome::Failed { kind } => format!("Outcome::Failed {{ kind: {kind:?}.into() }}"),
            };
            format!("Said::TurnFinished {{ outcome: {outcome} }}")
        }
        Said::Error { kind, retrying } => format!("Said::Error {{ kind: {kind:?}.into(), retrying: {retrying} }}"),
        Said::Exited { code } => format!("Said::Exited {{ code: {code:?} }}"),
        Said::MessageElsewhere => "Said::MessageElsewhere".into(),
        Said::WatchFromStart => "Said::WatchFromStart".into(),
    }
}

/// `RestoreMarks` from the JSON of its marks and of the file's own keys, as `code` writes it.
fn restore_marks(marks: &str, extra: &str) -> Action {
    let read = "a regression's marks are JSON";
    Action::RestoreMarks {
        marks: serde_json::from_str(marks).expect(read),
        extra: serde_json::from_str(extra).expect(read),
    }
}

/// `RestoreTasks` from the JSON of its tasks, and `SessionWork` from its work's, as `code` writes them.
fn restore_tasks(tasks: &str, floor: TaskId) -> Action {
    Action::RestoreTasks { tasks: serde_json::from_str(tasks).expect("a regression's tasks are JSON"), floor }
}

fn session_work(work: &str) -> Action {
    Action::SessionWork { work: serde_json::from_str(work).expect("a regression's work is JSON") }
}

/// `RestoreLayout` from the JSON of its file, as `code` writes it.
fn restore_layout(layout: &str, gone: &str, home: &str, launch: bool) -> Action {
    let read = "a regression's layout is JSON";
    let layout = Box::new(serde_json::from_str(layout).expect(read));
    let (gone, home) = (serde_json::from_str(gone).expect(read), serde_json::from_str(home).expect(read));
    Action::RestoreLayout { layout, gone, home, launch }
}

/// `RestoreWorkbooks` from the JSON of its workbooks, as `code` writes it.
fn restore_workbooks(workbooks: &str) -> Action {
    Action::RestoreWorkbooks { workbooks: serde_json::from_str(workbooks).expect("a regression's workbooks are JSON") }
}

#[test]
fn the_makers_make_every_action() {
    let (mut rng, m) = (Rng::new(1), Model::default());
    let mut made: Vec<usize> = MAKERS.iter().map(|make| index(&make(&mut Gen { rng: &mut rng, m: &m }))).collect();
    made.sort_unstable();
    assert_eq!(made, (0..COUNT).collect::<Vec<_>>(), "one maker for every action");
}

#[test]
fn every_start_is_in_its_mode() {
    for seed in 0..(BASES.len() * MODES.len()) as u64 {
        let (base, mode, setup) = start(seed);
        let mut run = Run::default();
        for make in setup {
            run.step(make(&run.m).sent_by(Origin::User)).unwrap_or_else(|why| panic!("{base}, {mode:?}: {why}"));
        }
        assert_eq!(run.m.mode(), mode, "{base}");
    }
    // The bases are what they say.
    let model = |seed: u64| {
        let mut run = Run::default();
        start(seed).2.for_each(|make| run.step(make(&run.m).sent_by(Origin::User)).unwrap());
        run.m
    };
    let (splits, tasks, hosted, marks) = (model(1), model(2), model(3), model(4));
    assert_eq!((splits.tabs.len(), splits.panes.len()), (2, 6));
    assert_eq!((tasks.tasks.list.len(), tasks.sessions_bar().count, tasks.sidebar), (3, 2, Some(Section::Tasks)));
    assert!(hosted.pane(1).is_some_and(|pane| pane.fit.is_some()));
    assert_eq!((marks.marks.list.len(), marks.marks.unfilled().len(), marks.marks.removed().is_some()), (3, 1, true));
}

/// What `code` writes builds the action it was given: here for one of each, by eye for the
/// regressions below, which are its output.
#[test]
fn every_action_can_be_written_as_code() {
    let (mut rng, mut run) = (Rng::new(7), Run::default());
    start(4).2.for_each(|make| drop(run.m.update(make(&run.m))));
    for _ in 0..40 {
        for make in MAKERS {
            let written = code(&make(&mut Gen { rng: &mut rng, m: &run.m }).sent_by(Origin::Socket));
            assert!(!written.contains("UNKNOWN::"), "{written}");
        }
    }
    let wants =
        Action::PaneWants { pane: 1, content: [f32::NAN, 600.], outer: [406., f32::NEG_INFINITY], window: [1e9, 0.] };
    assert_eq!(
        code(&wants.sent_by(Origin::System)),
        "Action::PaneWants { pane: 1, content: [f32::NAN as _, 600.0], outer: [406.0, -f32::INFINITY as _], \
         window: [1000000000.0, 0.0] }.sent_by(Origin::System)"
    );
    let open = Action::OpenPicker {
        axis: Axis::Down,
        new_tab: true,
        items: vec![PickItem::App { name: "a\"b".into() }, PickItem::Terminal],
    };
    assert_eq!(
        code(&open.sent_by(Origin::User)),
        "Action::OpenPicker { axis: Axis::Down, new_tab: true, items: vec![PickItem::App { name: \"a\\\"b\".into() }, \
         PickItem::Terminal] }.sent_by(Origin::User)"
    );
    // The helpers its output calls for actions read from files build them again.
    let work = serde_json::to_string(&progress::Work::default()).expect("work is JSON");
    let tasks = Action::RestoreTasks { tasks: vec![], floor: 3 };
    assert_eq!(restore_tasks("[]", 3), tasks);
    assert_eq!(code(&tasks.sent_by(Origin::System)), "restore_tasks(\"[]\", 3).sent_by(Origin::System)");
    assert_eq!(session_work(&work), Action::SessionWork { work: progress::Work::default() });
}

#[test]
fn a_failure_is_shrunk_to_the_steps_it_needs() {
    // "Fails" once a tab is closed after a split: the two steps are all that is left.
    let fails = |steps: &[Envelope]| {
        let split = steps.iter().position(|sent| matches!(sent.action, Action::Split { .. }));
        split.is_some_and(|at| steps[at..].iter().any(|sent| matches!(sent.action, Action::CloseTab { .. })))
    };
    let (mut rng, m) = (Rng::new(3), Model::default());
    let mut steps: Vec<Envelope> = (0..200).map(|_| random(&mut rng, &m)).collect();
    // One split for sure, whatever the makers drew.
    steps.insert(50, Action::Split { axis: Axis::Right, kind: Kind::Empty }.into());
    steps.push(Action::CloseTab { index: None }.into());
    let least = fuzz::shrink(steps, fails);
    assert!(matches!(
        least.as_slice(),
        [Envelope { action: Action::Split { .. }, .. }, Envelope { action: Action::CloseTab { .. }, .. }]
    ));
    // A panic is a failure like any other, and names itself.
    assert_eq!(fuzz::caught(|| panic!("boom")).unwrap_or_else(|why| why), "panicked: boom");
}

/// 2,000 sequences of 200 actions, from every start.
#[test]
fn random_actions_keep_the_model_sound() {
    explore(2_000, 200, true);
}

/// `cargo test -p ottospace-core --release -- --ignored random_actions`: minutes.
#[test]
#[ignore = "long: 100,000 sequences of 400 actions"]
fn random_actions_keep_the_model_sound_for_long() {
    explore(100_000, 400, false);
}

// Regressions: failures the run above found, shrunk as `sequence` printed them, then tidied by
// hand (a helper for a long action, `.into()` for the user's). Each says what was wrong.

fn by(origin: Origin, action: Action) -> Envelope {
    action.sent_by(origin)
}

fn split(axis: Axis, kind: Kind) -> Envelope {
    Action::Split { axis, kind }.into()
}

/// ← from the right rail's cell of the conversation on show gave the keyboard to the drawer
/// under the inspector, which still covered it.
#[test]
fn back_from_the_rail_under_the_inspector_shows_the_conversation() {
    let slack =
        Kind::App { window: 7, title: "general".into(), bundle: Some("com.slack".into()), name: "Slack".into() };
    replay(&[
        Action::Open { kind: slack, axis: Axis::Right, beside: None }.into(),
        by(Origin::System, Action::OpenCompose { text: "flicker".into() }),
        Action::ComposeSubmit { open: true }.into(),
        Action::FocusDir { dir: Dir::Right }.into(),
        Action::InspectorOpen { pane: 1, app: "Slack".into() }.into(),
        Action::FocusDir { dir: Dir::Left }.into(),
    ])
    .unwrap();
}

/// An app's tree arrived after the user's first letter, and a new app label went on with it: the
/// user's next letter pressed an element they never saw labelled. (Found by review.)
#[test]
fn an_app_label_drawn_after_the_first_letter_is_not_pressed_by_the_second() {
    let own =
        |id: &str, x: f64| Target { frame: Frame { x, y: 0., w: 20., h: 20. }, source: Source::Own { id: id.into() } };
    // 17 controls take one letter each; the ×'s two-letter label starts with the 18th, `o`.
    let mut targets: Vec<Target> = (0..17).map(|i| own(&format!("c{i}"), i as f64 * 30.)).collect();
    targets.push(own("close:1", 600.));
    let button = |x: f64| crate::ax::Node {
        role: "AXButton".into(),
        actions: vec!["AXPress".into()],
        frame: Some(Frame { x, y: 60., w: 30., h: 20. }),
        ..Default::default()
    };
    let root =
        crate::ax::Node { role: "AXWindow".into(), children: vec![button(10.), button(50.)], ..Default::default() };
    let window = Frame { x: 0., y: 0., w: 400., h: 300. };
    let read = sneak_apps::AppRead { window, crop: 0., content: window, root };
    replay(&[
        split(Axis::Right, Kind::Empty),
        Action::Sneak { targets }.into(),
        Action::SneakType { text: "o".into() }.into(),
        by(Origin::System, Action::SneakApp { pane: 1, read: Some(read) }),
        Action::SneakType { text: "s".into() }.into(),
    ])
    .unwrap();
}

/// An app's control had a one-letter label (`s`): one stray key pressed it. (Found by review.)
#[test]
fn one_key_never_presses_in_an_app() {
    let own = Target { frame: Frame { x: 0., y: 0., w: 20., h: 20. }, source: Source::Own { id: "c0".into() } };
    let button = crate::ax::Node {
        role: "AXButton".into(),
        actions: vec!["AXPress".into()],
        frame: Some(Frame { x: 10., y: 60., w: 30., h: 20. }),
        ..Default::default()
    };
    let root = crate::ax::Node { role: "AXWindow".into(), children: vec![button], ..Default::default() };
    let window = Frame { x: 0., y: 0., w: 400., h: 300. };
    let read = sneak_apps::AppRead { window, crop: 0., content: window, root };
    replay(&[
        split(Axis::Right, Kind::Empty),
        Action::Sneak { targets: vec![own] }.into(),
        by(Origin::System, Action::SneakApp { pane: 1, read: Some(read) }),
        Action::SneakType { text: "s".into() }.into(),
    ])
    .unwrap();
}

/// A listed directory with no text was removed: the change to the settings named nothing.
#[test]
fn a_directory_with_no_text_is_not_removed() {
    replay(&[
        Action::SetRepoDirs {
            dirs: RepoDirs { dirs: vec![RepoDir { text: "".into(), path: "".into() }], fixed: false },
        }
        .sent_by(Origin::Socket),
        Action::RepoRemove { path: "".into() }.sent_by(Origin::User),
    ])
    .unwrap();
}

/// `Open` beside an empty pane of a background tab filled it and focused it: the tab on show
/// then focused a pane it did not hold. The pane's tab now comes forward.
#[test]
fn open_into_an_empty_pane_of_a_background_tab_brings_its_tab_forward() {
    replay(&[
        split(Axis::Right, Kind::Empty),
        Action::NewTab.into(),
        terminal().into(),
        Action::Open { kind: Kind::Terminal, axis: Axis::Down, beside: Some(1) }.into(),
    ])
    .unwrap();
}

/// A terminal took a fit from `PaneWants`, and `Fit` then moved the splits around it.
#[test]
fn only_an_app_pane_takes_a_fit() {
    replay(&[
        terminal().into(),
        Action::PaneWants { pane: 1, content: [400., 600.], outer: [406., 638.], window: [520., 600.] }.into(),
    ])
    .unwrap();
}

/// A measure with a whole size that is no number made `Fit` set a split's ratio to NaN.
#[test]
fn a_fit_from_a_measure_that_is_no_number_moves_nothing() {
    replay(&[
        split(Axis::Right, Kind::Terminal),
        Action::Open { kind: app(1), axis: Axis::Right, beside: None }.into(),
        Action::PaneWants { pane: 2, content: [520., 232.], outer: [f32::INFINITY, 406.], window: [-5., 400.] }.into(),
        Action::Fit { pane: Some(2) }.into(),
    ])
    .unwrap();
}

/// `RestoreTasks` left a tab, and the bar, on a task that was no longer there.
#[test]
fn a_tab_of_a_task_that_is_not_restored_is_a_tab_of_no_task() {
    let add = Action::TaskAdd {
        title: "a".into(),
        priority: 3,
        state: State::Running,
        tab: Some(1),
        assign: Assign::default(),
    };
    replay(&[add.into(), by(Origin::System, Action::RestoreTasks { tasks: Vec::new(), floor: 0 })]).unwrap();
}

/// `ListMove` and `ListSection` added `delta` as an i32: i32::MAX from the socket overflowed.
#[test]
fn a_list_move_by_any_i32_does_not_overflow() {
    replay(&[
        Action::SetApps { names: vec!["Slack".into(), "Safari".into()] }.into(),
        Action::ToggleSection { section: Section::Apps }.into(),
        Action::FocusList.into(),
        Action::ListMove { delta: 1 }.into(),
        by(Origin::Socket, Action::ListMove { delta: i32::MAX }),
        by(Origin::Socket, Action::ListSection { delta: i32::MAX }),
        by(Origin::Socket, Action::ListSection { delta: i32::MIN }),
    ])
    .unwrap();
}

/// A frame that drew none of the labelled controls left sneak mode on with nothing to type.
#[test]
fn sneak_mode_ends_when_no_labelled_control_is_drawn() {
    replay(&[
        Action::Sneak { targets: controls(2) }.into(),
        by(Origin::System, Action::SneakDrawn { targets: Vec::new() }),
    ])
    .unwrap();
}

/// "Add and start" by the user in visual mode changed tabs under the visual cursor: `TaskOpen`
/// alone would have waited.
#[test]
fn add_and_start_waits_where_opening_a_task_waits() {
    replay(&[
        terminal().into(),
        Action::ToggleVisual.into(),
        Action::TaskFromLine { line: "x".into(), known: Known::default(), open: true }.into(),
    ])
    .unwrap();
}

/// A pane mark went on standing for a pane that was closed.
#[test]
fn a_mark_does_not_stand_for_a_closed_pane() {
    replay(&[terminal().into(), mark(MarkOf::Pane(None), 'a', "u1").into(), Action::Close { pane: None }.into()])
        .unwrap();
}

/// `MarkBind` bound any mark to any pane: a tab mark then stood for the tab with the pane's
/// id, and a terminal's mark for an app pane.
#[test]
fn a_mark_is_bound_only_to_a_pane_of_its_kind() {
    replay(&[
        terminal().into(),
        Action::Open { kind: slack(7), axis: Axis::Right, beside: None }.into(),
        mark(MarkOf::Pane(None), 'b', "u2").into(),
        mark(MarkOf::Tab(None), 'c', "u3").into(),
        terminal().into(),
        by(Origin::System, Action::MarkBind { mark: 2, pane: 3 }),
        by(Origin::System, Action::MarkBind { mark: 1, pane: 3 }),
    ])
    .unwrap();
}

/// A tab mark that was removed kept its tab's id; the tab closed; `MarkUndo` put back a mark of
/// a tab that was gone. With one tab it was worse: the empty tab that took its place has its id.
#[test]
fn a_removed_mark_put_back_does_not_stand_for_a_tab_closed_since() {
    let steps = [
        terminal().into(),
        mark(MarkOf::Tab(None), 'a', "u1").into(),
        by(Origin::Socket, Action::MarkRemove { mark: MarkRef::Place(0) }),
        by(Origin::System, Action::CloseTab { index: None }),
        Action::MarkUndo.into(),
    ];
    replay(&[vec![Action::NewTab.into()], steps.to_vec()].concat()).unwrap();
    let mut m = Model::default();
    steps.into_iter().for_each(|step| drop(m.update(step)));
    assert_eq!((m.tabs[0].id, m.marks.list[0].live, m.mark_rows()[0].open), (1, None, false));
}

/// Two marks restored under one `uid` both took the id of the mark that had it.
#[test]
fn two_restored_marks_with_one_uid_get_two_ids() {
    let shell = r#"{"id":"u1","name":"a","target":{"pane":{"kind":{"type":"terminal"}}}}"#;
    replay(&[
        terminal().into(),
        mark(MarkOf::Pane(None), 'a', "u1").into(),
        by(Origin::System, restore_marks(&format!("[{shell},{shell}]"), "{}")),
    ])
    .unwrap();
}

/// A restored mark kept the pane its `uid` stood for, though the file now says it is an app's.
#[test]
fn a_restored_mark_of_another_kind_stands_for_nothing() {
    let safari =
        r#"[{"id":"u1","name":"x","target":{"pane":{"kind":{"type":"app","name":"Safari","title":"general"}}}}]"#;
    replay(&[
        terminal().into(),
        mark(MarkOf::Pane(None), 'a', "u1").into(),
        by(Origin::Socket, restore_marks(safari, "{}")),
    ])
    .unwrap();
}

/// `RestoreMarks` without the mark whose name was being typed left the wait on a mark that was gone.
#[test]
fn a_mark_that_is_not_restored_no_longer_waits_for_its_name() {
    let other = r#"[{"id":"u4","name":"deploy","target":{"tab":{"title":"t","layout":null,"panes":[]}}}]"#;
    replay(&[
        terminal().into(),
        mark(MarkOf::Pane(None), 'a', "u1").into(),
        by(Origin::Socket, Action::MarkEdit { mark: MarkRef::Id(1), field: marks::Field::Name }),
        by(Origin::System, restore_marks(other, "{}")),
    ])
    .unwrap();
}

/// The second mark of a name of `MAX_NAME` characters was named with " 2" after them: 42.
#[test]
fn a_numbered_mark_name_is_no_longer_than_a_name_may_be() {
    let named = |uid: &str| Action::MarkAdd {
        of: MarkOf::Pane(None),
        name: Some("a".repeat(marks::MAX_NAME)),
        key: None,
        uid: uid.into(),
        cwds: Vec::new(),
    };
    replay(&[terminal().into(), named("u1").into(), split(Axis::Down, Kind::Terminal), named("u2").into()]).unwrap();
}

/// Keys this build does not know are written back from every level of `marks.json`: the file,
/// a mark, a tab, a pane, an app. (`check_file` holds the same of every model a run reaches.)
#[test]
fn marks_json_keeps_the_keys_it_does_not_know() {
    let text = r#"{"version": 1, "file": 1, "marks": [
        {"id": "u1", "name": "a", "mark": 2, "target": {"pane": {"kind": {"type": "app", "name": "Slack", "title": "", "app": 3}, "pane": 4}}},
        {"id": "u2", "name": "b", "target": {"tab": {"title": "t", "tab": 5, "layout": {"leaf": 1},
            "panes": [{"id": 1, "kind": {"type": "terminal"}, "pane": 6}]}}}]}"#;
    let loaded = marks::parse(text).unwrap();
    let json = |text: &str| serde_json::from_str::<serde_json::Value>(text).unwrap();
    assert_eq!(json(&marks::write(&loaded.marks, &loaded.extra)), json(text));
}

/// The second half of 12: a pane's id and a tab's count from 1 alike, and a mark restored as
/// the other sort of mark kept the number: pane 1's mark, now a tab's, was open on tab 1.
#[test]
fn a_restored_mark_of_the_other_sort_does_not_keep_an_id_that_happens_to_exist() {
    let tab = r#"[{"id":"u1","name":"x","target":{"tab":{"title":"t","layout":{"leaf":1},"panes":[{"id":1,"kind":{"type":"terminal"}}]}}}]"#;
    let pane = r#"[{"id":"u1","name":"x","target":{"pane":{"kind":{"type":"terminal"}}}}]"#;
    for (of, file) in [(MarkOf::Pane(None), tab), (MarkOf::Tab(None), pane)] {
        // Pane 1 is a terminal in tab 1, so either id would pass for the other.
        let steps = [terminal().into(), mark(of, 'a', "u1").into(), by(Origin::System, restore_marks(file, "{}"))];
        replay(&steps).unwrap();
        let mut m = Model::default();
        steps.into_iter().for_each(|step| drop(m.update(step)));
        assert_eq!((m.marks.list[0].live, m.mark_rows()[0].open), (None, false), "{file}");
    }
}

/// A mark made here was one `marks::parse` skips at the next start, and a file with a skipped
/// mark is never written again. A title is now cut to what the file takes; an app with no name
/// and a directory that is not absolute are not marked, and `mark_refusal` says why.
#[test]
fn a_mark_made_here_reads_back() {
    let app = |title: &str, name: &str| Kind::App { window: 1, title: title.into(), bundle: None, name: name.into() };
    let marked = |kind: Kind, cwd: &str| {
        let mut m = Model::default();
        m.update(Action::Open { kind, axis: Axis::Right, beside: None });
        let add = |of| Action::MarkAdd { of, name: None, key: None, uid: String::new(), cwds: vec![(1, cwd.into())] };
        m.update(add(MarkOf::Pane(None)));
        m.update(add(MarkOf::Tab(None)));
        check_file(&m).unwrap();
        m
    };
    // Cut: 600 characters to 512, and a line break out.
    let title = |m: &Model| match &m.marks.list[0].target {
        marks::Target::Pane(PaneRecord { kind: RecordKind::App { title, .. }, .. }) => title.clone(),
        other => panic!("{other:?}"),
    };
    let long = marked(app(&"t".repeat(600), "Safari"), "/");
    assert_eq!((long.marks.list.len(), title(&long).len()), (2, 512));
    assert_eq!(title(&marked(app("a\nb", "Safari"), "/")), "ab");
    // Refused, pane and tab both.
    let nameless = marked(app("t", ""), "/");
    assert_eq!(nameless.marks.list, []);
    assert_eq!(nameless.mark_refusal(MarkOf::Pane(None)), Some("pane 1 cannot be marked: its app has no name".into()));
    assert_eq!(nameless.mark_refusal(MarkOf::Tab(None)), Some("tab 1 cannot be marked: its app has no name".into()));
    assert_eq!(marked(Kind::Terminal, "relative").marks.list, []);
    // An open mark's title follows its window, cut the same way.
    let mut m = marked(app("t", "Safari"), "/");
    m.update(Action::Titles { titles: vec![(1, "n\n".repeat(600))] });
    m.update(Action::MarkGo { mark: MarkRef::Place(0), axis: Axis::Right });
    assert_eq!(title(&m), "n".repeat(512));
    check_file(&m).unwrap();
}

/// ⌘/ over the new-task box. No slip: the picker closes for the key reference, the box stays
/// open under it. Its line is kept and nothing edits it there (the shell sends the reference
/// its keys), and Esc gives the box the keyboard back.
#[test]
fn the_new_task_box_keeps_its_line_under_the_key_reference() {
    let mut m = Model::default();
    m.update(Action::OpenCompose { text: "fix".into() });
    m.update(Action::ToggleKeys);
    assert_eq!((m.keys, m.mode(), m.field().is_none()), (true, Mode::Compose, true));
    m.update(Action::FieldEdit { edit: Edit::Insert("x".into()) });
    m.update(Action::CloseKeys);
    assert_eq!((m.keys, m.field().map(|(of, field)| (of, field.text()))), (false, Some(("compose", "fix"))));
    check(&m).unwrap();
}

/// Found by the random run: an app with no name was pinned, and the settings told to hold `""`.
#[test]
fn an_app_with_no_name_is_not_pinned() {
    let app = |name: &str| Installed { name: name.into(), path: format!("/Applications/{name}.app") };
    replay(&[
        Action::SetAppSettings { pinned: vec![], hidden: vec![], suggest: true }.sent_by(Origin::User),
        Action::SetInstalled { apps: vec![app("Slides"), app("")], running: vec![] }.sent_by(Origin::User),
        Action::SetDock { apps: vec![app("")] }.sent_by(Origin::User),
        Action::AppPin { path: "/Applications/.app".into() }.sent_by(Origin::User),
        Action::AppHide { path: "/Applications/.app".into() }.sent_by(Origin::User),
    ])
    .unwrap();
    // Nor is an empty line of the settings unpinned: it names nothing, also once the scan is in.
    let empty = crate::apps::Entry { text: "".into(), app: Some(app("")) };
    replay(&[
        Action::SetInstalled { apps: vec![app("")], running: vec![] }.sent_by(Origin::System),
        Action::SetAppSettings { pinned: vec![empty], hidden: vec![], suggest: true }.sent_by(Origin::System),
        Action::AppPin { path: "/Applications/.app".into() }.sent_by(Origin::User),
    ])
    .unwrap();
}

/// Found by the random run: a pin of an app the settings name already wrote them for no change.
#[test]
fn a_pin_of_an_app_the_settings_name_already_writes_nothing() {
    let sketch = Installed { name: "Sketch".into(), path: "/Applications/Sketch.app".into() };
    let named = crate::apps::Entry { text: "Sketch".into(), app: None };
    let mut m = Model::default();
    m.update(Action::SetAppSettings { pinned: vec![named], hidden: vec![], suggest: true });
    assert!(m.apps_bar().not_running.is_empty(), "no scan yet: the line names no app found");
    m.update(Action::SetInstalled { apps: vec![sketch.clone()], running: vec![] });
    assert_eq!(m.apps_bar().not_running.len(), 1, "the scan came in: it is the pinned app");
    // Pinned already: a pin unpins it, and that is one line off the file.
    let effects = m.update(Action::AppPin { path: sketch.path.clone() });
    assert!(matches!(&effects[..], [Effect::SaveApps { changes }] if changes.len() == 1 && !changes[0].add));
    check(&m).unwrap();
}

/// Found by the long random run: thirteen panes in a row, evened out, and one closed. The
/// ratios `Equalize` gave thirteen left a side of twelve less than its least share.
#[test]
fn a_pane_closed_from_a_long_even_row_leaves_every_side_its_share() {
    let app = |window: u32, name: &str| Kind::App { window, title: "w1".into(), bundle: None, name: name.into() };
    replay(&[
        Action::Open { kind: app(1, "app"), axis: Axis::Right, beside: None }.sent_by(Origin::User),
        Action::Split { axis: Axis::Right, kind: Kind::Terminal }.sent_by(Origin::User),
        Action::Open { kind: app(2, "Slack"), axis: Axis::Down, beside: Some(1) }.sent_by(Origin::User),
        Action::Open { kind: Kind::Terminal, axis: Axis::Right, beside: None }.sent_by(Origin::Socket),
        Action::Open { kind: Kind::Terminal, axis: Axis::Right, beside: Some(2) }.sent_by(Origin::System),
        Action::Open { kind: Kind::Terminal, axis: Axis::Right, beside: Some(7) }.sent_by(Origin::User),
        Action::Open { kind: app(2, "Safari"), axis: Axis::Right, beside: Some(3) }.sent_by(Origin::User),
        Action::DropPane { source: 1, target: 3, zone: Zone::Left }.sent_by(Origin::Socket),
        Action::Adopt { kind: app(3, "Slack"), beside: Some(5) }.sent_by(Origin::User),
        Action::Adopt { kind: Kind::Empty, beside: Some(9) }.sent_by(Origin::User),
        Action::Open { kind: Kind::Terminal, axis: Axis::Right, beside: None }.sent_by(Origin::User),
        Action::Open { kind: Kind::Empty, axis: Axis::Right, beside: Some(1) }.sent_by(Origin::Socket),
        Action::Equalize.sent_by(Origin::System),
        Action::Close { pane: None }.sent_by(Origin::Socket),
    ])
    .unwrap();
}

/// Found by the random run once the Apps filter and a mark's wait met: a jump to a mark asked
/// for while the filter was typed in left the filter with the keyboard, in the mark's mode.
#[test]
fn a_mark_that_waits_for_a_letter_is_over_the_apps_filter() {
    let mut m = Model::default();
    m.update(Action::ToggleSection { section: Section::Apps });
    m.update(Action::AppsFilter { on: true });
    assert_eq!(m.field().map(|(of, _)| of), Some("apps"));
    m.update(Action::MarkPrefix.sent_by(Origin::Socket));
    assert_eq!((m.mode(), m.field().map(|(of, _)| of)), (Mode::Mark, None));
    check(&m).unwrap();
    // Found by the long run: an edit then ends the wait, and types nothing into the filter under it.
    m.update(Action::FieldEdit { edit: Edit::Insert("x".into()) });
    assert_eq!((m.mode(), m.field().map(|(of, field)| (of, field.text()))), (Mode::List, Some(("apps", ""))));
    check(&m).unwrap();
}

/// Found by the long random run: a repository whose name holds a control character was
/// offered, and taken it read as another name. It is not offered.
#[test]
fn a_name_the_line_cannot_hold_is_not_offered() {
    let repo = |name: &str| Repo { name: name.into(), path: format!("/code/{name}"), branch: None, worktrees: vec![] };
    let mut m = Model::default();
    m.update(Action::SetRepos { repos: vec![repo("a\u{7f}"), repo("app")] });
    m.update(Action::OpenCompose { text: "fix >".into() });
    let names: Vec<String> = m.dropdown().unwrap().rows.into_iter().map(|row| row.name).collect();
    assert_eq!(names, ["app"]);
    m.update(Action::ComposeComplete);
    assert_eq!(m.draft(m.compose_line().unwrap()).repo.as_deref(), Some("app"));
    check(&m).unwrap();
}

/// Annotate mode ended (its pane's tab left the stage) with the button still down on the
/// picture: the drag went on under no mode.
#[test]
fn a_drag_on_a_picture_ends_with_annotate_mode() {
    let press = Action::AnnotatePress { at: [0.75, 0.5], shift: true, alt: false, size: [800.0, f64::NAN] };
    replay(&[
        Action::TaskAdd {
            title: "zoom".into(),
            priority: 1,
            state: State::Cleaned,
            tab: Some(1),
            assign: Assign::default(),
        }
        .into(),
        split(Axis::Down, Kind::Terminal),
        capture(None).into(),
        by(Origin::Socket, press),
        by(Origin::System, Action::CloseTab { index: None }),
    ])
    .unwrap();
}

fn capture(pane: Option<PaneId>) -> Action {
    Action::CaptureStart { pane, draft: "u2".into(), word: "k3f9a2c41b7d".into(), facts: vec![], images: false }
}

/// ↵ in annotate mode gave the draft's title the keyboard while another pane's app was live.
#[test]
fn the_drafts_title_takes_the_keyboard_from_a_live_app() {
    replay(&[
        Action::Open { kind: Kind::Terminal, axis: Axis::Right, beside: None }.into(),
        capture(None).into(),
        Action::Open { kind: slack(4), axis: Axis::Down, beside: None }.into(),
        by(Origin::Socket, Action::Enter { pane: 2 }),
        by(Origin::System, Action::AnnotateLeave { title: true }),
    ])
    .unwrap();
}

/// The way back entered its app under the key reference, and with a row of the list selected.
#[test]
fn the_way_back_to_an_app_waits_and_leaves_the_list_as_entering_it_does() {
    let live = [Action::Open { kind: slack(1), axis: Axis::Right, beside: None }, Action::Enter { pane: 1 }];
    let live = || live.iter().cloned().map(Envelope::from);
    let under =
        |over: Envelope| live().chain([capture(None).into(), over, Action::CaptureBack.into()]).collect::<Vec<_>>();
    replay(&under(by(Origin::Socket, Action::ToggleKeys))).unwrap();
    replay(&under(Action::FocusList.into())).unwrap();
}

/// "Add and start" on the draft in the bar changed tabs under visual mode's cursor.
#[test]
fn add_and_start_from_the_bar_waits_where_opening_a_task_waits() {
    replay(&[
        terminal().into(),
        capture(Some(99)).into(),
        Action::FieldEdit { edit: Edit::Insert("fix".into()) }.into(),
        Action::Focus { pane: 1 }.into(),
        Action::ToggleVisual.into(),
        Action::ComposeSubmit { open: true }.into(),
    ])
    .unwrap();
}

/// A task's brief stayed on show when the tasks were read again without its captures.
#[test]
fn a_brief_goes_when_its_task_no_longer_names_a_capture() {
    let mut run = Run::default();
    let steps = [
        Action::Open { kind: Kind::Terminal, axis: Axis::Right, beside: None },
        capture(None),
        Action::AnnotateLeave { title: true },
        Action::FieldEdit { edit: Edit::Insert("fix".into()) },
        Action::ComposeSubmit { open: true },
        Action::BriefShow { id: 1 },
    ];
    steps.into_iter().for_each(|action| run.step(action.into()).unwrap());
    assert_eq!(run.m.brief, Some(1));
    let bare = Task { materials: Vec::new(), ..run.m.tasks.list[0].clone() };
    run.step(Action::RestoreTasks { tasks: vec![bare], floor: 0 }.into()).unwrap();
    assert_eq!(run.m.brief, None);
}

/// → from the Apps list left the filter with the keyboard and no list.
#[test]
fn the_apps_filter_keeps_its_keys_when_focus_moves() {
    replay(&[
        Action::FocusDir { dir: Dir::Left }.sent_by(Origin::User),
        Action::FocusDir { dir: Dir::Right }.sent_by(Origin::User),
        Action::AppsFilter { on: true }.sent_by(Origin::User),
        Action::FocusDir { dir: Dir::Right }.sent_by(Origin::User),
    ])
    .unwrap();
}

// Known, and left: a decision to make, not a slip.

/// A key this build does not know beside a terminal's or an empty pane's `type` is dropped: only
/// an app's `kind` keeps its own. Every other level of the file keeps them.
#[test]
#[ignore = "a decision: `RecordKind::Terminal` and `Empty` get a place for unknown keys, or the format says they have none"]
fn marks_json_keeps_an_unknown_key_of_a_terminals_kind() {
    let text = r#"{"version": 1, "marks": [{"id": "u1", "name": "a", "target": {"pane": {"kind": {"type": "terminal", "future": 7}}}}]}"#;
    let loaded = marks::parse(text).unwrap();
    assert!(marks::write(&loaded.marks, &loaded.extra).contains("future"));
}

#[test]
fn a_workbook_whose_file_names_a_task_opens_as_a_tab_of_no_task() {
    let mut m = Model::default();
    m.update(Action::TaskAdd {
        title: "t".into(),
        priority: 3,
        state: State::Captured,
        tab: None,
        assign: Assign::default(),
    });
    let kept = r#"[{"version": 2, "id": "00000000000000a1", "name": "dri", "tabs": [{"title": "x", "task": 1,
        "layout": {"leaf": 1}, "panes": [{"id": 1, "kind": {"type": "terminal"}, "command": "rm -rf ~"}]}]}]"#;
    m.update(by(Origin::System, restore_workbooks(kept)));
    let effects = m.update(Action::WorkbookOpen { workbook: 1, tab: None, home: None });
    // A terminal and nothing typed into it: no effect carries a command.
    assert_eq!(effects, vec![Effect::Opened(1)]);
    assert_eq!((m.tabs[m.active].task, m.tabs[m.active].title.as_str()), (None, "dri"));
}

/// A system `CloseTab` took the capture's pane off show, which ends annotate mode and keeps the
/// note being typed: the check took that for a change to the drawings. A note being typed is one.
#[test]
fn closing_a_tab_by_the_system_leaves_a_drafts_title_and_drawings_as_they_are() {
    replay(&[
        Action::Open {
            kind: Kind::App { window: 9, title: "w9".into(), bundle: None, name: "app".into() },
            axis: Axis::Right,
            beside: None,
        }
        .sent_by(Origin::User),
        Action::Adopt {
            kind: Kind::App {
                window: 3,
                title: "general".into(),
                bundle: Some("com.safari".into()),
                name: "Slack".into(),
            },
            beside: None,
        }
        .sent_by(Origin::Socket),
        Action::CaptureStart {
            pane: Some(2),
            draft: "".into(),
            word: "w0rdw0rdw0rd".into(),
            facts: vec![],
            images: false,
        }
        .sent_by(Origin::User),
        Action::AnnotatePress { at: [0.5, 0.75], shift: true, alt: true, size: [800.0, f32::NAN as _] }
            .sent_by(Origin::User),
        Action::AnnotateRelease { at: [f32::NAN as _, 0.75] }.sent_by(Origin::User),
        Action::FieldEdit { edit: Edit::Insert("x\t$,\t  ".into()) }.sent_by(Origin::User),
        Action::CloseTab { index: Some(0) }.sent_by(Origin::System),
    ])
    .unwrap();
}

/// Seed 2563: a settings entry that did not resolve (`""`) was removed as a repository's
/// directory, and the settings were to be written for a change that names nothing.
#[test]
fn a_repository_is_never_removed_by_a_path_that_is_no_directory() {
    replay(&[
        Action::SetRepoDirs {
            dirs: RepoDirs {
                dirs: vec![
                    RepoDir { text: "".into(), path: "".into() },
                    RepoDir { text: "  S'".into(), path: "  S'".into() },
                ],
                fixed: false,
            },
        }
        .sent_by(Origin::User),
        Action::RepoRemove { path: "".into() }.sent_by(Origin::User),
    ])
    .unwrap();
}

#[test]
fn a_layout_restored_over_an_emptied_tab_leaves_no_workbook_standing_for_it() {
    replay(&[
        Action::NewTab.sent_by(Origin::User),
        Action::Open { kind: Kind::App { window: 3, title: "w3".into(), bundle: None, name: "app".into() }, axis: Axis::Right, beside: None }.sent_by(Origin::User),
        Action::SelectTab { index: 0 }.sent_by(Origin::User),
        Action::CloseTab { index: None }.sent_by(Origin::User),
        Action::WorkbookSaveAll { uid: "00000000000000a2".into(), cwds: vec![], histories: vec![] }.sent_by(Origin::User),
        Action::Close { pane: None }.sent_by(Origin::User),
        restore_layout("{\"version\":1,\"selected\":2,\"section\":\"repos\",\"drawer\":{\"task\":1,\"open\":false,\"width\":480.0,\"wide\":false},\"tabs\":[{\"title\":\"Tab 2\",\"layout\":null,\"panes\":[],\"workbook\":{\"id\":\"00000000000000a2\",\"tab\":0}},{\"title\":\"Tab 1\",\"layout\":null,\"panes\":[]}]}", "[\"/Users/me/My Notes\",\"relative\"]", "null", false).sent_by(Origin::User),
    ])
    .unwrap();
}

/// Seed 1618: a layout restored after the user had opened the drawer's task listed the task twice
/// on the right rail.
#[test]
fn a_restored_drawer_task_is_on_the_rail_once() {
    replay(&[
        by(Origin::System, Action::OpenCompose { text: "flicker".into() }),
        Action::ComposeSubmit { open: true }.into(),
        restore_layout(
            r#"{"version":1,"selected":null,"section":null,"drawer":{"task":1,"open":false,"width":480.0,"wide":false},"tabs":[{"title":"Tab 1","layout":null,"panes":[]}]}"#,
            "[]",
            "\"/code/app\"",
            true,
        )
        .sent_by(Origin::System),
    ])
    .unwrap();
}

/// Seed 1507: a wheel and a peek check at the clock's last millisecond overflowed the peek's end.
#[test]
fn a_peek_at_the_clocks_end_does_not_overflow() {
    replay(&[
        Action::Open {
            kind: Kind::App { window: 9, title: "w9".into(), bundle: None, name: "app".into() },
            axis: Axis::Right,
            beside: None,
        }
        .into(),
        Action::AppWheel { pane: 1, now: u64::MAX }.into(),
        Action::PeekCheck { now: u64::MAX, over: true, pressed: false, scrolled: None }.into(),
    ])
    .unwrap();
}
