//! How a conversation's task ends (the canvas "Later vs Done"): ✓ Done, the task is finished;
//! Later, it is put away on hold, to be resumed. While a turn runs the header has no Done: the
//! composer's Stop stops that turn (the owner: one stop button). Both Done and Later stop its
//! agent and take it off the drawer's rail.
//! Only the user's own click or key does any of it. Design: `docs/ai/done.md`.

use serde::Serialize;

use super::{Action, Confirm, Effect, Model};
use crate::notice::{self, Notice};
use crate::talk::Subject;
use crate::tasks::{State, TaskId};
use crate::transcript::{Entry, View};

/// Why a client's Done, Later, Undo or answer to "Stop it and put it away?" is refused.
pub const USERS_ALONE: &str = "only the user marks a task done or puts it away";
/// Why Done does nothing while a turn runs: the composer's Stop is for that.
pub const TURN_RUNS: &str = "its turn runs: Stop it first";
/// Why Done kept the task's own worktree, when it was not git that said.
pub const KEPT_TERMINAL: &str = "Kept the worktree: a terminal is in it";
pub const KEPT_TASK: &str = "Kept the worktree: another task works in it";
/// Why a client's word on the day is refused.
pub const DAY_SHELLS: &str = "only ottospace says when the day began";
/// Why a client's report of a worktree removed is refused.
pub const SHELLS_ALONE: &str = "only ottospace says what git did with a worktree";
/// How long "Done · <task>" offers its Undo, in milliseconds: as every other Undo.
pub const DONE_MS: u64 = crate::tasks::NOTICE_MS;

/// The tasks the user put away and the Done that may be taken back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Finishing {
    /// Done or Later by the user: off the rail, never in front, until they open it again.
    pub(crate) away: Vec<TaskId>,
    pub offer: Option<DoneOffer>,
    /// The worktrees an Undo asked git to add back, as asked: their answer opens nothing.
    pub(crate) readding: Vec<String>,
    /// The worktrees Done asked git to remove, by task: only their answer counts.
    pub(crate) removing: Vec<(TaskId, String)>,
    /// The workspaces whose default fell back to the repository when a task's worktree went.
    pub(crate) fell_back: Vec<(TaskId, Vec<crate::workbook::WorkbookId>)>,
    /// Done tasks opened again whose worktree git adds back: their session goes on once it is.
    pub(crate) resume: Vec<TaskId>,
}

/// "Done · <task>" on the notice bar, with Undo, until `until` (ms since 1970).
#[derive(Clone, Debug, PartialEq)]
pub struct DoneOffer {
    pub task: TaskId,
    pub title: String,
    /// Its state before: what Undo gives back.
    pub(crate) before: State,
    pub until: u64,
    /// Why its own worktree was kept: "Kept the worktree: 3 uncommitted changes".
    pub kept: Option<String>,
}

/// What a task's conversation header offers at its right end, given its agent (`Model::finish`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Buttons {
    /// Later: always, but while its agent stops.
    pub later: bool,
    /// ✓ Done: once its agent idles. Mid-turn the composer's Stop is the one stop.
    pub done: bool,
}

impl Model {
    /// True if the user put the task away (Done or Later) and has not opened it since.
    pub fn put_away(&self, id: TaskId) -> bool {
        self.finish.away.contains(&id)
    }

    /// A native turn runs.
    fn native_turn(&self, subject: &Subject) -> bool {
        self.chat(subject).is_some_and(|chat| chat.transcript.live && chat.transcript.turn.is_some())
    }

    /// True if its agent is at work: a native turn runs, or in a terminal its task is Running.
    pub(super) fn works(&self, subject: &Subject) -> bool {
        self.native_turn(subject) || (self.talk_view(subject) != View::Native && self.talk_working(subject))
    }

    /// The header's Later and Done, for a live task that is not done. No Done while its agent
    /// works (a turn runs, or in a terminal it runs) or stops.
    pub fn finish(&self, subject: &Subject) -> Option<Buttons> {
        let task = self.tasks.live(subject.task()?)?;
        if task.state == State::Done {
            return None;
        }
        let ending = self.is_ending(subject);
        Some(Buttons { later: !ending, done: !ending && !self.works(subject) })
    }

    /// "Got what you needed? ✓ Done · Later" under the last reply: the shown conversation's agent
    /// answered and idles, nothing waits on the user, and they have not begun to type.
    pub fn done_offered(&self, subject: &Subject) -> bool {
        let Some(chat) = self.chat(subject).filter(|_| self.finish(subject).is_some_and(|b| b.done)) else {
            return false;
        };
        let t = &chat.transcript;
        let quiet = t.live && t.turn.is_none() && t.cards.is_empty() && chat.draft.text().is_empty();
        let said = t.items().rev().find(|item| matches!(item.entry, Entry::User { .. } | Entry::Assistant { .. }));
        quiet && chat.queued.is_none() && said.is_some_and(|item| matches!(item.entry, Entry::Assistant { .. }))
    }

    /// ⌘W: Later on the drawer's task while the keyboard is in the drawer; else the pane closes.
    pub fn close_key(&self) -> Action {
        let talk = self.talk.as_ref().filter(|talk| talk.open && talk.keys);
        match talk.and_then(|talk| talk.task.task()).filter(|id| self.finish(&Subject::Task(*id)).is_some()) {
            Some(id) => Action::TaskLater { id },
            None => Action::Close { pane: None },
        }
    }

    /// ⌘↵ in the conversation: its Done, while Done is offered.
    pub fn done_key(&self) -> Option<Action> {
        let talk = self.talk.as_ref().filter(|talk| talk.open && talk.keys)?;
        let id = talk.task.task()?;
        self.finish(&talk.task)?.done.then_some(Action::TaskDone { id })
    }

    /// The seal of the user's Done plays on while its task is done and put away: an Undo, or the
    /// task opened again, ends it at once.
    pub fn seal_plays(&self, id: TaskId) -> bool {
        self.put_away(id) && self.tasks.live(id).is_some_and(|t| t.state == State::Done)
    }

    /// The Done line offers New task here: its task's workspace is the one on show.
    pub fn done_here(&self) -> bool {
        self.finish.offer.as_ref().is_some_and(|offer| self.task_space(offer.task) == *self.space())
    }

    /// "Done · <task>" while its Undo is offered.
    pub fn done_notice(&self) -> Option<Notice> {
        let offer = self.finish.offer.as_ref()?;
        let text = match &offer.kept {
            Some(kept) => format!("Done · {} · {kept}", offer.title),
            None => format!("Done · {}", offer.title),
        };
        Some(Notice { kind: notice::Kind::Undo, text })
    }

    pub(super) fn finish_act(&mut self, action: Action, effects: &mut Vec<Effect>) {
        match action {
            Action::TaskDone { id } => {
                let subject = Subject::Task(id);
                let Some(task) = self.tasks.live(id).filter(|t| t.state != State::Done) else { return };
                if self.works(&subject) {
                    return effects.push(Effect::Notice { text: TURN_RUNS.into() });
                }
                let (before, title) = (task.state, task.title.clone());
                // The seal plays where it was pressed: the drawer on this task.
                let shown = self.talk.as_ref().is_some_and(|talk| talk.open && talk.task == subject);
                self.put_task_away(id, State::Done, true, effects);
                let kept = self.remove_worktree(id, effects);
                let until = self.now.saturating_add(DONE_MS);
                self.finish.offer = Some(DoneOffer { task: id, title, before, until, kept });
                if shown {
                    effects.push(Effect::Seal { task: id });
                }
            }
            Action::TaskLater { id } => {
                let Some(title) = self.tasks.live(id).filter(|t| t.state != State::Done).map(|t| t.title.clone())
                else {
                    return;
                };
                if self.works(&Subject::Task(id)) {
                    // The question is ottospace's own UI: a live window would cover it.
                    self.leave(effects);
                    self.confirm = Some(Confirm { later: Some((id, title)), ..Confirm::default() });
                    return;
                }
                self.put_task_away(id, State::Parked, false, effects);
            }
            Action::DoneUndo => {
                let Some(offer) = self.finish.offer.take() else { return };
                if self.tasks.live(offer.task).is_none_or(|t| t.state != State::Done) {
                    return;
                }
                self.tasks.set_state(offer.task, offer.before);
                self.finish.away.retain(|id| *id != offer.task);
                self.readd_worktree(offer.task, effects);
                self.set_front(offer.task);
                // Back where it was: the drawer on it, if its workspace is the one on show.
                if self.task_space(offer.task) == *self.space() {
                    if let Some(talk) = &mut self.talk {
                        talk.open = true;
                    }
                    self.drawer_open = true;
                }
            }
            Action::WorktreeRemoved { task, path, kept } => self.worktree_removed(task, &path, kept, effects),
            _ => {}
        }
    }

    /// Yes to "Stop it and put it away?": as Later on an idle one.
    pub(super) fn later_confirmed(&mut self, id: TaskId, effects: &mut Vec<Effect>) {
        if self.tasks.live(id).is_some_and(|t| t.state != State::Done) {
            self.put_task_away(id, State::Parked, false, effects);
        }
    }

    /// The user's ↵ or click on a task put away, or done: it is a task like any other again.
    /// True if it was done: its session is gone on with.
    pub(super) fn revive(&mut self, id: TaskId, effects: &mut Vec<Effect>) -> bool {
        let away = self.put_away(id);
        self.finish.away.retain(|task| *task != id);
        let Some(state) = self.tasks.live(id).map(|t| t.state) else { return false };
        let done = state == State::Done;
        if done || (away && state == State::Parked) {
            self.tasks.set_state(id, State::NeedsYou);
        }
        // Its own worktree, removed by its Done: back first, then its session goes on there.
        if done && self.readd_worktree(id, effects) {
            self.finish.resume.push(id);
            return false;
        }
        if self.finish.offer.as_ref().is_some_and(|offer| offer.task == id) {
            self.finish.offer = None;
        }
        done
    }

    /// Its agent stops (as Stop stops it), it is `state`, off the rail, and the next task of its
    /// workspace is in front. The drawer folds into the rail on `fold`, or when none is left.
    fn put_task_away(&mut self, id: TaskId, state: State, fold: bool, effects: &mut Vec<Effect>) {
        let subject = Subject::Task(id);
        let of = self.task_space(id);
        let was_front = self.front(&of) == Some(id);
        self.act(Action::TaskStop { id: subject.clone() }, effects);
        self.tasks.set_state(id, state);
        if !self.finish.away.contains(&id) {
            self.finish.away.push(id);
        }
        // Another task's rail ×: the one in front stays.
        let next = self.members(&of).into_iter().filter(|t| !self.put_away(*t)).min_by_key(|t| self.urgency(*t));
        if let Some(next) = next.filter(|_| was_front) {
            self.set_front(next);
        }
        let shown = self.talk.as_ref().is_some_and(|talk| talk.task == subject);
        if shown && (fold || next.is_none()) {
            self.talk_keys(false, effects);
            if let Some(talk) = &mut self.talk {
                talk.open = false;
            }
            self.drawer_open = false;
        }
    }

    /// After every step: a task put away is one still there, in the state it was put away in
    /// or whatever its runner said since. The offer ends with its task.
    pub(super) fn settle_finish(&mut self) {
        // One whose state anyone changed since (a runner's report, a client's) is news: back on the rail.
        let tasks = &self.tasks;
        self.finish.away.retain(|id| tasks.live(*id).is_some_and(|t| matches!(t.state, State::Done | State::Parked)));
        let gone = self
            .finish
            .offer
            .as_ref()
            .is_some_and(|offer| tasks.live(offer.task).is_none_or(|t| t.state != State::Done));
        if gone {
            self.finish.offer = None;
        }
    }

    /// Done's step for the task's own worktree (`Task::made`), still listed: removed (git refuses
    /// a dirty one: `WorktreeRemoved` says), unless another task works there or a terminal is in
    /// it, which keeps it. The repository's own checkout and a worktree it did not make are never
    /// touched. `Some`: why it was kept.
    fn remove_worktree(&mut self, id: TaskId, effects: &mut Vec<Effect>) -> Option<String> {
        let made = self.tasks.get(id)?.made.clone().filter(|made| !made.removed)?;
        self.own_worktree(&made)?;
        // Listed, on its own branch: one the user made in its place since, or switched, is not its.
        let mut listed = self.repos.iter().filter(|r| r.path == made.repo).flat_map(|r| &r.worktrees);
        if !listed.any(|w| w.path == made.path && w.branch.as_deref() == Some(made.branch.as_str())) {
            return None;
        }
        let inside = |dir: &str| crate::sessions::below(dir, &made.path);
        let working = |t: &&crate::tasks::Task| t.id != id && !t.dismissed && t.state != State::Done;
        let others = self.tasks.list.iter().filter(working).any(|t| self.task_worktree(t.id).is_some_and(inside));
        if others {
            return Some(KEPT_TASK.into());
        }
        // A terminal's shell, or the program in front of it, or the folder it started in.
        let terminal = |pane: &&super::Pane| pane.kind == super::Kind::Terminal;
        let mut cwds = self.panes.iter().filter(terminal).flat_map(|pane| {
            let live = self.terminals.iter().filter(|t| t.pane == pane.id);
            let live: Vec<&str> = live.flat_map(|t| [t.cwd.as_deref(), t.shell.as_deref()]).flatten().collect();
            live.into_iter().chain(pane.cwd.as_deref())
        });
        if cwds.any(inside) {
            return Some(KEPT_TERMINAL.into());
        }
        self.finish.removing.push((id, made.path.clone()));
        effects.push(Effect::RemoveWorktree { task: id, repo: made.repo, path: made.path });
        None
    }

    /// What git did with Done's removal: gone (the list is read again; a workspace whose default
    /// it was falls back to its repository), or kept, and why.
    fn worktree_removed(&mut self, id: TaskId, path: &str, kept: Option<String>, effects: &mut Vec<Effect>) {
        let Some(at) = self.finish.removing.iter().position(|(task, asked)| *task == id && asked == path) else {
            return;
        };
        self.finish.removing.remove(at);
        let Some(made) = self.tasks.get(id).and_then(|t| t.made.clone()).filter(|made| made.path == path) else {
            return;
        };
        // Kept: the Done line says why. Its line gone (an Undo, its time up): git's answer is no
        // act of the user's, so no notice.
        if let Some(why) = kept {
            if let Some(offer) = self.finish.offer.as_mut().filter(|offer| offer.task == id) {
                offer.kept = Some(why);
            }
            return;
        }
        if let Some(task) = self.tasks.list.iter_mut().find(|t| t.id == id) {
            task.made = Some(crate::tasks::Made { removed: true, ..made.clone() });
        }
        let books = self.workbooks.default_gone(path, &made.repo);
        self.finish.fell_back.retain(|(task, _)| *task != id);
        self.finish.fell_back.push((id, books));
        effects.push(Effect::ReadRepos);
        // Undone, or opened again, while git removed it: added back at once.
        if self.tasks.get(id).is_some_and(|t| t.state != State::Done) {
            self.readd_worktree(id, effects);
        }
    }

    /// The task's own worktree made after its Done (git was still at it): removed as Done would.
    pub(super) fn late_worktree(&mut self, id: TaskId, effects: &mut Vec<Effect>) {
        if self.put_away(id) && self.tasks.get(id).is_some_and(|t| t.state == State::Done) {
            let kept = self.remove_worktree(id, effects);
            if let Some(offer) = self.finish.offer.as_mut().filter(|offer| offer.task == id) {
                offer.kept = kept;
            }
        }
    }

    /// A removal of the task's own worktree is asked for and not answered yet.
    pub(super) fn removing(&self, path: &str) -> Option<&crate::tasks::Made> {
        let (id, _) = self.finish.removing.iter().find(|(_, p)| p == path)?;
        self.tasks.get(*id)?.made.as_ref()
    }

    /// The worktrees folder, if the task's record names the place the box makes for its repository
    /// and branch (`worktree_dir`), git's `path` that same place (the setting is resolved), of a
    /// listed repository, on a branch git takes. The record is read from the tasks folder, maybe
    /// a shared notes repository: its own `root` is not trusted, the setting is.
    fn own_worktree(&self, made: &crate::tasks::Made) -> Option<String> {
        let root = self.repo_dirs.worktrees.clone()?;
        let listed = self.repos.iter().any(|r| r.path == made.repo);
        let branch = crate::repos::check_branch(&made.branch).is_ok();
        let at = made.asked == crate::repos::worktree_dir(&root, &made.repo, &self.repos, &made.branch);
        (at && made.path == made.asked && listed && branch).then_some(root)
    }

    /// Undo of a Done that removed its worktree: git adds it back on its branch, where it was.
    /// True if one is asked for. Only in the worktrees folder the setting names now.
    pub(super) fn readd_worktree(&mut self, id: TaskId, effects: &mut Vec<Effect>) -> bool {
        let Some(made) = self.tasks.get(id).and_then(|t| t.made.clone()).filter(|made| made.removed) else {
            return false;
        };
        let Some(root) = self.own_worktree(&made) else { return false };
        if self.finish.readding.contains(&made.asked) {
            return true;
        }
        self.finish.readding.push(made.asked.clone());
        let crate::tasks::Made { repo, branch, asked, .. } = made;
        effects.push(Effect::AddWorktree { repo, branch, path: asked, root, fresh: false });
        true
    }

    /// Git's answer for a worktree asked for at `asked`: true if an Undo asked for it. Added
    /// (`WorktreeAdded`), its task's again; it opens nothing.
    /// Its workspaces' default is the worktree again, and a task opened again goes on with its session.
    pub(super) fn readded(&mut self, asked: &str, added: bool, effects: &mut Vec<Effect>) -> bool {
        let Some(at) = self.finish.readding.iter().position(|a| a == asked) else { return false };
        self.finish.readding.remove(at);
        let of = self.tasks.list.iter().find(|t| t.made.as_ref().is_some_and(|made| made.asked == asked));
        let Some((id, made)) = of.and_then(|t| Some((t.id, t.made.clone()?))) else { return true };
        let resumes = self.finish.resume.contains(&id);
        self.finish.resume.retain(|task| *task != id);
        if !added {
            return true;
        }
        if let Some(task) = self.tasks.list.iter_mut().find(|t| t.id == id) {
            task.made = Some(crate::tasks::Made { removed: false, ..made.clone() });
        }
        if let Some(at) = self.finish.fell_back.iter().position(|(task, _)| *task == id) {
            let (_, books) = self.finish.fell_back.remove(at);
            self.workbooks.default_back(&books, &made.repo, &made.path);
        }
        // Put away again (Done, Later) while git added it back: no resume.
        let resumes = resumes && !self.put_away(id);
        let subject = Subject::Task(id);
        let start = self.agent_start(&subject).filter(|_| resumes && !self.agent_runs(&subject));
        let start = start.filter(|(start, _)| matches!(start, crate::agents::Start::Resume { .. }));
        if let Some(start) = start {
            self.launch(subject, start, None, effects);
        }
        true
    }

    /// × on the bar over "Done · <task>": the offer ends.
    pub(super) fn done_dismiss(&mut self, text: &str) {
        if self.done_notice().is_some_and(|said| said.text == text) {
            self.finish.offer = None;
        }
    }

    pub(super) fn finish_tick(&mut self, now: u64) {
        self.finish.offer = self.finish.offer.take().filter(|offer| offer.until > now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Origin;
    use crate::model::chat::Primary;
    use crate::model::rail::RailItem;
    use crate::model::tests::started;
    use crate::transcript::{Event, Outcome, Text};

    /// One task whose native agent answered and idles, its conversation shown with the keyboard.
    fn idle() -> Model {
        let mut m = Model::default();
        let assign = crate::tasks::Assign::default();
        m.update(Action::TaskAdd { title: "fix login".into(), priority: 3, state: State::Triaged, tab: None, assign });
        started(&mut m, Action::TaskTalk { id: 1 });
        let session = Some("8f14e45f-ceea-467f-a8d3-0242ac130003".into());
        m.update(Action::AgentStarted { task: Subject::Task(1), session, at: 1 }.sent_by(Origin::System));
        replied(&mut m);
        m
    }

    fn events(m: &mut Model, events: Vec<Event>) -> Vec<Effect> {
        m.update(Action::AgentEvents { task: Subject::Task(1), events, at: 5 }.sent_by(Origin::System))
    }

    fn replied(m: &mut Model) {
        let text = Event::Text { text: Text::new("Fixed: the redirect keeps the cookie.") };
        events(m, vec![Event::TurnStarted, text, Event::TurnFinished { outcome: Outcome::Completed }]);
    }

    fn state(m: &Model, id: TaskId) -> Option<State> {
        m.tasks.get(id).map(|t| t.state)
    }

    const ONE: Subject = Subject::Task(1);

    #[test]
    fn done_on_an_idle_task_stops_its_agent_seals_it_and_undo_brings_it_back() {
        let mut m = idle();
        assert_eq!(m.finish(&ONE), Some(Buttons { later: true, done: true }));
        assert!(m.done_offered(&ONE), "the quiet offer under the reply");
        assert_eq!((m.rail(), m.done_key()), (vec![RailItem::Task { id: 1 }], Some(Action::TaskDone { id: 1 })));
        let effects = m.update(Action::TaskDone { id: 1 });
        assert_eq!(effects, [Effect::StopAgent { task: ONE }, Effect::Seal { task: 1 }]);
        assert_eq!((state(&m, 1), m.put_away(1), m.rail()), (Some(State::Done), true, vec![]));
        // The drawer folded into the rail; the workspace stays on the row, with no task in front.
        assert!(m.drawer_collapsed());
        assert_eq!((m.front(m.space()), m.space().clone()), (None, crate::model::Owner::Task { id: 1 }));
        assert_eq!(m.done_notice().map(|n| (n.kind, n.text)), Some((notice::Kind::Undo, "Done · fix login".into())));
        // Its stopping agent's last word moves it nowhere.
        events(&mut m, vec![Event::Exited { code: Some(0), signal: None, details: false }]);
        m.update(Action::AgentExited { task: ONE }.sent_by(Origin::System));
        assert_eq!(state(&m, 1), Some(State::Done));
        // Undo: as it was, in front, the drawer on it; its agent is not started again.
        assert!(m.seal_plays(1));
        assert_eq!(m.update(Action::DoneUndo), []);
        assert!(!m.seal_plays(1), "an Undo ends the seal");
        assert_eq!(
            (state(&m, 1), m.put_away(1), m.rail()),
            (Some(State::NeedsYou), false, vec![RailItem::Task { id: 1 }])
        );
        assert_eq!((m.drawer_collapsed(), m.done_notice()), (false, None));
        assert_eq!(m.composer_button(), Some(Primary::Resume));
    }

    #[test]
    fn the_offer_to_take_done_back_lasts_six_seconds_or_until_dismissed() {
        let mut m = idle();
        m.update(Action::Tick { now: 10_000 }.sent_by(Origin::System));
        m.update(Action::TaskDone { id: 1 });
        assert_eq!(m.next_due(), Some(10_000 + DONE_MS));
        m.update(Action::Tick { now: 10_000 + DONE_MS - 1 }.sent_by(Origin::System));
        assert!(m.done_notice().is_some());
        m.update(Action::Tick { now: 10_000 + DONE_MS }.sent_by(Origin::System));
        assert_eq!(m.done_notice(), None);
        m.update(Action::DoneUndo);
        assert_eq!(state(&m, 1), Some(State::Done), "its time is up");
        // A second one, dismissed by its ×.
        let mut m = idle();
        m.update(Action::TaskDone { id: 1 });
        let text = m.done_notice().map(|n| n.text).unwrap_or_default();
        m.update(Action::NoticeDismiss { notice: notice::Kind::Undo, text });
        assert_eq!(m.done_notice(), None);
    }

    #[test]
    fn while_a_turn_runs_the_header_has_no_done_and_the_composers_stop_stops_only_the_turn() {
        let mut m = idle();
        events(&mut m, vec![Event::TurnStarted]);
        assert_eq!(m.finish(&ONE), Some(Buttons { later: true, done: false }));
        assert_eq!(m.composer_button(), Some(Primary::Stop), "the one stop: the composer's");
        assert!(!m.done_offered(&ONE));
        assert_eq!(m.done_key(), None, "⌘↵ marks nothing done mid-turn");
        // Done sent anyway: it says why, and changes nothing.
        assert_eq!(m.update(Action::TaskDone { id: 1 }), [Effect::Notice { text: TURN_RUNS.into() }]);
        assert_eq!(state(&m, 1), Some(State::Running));
        assert_eq!(m.update(Action::TalkInterrupt), [Effect::AgentInterrupt { task: ONE }]);
        events(&mut m, vec![Event::TurnFinished { outcome: Outcome::Interrupted }]);
        assert_eq!((m.agent_runs(&ONE), m.put_away(1)), (true, false), "the conversation goes on");
        assert_eq!(m.finish(&ONE).map(|b| b.done), Some(true), "the turn over: ✓ Done again");
    }

    #[test]
    fn later_on_an_idle_task_puts_it_on_hold_and_its_row_brings_it_back() {
        let mut m = idle();
        assert_eq!(m.close_key(), Action::TaskLater { id: 1 }, "⌘W in the drawer");
        assert_eq!(m.update(Action::TaskLater { id: 1 }), [Effect::StopAgent { task: ONE }]);
        assert_eq!(
            (state(&m, 1), m.put_away(1), m.rail(), m.confirm.is_none()),
            (Some(State::Parked), true, vec![], true)
        );
        assert_eq!(m.done_notice(), None, "Later offers no Undo: Tasks has it");
        assert_eq!(m.close_key(), Action::Close { pane: None }, "the drawer folded: ⌘W closes the pane");
        m.update(Action::AgentExited { task: ONE }.sent_by(Origin::System));
        // Its row: a task again, in front, its conversation open; nothing starts by the click.
        let effects = m.update(Action::TaskTalk { id: 1 });
        assert!(!effects.iter().any(|e| matches!(e, Effect::StartAgent { .. })), "{effects:?}");
        assert_eq!(
            (state(&m, 1), m.put_away(1), m.rail()),
            (Some(State::NeedsYou), false, vec![RailItem::Task { id: 1 }])
        );
    }

    #[test]
    fn later_asks_first_while_a_turn_runs() {
        let mut m = idle();
        events(&mut m, vec![Event::TurnStarted]);
        assert_eq!(m.update(Action::TaskLater { id: 1 }), []);
        let asked = m.confirm.as_ref().map(|c| (c.later.clone(), c.title()));
        assert_eq!(asked, Some((Some((1, "fix login".into())), "Stop \"fix login\" and put it away?".into())));
        // Only the user answers; Keep running leaves it as it was.
        assert_eq!(m.refusal(&Action::QuitConfirm, Origin::Socket), Some(USERS_ALONE));
        m.update(Action::QuitCancel);
        assert_eq!((m.confirm.is_none(), state(&m, 1), m.put_away(1)), (true, Some(State::Running), false));
        m.update(Action::TaskLater { id: 1 });
        assert_eq!(m.update(Action::QuitConfirm), [Effect::StopAgent { task: ONE }]);
        assert_eq!((state(&m, 1), m.put_away(1)), (Some(State::Parked), true));
        // The turn ending by itself while it asks: the question goes, nothing is put away.
        let mut m = idle();
        events(&mut m, vec![Event::TurnStarted]);
        m.update(Action::TaskLater { id: 1 });
        events(&mut m, vec![Event::TurnFinished { outcome: Outcome::Completed }]);
        assert_eq!((m.confirm.is_none(), m.put_away(1)), (true, false));
    }

    #[test]
    fn only_the_user_marks_done_puts_away_or_takes_back() {
        let m = idle();
        for origin in [Origin::Socket, Origin::System] {
            for action in
                [Action::TaskDone { id: 1 }, Action::TaskLater { id: 1 }, Action::DoneUndo, Action::TalkInterrupt]
            {
                assert!(m.refusal(&action, origin).is_some(), "{action:?} from {origin:?}");
            }
            assert_eq!(m.refusal(&Action::TaskDone { id: 1 }, origin), Some(USERS_ALONE));
        }
        assert_eq!(m.refusal(&Action::TaskDone { id: 1 }, Origin::User), None);
    }

    #[test]
    fn the_offer_line_goes_once_the_user_types_and_comes_back_after_the_next_reply() {
        let mut m = idle();
        m.update(Action::FieldEdit { edit: crate::edit::Edit::Replace { range: None, text: "and".into() } });
        assert!(!m.done_offered(&ONE));
        m.update(Action::TalkSend);
        assert!(!m.done_offered(&ONE), "the agent at work");
        replied(&mut m);
        assert!(m.done_offered(&ONE));
    }

    /// Three tasks in one workspace: Later on the one in front shows the next, the drawer
    /// open; Done folds it into the rail, the next in front.
    #[test]
    fn in_a_workspace_of_several_the_next_task_comes_in_front() {
        let mut m = Model::default();
        m.update(Action::Split { axis: crate::Axis::Right, kind: crate::model::Kind::Terminal });
        for title in ["one", "two", "three"] {
            m.update(Action::OpenCompose { text: title.into() });
            m.update(Action::ComposeSubmit { open: true });
        }
        let front = |m: &Model| m.front(m.space());
        assert_eq!(front(&m), Some(3));
        m.update(Action::TaskLater { id: 3 });
        assert_eq!((front(&m), m.drawer_collapsed(), m.put_away(3)), (Some(1), false, true));
        m.update(Action::TaskDone { id: 1 });
        assert_eq!((front(&m), m.drawer_collapsed()), (Some(2), true));
        let ids: Vec<RailItem> = m.rail();
        assert_eq!(ids, [RailItem::Task { id: 2 }]);
    }

    /// ↵ on a task's row in list mode brings one put away back, as its click does.
    #[test]
    fn the_keyboard_on_its_row_brings_a_task_put_away_back() {
        let mut m = idle();
        m.update(Action::TaskLater { id: 1 });
        m.update(Action::AgentExited { task: ONE }.sent_by(Origin::System));
        m.update(Action::ToggleSection { section: crate::model::Section::Tasks });
        m.update(Action::FocusList);
        let at = m.list_rows().iter().position(|row| *row == crate::model::ListRow::Task(1));
        m.list = at;
        m.update(Action::ListChoose { axis: crate::Axis::Right });
        assert_eq!((state(&m, 1), m.put_away(1), m.front(m.space())), (Some(State::NeedsYou), false, Some(1)));
        assert!(m.talk.as_ref().is_some_and(|talk| talk.open && talk.task == ONE));
    }

    /// The × on a task's rail item that is not in front: the one in front stays.
    #[test]
    fn later_on_another_tasks_rail_item_leaves_the_one_in_front() {
        let mut m = Model::default();
        m.update(Action::Split { axis: crate::Axis::Right, kind: crate::model::Kind::Terminal });
        for title in ["one", "two", "three"] {
            m.update(Action::OpenCompose { text: title.into() });
            m.update(Action::ComposeSubmit { open: true });
        }
        m.update(Action::TaskSetState { id: 1, state: State::NeedsYou }.sent_by(Origin::System));
        assert_eq!(m.front(m.space()), Some(3));
        m.update(Action::TaskLater { id: 2 });
        assert_eq!((m.front(m.space()), m.put_away(2), m.drawer_collapsed()), (Some(3), true, false));
    }

    /// A state anyone reports since is news: the task is back on the rail.
    #[test]
    fn a_task_put_away_whose_state_is_reported_comes_back() {
        let mut m = idle();
        m.update(Action::TaskLater { id: 1 });
        m.update(Action::TaskSetState { id: 1, state: State::NeedsYou }.sent_by(Origin::System));
        assert_eq!((m.put_away(1), m.rail()), (false, vec![RailItem::Task { id: 1 }]));
    }

    /// Later's question is no quit: only ↵ stops the agent; Q and ⌘Q keep it running.
    #[test]
    fn only_return_answers_yes_to_later() {
        use crate::keys::Mods;
        let cmd = Mods { cmd: true, ..Mods::default() };
        assert_eq!(Confirm::answer_later("enter", Mods::default(), Origin::User), Some(Action::QuitConfirm));
        for (key, mods) in [("q", Mods::default()), ("q", cmd), ("escape", Mods::default())] {
            assert_eq!(Confirm::answer_later(key, mods, Origin::User), Some(Action::QuitCancel), "{key}");
        }
        assert_eq!(Confirm::answer_later("q", Mods::default(), Origin::Socket), None);
    }

    const ROOT: &str = "/home/.ottospace/worktrees";
    const WORKTREE: &str = "/home/.ottospace/worktrees/app/fix-login";

    fn app(worktrees: &[&str]) -> Vec<crate::repos::Repo> {
        let tree = |path: &&str| crate::repos::Worktree {
            name: "fix-login".into(),
            path: path.to_string(),
            branch: Some("fix-login".into()),
        };
        let worktrees = worktrees.iter().map(tree).collect();
        vec![crate::repos::Repo {
            name: "app".into(),
            path: "/code/app".into(),
            branch: Some("main".into()),
            worktrees,
        }]
    }

    /// The user's workspace of `app`, a task started there in a new worktree the box made, its
    /// agent answered and idle.
    fn in_its_worktree() -> Model {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: app(&[]) });
        let dirs = crate::repos::RepoDirs { worktrees: Some(ROOT.into()), ..Default::default() };
        m.update(Action::SetRepoDirs { dirs });
        m.update(Action::RepoClick { path: "/code/app".into(), axis: crate::Axis::Right });
        m.update(Action::OpenCompose { text: "fix login".into() });
        m.update(Action::ComposeWorktree { new: Some(true) });
        m.update(Action::ComposeEnter);
        let made = Action::WorktreeAdded {
            repo: "/code/app".into(),
            path: WORKTREE.into(),
            asked: WORKTREE.into(),
            fresh: true,
            repos: app(&[WORKTREE]),
        };
        m.update(made.sent_by(Origin::System));
        let session = Some("8f14e45f-ceea-467f-a8d3-0242ac130003".into());
        m.update(Action::AgentStarted { task: ONE, session, at: 1 }.sent_by(Origin::System));
        replied(&mut m);
        assert_eq!(m.space_worktree(m.space()), Some(WORKTREE), "the workspace of one task follows it");
        m
    }

    fn removes(effects: &[Effect]) -> bool {
        effects.contains(&Effect::RemoveWorktree { task: 1, repo: "/code/app".into(), path: WORKTREE.into() })
    }

    fn removed(m: &mut Model, kept: Option<&str>) -> Vec<Effect> {
        let kept = kept.map(String::from);
        m.update(Action::WorktreeRemoved { task: 1, path: WORKTREE.into(), kept }.sent_by(Origin::System))
    }

    /// Done removes the worktree the task's box made; the branch stays (git's `worktree remove`
    /// keeps it). The workspace stays, its default back on the repository: ⌘N there starts the
    /// next task in it, from `app`, with a new worktree offered as ever. Undo adds it back.
    #[test]
    fn done_removes_its_own_clean_worktree_and_a_new_task_there_starts_from_the_repository() {
        let mut m = in_its_worktree();
        let of = m.space().clone();
        assert!(removes(&m.update(Action::TaskDone { id: 1 })));
        assert_eq!(
            m.refusal(&Action::WorktreeRemoved { task: 1, path: WORKTREE.into(), kept: None }, Origin::Socket),
            Some(SHELLS_ALONE)
        );
        assert_eq!(removed(&mut m, None), [Effect::ReadRepos]);
        assert_eq!(m.tasks.get(1).and_then(|t| t.made.as_ref()).map(|made| made.removed), Some(true));
        m.update(Action::SetRepos { repos: app(&[]) }.sent_by(Origin::System));
        assert_eq!((m.space(), m.space_worktree(&of)), (&of, Some("/code/app")));
        // ⌘N in the workspace: the box is in it, its folder the repository, a new worktree offered.
        m.update(Action::OpenCompose { text: "write docs".into() });
        assert_eq!(m.compose_plan().and_then(|plan| plan.dir).as_deref(), Some("app"), "the repository, by name");
        assert!(m.compose_worktree().is_some_and(|choice| !choice.new && choice.repo == "app"));
        m.update(Action::ComposeEnter);
        assert_eq!((m.task_space(2), m.front(&of)), (of.clone(), Some(2)));
        assert_eq!(m.tasks.get(2).and_then(|t| t.dir().cloned()).as_deref(), Some("/code/app"));
    }

    #[test]
    fn undo_of_done_adds_its_worktree_back_on_its_branch_and_opens_nothing() {
        let mut m = in_its_worktree();
        m.update(Action::TaskDone { id: 1 });
        removed(&mut m, None);
        m.update(Action::SetRepos { repos: app(&[]) }.sent_by(Origin::System));
        let spaces = m.spaces.len();
        let effects = m.update(Action::DoneUndo);
        let add = Effect::AddWorktree {
            repo: "/code/app".into(),
            branch: "fix-login".into(),
            path: WORKTREE.into(),
            root: ROOT.into(),
            fresh: false,
        };
        assert!(effects.contains(&add), "{effects:?}");
        let made = Action::WorktreeAdded {
            repo: "/code/app".into(),
            path: WORKTREE.into(),
            asked: WORKTREE.into(),
            fresh: false,
            repos: app(&[WORKTREE]),
        };
        m.update(made.sent_by(Origin::System));
        assert_eq!(m.spaces.len(), spaces, "no workspace opened for it");
        assert_eq!(m.tasks.get(1).and_then(|t| t.made.as_ref()).map(|made| made.removed), Some(false));
        assert_eq!((state(&m, 1), m.task_worktree(1)), (Some(State::NeedsYou), Some(WORKTREE)));
    }

    #[test]
    fn a_dirty_worktree_or_one_a_terminal_is_in_is_kept_and_the_line_says_why() {
        // Git refused: uncommitted changes.
        let mut m = in_its_worktree();
        m.update(Action::TaskDone { id: 1 });
        assert_eq!(removed(&mut m, Some("Kept the worktree: 3 uncommitted changes")), []);
        let said = m.done_notice().map(|n| n.text);
        assert_eq!(said.as_deref(), Some("Done · fix login · Kept the worktree: 3 uncommitted changes"));
        assert_eq!(m.tasks.get(1).and_then(|t| t.made.as_ref()).map(|made| made.removed), Some(false));
        // A terminal of the workspace is in it: kept, not asked of git.
        let mut m = in_its_worktree();
        m.update(Action::Split { axis: crate::Axis::Right, kind: crate::model::Kind::Terminal });
        let pane = m.panes.last().map(|p| p.id).unwrap_or_default();
        let cwd = Some(format!("{WORKTREE}/src"));
        let terminals = vec![crate::model::TerminalInfo { pane, cwd, program: None, shell: None }];
        m.update(Action::SetTerminals { terminals, home: None }.sent_by(Origin::System));
        assert!(!removes(&m.update(Action::TaskDone { id: 1 })));
        assert_eq!(m.done_notice().map(|n| n.text), Some(format!("Done · fix login · {KEPT_TERMINAL}")));
        // An answer nobody asked for changes nothing.
        assert_eq!(removed(&mut m, None), []);
    }

    /// A task with no worktree of its own (its repository's checkout): Done touches nothing on disk.
    #[test]
    fn done_never_removes_a_checkout_the_task_did_not_make() {
        let mut m = idle();
        m.update(Action::SetRepos { repos: app(&[WORKTREE]) });
        let effects = m.update(Action::TaskDone { id: 1 });
        assert!(!effects.iter().any(|e| matches!(e, Effect::RemoveWorktree { .. })), "{effects:?}");
    }

    /// Done since the local day began is under "Done today", below the open tasks; older, under
    /// Done, folded. A click on one there makes it a task again and goes on with its session.
    #[test]
    fn done_today_is_its_own_group_and_its_row_resumes_the_task() {
        use crate::tasks::Heading;
        let mut m = idle();
        m.local_day = 50_000;
        m.update(Action::Tick { now: 60_000 }.sent_by(Origin::System));
        m.update(Action::TaskDone { id: 1 });
        let headings: Vec<(Heading, bool)> = m.task_sections(60_000).iter().map(|s| (s.heading, s.collapsed)).collect();
        assert_eq!(headings, [(Heading::DoneToday, false)]);
        m.local_day = 70_000;
        let headings: Vec<Heading> = m.task_sections(80_000).iter().map(|s| s.heading).collect();
        assert_eq!(headings, [Heading::Done], "yesterday's");
        m.update(Action::AgentExited { task: ONE }.sent_by(Origin::System));
        let effects = m.update(Action::TaskTalk { id: 1 });
        // Its session read first (the Tasks section is closed), then gone on with.
        let resumes = effects.iter().any(|e| matches!(e, Effect::ReadSession { .. }));
        assert!(resumes, "{effects:?}");
        assert_eq!(
            (state(&m, 1), m.put_away(1), m.rail()),
            (Some(State::NeedsYou), false, vec![RailItem::Task { id: 1 }])
        );
    }

    /// Done while git still makes the task's worktree: once it is made, no agent starts for it.
    #[test]
    fn a_worktree_made_after_done_starts_no_agent() {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: app(&[]) });
        let dirs = crate::repos::RepoDirs { worktrees: Some(ROOT.into()), ..Default::default() };
        m.update(Action::SetRepoDirs { dirs });
        m.update(Action::RepoClick { path: "/code/app".into(), axis: crate::Axis::Right });
        m.update(Action::OpenCompose { text: "fix login".into() });
        m.update(Action::ComposeWorktree { new: Some(true) });
        m.update(Action::ComposeEnter);
        m.update(Action::TaskDone { id: 1 });
        let made = Action::WorktreeAdded {
            repo: "/code/app".into(),
            path: WORKTREE.into(),
            asked: WORKTREE.into(),
            fresh: true,
            repos: app(&[WORKTREE]),
        };
        let effects = m.update(made.sent_by(Origin::System));
        assert!(!effects.iter().any(|e| matches!(e, Effect::StartAgent { .. })), "{effects:?}");
        assert_eq!(state(&m, 1), Some(State::Done));
        assert!(removes(&effects), "removed as its Done would have: {effects:?}");
    }

    fn adds_back(effects: &[Effect]) -> bool {
        effects.iter().any(
            |e| matches!(e, Effect::AddWorktree { path, root, fresh: false, .. } if path == WORKTREE && root == ROOT),
        )
    }

    /// Undo while git still removes it: once it is gone, it is added back, and the workspace's
    /// default with it.
    #[test]
    fn undo_while_git_removes_the_worktree_adds_it_back_after() {
        let mut m = in_its_worktree();
        let of = m.space().clone();
        m.update(Action::TaskDone { id: 1 });
        assert!(!adds_back(&m.update(Action::DoneUndo)), "nothing removed yet");
        let effects = removed(&mut m, None);
        assert!(adds_back(&effects), "{effects:?}");
        assert_eq!(m.space_worktree(&of), Some("/code/app"));
        m.update(
            Action::WorktreeAdded {
                repo: "/code/app".into(),
                path: WORKTREE.into(),
                asked: WORKTREE.into(),
                fresh: false,
                repos: app(&[WORKTREE]),
            }
            .sent_by(Origin::System),
        );
        assert_eq!((m.space_worktree(&of), state(&m, 1)), (Some(WORKTREE), Some(State::NeedsYou)));
    }

    /// A done task opened again from Done today: its worktree back first, then its session goes on there.
    #[test]
    fn a_done_task_opened_again_gets_its_worktree_back_then_resumes() {
        let mut m = in_its_worktree();
        m.update(Action::TaskDone { id: 1 });
        removed(&mut m, None);
        m.update(Action::AgentExited { task: ONE }.sent_by(Origin::System));
        m.update(Action::SetRepos { repos: app(&[]) }.sent_by(Origin::System));
        m.update(Action::Tick { now: 99_000 }.sent_by(Origin::System));
        let effects = m.update(Action::TaskTalk { id: 1 });
        assert!(adds_back(&effects), "{effects:?}");
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::StartAgent { .. } | Effect::ReadSession { .. })),
            "{effects:?}"
        );
        let effects = m.update(
            Action::WorktreeAdded {
                repo: "/code/app".into(),
                path: WORKTREE.into(),
                asked: WORKTREE.into(),
                fresh: false,
                repos: app(&[WORKTREE]),
            }
            .sent_by(Origin::System),
        );
        let resumed = effects.iter().any(|e| matches!(e, Effect::StartAgent { start: crate::agents::Start::Resume { .. }, cwd: Some(cwd), .. } if cwd == WORKTREE));
        assert!(resumed, "{effects:?}");
    }

    /// The setting moved since its Done: not added back, outside the worktrees folder it names.
    #[test]
    fn a_worktree_made_in_another_worktrees_folder_is_not_added_back() {
        let mut m = in_its_worktree();
        m.update(Action::TaskDone { id: 1 });
        removed(&mut m, None);
        let dirs = crate::repos::RepoDirs { worktrees: Some("/elsewhere".into()), ..Default::default() };
        m.update(Action::SetRepoDirs { dirs });
        let effects = m.update(Action::DoneUndo);
        assert!(!effects.iter().any(|e| matches!(e, Effect::AddWorktree { .. })), "{effects:?}");
    }

    /// Opened again, then put away (Later) before git adds its worktree back: no resume.
    #[test]
    fn a_task_put_away_while_its_worktree_comes_back_does_not_resume() {
        let mut m = in_its_worktree();
        m.update(Action::TaskDone { id: 1 });
        removed(&mut m, None);
        m.update(Action::AgentExited { task: ONE }.sent_by(Origin::System));
        m.update(Action::SetRepos { repos: app(&[]) }.sent_by(Origin::System));
        m.update(Action::Tick { now: 99_000 }.sent_by(Origin::System));
        assert!(adds_back(&m.update(Action::TaskTalk { id: 1 })));
        m.update(Action::TaskLater { id: 1 });
        assert!(m.put_away(1));
        let back = Action::WorktreeAdded {
            repo: "/code/app".into(),
            path: WORKTREE.into(),
            asked: WORKTREE.into(),
            fresh: false,
            repos: app(&[WORKTREE]),
        };
        let effects = m.update(back.sent_by(Origin::System));
        assert!(!effects.iter().any(|e| matches!(e, Effect::StartAgent { .. })), "{effects:?}");
    }

    /// One the user made in its place since, on another branch, is not the task's.
    #[test]
    fn done_leaves_a_worktree_on_another_branch_at_its_path() {
        let mut m = in_its_worktree();
        let mut repos = app(&[WORKTREE]);
        repos[0].worktrees[0].branch = Some("someone-else".into());
        m.update(Action::SetRepos { repos }.sent_by(Origin::System));
        assert!(!removes(&m.update(Action::TaskDone { id: 1 })));
    }

    /// While git removes it a new task there starts from the repository; a shell in it, with
    /// a program elsewhere in front, keeps it.
    #[test]
    fn a_worktree_being_removed_is_no_new_tasks_folder_and_a_shell_in_one_keeps_it() {
        let mut m = in_its_worktree();
        m.update(Action::TaskDone { id: 1 });
        m.update(Action::OpenCompose { text: "write docs".into() });
        assert_eq!(m.compose_plan().and_then(|plan| plan.dir).as_deref(), Some("app"));
        let mut m = in_its_worktree();
        m.update(Action::Split { axis: crate::Axis::Right, kind: crate::model::Kind::Terminal });
        let pane = m.panes.last().map(|p| p.id).unwrap_or_default();
        let info = crate::model::TerminalInfo {
            pane,
            cwd: Some("/tmp".into()),
            program: Some("sleep".into()),
            shell: Some(WORKTREE.into()),
        };
        m.update(Action::SetTerminals { terminals: vec![info], home: None }.sent_by(Origin::System));
        assert!(!removes(&m.update(Action::TaskDone { id: 1 })));
    }

    /// A task read from the tasks folder (maybe a shared notes repository) whose record names a
    /// worktree elsewhere: `root` and all, it is not trusted.
    fn planted(state: &str, removed: bool, path: &str, root: &str) -> Model {
        let mut m = Model::default();
        m.update(Action::SetRepos { repos: app(&[path]) });
        let dirs = crate::repos::RepoDirs { worktrees: Some(ROOT.into()), ..Default::default() };
        m.update(Action::SetRepoDirs { dirs });
        let kept = format!(
            r#"{{"id":1,"title":"x","state":"{state}","priority":3,"repo":"{path}","made":{{"repo":"/code/app",
                "branch":"fix-login","path":"{path}","asked":"{path}","root":"{root}","removed":{removed}}}}}"#
        );
        let task: crate::tasks::Task = serde_json::from_str(&kept).expect("a task as kept");
        m.update(Action::RestoreTasks { tasks: vec![task], floor: 1 }.sent_by(Origin::System));
        m
    }

    /// Opened again: no worktree is made outside the worktrees folder the settings name.
    #[test]
    fn a_kept_record_adds_back_no_worktree_outside_the_worktrees_folder() {
        let mut m = planted("done", true, "/Users/me/Dropbox/app/fix-login", "/Users/me/Dropbox");
        let effects = m.update(Action::TaskTalk { id: 1 });
        assert!(!effects.iter().any(|e| matches!(e, Effect::AddWorktree { .. })), "{effects:?}");
    }

    /// Done: a listed worktree outside the worktrees folder is not the task's to remove.
    #[test]
    fn a_kept_record_removes_no_worktree_outside_the_worktrees_folder() {
        let mut m = planted("needs_you", false, "/code/app-hand-made", "/code");
        let effects = m.update(Action::TaskDone { id: 1 });
        assert!(!effects.iter().any(|e| matches!(e, Effect::RemoveWorktree { .. })), "{effects:?}");
        // In the folder, not where the box puts its repository's branch: one the user put there.
        let mut m = planted("needs_you", false, &format!("{ROOT}/manual/keep"), ROOT);
        let effects = m.update(Action::TaskDone { id: 1 });
        assert!(!effects.iter().any(|e| matches!(e, Effect::RemoveWorktree { .. })), "{effects:?}");
        // Its `asked` inside, git's `path` elsewhere, ending the same: not the one the box made there.
        let mut m = planted("needs_you", false, "/code/app/fix-login", ROOT);
        if let Some(made) = m.tasks.list[0].made.as_mut() {
            made.asked = WORKTREE.into();
        }
        let effects = m.update(Action::TaskDone { id: 1 });
        assert!(!effects.iter().any(|e| matches!(e, Effect::RemoveWorktree { .. })), "{effects:?}");
    }

    /// New task here only while the done task's workspace is on show.
    #[test]
    fn new_task_here_is_offered_in_the_done_tasks_workspace_only() {
        let mut m = in_its_worktree();
        m.update(Action::TaskDone { id: 1 });
        assert!(m.done_here());
        m.update(Action::SpaceSelect { index: 0 });
        assert!(!m.done_here());
    }
}
