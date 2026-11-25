//! The status bar's segments, left to right: where you are (the focused pane's folder and
//! branch, or its app), the mode and messages, what's next (the meeting soon or on now), and
//! who's working (agents in every workspace). What each says, and which shrink first when the
//! window is narrow.

use crate::calendar::{Calendar, Stage};
use crate::model::{Kind, Model, Pane};
use crate::tasks::Segments;

/// Why a client's click on the folder copies nothing: the clipboard is the user's.
pub const USERS_COPY: &str = "only the user's click copies the folder's path";
/// A meeting shows from this long before its start.
pub const MEETING_FROM: u64 = 30 * 60_000;
/// The narrowest the folder is cut to before the meeting and the agents shorten.
pub const FOLDER_MIN: f32 = 80.;

/// What the focused pane is: a folder (a terminal's, a file's), or an app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    Folder(String),
    App(String),
}

impl Model {
    /// The pane the bar names: the focused one, while a tab shows (not Home).
    pub fn status_pane(&self) -> Option<&Pane> {
        self.layout.as_ref()?;
        self.pane(self.focused?)
    }
}

/// Where the focused `pane` is. `cwd`: its terminal's directory now, as the shell read it. An
/// empty pane, or a terminal whose directory is not known: nowhere.
pub fn place(pane: Option<&Pane>, cwd: Option<&str>) -> Option<Place> {
    match &pane?.kind {
        Kind::Terminal => cwd.map(|cwd| Place::Folder(cwd.to_string())),
        Kind::File { path } => {
            let dir = std::path::Path::new(path).parent()?.to_str()?;
            Some(Place::Folder(dir.to_string()))
        }
        Kind::Explorer { root } => Some(Place::Folder(root.clone())),
        Kind::App { name, .. } | Kind::Slot { name, .. } => Some(Place::App(name.clone())),
        Kind::Empty | Kind::Decision => None,
    }
}

/// A folder as the bar says it: under `home` with `~` (`~/code/app`), or with `name_only` its
/// last part (`app`; `~` for home itself).
pub fn folder_text(path: &str, home: Option<&str>, name_only: bool) -> String {
    let path = match path.trim_end_matches('/') {
        "" => "/",
        trimmed => trimmed,
    };
    let home = home.map(|h| h.trim_end_matches('/')).filter(|h| !h.is_empty());
    let under = home.and_then(|home| path.strip_prefix(home)).filter(|rest| rest.is_empty() || rest.starts_with('/'));
    if name_only {
        return match under {
            Some("") => "~".into(),
            _ if path == "/" => path.into(),
            _ => crate::repos::name_of(path),
        };
    }
    match under {
        Some(rest) => format!("~{rest}"),
        None => path.to_string(),
    }
}

/// The meeting the bar names: from `MEETING_FROM` before its start, and while it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meeting {
    pub id: u64,
    pub title: String,
    /// Minutes to its start, rounded up; `None`: it runs.
    pub in_min: Option<u64>,
    /// It runs and has a link to join.
    pub join: bool,
}

impl Meeting {
    /// `Team sync in 12 min`, `Team sync now`; `short`: `12 min`, `now`. Join is drawn apart.
    pub fn text(&self, short: bool) -> String {
        let when = match self.in_min {
            Some(n) => format!("{n} min"),
            None => "now".into(),
        };
        match (short, self.in_min) {
            (true, _) => when,
            (false, Some(_)) => format!("{} in {when}", self.title),
            (false, None) => format!("{} {when}", self.title),
        }
    }
}

/// The meeting at `now`: the one the band's now card names (`Calendar::next`), if it runs or
/// starts within `MEETING_FROM`.
pub fn meeting(calendar: &Calendar, now: u64) -> Option<Meeting> {
    // `next` looks past midnight only `APPROACHING` ahead: a meeting at 00:15 counts from 23:45.
    let after = || {
        let soon = |e: &&crate::calendar::Event| !e.all_day && e.start > now && e.start - now <= MEETING_FROM;
        calendar.events.iter().find(soon).map(|e| (e, crate::calendar::stage(e, now)))
    };
    let (event, stage) = calendar.next(now).or_else(after).filter(|_| calendar.on)?;
    let in_min = match stage {
        Stage::Running => None,
        _ if event.start.saturating_sub(now) <= MEETING_FROM => Some(event.start.saturating_sub(now).div_ceil(60_000)),
        _ => return None,
    };
    let join = in_min.is_none() && event.link.as_deref().is_some_and(crate::calendar::joinable);
    Some(Meeting { id: event.id, title: event.title.clone(), in_min, join })
}

/// Agents in every workspace: tasks that run, and that wait on the user. `working` counts the
/// sessions at work in a terminal too (`in_terminal`), no task's: each agent is counted once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Agents {
    pub working: usize,
    pub in_terminal: usize,
    pub needs_you: usize,
}

/// What a part of the agents says: how it is coloured, and where a click on it goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Working,
    /// Agents in a terminal: a click goes to the first one.
    Terminal,
    NeedsYou,
    /// `No agents`.
    None,
}

/// Whether the bar may run `git status` in the checkout whose top folder is `top` (resolved):
/// only if it is one the user listed in Repos, or a worktree of one (`roots`, resolved). Not a
/// repository nested inside one, nor a link to another: a repository's own config and
/// attributes can run programs (`core.fsmonitor`, filters). Elsewhere the branch alone, from `HEAD`.
pub fn reads_changes(top: &str, roots: &[String]) -> bool {
    roots.iter().any(|root| root == top)
}

impl Model {
    /// The folders `reads_changes` allows: each listed repository's and worktree's.
    pub fn checkout_roots(&self) -> Vec<String> {
        self.repo_names().map(|(_, path)| path.to_string()).collect()
    }

    pub fn agents(&self) -> Agents {
        let Segments { needs_you, running, .. } = Segments::of(&self.tasks.inbox());
        if self.features.on(crate::features::Feature::TerminalTasks) {
            // The terminal sessions listed with the tasks, as their cards say: an ended one, nowhere.
            let cards = self.session_cards();
            let count = |state| cards.iter().filter(|c| c.state == state).count();
            let (waiting, in_terminal) =
                (count(crate::sessions::CardState::NeedsYou), count(crate::sessions::CardState::Running));
            return Agents { working: running + in_terminal, in_terminal, needs_you: needs_you + waiting };
        }
        let in_terminal = self.in_terminal().len();
        // A session in a terminal that waits on you counts as a task that needs you.
        Agents { working: running + in_terminal, in_terminal, needs_you: needs_you + self.waits_unadopted().count() }
    }

    /// The agents in a terminal, as their hover lists them: `Fix the form · claude`, else the
    /// repository's name for the title.
    pub fn in_terminal_names(&self) -> Vec<String> {
        if self.features.on(crate::features::Feature::TerminalTasks) {
            let running = self.session_cards().into_iter().filter(|c| c.state == crate::sessions::CardState::Running);
            return running.map(|c| format!("{} · {}", c.row.title(), c.row.runner)).collect();
        }
        let name = |s: &crate::sessions::Found| s.title.clone().unwrap_or_else(|| self.session_name(s));
        self.in_terminal().into_iter().map(|s| format!("{} · {}", name(s), s.runner)).collect()
    }
}

impl Agents {
    /// Its parts: `● 2 working`, `● 1 needs you` (short: `● 2`, `● 1`); none counted: `No
    /// agents`. Agents in a terminal are named only beside a task's: `● 3 working`, `1 in a
    /// terminal`; or alone: `● 1 working in a terminal`. Short, the total alone.
    pub fn parts(&self, short: bool) -> Vec<(String, Part)> {
        let part = |n: usize, word: &str, part: Part| {
            let text = if short { format!("● {n}") } else { format!("● {n} {word}") };
            (n > 0).then_some((text, part))
        };
        let tasks = self.working - self.in_terminal;
        let working = match (tasks, self.in_terminal) {
            (_, 0) => part(self.working, "working", Part::Working),
            (0, _) => part(self.working, "working in a terminal", Part::Terminal),
            _ => part(self.working, "working", Part::Working),
        };
        let terminal = (!short && tasks > 0 && self.in_terminal > 0)
            .then(|| (format!("{} in a terminal", self.in_terminal), Part::Terminal));
        let parts: Vec<_> =
            [working, terminal, part(self.needs_you, "needs you", Part::NeedsYou)].into_iter().flatten().collect();
        match parts.is_empty() {
            true => vec![("No agents".into(), Part::None)],
            false => parts,
        }
    }

    /// The parts as one line: `● 2 working · ● 1 needs you`, `● 2 ● 1`.
    pub fn text(&self, short: bool) -> String {
        let parts: Vec<String> = self.parts(short).into_iter().map(|(text, _)| text).collect();
        parts.join(if short { " " } else { " · " })
    }
}

/// Each segment's width drawn whole and shortened, in points. One not shown: 0 and 0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Widths {
    pub folder: f32,
    /// The folder's name alone, with its branch.
    pub folder_name: f32,
    pub meeting: f32,
    pub meeting_short: f32,
    pub agents: f32,
    pub agents_short: f32,
}

/// How the segments are drawn in the room the bar has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fit {
    /// The folder's name alone; cut further by the bar if still too wide.
    pub folder_name: bool,
    pub meeting_short: bool,
    pub agents_short: bool,
}

impl Fit {
    /// How many steps it took: 0 (all whole) to 3.
    pub fn steps(&self) -> usize {
        [self.folder_name, self.meeting_short, self.agents_short].iter().filter(|s| **s).count()
    }
}

/// The shortening that fits `room` (the bar less its middle and gaps): the folder first (its
/// name alone, then cut to `FOLDER_MIN`), then the meeting (`12 min`), then the agents (`● 2 ● 1`).
pub fn fit(room: f32, w: Widths) -> Fit {
    let fits = |folder: f32, meeting: f32, agents: f32| folder + meeting + agents <= room;
    let cut = w.folder_name.min(FOLDER_MIN);
    match () {
        _ if fits(w.folder, w.meeting, w.agents) => Fit::default(),
        _ if fits(cut, w.meeting, w.agents) => Fit { folder_name: true, ..Fit::default() },
        _ if fits(cut, w.meeting_short, w.agents) => {
            Fit { folder_name: true, meeting_short: true, agents_short: false }
        }
        _ => Fit { folder_name: true, meeting_short: true, agents_short: true },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::{BACK, DAYS, Event};
    use crate::model::Action;
    use crate::tasks::Assign;
    use crate::tasks::State;

    const M: u64 = 60_000;
    /// 09:00 of a day, in ms.
    const NINE: u64 = 1_794_560_400_000;

    fn pane(kind: Kind) -> Pane {
        Pane { id: 1, kind, cwd: None, fit: None, owned: false }
    }

    #[test]
    fn a_terminal_is_its_folder_a_file_its_folder_an_app_its_name_and_an_empty_pane_nowhere() {
        let terminal = pane(Kind::Terminal);
        assert_eq!(place(Some(&terminal), Some("/code/app")), Some(Place::Folder("/code/app".into())));
        assert_eq!(place(Some(&terminal), None), None, "its directory not read yet");
        let file = pane(Kind::File { path: "/code/app/README.md".into() });
        assert_eq!(place(Some(&file), Some("/elsewhere")), Some(Place::Folder("/code/app".into())));
        let app = Kind::App { window: 7, title: "Inbox".into(), bundle: None, name: "Zoom Workplace".into() };
        assert_eq!(place(Some(&pane(app)), None), Some(Place::App("Zoom Workplace".into())));
        let slot = Kind::Slot { name: "Slack".into(), bundle: None };
        assert_eq!(place(Some(&pane(slot)), None), Some(Place::App("Slack".into())));
        assert_eq!(place(Some(&pane(Kind::Empty)), Some("/code")), None);
        assert_eq!(place(None, Some("/code")), None);
    }

    #[test]
    fn the_bar_names_the_focused_pane_and_nothing_on_home() {
        let mut m = Model::default();
        assert_eq!(m.status_pane(), None, "Home");
        m.update(Action::Split { axis: crate::Axis::Right, kind: Kind::Terminal });
        let focused = m.focused;
        assert_eq!(m.status_pane().map(|p| p.id), focused);
        m.update(Action::Close { pane: None });
        m.update(Action::Close { pane: None });
        assert_eq!((m.layout.is_none(), m.status_pane()), (true, None));
    }

    #[test]
    fn only_the_users_click_copies_the_folder() {
        use crate::model::{Effect, Origin};
        let mut m = Model::default();
        let copy = || Action::CopyPath { path: "/code/app".into() };
        assert_eq!(m.update(copy().sent_by(Origin::User)), [Effect::Clip { text: "/code/app".into() }]);
        for origin in [Origin::Socket, Origin::System] {
            assert_eq!(m.refusal(&copy(), origin), Some(USERS_COPY));
            assert_eq!(m.update(copy().sent_by(origin)), []);
        }
    }

    #[test]
    fn changes_are_read_only_in_a_checkout_listed_in_repos() {
        use crate::repos::{Repo, Worktree};
        let mut m = Model::default();
        let worktree = Worktree { name: "app-2".into(), path: "/code/app-2".into(), branch: None };
        let repo = Repo { name: "app".into(), path: "/code/app".into(), branch: None, worktrees: vec![worktree] };
        m.update(Action::SetRepos { repos: vec![repo] });
        let roots = m.checkout_roots();
        assert!(reads_changes("/code/app", &roots) && reads_changes("/code/app-2", &roots));
        // A repository nested in a listed one has a top of its own: not listed.
        for elsewhere in ["/code/app/vendor/evil", "/code/application", "/code", "/tmp/cloned", "/code/app/../evil"] {
            assert!(!reads_changes(elsewhere, &roots), "{elsewhere}");
        }
    }

    #[test]
    fn a_folder_under_home_starts_with_a_tilde_and_its_short_form_is_its_name() {
        let home = Some("/Users/me");
        assert_eq!(folder_text("/Users/me/code/app", home, false), "~/code/app");
        assert_eq!(folder_text("/Users/me/code/app/", home, true), "app");
        assert_eq!(folder_text("/Users/me", home, false), "~");
        assert_eq!(folder_text("/Users/me", home, true), "~");
        assert_eq!(folder_text("/Users/meg/x", home, false), "/Users/meg/x", "not under home");
        assert_eq!(folder_text("/tmp", None, false), "/tmp");
        assert_eq!(folder_text("/", home, true), "/");
    }

    fn event(id: u64, start: u64, minutes: u64, link: Option<&str>) -> Event {
        Event {
            id,
            start,
            end: start + minutes * M,
            all_day: false,
            title: "Team sync".into(),
            location: String::new(),
            link: link.map(str::to_string),
            show: crate::ics::Show::Busy,
        }
    }

    fn calendar(events: Vec<Event>) -> Calendar {
        let days: [u64; DAYS] =
            std::array::from_fn(|n| NINE - 9 * 60 * M + (n as u64) * 24 * 60 * M - BACK as u64 * 24 * 60 * M);
        let mut c = Calendar::default();
        c.on = true;
        c.read(Some(events), days, NINE - 60 * M);
        c
    }

    #[test]
    fn a_meeting_shows_from_30_minutes_before_then_now_with_join_while_it_runs() {
        let link = Some("https://example.zoom.us/j/1");
        let c = calendar(vec![event(1, NINE, 30, link)]);
        let at = |now: u64| meeting(&c, now).map(|m| (m.text(false), m.text(true), m.join));
        assert_eq!(at(NINE - 31 * M), None, "too far");
        assert_eq!(at(NINE - 30 * M), Some(("Team sync in 30 min".into(), "30 min".into(), false)));
        assert_eq!(at(NINE - 12 * M + 1), Some(("Team sync in 12 min".into(), "12 min".into(), false)), "rounded up");
        assert_eq!(at(NINE - 1), Some(("Team sync in 1 min".into(), "1 min".into(), false)));
        assert_eq!(at(NINE), Some(("Team sync now".into(), "now".into(), true)));
        assert_eq!(at(NINE + 30 * M), None, "over");
        // No link, nothing to join; a calendar turned off, nothing at all.
        let plain = calendar(vec![event(1, NINE, 30, None)]);
        assert_eq!(meeting(&plain, NINE).map(|m| m.join), Some(false));
        let mut off = c.clone();
        off.on = false;
        assert_eq!(meeting(&off, NINE), None);
    }

    #[test]
    fn the_next_one_soon_wins_over_one_that_runs_as_on_the_bands_card() {
        let c = calendar(vec![event(1, NINE, 60, None), event(2, NINE + 30 * M, 30, None)]);
        assert_eq!(meeting(&c, NINE + 10 * M).map(|m| (m.id, m.in_min)), Some((1, None)), "20 min off: the one on");
        assert_eq!(meeting(&c, NINE + 22 * M).map(|m| (m.id, m.in_min)), Some((2, Some(8))));
        // Past midnight, from 30 minutes before: the day's list ends at midnight.
        let midnight = NINE + 15 * 60 * M;
        let late = calendar(vec![event(4, midnight + 15 * M, 30, None)]);
        assert_eq!(meeting(&late, midnight - 5 * M).map(|m| m.in_min), Some(Some(20)));
        assert_eq!(meeting(&late, midnight - 16 * M), None);
        let all_day = Event { all_day: true, ..event(3, NINE, 60, None) };
        assert_eq!(meeting(&calendar(vec![all_day]), NINE + M), None, "an all-day entry is no meeting");
    }

    #[test]
    fn agents_are_the_tasks_that_run_and_that_need_you_in_every_workspace() {
        let mut m = Model::default();
        assert_eq!(m.agents(), Agents::default());
        for (title, state) in [("a", State::Running), ("b", State::Running), ("c", State::NeedsYou), ("d", State::Done)]
        {
            let add = Action::TaskAdd { title: title.into(), priority: 1, state, tab: None, assign: Assign::default() };
            m.update(add.sent_by(crate::model::Origin::Socket));
        }
        assert_eq!(m.agents(), Agents { working: 2, in_terminal: 0, needs_you: 1 });
    }

    #[test]
    fn the_agents_say_how_many_run_and_wait_or_that_there_are_none() {
        let both = Agents { working: 2, in_terminal: 0, needs_you: 1 };
        assert_eq!(both.text(false), "● 2 working · ● 1 needs you");
        assert_eq!(both.text(true), "● 2 ● 1");
        assert_eq!(both.parts(true), [("● 2".into(), Part::Working), ("● 1".into(), Part::NeedsYou)]);
        assert_eq!(Agents { working: 0, in_terminal: 0, needs_you: 3 }.text(false), "● 3 needs you");
        assert_eq!(Agents::default().parts(false), [("No agents".into(), Part::None)]);
    }

    #[test]
    fn agents_in_a_terminal_are_named_apart_only_beside_a_tasks() {
        let split = Agents { working: 3, in_terminal: 1, needs_you: 1 };
        assert_eq!(split.text(false), "● 3 working · 1 in a terminal · ● 1 needs you");
        assert_eq!(split.parts(false)[1], ("1 in a terminal".into(), Part::Terminal));
        assert_eq!(split.text(true), "● 3 ● 1", "short: the total");
        let alone = Agents { working: 1, in_terminal: 1, needs_you: 0 };
        assert_eq!(alone.parts(false), [("● 1 working in a terminal".into(), Part::Terminal)]);
        assert_eq!(alone.text(true), "● 1");
    }

    const WIDE: Widths =
        Widths { folder: 200., folder_name: 120., meeting: 150., meeting_short: 50., agents: 180., agents_short: 60. };

    #[test]
    fn the_folder_shortens_first_then_the_meeting_then_the_agents() {
        let at = |room: f32| fit(room, WIDE).steps();
        assert_eq!(at(530.), 0, "all whole");
        assert_eq!(at(529.), 1, "the folder's name alone");
        // Its name is cut down to FOLDER_MIN before the meeting shortens.
        assert_eq!(at(FOLDER_MIN + 150. + 180.), 1);
        assert_eq!(at(FOLDER_MIN + 150. + 180. - 1.), 2, "then `12 min`");
        assert_eq!(at(FOLDER_MIN + 50. + 180.), 2);
        assert_eq!(at(FOLDER_MIN + 50. + 180. - 1.), 3, "then `● 2 ● 1`");
        assert_eq!(fit(0., WIDE), Fit { folder_name: true, meeting_short: true, agents_short: true });
    }

    #[test]
    fn a_segment_not_shown_takes_no_room() {
        let no_meeting = Widths { meeting: 0., meeting_short: 0., ..WIDE };
        assert_eq!(fit(380., no_meeting).steps(), 0);
        // A short name is not cut further: what is left goes to the others.
        let short = Widths { folder: 60., folder_name: 30., ..WIDE };
        assert_eq!(fit(30. + 150. + 180., short), Fit { folder_name: true, ..Fit::default() });
        assert_eq!(fit(1_000., Widths::default()), Fit::default());
    }
}
