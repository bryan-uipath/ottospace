//! Random actions against the model, with what must hold after every one of them (`check`,
//! `Run::step`). Seeded: a run is the same on every machine. A failure prints its seed and the
//! least steps that still fail, as a test to paste under "Regressions" below.
//!
//! `OTTOSPACE_PROP_SEED=<n>` runs that one sequence again.

use super::*;
use crate::ax::{Frame, Source};
use crate::compose::{Known, Names, PathFound};
use crate::fuzz::{self, Rng};
use crate::marks::{Mark, PaneRecord, RecordKind, TabRecord};
use crate::repos::{MAX_DIRS, RepoDir, RepoDirs, Worktree};
use crate::sessions::{Link, Seen};
use crate::settings::ListChange;
use crate::workbook::{self, Workbook, WorkbookId};

const WORDS: &[&str] = &["fix", "zoom", "flicker", "deploy", "x"];
const REPOS: &[&str] = &["app", "canvas", "My Project", "2048", "Ottospace"];
const DIRS: &[&str] = &["/code/app", "/code/canvas", "/code/app-2", "/tmp", "/Users/me/My Notes", "relative"];
const AGENTS: &[&str] = &["claude", "codex", "Claude Code", "claude-opus-5-5", "codex-gpt-6-astra"];
const SKILLS: &[&str] = &["code-review", "deploy"];
const APPS: &[&str] = &["Slack", "Safari", "Finder", "Sketch", "Slides", ""];
const UIDS: &[&str] = &["u1", "u2", "u3", "u4", ""];
/// A workbook's id is its file's name: three that may be one, and some that may not.
const BOOK_UIDS: &[&str] = &["00000000000000a1", "00000000000000a2", "00000000000000a3", "../../x", "A1", ""];
const SESSIONS: &[&str] = &["s1", "s2", "s3"];
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

    fn task(&mut self) -> TaskId {
        let real: Vec<TaskId> = self.m.tasks.list.iter().map(|t| t.id).collect();
        self.one_of(&real, Self::stray)
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

    /// A plain name from `pool`, else hostile text.
    fn name(&mut self, pool: &[&str]) -> String {
        if self.rng.one_in(4) { self.text() } else { self.rng.pick(pool).to_string() }
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
        Link {
            runner: self.name(&sessions::RUNNERS),
            id: self.name(SESSIONS),
            state: self.rng.coin().then(|| self.state()),
        }
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

    fn found(&mut self) -> Found {
        Found {
            runner: self.name(&sessions::RUNNERS),
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
        }
    }

    /// A session by (runner, id): a listed one half the time.
    fn session(&mut self) -> (String, String) {
        let real: Vec<(String, String)> = self.m.found.iter().map(|s| (s.runner.clone(), s.id.clone())).collect();
        self.one_of(&real, |g| (g.name(&sessions::RUNNERS), g.name(SESSIONS)))
    }

    /// Controls a frame drew. A few; one time in 40, hundreds: three-letter labels.
    fn targets(&mut self) -> Vec<Target> {
        let n = if self.rng.one_in(40) { 400 } else { self.rng.below(7) };
        let place = |g: &mut Self| g.rng.pick(&[0., 20., 30., 150., -4., f64::NAN, f64::INFINITY]);
        let target = |g: &mut Self, i: usize| Target {
            frame: Frame { x: place(g), y: place(g), w: place(g), h: place(g) },
            source: match g.rng.below(8) {
                0 => Source::App { pane: g.pane(), path: vec![0, i], role: "AXButton".into() },
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

    fn letters(&mut self) -> String {
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
    |g| Action::TaskOpen { id: g.rng.coin().then(|| g.task()) },
    |_| Action::TaskLeave,
    |g| Action::SelectVisibleTab { index: g.index(g.m.visible_tabs().len()) },
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
        Action::ToggleGroup { heading: g.rng.pick(&[NeedsYou, Open, Running, Captured, OnHold, Done, Sessions]) }
    },
    |g| Action::SetSessions { found: (0..g.rng.below(5)).map(|_| g.found()).collect() },
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
    |_| Action::ToggleOlderSessions,
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
    |g| Action::WorkbookSave { uid: g.rng.pick(BOOK_UIDS).to_string(), cwds: g.read(DIRS) },
    |g| Action::WorkbookAddTab { workbook: g.workbook(), cwds: g.read(DIRS) },
    |g| Action::WorkbookSaveAll { uid: g.rng.pick(BOOK_UIDS).to_string(), cwds: g.read(DIRS) },
    |g| Action::WorkbookOpen { workbook: g.workbook(), tab: g.book_tab(), home: g.rng.coin().then(|| g.name(DIRS)) },
    |g| Action::WorkbookEdit { workbook: g.workbook(), tab: g.book_tab() },
    |g| Action::WorkbookRemove { workbook: g.workbook(), tab: g.book_tab() },
    |g| Action::WorkbookFold { workbook: g.workbook() },
    |_| Action::WorkbookUndo,
];

/// The way out of each box and mode. Picked one time in 8: with every action as likely as any
/// other, a run would sit behind the key reference for 40 steps.
const WAYS_OUT: &[Action] = &[
    Action::ClosePicker,
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

fn unique<T: PartialEq>(list: &[T]) -> bool {
    list.iter().enumerate().all(|(i, item)| !list[..i].contains(item))
}

/// What must hold of a model at rest, whatever was done to it.
fn check(m: &Model) -> Result<(), String> {
    // Tabs, their trees and the panes.
    ensure!(!m.tabs.is_empty(), "no tabs");
    ensure!(m.active < m.tabs.len(), "tab {} is on show, of {}", m.active, m.tabs.len());
    let tab_ids: Vec<u32> = m.tabs.iter().map(|tab| tab.id).collect();
    ensure!(unique(&tab_ids), "two tabs share an id: {tab_ids:?}");
    let on_show = &m.tabs[m.active];
    ensure!(on_show.layout.is_none() && on_show.focused.is_none(), "the tab on show keeps a second tree or focus");
    ensure!(
        on_show.task == m.current_task,
        "the tab on show is of {:?}, the bar of {:?}",
        on_show.task,
        m.current_task
    );
    ensure!(m.visible_tabs().contains(&m.active), "the tab on show is not on the bar");
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
            Mode::Pick | Mode::Compose => true,
            Mode::Annotate => m.note().is_some(),
            Mode::List => m.app_lists.filtering,
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
        ensure!(
            sneak.targets.iter().all(|t| matches!(t.target.source, Source::Own { .. })),
            "a label is on an app's node"
        );
        let covered = boxed || m.keys || m.live.is_some() || m.visual.is_some() || m.mark_input.is_some();
        ensure!(!covered && !m.ax_overlay && m.confirm.is_none(), "sneak mode is on under something else");
        let _ = sneak.visible().count();
    }
    if let Some(confirm) = &m.confirm {
        let loses = !confirm.running.is_empty() || m.quit_discards() > 0;
        ensure!(loses, "the quit question names no terminal, and no draft holds a capture");
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
    let briefed =
        |id: TaskId| m.current_task == Some(id) && m.tasks.get(id).is_some_and(|task| !task.materials.is_empty());
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
    ensure!(m.current_task.is_none_or(|id| m.tasks.get(id).is_some()), "the current task {:?} is gone", m.current_task);
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

/// The marks' file: written and read again, every mark is there as it was, and so are the keys
/// this build does not know.
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
    let _ = (m.task_sections(m.now), m.sessions_bar(), m.compose_spans(), m.home(&[]), m.mark_rows());
    let _ = (m.keyboard_terminal(), m.due(m.now), m.followed_sessions(), m.compose_read(), m.needs());
    let _ = (m.workbook_rows(), m.workbook_save_refusal(), m.workbook_of_tab(), m.workbooks.offered());
    let _: Vec<Vec<String>> = m.workbooks.list.iter().map(|book| m.workbook_gone(book.id, None)).collect();
    let _: Vec<Option<String>> = m.workbooks.list.iter().map(|book| m.workbook_add_refusal(book.id)).collect();
    let _ = m.workbook_save_all_refusal();
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
}

impl Run {
    /// Takes a step, then checks the model, the effects and what the step may have changed.
    fn step(&mut self, sent: Envelope) -> Result<(), String> {
        self.steps.push(sent.clone());
        let Envelope { action, origin } = sent.clone();
        let m = &mut self.m;
        let refused = m.refusal(&action, origin).map(|_| format!("{m:?}"));
        let panes: Vec<PaneId> = m.panes.iter().map(|p| p.id).collect();
        let sneak = m.sneak.as_ref().map(|sneak| sneak.typed.clone());
        let labelled: Vec<String> =
            m.sneak.iter().flat_map(|s| s.targets.iter()).map(|t| format!("{:?}", t.target.source)).collect();
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
            (Action::ComposeSubmit { open: false }, Some(row)) => m.changes(row),
            _ => false,
        };
        // While the bar asks before discarding, ↵ is the answer: it files nothing.
        let answers = m.capture.as_ref().is_some_and(|draft| draft.asked);
        let submits = matches!(action, Action::ComposeSubmit { .. });
        let filing = m.compose_line().filter(|_| submits && !takes && !answers && refused.is_none());
        // A draft with captures is filed under a name of its own when none was typed.
        let named = m.capture_title();
        let filing = filing.map(|line| (line.clone(), m.draft(line), m.compose_host()));
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
        let drafted: Option<(Option<String>, Vec<Vec<crate::capture::Shape>>)> = m
            .capture
            .as_ref()
            .map(|draft| (m.compose_line().cloned(), draft.materials.iter().map(|m| m.marks.shapes.clone()).collect()));
        let line_before = m.compose_line().cloned();
        let composing = m.composing();

        let effects = m.update(sent);
        check(m)?;
        if !self.sparse || self.steps.len().is_multiple_of(8) {
            check_state(m)?;
            check_file(m)?;
        }

        // A draft's title and drawings are the user's: nobody else changes them, and a line
        // someone else wrote is never a draft's title.
        if let (Some(before), Some(draft), false) = (&drafted, &m.capture, origin == Origin::User) {
            let now = (m.compose_line().cloned(), draft.materials.iter().map(|m| m.marks.shapes.clone()).collect());
            ensure!(*before == now, "a draft's title or drawings changed, and not by the user: {before:?}");
        }
        ensure!(m.capture.is_none() || !m.compose_theirs, "a draft's title holds what a client wrote");
        let held = m.capture.as_ref().map_or(0, |draft| draft.materials.len());
        ensure!(unasked == 0 || held == unasked, "a draft's {unasked} captures went with no question");
        // Only the user starts a capture: a client's or a timer's is refused, whatever the mode.
        let foreign = matches!(action, Action::CaptureStart { .. }) && origin != Origin::User;
        ensure!(!foreign || refused.is_some(), "a capture that is not the user's was not refused");
        // An action refused for who sent it changes nothing at all.
        if let Some(before) = &refused {
            ensure!(
                effects.is_empty() && format!("{m:?}") == *before,
                "a refused action changed the model, or had effects"
            );
        }
        let user = origin == Origin::User;
        let sneak_own = matches!(
            action,
            Action::Sneak { .. }
                | Action::SneakEnter { .. }
                | Action::SneakDrawn { .. }
                | Action::SneakType { .. }
                | Action::SneakBackspace
                | Action::LeaveSneak
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
                // These start the typing of a workbook's name.
                | Action::WorkbookSave { .. }
                | Action::WorkbookSaveAll { .. }
                | Action::WorkbookEdit { .. }
        );
        // A name being typed is a text field: an edit of it is the wait's own, whoever sends it.
        let typing = mark_input.as_ref().is_some_and(|input| input.text().is_some());
        let mark_own = mark_own || (typing && matches!(action, Action::FieldEdit { .. }));
        // Only the user, or the mode's own keys, end sneak mode, the quit question or a mark's wait.
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
        if matches!(action, Action::ComposeAttach { .. } | Action::ComposeRef { .. }) && !composing {
            ensure!(m.compose_line() == line_before.as_ref(), "the line changed with no keyboard: {line_before:?}");
        }
        if let Some((line, draft, host)) = filing {
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
                // On the home screen the line stays for the next task, empty, unless the task is started.
                let stays = host == Some(Host::Home) && !matches!(action, Action::ComposeSubmit { open: true });
                let left = if stays { m.compose_line().is_some_and(String::is_empty) } else { m.compose.is_none() };
                ensure!(filed.len() == 1 && left, "{line:?} filed {} tasks; the box is {:?}", filed.len(), m.compose);
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
                Effect::Live(pane) => {
                    ensure!(self.shell.live.is_none(), "pane {pane} goes live over {:?}", self.shell.live);
                    self.shell.live = Some(*pane);
                }
                Effect::Parked(pane) => {
                    ensure!(self.shell.live == Some(*pane), "pane {pane} is parked and {:?} was live", self.shell.live);
                    self.shell.live = None;
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
            |_| Action::TaskOpen { id: Some(3) },
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
                };
                Action::SetSessions {
                    found: vec![s.clone(), Found { id: "s2".into(), active: false, last_activity: Some(5), ..s }],
                }
            },
            |_| Action::ToggleSection { section: Section::Tasks },
            |_| Action::TaskLeave,
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
    ];
    const ENUM_OF: &[(&str, &str)] = &[
        ("axis", "Axis"),
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
    ];
    // A mark is long, and half of it is JSON already: it is given as its file's text.
    if let Action::RestoreMarks { marks, extra } = &sent.action {
        let (marks, extra) =
            (serde_json::to_string(marks).unwrap_or_default(), serde_json::to_string(extra).unwrap_or_default());
        return format!("restore_marks({marks:?}, {extra:?}).sent_by(Origin::{:?})", sent.origin);
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

/// `RestoreMarks` from the JSON of its marks and of the file's own keys, as `code` writes it.
fn restore_marks(marks: &str, extra: &str) -> Action {
    let read = "a regression's marks are JSON";
    Action::RestoreMarks {
        marks: serde_json::from_str(marks).expect(read),
        extra: serde_json::from_str(extra).expect(read),
    }
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
        by(Origin::System, Action::TaskLeave),
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
    ];
    steps.into_iter().for_each(|action| run.step(action.into()).unwrap());
    assert_eq!(run.m.brief, Some(1));
    let bare = Task { materials: Vec::new(), ..run.m.tasks.list[0].clone() };
    run.step(Action::RestoreTasks { tasks: vec![bare], floor: 0 }.into()).unwrap();
    assert_eq!(run.m.brief, None);
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
    assert_eq!((m.tabs[m.active].task, m.current_task, m.tabs[m.active].title.as_str()), (None, None, "dri"));
}
