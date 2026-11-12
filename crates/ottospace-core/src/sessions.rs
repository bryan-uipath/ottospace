//! Agent sessions found on this machine that no task has adopted: what the Tasks section lists
//! under "Sessions" (a Claude Code or Codex started by hand in some terminal). The shell reads
//! them, off the main thread, and passes them in. Pure.
//!
//! An "agent" is a saved definition; a running conversation is a "session".
//!
//! What comes in is derived facts only, but a model had a hand in some of it (a label is built
//! from names in a transcript), so every text is checked or cut here before it is kept.

use serde::{Deserialize, Serialize};

use crate::model::PaneId;
use crate::tasks::State;

/// The runners whose sessions are listed.
pub const RUNNERS: [&str; 2] = ["claude", "codex"];

/// An idle session whose last activity is older than this is behind "N older".
pub const OLD_MS: u64 = 86_400_000;

/// The most sessions listed (after those that are a task's or hidden are left out), the most
/// kept of what the shell read, and the longest label shown.
pub const MAX_FOUND: usize = 200;
pub const MAX_KEPT: usize = 2000;
pub const MAX_LABEL: usize = 40;

/// The session a task was adopted from: such a session is the task's, not listed by itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Link {
    /// `claude` or `codex`.
    pub runner: String,
    /// The runner's own id for the session.
    pub id: String,
    /// The state the task last took from its session (running while the session works, needs
    /// you when it went quiet). The task goes on following only while it still has that state:
    /// a state the user or an agent set since is theirs to keep. No path from a transcript is
    /// kept here: the id and the runner say which session it is.
    #[serde(default)]
    pub state: Option<State>,
}

/// A session as the shell read it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    pub runner: String,
    pub id: String,
    pub cwd: String,
    /// The listed repository or worktree `cwd` is in.
    pub repo: String,
    /// When it started, if its file says (milliseconds since 1970).
    #[serde(default)]
    pub started: Option<u64>,
    /// When its file last changed (milliseconds since 1970).
    #[serde(default)]
    pub last_activity: Option<u64>,
    pub active: bool,
    /// Where its work goes now, while it is active: `shell`, `files`, `network`, ...
    #[serde(default)]
    pub station: Option<String>,
    /// What it runs now, from the reader's fixed tables: `cargo test`, `src/main.rs`.
    #[serde(default)]
    pub label: Option<String>,
}

/// A row of the Sessions group, as drawn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Row {
    pub runner: String,
    pub id: String,
    /// The repository or worktree it runs in, by name.
    pub name: String,
    pub active: bool,
    /// While active: `shell · cargo test`.
    pub doing: Option<String>,
    /// How long ago it was last active: `now`, `5m`, `3h`, `2d`.
    pub age: Option<String>,
    /// The terminal pane it runs in, when there is no doubt. A click focuses it.
    pub pane: Option<PaneId>,
}

/// The Sessions group, as drawn. No sessions: no group.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Bar {
    /// Sessions listed, the older ones included.
    pub count: usize,
    pub collapsed: bool,
    /// None while collapsed. The older ones only while they are shown.
    pub rows: Vec<Row>,
    /// How many are behind "N older".
    pub older: usize,
    pub older_open: bool,
}

impl Bar {
    /// How many of `rows` are of the last day: they come first, before "N older" and the older ones.
    pub fn recent(&self) -> usize {
        self.rows.len().saturating_sub(if self.older_open { self.older } else { 0 })
    }
}

/// A text that may be shown: no control characters, at most `max` characters, not empty.
fn shown(text: &str, max: usize) -> Option<String> {
    let text: String = text.chars().filter(|c| !c.is_control()).take(max).collect();
    Some(text.trim().to_string()).filter(|text| !text.is_empty())
}

/// What is kept of what the shell read: sessions of a known runner with a plain id and an
/// absolute directory, each once. Labels are cut. The active ones come first, in an order that
/// does not change as they work (by when each started, then runner and id): two sessions
/// writing in turn must not swap rows under the pointer. Then the idle ones, the most recently
/// active first.
pub fn kept(found: Vec<Found>) -> Vec<Found> {
    let plain = |text: &str, max: usize| !text.is_empty() && text.len() <= max && !text.chars().any(char::is_control);
    let name = |text: &str| text.chars().all(|c| c.is_ascii_lowercase() || c == '_') && plain(text, 16);
    let mut list: Vec<Found> = Vec::new();
    for mut session in found {
        let known = RUNNERS.contains(&session.runner.as_str()) && plain(&session.id, 128);
        let placed = [&session.cwd, &session.repo].iter().all(|dir| dir.starts_with('/') && plain(dir, 1024));
        if !known || !placed || list.iter().any(|s| (&s.runner, &s.id) == (&session.runner, &session.id)) {
            continue;
        }
        session.station = session.station.filter(|station| name(station));
        session.label = session.label.and_then(|label| shown(&label, MAX_LABEL));
        list.push(session);
    }
    list.sort_by(|a, b| {
        let steady = |s: &Found| (s.started, s.runner.clone(), s.id.clone());
        let idle = b.last_activity.cmp(&a.last_activity);
        let within = if a.active { steady(a).cmp(&steady(b)) } else { idle.then_with(|| steady(a).cmp(&steady(b))) };
        b.active.cmp(&a.active).then(within)
    });
    list.truncate(MAX_KEPT);
    list
}

impl Found {
    /// Behind "N older": idle, and last active more than a day before `now` (or never known).
    pub fn old(&self, now: u64) -> bool {
        !self.active && self.last_activity.is_none_or(|at| now.saturating_sub(at) > OLD_MS)
    }

    /// What a task adopted from this session should be, given what it last took from it
    /// (`link`) and what it is now. `None`: leave the task alone. While the session works the
    /// task runs; when it goes quiet a task that was running for it needs the user, once; when
    /// it works again the task runs again. A task whose state is not the one it took from the
    /// session was changed by the user or an agent, and keeps it.
    pub fn follow(&self, link: Option<State>, now: State) -> Option<State> {
        let next = match (self.active, now) {
            (true, State::Running) => return None,
            (true, _) => State::Running,
            (false, State::Running) => State::NeedsYou,
            (false, _) => return None,
        };
        (link == Some(now)).then_some(next)
    }

    /// `shell · cargo test`, `files`, or nothing: only while active.
    pub fn doing(&self) -> Option<String> {
        let station = self.station.as_deref().filter(|_| self.active)?;
        Some(match &self.label {
            Some(label) if label != station => format!("{station} · {label}"),
            _ => station.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(runner: &str, id: &str, cwd: &str, active: bool, at: u64) -> Found {
        let (runner, id, cwd, repo) = (runner.to_string(), id.to_string(), cwd.to_string(), cwd.to_string());
        Found { runner, id, cwd, repo, started: None, last_activity: Some(at), active, station: None, label: None }
    }

    #[test]
    fn what_is_kept_is_checked_cut_and_in_order() {
        let mut long = found("claude", "b", "/code/app", true, 50);
        (long.station, long.label) = (Some("shell".into()), Some(format!("cargo\u{7} test{}", "x".repeat(80))));
        let mut odd = found("codex", "c", "/code/app", true, 60);
        (odd.station, odd.label) = (Some("Not A Station".into()), Some(" \n ".into()));
        let list = kept(vec![
            found("claude", "a", "/code/app", false, 90),
            long,
            odd,
            found("claude", "a", "/code/elsewhere", true, 99), // the same session twice: the first counts
            found("gemini", "d", "/code/app", true, 99),       // not a runner that is listed
            found("claude", "e\u{1b}[31m", "/code/app", true, 99), // an id is a plain name
            found("claude", "f", "code/app", true, 99),        // a directory is absolute
        ]);
        // Active first, in a steady order (runner, then id, when neither says when it started).
        assert_eq!(list.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["b", "c", "a"]);
        assert_eq!((list[1].station.as_deref(), list[1].label.as_deref(), list[1].doing()), (None, None, None));
        let label = list[0].label.clone().unwrap();
        assert!(label.starts_with("cargo test") && label.chars().count() == MAX_LABEL, "{label}");
        assert_eq!(list[0].doing(), Some(format!("shell · {label}")));
        // An idle session says nothing of what it does.
        let mut idle = found("claude", "a", "/code/app", false, 90);
        idle.station = Some("files".into());
        assert_eq!(idle.doing(), None);
    }

    #[test]
    fn active_sessions_keep_their_rows_as_they_write_and_idle_ones_go_by_recency() {
        let started = |id: &str, at: u64, active: bool, last: u64| {
            let mut session = found("claude", id, "/code/app", active, last);
            session.started = Some(at);
            session
        };
        let ids = |list: Vec<Found>| list.into_iter().map(|s| s.id).collect::<Vec<_>>();
        // The one that wrote last is not first: the one that started first is.
        let first =
            kept(vec![started("late", 20, true, 99), started("early", 10, true, 50), started("idle", 5, false, 98)]);
        let then =
            kept(vec![started("late", 20, true, 100), started("early", 10, true, 200), started("idle", 5, false, 98)]);
        assert_eq!(
            (ids(first), ids(then)),
            (vec!["early".into(), "late".into(), "idle".into()], vec!["early".into(), "late".into(), "idle".into()])
        );
        // Idle ones: the most recently active first.
        assert_eq!(ids(kept(vec![started("x", 1, false, 10), started("y", 2, false, 30)])), ["y", "x"]);
    }

    #[test]
    fn a_task_follows_its_session_until_someone_else_sets_its_state() {
        let (working, quiet) = (found("claude", "a", "/a", true, 0), found("claude", "a", "/a", false, 0));
        // Adopted idle (captured), it runs when the session works, needs you when it goes quiet,
        // and runs again when it works again.
        assert_eq!(working.follow(Some(State::Captured), State::Captured), Some(State::Running));
        assert_eq!(quiet.follow(Some(State::Running), State::Running), Some(State::NeedsYou));
        assert_eq!(working.follow(Some(State::NeedsYou), State::NeedsYou), Some(State::Running));
        // Nothing to do: it has the state already, or a quiet session says nothing of a task at rest.
        assert_eq!(working.follow(Some(State::Running), State::Running), None);
        assert_eq!(quiet.follow(Some(State::NeedsYou), State::NeedsYou), None, "needs you once");
        assert_eq!(quiet.follow(Some(State::Captured), State::Captured), None);
        // The user put it on hold, or an agent said done: not the state it took from the session.
        assert_eq!(working.follow(Some(State::NeedsYou), State::Parked), None);
        assert_eq!(quiet.follow(Some(State::Running), State::Done), None);
        assert_eq!(working.follow(None, State::Captured), None, "a link from before this was kept");
    }

    #[test]
    fn old_is_idle_for_more_than_a_day() {
        let now = 10 * OLD_MS;
        assert!(found("claude", "a", "/a", false, now - OLD_MS - 1).old(now));
        assert!(!found("claude", "a", "/a", false, now - OLD_MS).old(now));
        assert!(!found("claude", "a", "/a", true, 0).old(now), "an active one is never old");
        let unknown = Found { last_activity: None, ..found("claude", "a", "/a", false, 0) };
        assert!(unknown.old(now));
    }
}
