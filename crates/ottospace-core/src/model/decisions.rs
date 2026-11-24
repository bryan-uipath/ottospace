//! Decisions (`decisions` flag): an ephemeral workspace that walks the person through the
//! questions their agents wait on, one card at a time. One pane, the card; the drawer and the right
//! rail are on the card's task. It closes itself after the last card. Only the user opens it,
//! answers, skips, puts off or closes it. Design: `docs/ai/attention/decisions.md`.

use serde::Serialize;

use super::chat::{self, Key};
use super::{Action, Effect, Kind, Model, Owner, PaneId};
use crate::ask::Form;
use crate::features::Feature;
use crate::layout::Axis;
use crate::talk::Subject;
use crate::tasks::{State, TaskId};

/// Its name on the top row and its tab.
pub const NAME: &str = "Decisions";
/// Why a client's or a timer's open, answer, skip, later or close is refused.
pub const USERS_ALONE: &str = "only the user opens, answers, skips, puts off or closes Decisions";
/// Why Decisions is not saved in a workspace.
pub const UNKEPT: &str = "Decisions ends with its run: it is not saved";
/// How long L puts a card's task off: as a row's snooze.
pub const LATER_MS: u64 = 3_600_000;

/// A card waiting, and when it was first seen waiting (`Model::now` then).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Seen {
    pub task: TaskId,
    pub ticket: u64,
    pub since: u64,
}

/// A card in the queue, with what puts it where it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Waiting {
    pub task: TaskId,
    pub ticket: u64,
    pub priority: u8,
    pub since: u64,
    /// This week's area it is in, by place (`decision_sources`); never its name.
    pub area: Option<usize>,
    /// What placed it: its line and the line's hover.
    #[serde(skip)]
    pub why: crate::rank::Why,
}

/// Triage on show (`decision_sources`): the card "Sort 6 new items" (`item` null), or the item
/// being sorted, one key each, and what the user left on it before its key.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Sorting {
    pub item: Option<u64>,
    /// `u`: its claims (due, someone waits) count once sorted. Off at each item.
    pub claims: bool,
    /// `c`: its role, from its suggestion at each item.
    pub role: Option<crate::filed::Role>,
    /// Sorted in this run, and waiting still.
    pub done: usize,
    pub left: usize,
    /// The item's revision on show: a key for another (its filer changed it meanwhile) does nothing,
    /// and its card starts again with nothing left on it.
    pub revision: u32,
    /// What `z` takes back, the last first.
    #[serde(skip)]
    pub(crate) undo: Vec<Undo>,
}

/// A sort or a promotion, to take back.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Undo {
    Sorted(Box<crate::filed::Decision>),
    Promoted { source: crate::filed::Source, scope: String },
}

/// The Decisions workspace while it is open.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Run {
    /// The tab that holds its card pane, and the pane.
    pub tab: u32,
    pub pane: PaneId,
    /// The card on show: its task's agent's question `ticket`; neither while triage shows.
    pub task: Option<TaskId>,
    pub ticket: Option<u64>,
    /// Triage on show, or null. It comes after the agents' questions.
    pub sort: Option<Sorting>,
    /// The week's focus card on show (an agent's proposal, or "keep last week's?"), or null. After
    /// the agents' questions, before triage.
    pub week: Option<super::focus::WeekCard>,
    /// "`at` of `of`": the cards dealt with in this run, and those left.
    pub at: usize,
    pub of: usize,
    /// The cards left, the one on show first.
    pub queue: Vec<Waiting>,
    /// Cards dealt with in this run (answered, skipped, put off, or answered elsewhere while shown).
    #[serde(skip)]
    pub(crate) done: Vec<(TaskId, u64)>,
    /// The main window's tab when it opened: shown again when it closes on show.
    #[serde(skip)]
    pub(crate) from: Option<u32>,
    /// Held: the person typed in the drawer to its task when another's step would have moved it on.
    #[serde(skip)]
    pub(crate) held: bool,
    /// Triage was skipped or finished in this run: its card does not come again.
    #[serde(skip)]
    pub(crate) sorted: bool,
    /// The week's card was answered or left in this run: it does not come again.
    #[serde(skip)]
    pub(crate) weeked: bool,
    /// The card on show changed since it was first drawn (a newer proposal, its filer's update):
    /// the next key only says the user saw it (`DecisionsSeen`), nothing is taken.
    pub changed: bool,
    /// What was on show at the last step, and its revision: another revision, or another card put
    /// there by a step that was not an answer, sets `changed`.
    #[serde(skip)]
    pub(crate) seen: Option<(On, u32)>,
}

/// Which card is on show, for `changed`: an agent's question, the week's card, triage (its item).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum On {
    Ask(TaskId, u64),
    Week,
    Sort(Option<u64>),
}

impl Model {
    /// ⌘J, what needs the person next (owner, 2026-11-19): with `decisions` on, Decisions, which
    /// with no card waiting is the next task too; off, the next task.
    pub fn next_needs_you(&self) -> Action {
        match self.features.on(Feature::Decisions) {
            true => Action::DecisionsOpen,
            false => Action::TaskGo { id: None },
        }
    }

    /// The card on show in Decisions: its task and ticket. None while triage shows.
    pub fn decision_card(&self) -> Option<(TaskId, u64)> {
        self.decisions.as_ref().and_then(|run| run.task.zip(run.ticket))
    }

    /// Items wait in triage (`decision_sources`): Decisions has a sort card to show.
    pub fn sortable(&self) -> bool {
        self.features.on(Feature::DecisionSources) && self.filed.to_sort(self.now) > 0
    }

    /// The sort card's run state at item `item`: its claims off, its role as suggested.
    pub(crate) fn sorting_at(&self, item: Option<u64>, done: usize, undo: Vec<Undo>) -> Sorting {
        let shown = item.and_then(|id| self.filed.get(id));
        let (role, revision) = (shown.and_then(|d| d.suggest.role), shown.map_or(0, |d| d.revision));
        let left = self.filed.to_sort(self.now);
        Sorting { item, claims: false, role, done, left, revision, undo }
    }

    /// The questions waiting on the person: each task whose native agent runs and shows a question
    /// card first, unless the task is done, put away, dismissed or snoozed.
    fn asking(&self) -> Vec<(TaskId, u64)> {
        let now = self.now;
        let free = |id: TaskId| {
            let task = self.tasks.live(id).filter(|task| task.state != State::Done);
            let snoozed = task.and_then(|task| task.snoozed_until).is_some_and(|until| until > now);
            task.is_some() && !snoozed && !self.put_away(id)
        };
        let card = |chat: &chat::Chat| {
            let id = chat.subject.task()?;
            let front = chat.transcript.cards.front()?;
            (chat.transcript.live && chat.form().is_some() && free(id)).then_some((id, front.ticket))
        };
        self.chats.iter().filter_map(card).collect()
    }

    /// The queue, in order: the card on show first while it waits (nothing moves under the
    /// person); then the rest as `ranked` puts them (this week's focus first, then priority,
    /// then the one waiting longest). Each dealt with in this run is left out. Every card is an
    /// agent stopped on the person.
    pub fn decision_queue(&self) -> Vec<Waiting> {
        let done = self.decisions.as_ref().map(|run| run.done.clone()).unwrap_or_default();
        let shown = self.decision_card();
        let mut queue: Vec<Waiting> = (self.ranked().into_iter())
            .filter_map(|e| match e.card {
                crate::rank::Card::Ask { task, ticket } if !done.contains(&(task, ticket)) => {
                    Some(Waiting { task, ticket, priority: e.priority, since: e.since, area: e.why.area, why: e.why })
                }
                _ => None,
            })
            .collect();
        // Stable: the card on show first, the rest in rank.
        queue.sort_by_key(|w| shown != Some((w.task, w.ticket)));
        queue
    }

    /// The Tasks inbox (`Tasks::inbox`), Needs you in this week's focus first (`decision_sources`),
    /// then as before: P1 first, then the oldest.
    pub fn task_inbox(&self) -> Vec<(crate::tasks::Group, &crate::tasks::Task)> {
        let mut rows = self.tasks.inbox();
        if self.features.on(Feature::DecisionSources) && !self.focus.areas().is_empty() {
            let out = |task: &crate::tasks::Task| self.task_area(task).is_none();
            // Stable: within each tier the inbox's own order.
            rows.sort_by_key(|(group, task)| (*group, *group == crate::tasks::Group::NeedsYou && out(task)));
        }
        rows
    }

    /// This week's area task `task` is in: by its `#project`, or its repository (a worktree counts
    /// as the repository it is of).
    fn task_area(&self, task: &crate::tasks::Task) -> Option<usize> {
        let repo = task.repo.as_deref().map(|path| {
            let at = |p: &str| std::path::Path::new(path).starts_with(p);
            let owner = self.repos.iter().find(|r| at(&r.path) || r.worktrees.iter().any(|w| at(&w.path)));
            owner.map_or(path, |r| r.path.as_str())
        });
        self.focus.area_of(task.project.as_deref(), repo)
    }

    /// The task ⌘J opens: the first under Needs you, in focus first.
    pub fn next_task(&self) -> Option<TaskId> {
        let rows = self.task_inbox().into_iter();
        rows.into_iter().find(|(group, _)| *group == crate::tasks::Group::NeedsYou).map(|(_, task)| task.id)
    }

    /// Every card that needs the person, in order (`rank::order`): the agents' questions and,
    /// with `decision_sources`, the filed decisions sorted or trusted. Focus counts only with
    /// that flag on.
    pub fn ranked(&self) -> Vec<crate::rank::Entry> {
        use crate::filed::{Role, Triage};
        use crate::rank::{Card, Entry, Why};
        let on = self.features.on(Feature::DecisionSources);
        let focus_set = on && !self.focus.areas().is_empty();
        let area = |project: Option<&str>, repo: Option<&str>| {
            if on { self.focus.area_of(project, repo) } else { None }
        };
        let task_area = |id: TaskId| self.tasks.get(id).filter(|_| on).and_then(|task| self.task_area(task));
        let since = |task, ticket| {
            let seen = self.waiting.iter().find(|seen| seen.task == task && seen.ticket == ticket);
            seen.map_or(self.now, |seen| seen.since)
        };
        let asking = self.asking();
        let mut entries: Vec<Entry> = asking
            .iter()
            .map(|&(task, ticket)| {
                // An agent's question: core, nothing claimed (C × U = 2).
                let why = Why {
                    area: task_area(task),
                    focus_set,
                    agent_waits: true,
                    role: Some(Role::Core),
                    ..Why::default()
                };
                let priority = self.tasks.get(task).map_or(u8::MAX, |task| task.priority);
                let since = since(task, ticket);
                Entry {
                    card: Card::Ask { task, ticket },
                    source: crate::filed::Source::AgentSession,
                    why,
                    priority,
                    since,
                    id: u64::from(task),
                }
            })
            .collect();
        let ready = self.filed.list.iter().filter(|d| on && d.open() && d.triage != Triage::Pending);
        entries.extend(ready.map(|d| {
            // Sorted: what the user accepted. Trusted: the filer's word, as filed.
            let trusted = d.triage == Triage::Skipped;
            let project = if trusted { d.suggest.projects.first().cloned() } else { d.accepted.project.clone() };
            let role = if trusted { d.suggest.role } else { d.accepted.role };
            let counted = trusted || d.accepted.claims;
            let in_area = area(project.as_deref(), None).or_else(|| d.task.and_then(task_area));
            // An agent waits on it only when a task it blocks asks now: the core's check, never the claim's.
            let agent_waits = d.claims.blocks.iter().any(|t| asking.iter().any(|(task, _)| task == t));
            let why = Why {
                area: in_area,
                focus_set,
                taken_now: d.accepted.now_until.is_some_and(|until| until > self.now),
                agent_waits,
                role,
                person_waits: counted && d.claims.person_waits,
                due: d.claims.due.filter(|_| counted),
            };
            let priority = d.task.and_then(|t| self.tasks.get(t)).map_or(u8::MAX, |t| t.priority);
            Entry { card: Card::Filed(d.id), source: d.source, why, priority, since: d.filed, id: d.id }
        }));
        crate::rank::order(entries, self.now)
    }

    /// A key on the card pane, while it has the keyboard: 1 to 9 pick, ↑↓ j k move, space picks,
    /// ↵ sends, ⇥ between questions; S skips, L puts it off until `now + LATER_MS`, Esc closes Decisions.
    pub fn decision_key(&self, key: &str, mods: crate::keys::Mods, now: u64) -> Key {
        self.decision_key_drawn(key, mods, now, self.on_show())
    }

    /// The card on show and its revision: an agent's question (its ticket never changes), the
    /// week's card (the proposal's revision), triage's item (the decision's revision).
    pub fn on_show(&self) -> Option<(On, u32)> {
        let run = self.decisions.as_ref()?;
        if let Some(card) = run.week {
            let revision = match card {
                super::focus::WeekCard::Proposal(revision) => revision,
                super::focus::WeekCard::Carry => 0,
            };
            return Some((On::Week, revision));
        }
        if let Some(sort) = &run.sort {
            let revision = sort.item.and_then(|id| self.filed.get(id)).map_or(0, |d| d.revision);
            return Some((On::Sort(sort.item), revision));
        }
        run.task.zip(run.ticket).map(|(task, ticket)| (On::Ask(task, ticket), 0))
    }

    /// After every step, one rule for every card a filer can change while it is on show: another
    /// card or revision put there by a step that answered none (a filing, a withdrawal, an expiry)
    /// is marked changed, and its next key only says it was seen.
    pub(super) fn settle_changed(&mut self, answer: bool) {
        let now = self.on_show();
        let Some(run) = self.decisions.as_mut() else { return };
        match (run.seen, now) {
            (Some(seen), Some(on)) if seen == on => {}
            (seen, Some(_)) if seen != now && !answer => run.changed = true,
            _ => run.changed = false,
        }
        run.seen = now;
    }

    /// A key on the card pane, as `decision_key`, the shell saying which revision it drew: a key
    /// on a card whose revision moved since (a newer proposal, its filer's update) takes nothing,
    /// and while the card is marked changed, the next key only says the user saw it.
    pub fn decision_key_drawn(&self, key: &str, mods: crate::keys::Mods, now: u64, drawn: Option<(On, u32)>) -> Key {
        let bare = !mods.cmd && !mods.ctrl && !mods.alt && !mods.shift;
        if self.keyboard_decision()
            && key != "escape"
            && let Some(stop) = self.unseen(drawn)
        {
            return stop;
        }
        if let Some(card) = self.decisions.as_ref().and_then(|run| run.week).filter(|_| self.keyboard_decision()) {
            return self.week_key(card, key, bare, now);
        }
        let sort = self.decisions.as_ref().and_then(|run| run.sort.as_ref()).filter(|_| self.keyboard_decision());
        if let Some(sort) = sort {
            return self.sort_key(sort, key, bare, now);
        }
        let Some((task, ticket)) = self.decision_card().filter(|_| self.keyboard_decision()) else { return Key::Pass };
        match key {
            "escape" => Key::Act(Action::DecisionsClose),
            "s" if bare => Key::Act(Action::DecisionsSkip { task, ticket }),
            "l" if bare => Key::Act(Action::DecisionsLater { task, ticket, until: now.saturating_add(LATER_MS) }),
            _ => match chat::ask_key(key, mods, ticket, false) {
                Key::Act(Action::TalkAsk { act, .. }) => Key::Act(Action::DecisionsAsk { task, ticket, act }),
                other => other,
            },
        }
    }

    /// A key on triage: on its card ↵ sorts, S skips; on an item ↵ keeps it (under the project it
    /// suggests), T takes it now, L puts it off a day, N not mine, D dismisses, U counts its claims,
    /// C flips its role, P promotes the scope its key names, Z takes the last back. Esc closes.
    fn sort_key(&self, sort: &Sorting, key: &str, bare: bool, now: u64) -> Key {
        // Today ends at the user's next midnight; with no day yet, twelve hours on.
        let day = crate::filed::DAY_MS;
        let tonight = if self.local_day > 0 { self.local_day.saturating_add(day) } else { now.saturating_add(day / 2) };
        let revision = sort.item.and_then(|id| self.filed.get(id)).map_or(0, |d| d.revision);
        let act = |how| sort.item.map(|id| Action::SortItem { id, how, revision });
        let action = match (key, sort.item) {
            ("escape", _) => Some(Action::DecisionsClose),
            ("enter", None) => Some(Action::SortStart),
            ("s", None) if bare => Some(Action::SortSkip),
            ("enter", Some(_)) => act(crate::filed::Sort::Keep),
            ("z", _) if bare => Some(Action::SortUndo),
            (_, Some(id)) if bare => match key {
                "t" => act(crate::filed::Sort::Now { until: tonight.max(now) }),
                "l" => act(crate::filed::Sort::Later { until: now.saturating_add(day) }),
                "n" => act(crate::filed::Sort::NotMine),
                "d" => act(crate::filed::Sort::Dismiss),
                "u" => Some(Action::SortToggle { id, claims: true }),
                "c" => Some(Action::SortToggle { id, claims: false }),
                "p" => Some(Action::SortPromote { id }),
                _ => None,
            },
            _ => None,
        };
        action.map_or(Key::Pass, Key::Act)
    }

    /// A key on the week's card: ↵ takes the proposal (or keeps last week's), E edits it first, N
    /// keeps the focus as it is (or none for the new week), Esc closes Decisions.
    fn week_key(&self, card: super::focus::WeekCard, key: &str, bare: bool, now: u64) -> Key {
        use super::focus::{Seed, WeekCard};
        let action = match (key, card) {
            ("escape", _) => Action::DecisionsClose,
            ("enter", WeekCard::Proposal(revision)) => Action::FocusAccept { revision, at: now },
            ("enter", WeekCard::Carry) => Action::FocusCarry { keep: true, at: now },
            ("e", WeekCard::Proposal(revision)) if bare => Action::FocusEditOpen { from: Seed::Proposal(revision) },
            ("e", WeekCard::Carry) if bare => Action::FocusEditOpen { from: Seed::Last },
            ("n", WeekCard::Proposal(revision)) if bare => Action::FocusDecline { revision },
            ("n", WeekCard::Carry) if bare => Action::FocusCarry { keep: false, at: now },
            _ => return Key::Pass,
        };
        Key::Act(action)
    }

    /// A click on one of the week card's buttons: what its key does, wherever the keyboard is.
    pub fn week_click(&self, key: &str, now: u64, drawn: Option<(On, u32)>) -> Key {
        let Some(card) = self.decisions.as_ref().and_then(|run| run.week) else { return Key::Pass };
        if let Some(stop) = self.unseen(drawn) {
            return stop;
        }
        self.week_key(card, key, true, now)
    }

    /// A click on one of triage's buttons: what its key does, wherever the keyboard is.
    pub fn sort_click(&self, key: &str, mods: crate::keys::Mods, now: u64, drawn: Option<(On, u32)>) -> Key {
        let Some(sort) = self.decisions.as_ref().and_then(|run| run.sort.as_ref()) else { return Key::Pass };
        if let Some(stop) = self.unseen(drawn) {
            return stop;
        }
        self.sort_key(sort, key, !mods.cmd && !mods.ctrl && !mods.alt && !mods.shift, now)
    }

    /// A click or key on a card that is not the one drawn (another card, or another revision of it)
    /// takes nothing; on the one drawn while it is marked changed, it only says the user saw it
    /// (so the mark clears only once drawn). `None`: it may act.
    fn unseen(&self, drawn: Option<(On, u32)>) -> Option<Key> {
        let changed = self.decisions.as_ref().is_some_and(|run| run.changed);
        match (drawn != self.on_show(), changed) {
            (true, _) => Some(Key::Swallow),
            (false, true) => Some(Key::Act(Action::DecisionsSeen)),
            (false, false) => None,
        }
    }

    /// After every step, while triage shows: its item moves on once sorted (or gone); no item
    /// left, triage ends for this run. True while it still shows.
    fn settle_sort(&mut self) -> bool {
        let now = self.now;
        let Some(sort) = self.decisions.as_ref().and_then(|run| run.sort.as_ref()) else { return false };
        let on = self.features.on(Feature::DecisionSources);
        let waits = |id: u64| self.filed.get(id).is_some_and(|d| d.to_sort(now));
        // With nothing left it says "All sorted" while Z can still take the last back.
        let all_sorted = || (!sort.undo.is_empty()).then_some(None);
        let item = match sort.item {
            _ if !on => None,
            Some(id) if waits(id) => Some(Some(id)),
            Some(_) => self.filed.next_to_sort(now).map(Some).or_else(all_sorted),
            None => {
                (self.filed.to_sort(now) > 0).then_some(None).or_else(|| (sort.done > 0).then(all_sorted).flatten())
            }
        };
        let revised = sort.item.and_then(|id| self.filed.get(id)).is_some_and(|d| d.revision != sort.revision);
        let fresh = item.flatten() != sort.item || revised;
        let (done, undo) = (sort.done, sort.undo.clone());
        let next = item.map(|item| match fresh {
            true => self.sorting_at(item, done, undo),
            false => Sorting { left: self.filed.to_sort(now), ..sort.clone() },
        });
        let Some(run) = self.decisions.as_mut() else { return false };
        run.sort = next;
        run.sorted |= run.sort.is_none();
        run.sort.is_some()
    }

    /// The card pane is focused and ottospace has the keyboard: keys go to the card.
    pub fn keyboard_decision(&self) -> bool {
        let pane = self.decisions.as_ref().map(|run| run.pane);
        pane.is_some() && self.focused == pane && self.mode() == super::Mode::Normal && !self.keys
    }

    /// Whether a client's step would close the card or its workspace, or move the card out.
    pub(crate) fn touches_decisions(&self, action: &Action) -> bool {
        let Some(run) = &self.decisions else { return false };
        let tab_at = |index: &Option<usize>| self.tabs.get(index.unwrap_or(self.active)).map(|tab| &tab.space);
        let shown = self.shown_spaces();
        let selected = shown.iter().position(|space| space.of == *self.shown_space());
        let off_show = self.tabs[self.active].space != Owner::Decisions;
        let at_decisions = |index: &usize| self.tabs.get(*index).is_some_and(|tab| tab.space == Owner::Decisions);
        match action {
            // Nor takes the keyboard to it while the person is elsewhere.
            Action::Focus { pane } | Action::Enter { pane } => off_show && *pane == run.pane,
            Action::SelectTab { index } => off_show && at_decisions(index),
            Action::SpaceSelect { index } | Action::SpaceClick { index } => {
                off_show && shown.get(*index).is_some_and(|space| space.of == Owner::Decisions)
            }
            Action::SpaceOpen { space } => off_show && *space == Owner::Decisions,
            Action::Close { pane } => pane.or(self.focused) == Some(run.pane),
            Action::CloseTab { index } => tab_at(index) == Some(&Owner::Decisions),
            Action::SpaceHide { index } | Action::SpaceHideAsked { index, .. } => {
                index.or(selected).and_then(|at| shown.get(at)).is_some_and(|space| space.of == Owner::Decisions)
            }
            action => self.moves_decision_out(action),
        }
    }

    /// Whether a step would take the card pane out of its tab, or its tab out to another display:
    /// it does nothing, whoever sent it.
    pub(crate) fn moves_decision_out(&self, action: &Action) -> bool {
        let Some(run) = &self.decisions else { return false };
        let home = self.tab_of(run.pane);
        match action {
            Action::MovePane { pane, tab } => {
                pane.or(self.focused) == Some(run.pane) && self.tabs.get(*tab).map(|t| t.id) != home
            }
            Action::DropPane { source, target, .. } => {
                (*source == run.pane || *target == run.pane) && self.tab_of(*source) != self.tab_of(*target)
            }
            Action::TabOut { tab } => {
                let id = tab.unwrap_or(self.tabs[self.active].id);
                self.tabs.iter().any(|t| t.id == id && t.space == Owner::Decisions)
            }
            _ => false,
        }
    }

    pub(super) fn decisions_act(&mut self, action: Action, effects: &mut Vec<Effect>) {
        match action {
            Action::DecisionsOpen => self.open_decisions(effects),
            Action::DecisionsAsk { task, ticket, act } if self.decision_card() == Some((task, ticket)) => {
                let subject = Subject::Task(task);
                // As the drawer's card: a number picks, ↵ sends (owner, 2026-11-20).
                self.ask_act(&subject, ticket, act, effects);
                // Answered: the next card starts as one pane, before the step's own clean-ups.
                if !self.asking().contains(&(task, ticket)) {
                    self.decisions_clear(effects);
                }
                // Other…: the words are typed in the drawer, on the same card.
                let other = self.chat(&subject).and_then(chat::Chat::form).is_some_and(Form::on_other);
                if other && self.talk.as_ref().is_some_and(|talk| talk.task == subject) {
                    self.talk_keys(true, effects);
                }
            }
            Action::DecisionsSkip { task, ticket } if self.decision_card() == Some((task, ticket)) => {
                if let Some(run) = self.decisions.as_mut() {
                    run.done.push((task, ticket));
                }
                self.decisions_clear(effects);
            }
            Action::DecisionsLater { task, ticket, until } if self.decision_card() == Some((task, ticket)) => {
                if let Some(run) = self.decisions.as_mut() {
                    run.done.push((task, ticket));
                }
                self.tasks.snooze(task, until);
                // The snooze is Undo's now: an offer naming another move no longer stands.
                self.undo_offer = None;
                self.decisions_clear(effects);
            }
            Action::DecisionsClose => self.end_decisions(effects),
            _ => {}
        }
    }

    /// ⌘J, what needs the person next: a tab of Decisions on show, its card the queue's first.
    /// Open already: shown. Nothing waits: the next task, `TaskGo { id: None }` (owner, 2026-11-19).
    fn open_decisions(&mut self, effects: &mut Vec<Effect>) {
        if let Some(run) = &self.decisions {
            let pane = run.pane;
            return self.focus(pane, effects);
        }
        let queue = self.decision_queue();
        let (sortable, week) = (self.sortable(), self.week_card());
        if queue.is_empty() && !sortable && week.is_none() {
            return self.act(Action::TaskGo { id: None }, effects);
        }
        // Not from a window out: Decisions is the main window's.
        if self.away.is_some() {
            return;
        }
        let from = Some(self.tabs[self.active].id);
        self.leave(effects);
        self.leave_visual(effects);
        self.park_active();
        self.push_tab_of(Owner::Decisions);
        let pane = self.add(Kind::Decision, None, Axis::Right);
        effects.push(Effect::Opened(pane));
        let tab = self.tabs[self.active].id;
        // The agents' questions in focus first (all, with no focus set); then the week's card;
        // triage as one card; then the questions out of focus.
        let asks_first = queue.first().is_some_and(|w| w.why.tier() == 0);
        let week = week.filter(|_| !asks_first);
        let sort = (!asks_first && week.is_none() && sortable).then(|| self.sorting_at(None, 0, Vec::new()));
        let first = queue.first().filter(|_| week.is_none() && sort.is_none());
        let (task, ticket) = first.map(|first| (first.task, first.ticket)).unzip();
        let of = queue.len() + usize::from(sortable) + usize::from(self.week_card().is_some());
        let done = Vec::new();
        let (held, sorted, weeked, changed, seen) = (false, false, false, false, None);
        self.decisions = Some(Run {
            tab,
            pane,
            task,
            ticket,
            sort,
            week,
            at: 1,
            of,
            queue,
            done,
            from,
            held,
            sorted,
            weeked,
            changed,
            seen,
        });
        self.title_decisions();
    }

    /// Decisions ends: its card closes, and a pane the person added goes with its tab to the card
    /// task's workspace (or, the task gone, to a workspace as a stray tab does), still on show. Else, on show, the tab it was opened
    /// from shows again (else its neighbour's); in the background nothing on show changes.
    fn end_decisions(&mut self, effects: &mut Vec<Effect>) {
        let run = self.decisions.take();
        let (from, task) = (run.as_ref().and_then(|run| run.from), run.and_then(|run| run.task));
        // A pane the person added stays on show, its tab a workspace's now: nothing moves under them.
        let card = |id: &PaneId| self.pane(*id).is_some_and(|p| p.kind == Kind::Decision);
        let at = self.tabs.iter().position(|t| t.space == Owner::Decisions);
        let added =
            at.and_then(|at| self.tab_panes().get(at).cloned()).is_some_and(|panes| panes.iter().any(|id| !card(id)));
        // The person types to the card's agent in the drawer: they go on, in its task's workspace.
        let typing = self.talk.as_ref().filter(|talk| talk.keys).map(|talk| talk.task.clone());
        if let Some(subject) = typing.filter(|_| self.tabs[self.active].space == Owner::Decisions && !added) {
            self.open_space(self.subject_space(&subject), effects);
            if let Some(talk) = self.talk.as_mut().filter(|talk| talk.task == subject) {
                talk.keys = true;
            }
        }
        if self.tabs[self.active].space == Owner::Decisions && !added {
            let back = from.and_then(|id| self.tabs.iter().position(|t| t.id == id && t.space != Owner::Decisions));
            let home = || self.tabs.iter().position(|t| t.id == super::HOME_TAB);
            if let Some(to) = back.or_else(|| self.space_after(&Owner::Decisions)).or_else(home) {
                self.select_tab(to, effects);
            }
        }
        // Its task's workspace, while the task is there to have one; else a stray, for `settle_home`.
        // With nothing added it is empty, and closes as a stray on Home does.
        let to =
            task.filter(|id| added && self.tasks.live(*id).is_some()).map_or(Owner::Home, |id| self.task_space(id));
        for tab in self.tabs.iter_mut().filter(|t| t.space == Owner::Decisions) {
            tab.space = to.clone();
        }
        let cards: Vec<PaneId> = self.panes.iter().filter(|p| p.kind == Kind::Decision).map(|p| p.id).collect();
        for id in cards {
            self.close_pane(id, effects);
        }
        // What was of the card goes with it, as at the end of a step: this may come after it.
        let panes = &self.panes;
        self.inspector = self.inspector.take().filter(|i| panes.iter().any(|p| p.id == i.pane));
        self.settle_offers();
        self.settle_slots(effects);
        self.prune_unfilled();
    }

    /// After every step: the card on show is the queue's first. One answered (here or
    /// elsewhere), skipped, put off, or whose task ended leaves it, and the next shows; the panes
    /// the person added close with it. No card left, its pane gone, its space hidden, or
    /// the flag off: it ends. The age of each waiting card is kept whether it is open or not.
    /// Only the person's own act on the card (an answer, a skip, a later) closes the panes they
    /// added (`decisions_act`): a card that leaves by another's step leaves them as they are.
    /// While the person types in the drawer to the card's task (`typing`: when the step began),
    /// another's step moves no card: it is held (`decision_held`) until their own next step.
    pub(super) fn settle_decisions(&mut self, origin: super::Origin, typing: bool, effects: &mut Vec<Effect>) {
        let asking = self.asking();
        self.waiting.retain(|seen| asking.contains(&(seen.task, seen.ticket)));
        for (task, ticket) in asking.iter().copied() {
            if !self.waiting.iter().any(|seen| seen.task == task && seen.ticket == ticket) {
                self.waiting.push(Seen { task, ticket, since: self.now });
            }
        }
        let Some((pane, shown)) = self.decisions.as_ref().map(|run| (run.pane, run.task.zip(run.ticket))) else {
            // No run: nothing of one is left.
            let left = self.tabs.iter().any(|t| t.space == Owner::Decisions);
            if left || self.panes.iter().any(|p| p.kind == Kind::Decision) {
                self.end_decisions(effects);
            }
            return;
        };
        let at_home = self.tab_of(pane).and_then(|id| self.tabs.iter().find(|t| t.id == id));
        let hidden = self.spaces.iter().any(|s| s.of == Owner::Decisions && s.hidden);
        let held = at_home.is_some_and(|tab| tab.space == Owner::Decisions);
        if !self.features.on(Feature::Decisions) || !held || hidden {
            return self.end_decisions(effects);
        }
        // One tab: another made or moved here is a stray, for `settle_home` to give a workspace.
        let tab = self.tab_of(pane);
        for other in self.tabs.iter_mut().filter(|t| t.space == Owner::Decisions && Some(t.id) != tab) {
            other.space = Owner::Home;
        }
        // The week's card on show stays while it waits (a newer proposal takes its place); answered,
        // it does not come again this run.
        if self.decisions.as_ref().is_some_and(|run| run.week.is_some()) {
            let card = self.week_card();
            let queue = self.decision_queue();
            let sorts = usize::from(self.sortable());
            if let Some(run) = self.decisions.as_mut() {
                run.week = card;
                run.weeked |= card.is_none();
                if card.is_some() {
                    let done = run.done.len() + usize::from(run.sorted);
                    (run.at, run.of) = (done + 1, done + queue.len() + 1 + sorts);
                    run.queue = queue;
                    return self.title_decisions();
                }
            }
        }
        // Triage on show stays while items wait: nothing moves under the person.
        if self.settle_sort() {
            let queue = self.decision_queue();
            let weeks =
                usize::from(self.decisions.as_ref().is_some_and(|run| !run.weeked) && self.week_card().is_some());
            if let Some(run) = self.decisions.as_mut() {
                let done = run.done.len() + usize::from(run.weeked) + usize::from(run.sorted);
                (run.at, run.of) = (done + 1, done + queue.len() + 1 + weeks);
                run.queue = queue;
            }
            return self.title_decisions();
        }
        if let Some(shown) = shown {
            let on_card = self.talk.as_ref().is_some_and(|talk| talk.open && talk.task == Subject::Task(shown.0));
            // Only when the card would move: it left the queue.
            let was = self.decision_held();
            let held = origin != super::Origin::User
                && (typing || was)
                && (was || !asking.contains(&shown))
                && on_card
                && self.tasks.live(shown.0).is_some();
            let queue = self.decision_queue();
            if let Some(run) = self.decisions.as_mut() {
                run.held = held;
                if held {
                    run.queue = queue;
                    return;
                }
            }
            // Answered or withdrawn: dealt with. One only put behind another of its task waits still.
            let pending = self.chat(&Subject::Task(shown.0)).is_some_and(|chat| {
                chat.transcript.live && chat.transcript.cards.iter().any(|card| card.ticket == shown.1)
            });
            if !asking.contains(&shown)
                && !pending
                && let Some(run) = self.decisions.as_mut().filter(|run| !run.done.contains(&shown))
            {
                run.done.push(shown);
            }
        }
        let queue = self.decision_queue();
        // After the agents' questions in focus (all of them, with no focus set; and the one on
        // show stays), the week's card, then triage as one card, each once a run.
        let (weeked, sorted) = self.decisions.as_ref().map_or((true, true), |run| (run.weeked, run.sorted));
        let asks_first = queue.first().is_some_and(|w| w.why.tier() == 0 || shown == Some((w.task, w.ticket)));
        let week = self.week_card().filter(|_| !asks_first && !weeked);
        let sort = !asks_first && week.is_none() && self.sortable() && !sorted;
        let sorting = sort.then(|| self.sorting_at(None, 0, Vec::new()));
        if queue.is_empty() && sorting.is_none() && week.is_none() {
            return self.end_decisions(effects);
        }
        let weeks = usize::from(!weeked && self.week_card().is_some());
        let sorts = usize::from(sorting.is_some() || (!sorted && self.sortable()));
        if let Some(run) = self.decisions.as_mut() {
            // The week's card or triage on show: no question is.
            let first = queue.first().filter(|_| week.is_none() && sorting.is_none());
            (run.task, run.ticket) = first.map(|first| (first.task, first.ticket)).unzip();
            (run.sort, run.week) = (sorting, week);
            // The week's card and triage, dealt with in this run, count as one done each.
            let done = run.done.len() + usize::from(run.weeked) + usize::from(run.sorted);
            (run.at, run.of) = (done + 1, done + queue.len() + weeks + sorts);
            run.queue = queue;
        }
        self.title_decisions();
    }

    /// The card is held on show: the person typed in the drawer to its task (`settle_decisions`).
    pub fn decision_held(&self) -> bool {
        self.decisions.as_ref().is_some_and(|run| run.held)
    }

    /// The next card starts as one pane again: what the person added beside it closes.
    fn decisions_clear(&mut self, effects: &mut Vec<Effect>) {
        let Some(pane) = self.decisions.as_ref().map(|run| run.pane) else { return };
        let Some(tab) = self.tab_of(pane) else { return };
        let at = self.tabs.iter().position(|t| t.id == tab);
        let added: Vec<PaneId> = match at.and_then(|at| self.tab_panes().get(at).cloned()) {
            Some(panes) => panes.into_iter().filter(|id| *id != pane).collect(),
            None => Vec::new(),
        };
        for id in added {
            self.close_pane(id, effects);
        }
    }

    /// Its tab reads "Decisions · 2 of 3".
    fn title_decisions(&mut self) {
        let Some(run) = &self.decisions else { return };
        let (pane, title) = (run.pane, format!("{NAME} · {} of {}", run.at, run.of));
        let Some(id) = self.tab_of(pane) else { return };
        if let Some(run) = self.decisions.as_mut() {
            run.tab = id;
        }
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
            tab.title = title;
        }
    }
}

/// A step that answers the card on show, or opens Decisions: the card it puts there is the user's
/// next, not a change under them.
pub(super) fn answers(action: &Action) -> bool {
    use super::focus::Seed;
    matches!(
        action,
        Action::DecisionsOpen
            | Action::DecisionsAsk { .. }
            | Action::DecisionsSkip { .. }
            | Action::DecisionsLater { .. }
            | Action::SortStart
            | Action::SortSkip
            | Action::SortItem { .. }
            | Action::SortToggle { .. }
            | Action::SortPromote { .. }
            | Action::SortUndo
            | Action::FocusAccept { .. }
            | Action::FocusDecline { .. }
            | Action::FocusCarry { .. }
            | Action::FocusEditOpen { from: Seed::Proposal(_) | Seed::Last }
            | Action::FocusEditSave { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ask::Act;
    use crate::ask::tests::question;
    use crate::keys::Mods;
    use crate::model::Origin;
    use crate::model::tests::started;
    use crate::tasks::Assign;
    use crate::transcript::{Event, Outcome, Text};

    /// A task whose native agent runs, its first turn over.
    fn live(m: &mut Model, title: &str, priority: u8) -> TaskId {
        m.update(Action::TaskAdd {
            title: title.into(),
            priority,
            state: State::Triaged,
            tab: None,
            assign: Assign::default(),
        });
        let id = m.tasks.list.last().map(|task| task.id).unwrap_or_default();
        started(m, Action::TaskTalk { id });
        m.update(Action::AgentStarted { task: Subject::Task(id), session: None, at: 1 }.sent_by(Origin::System));
        events(m, id, vec![Event::TurnFinished { outcome: Outcome::Completed }]);
        id
    }

    fn events(m: &mut Model, id: TaskId, events: Vec<Event>) -> Vec<Effect> {
        m.update(Action::AgentEvents { task: Subject::Task(id), events, at: 5 }.sent_by(Origin::System))
    }

    /// Task `id`'s agent asks question `ticket`: of `options` options, single-select.
    fn asks(m: &mut Model, id: TaskId, ticket: u64, options: usize) {
        let (tool, label) = ("AskUserQuestion".to_string(), "AskUserQuestion".to_string());
        let ask = Event::Permission {
            ticket,
            tool,
            label,
            detail: Text::default(),
            len: 0,
            cut: false,
            claim: Text::default(),
            questions: vec![question(options, false)],
        };
        events(m, id, vec![Event::TurnStarted, ask]);
    }

    fn tick(m: &mut Model, at: u64) {
        m.update(Action::Tick { now: at }.sent_by(Origin::System));
    }

    /// Two tasks waiting on a question, the second of higher priority; the user on a terminal.
    fn two() -> (Model, TaskId, TaskId) {
        let mut m = Model::in_workspace();
        let a = live(&mut m, "login", 3);
        let b = live(&mut m, "cache", 1);
        tick(&mut m, 1_000);
        asks(&mut m, a, 1, 2);
        tick(&mut m, 2_000);
        asks(&mut m, b, 1, 3);
        (m, a, b)
    }

    fn queue(m: &Model) -> Vec<TaskId> {
        m.decision_queue().into_iter().map(|w| w.task).collect()
    }

    fn on_show(m: &Model) -> bool {
        m.tabs[m.active].space == Owner::Decisions
    }

    #[test]
    fn the_queue_is_priority_then_the_longest_waiting_and_the_card_on_show_stays_first() {
        let (mut m, a, b) = two();
        assert_eq!(queue(&m), [b, a], "P1 before P3");
        let c = live(&mut m, "docs", 1);
        tick(&mut m, 3_000);
        asks(&mut m, c, 1, 2);
        assert_eq!(queue(&m), [b, c, a], "two P1: the one waiting longest first");
        m.update(Action::DecisionsOpen);
        assert_eq!(m.decision_card(), Some((b, 1)));
        // A more urgent card that comes meanwhile is next, not now.
        let d = live(&mut m, "urgent", 1);
        asks(&mut m, d, 1, 2);
        assert_eq!(m.decision_card(), Some((b, 1)));
        assert_eq!(m.decisions.as_ref().map(|run| run.queue[0].task), Some(b));
    }

    #[test]
    fn open_answer_next_and_it_closes_after_the_last() {
        let (mut m, a, b) = two();
        let from = m.tabs[m.active].id;
        // Only the user opens it.
        for origin in [Origin::Socket, Origin::System] {
            m.update(Action::DecisionsOpen.sent_by(origin));
            assert!(m.decisions.is_none(), "{origin:?}");
        }
        m.update(Action::DecisionsOpen);
        let run = m.decisions.clone().unwrap();
        assert!(on_show(&m) && m.focused == Some(run.pane));
        assert_eq!((run.task, run.at, run.of), (Some(b), 1, 2));
        assert_eq!(m.tabs[m.active].title, "Decisions · 1 of 2");
        // The drawer and the right rail are on the card's task.
        assert_eq!(m.talk.as_ref().map(|talk| talk.task.clone()), Some(Subject::Task(b)));
        assert_eq!(m.rail().first(), Some(&crate::model::rail::RailItem::Task { id: b }));
        // A number picks and sends nothing, pressed twice too; ↵ sends the pick and the agent resumes.
        let key = m.decision_key("2", Mods::default(), 0);
        assert_eq!(key, Key::Act(Action::DecisionsAsk { task: b, ticket: 1, act: Act::Number { n: 2 } }));
        let Key::Act(pick) = key else { unreachable!() };
        for _ in 0..2 {
            let effects = m.update(pick.clone());
            assert!(!effects.iter().any(|e| matches!(e, Effect::AgentAnswers { .. })), "{effects:?}");
            assert_eq!(m.decision_card(), Some((b, 1)));
        }
        let Key::Act(send) = m.decision_key("enter", Mods::default(), 0) else { panic!() };
        let effects = m.update(send);
        let sent = effects.iter().find_map(|e| match e {
            Effect::AgentAnswers { ticket: 1, answers, .. } => Some(answers[0].options.clone()),
            _ => None,
        });
        assert_eq!(sent, Some(vec![1]), "the second option: {effects:?}");
        assert_eq!(m.decision_card(), Some((a, 1)));
        assert_eq!(m.tabs[m.active].title, "Decisions · 2 of 2");
        // The last: it closes, and the tab it was opened from shows again.
        m.update(Action::DecisionsAsk { task: a, ticket: 1, act: Act::Number { n: 1 } });
        m.update(Action::DecisionsAsk { task: a, ticket: 1, act: Act::Submit });
        assert!(m.decisions.is_none() && !on_show(&m) && m.tabs[m.active].id == from);
        assert!(
            !m.tabs.iter().any(|t| t.space == Owner::Decisions) && !m.panes.iter().any(|p| p.kind == Kind::Decision)
        );
    }

    /// ⌘J with no card waiting goes to the next task that needs the person, by `TaskGo`'s rule;
    /// with the flag off ⌘J is that rule alone.
    #[test]
    fn with_nothing_waiting_it_goes_to_the_next_task_as_next_task_does() {
        let (mut m, a, b) = two();
        assert_eq!(m.next_needs_you(), Action::DecisionsOpen);
        for (task, n) in [(b, 3), (a, 2)] {
            m.update(Action::DecisionsOpen);
            m.update(Action::DecisionsAsk { task, ticket: 1, act: Act::Number { n } });
            m.update(Action::DecisionsAsk { task, ticket: 1, act: Act::Submit });
        }
        let c = live(&mut m, "docs", 2);
        m.update(Action::TaskTalk { id: a });
        assert!(m.decision_queue().is_empty() && m.tasks.next().is_some_and(|t| t.id == c));
        // A client's is refused: nothing moves.
        let (talk, active) = (m.talk.clone(), m.active);
        m.update(Action::DecisionsOpen.sent_by(Origin::Socket));
        assert_eq!((&m.talk, m.active), (&talk, active));
        // The user's: c's space and conversation, as `TaskGo { id: None }` gives.
        m.update(Action::DecisionsOpen);
        assert!(m.decisions.is_none() && !on_show(&m));
        assert_eq!(m.talk.as_ref().map(|talk| talk.task.clone()), Some(Subject::Task(c)));
        assert_eq!(*m.shown_space(), m.task_space(c));
        // Nothing needs the person either: nothing happens, as for ⌘J with the flag off.
        let mut m = Model::in_workspace();
        let (tabs, active) = (m.tabs.len(), m.active);
        m.update(Action::DecisionsOpen);
        assert_eq!((m.decisions.is_none(), m.talk.is_none(), m.tabs.len(), m.active), (true, true, tabs, active));
        m.update(Action::SetFeatures { features: crate::features::Features::all().with(Feature::Decisions, false) });
        assert_eq!(m.next_needs_you(), Action::TaskGo { id: None });
    }

    #[test]
    fn skip_leaves_it_for_next_time_and_later_snoozes_its_task() {
        let (mut m, a, b) = two();
        m.update(Action::DecisionsOpen);
        let Key::Act(skip) = m.decision_key("s", Mods::default(), 0) else { panic!() };
        m.update(skip);
        assert_eq!((m.decision_card(), m.decisions.as_ref().map(|r| (r.at, r.of))), (Some((a, 1)), Some((2, 2))));
        let Key::Act(later) = m.decision_key("l", Mods::default(), 10_000) else { panic!() };
        assert_eq!(later, Action::DecisionsLater { task: a, ticket: 1, until: 10_000 + LATER_MS });
        // A client's is refused.
        m.update(later.clone().sent_by(Origin::Socket));
        assert_eq!(m.decision_card(), Some((a, 1)));
        m.update(later);
        assert!(m.decisions.is_none(), "the last card dealt with");
        assert_eq!(m.tasks.get(a).and_then(|t| t.snoozed_until), Some(10_000 + LATER_MS));
        // Next time: the skipped one, not the one put off.
        m.update(Action::DecisionsOpen);
        assert_eq!(queue(&m), [b]);
    }

    #[test]
    fn a_card_answered_elsewhere_leaves_and_the_view_moves_on_without_taking_the_screen() {
        let (mut m, a, b) = two();
        m.update(Action::DecisionsOpen);
        // The person goes back to their workspace; the card on show is answered in its drawer.
        m.update(Action::SpaceSelect { index: 1 });
        assert!(!on_show(&m));
        let before = m.tabs[m.active].id;
        events(&mut m, b, vec![Event::PermissionClosed { ticket: 1 }]);
        assert_eq!(m.decision_card(), Some((a, 1)));
        assert_eq!(m.tabs[m.active].id, before, "nothing on show changed");
        // Its task ends: the last card is gone, and Decisions with it, in the background.
        m.update(Action::AgentExited { task: Subject::Task(a) }.sent_by(Origin::System));
        assert!(m.decisions.is_none() && m.tabs[m.active].id == before);
    }

    #[test]
    fn panes_added_to_a_card_close_with_it_and_esc_closes_decisions() {
        let (mut m, _, b) = two();
        m.update(Action::DecisionsOpen);
        let card = m.decisions.as_ref().map(|run| run.pane).unwrap();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        assert_eq!(m.tab_panes()[m.active].len(), 2);
        // The card stays in its tab, and keys there are the terminal's.
        assert!(m.moves_decision_out(&Action::MovePane { pane: Some(card), tab: 0 }));
        assert_eq!(m.decision_key("s", Mods::default(), 0), Key::Pass);
        m.update(Action::DecisionsSkip { task: b, ticket: 1 });
        assert_eq!(m.tab_panes()[m.active], [card], "the next card starts as one pane");
        assert_ne!(m.decision_card().map(|(task, _)| task), Some(b));
        m.update(Action::Focus { pane: card });
        let Key::Act(close) = m.decision_key("escape", Mods::default(), 0) else { panic!() };
        m.update(close);
        assert!(m.decisions.is_none() && !on_show(&m));
    }

    #[test]
    fn a_client_neither_closes_nor_hides_it_and_the_flag_off_ends_it() {
        let (mut m, ..) = two();
        m.update(Action::DecisionsOpen);
        for action in
            [Action::Close { pane: None }, Action::CloseTab { index: None }, Action::SpaceHide { index: None }]
        {
            assert_eq!(m.refusal(&action, Origin::Socket), Some(USERS_ALONE), "{action:?}");
            m.update(action.sent_by(Origin::Socket));
            assert!(m.decisions.is_some());
        }
        // The user's ⌘W on the card closes it.
        m.update(Action::Close { pane: None });
        assert!(m.decisions.is_none());
        m.update(Action::DecisionsOpen);
        m.update(Action::SetFeatures { features: crate::features::Features::all().with(Feature::Decisions, false) });
        assert!(m.decisions.is_none() && !m.tabs.iter().any(|t| t.space == Owner::Decisions));
        m.update(Action::DecisionsOpen);
        assert!(m.decisions.is_none(), "off: nothing opens");
    }

    /// Task `id` is of project `project`; this week's focus is that project.
    fn in_focus(m: &mut Model, id: TaskId, project: &str) {
        if let Some(task) = m.tasks.list.iter_mut().find(|t| t.id == id) {
            task.project = Some(project.into());
        }
        m.update(Action::SetWeek { week: "2026-W48".into() }.sent_by(Origin::System));
        let area = crate::focus::Area { name: "Login rewrite".into(), projects: vec![project.into()], repos: vec![] };
        m.focus.set(&[area], crate::focus::SetBy::User, 1);
    }

    #[test]
    fn this_weeks_focus_puts_its_questions_first_then_triage_then_the_rest() {
        let (mut m, a, b) = two();
        assert_eq!(queue(&m), [b, a], "no focus: P1 first");
        in_focus(&mut m, a, "atlas");
        assert_eq!(queue(&m), [a, b], "login is in focus: before the P1 out of it");
        assert_eq!(m.decision_queue()[0].area, Some(0));
        m.update(
            Action::DecisionFile { filing: Box::new(crate::filed::tests::filing()), now: 2_500 }
                .sent_by(Origin::Socket),
        );
        m.update(Action::DecisionsOpen);
        assert_eq!(m.decision_card(), Some((a, 1)));
        let at = |m: &Model| m.decisions.as_ref().map(|run| (run.at, run.of));
        assert_eq!(at(&m), Some((1, 3)));
        // Skipped: the sort card comes before the question out of focus.
        let press = |m: &mut Model, key: &str| {
            let Key::Act(action) = m.decision_key(key, Mods::default(), 3_000) else { panic!("{key}") };
            m.update(action);
        };
        press(&mut m, "s");
        assert!(m.decisions.as_ref().is_some_and(|run| run.sort.is_some() && run.task.is_none()));
        assert_eq!(at(&m), Some((2, 3)));
        // Sorted: "All sorted", and ↵ goes on to the question left, the third of three.
        press(&mut m, "enter");
        press(&mut m, "enter");
        assert!(m.decisions.as_ref().is_some_and(|run| run.sort.as_ref().is_some_and(|s| s.item.is_none())));
        press(&mut m, "enter");
        assert_eq!((m.decision_card(), at(&m)), (Some((b, 1)), Some((3, 3))));
        // ⌘J's task and the Tasks list: in focus first.
        assert_eq!(m.next_task(), Some(a));
        let rows = m.task_list_rows(crate::model::TasksView::State, true);
        let place = |id| rows.iter().position(|r| *r == crate::list::ListRow::Task(id));
        assert!(place(a) < place(b), "{rows:?}");
    }

    #[test]
    fn with_every_question_out_of_focus_triage_comes_first() {
        let (mut m, a, b) = two();
        in_focus(&mut m, 99, "dune");
        m.update(
            Action::DecisionFile { filing: Box::new(crate::filed::tests::filing()), now: 2_500 }
                .sent_by(Origin::Socket),
        );
        m.update(Action::DecisionsOpen);
        assert!(m.decisions.as_ref().is_some_and(|run| run.sort.is_some() && run.task.is_none()));
        let Key::Act(skip) = m.decision_key("s", Mods::default(), 3_000) else { panic!() };
        m.update(skip);
        assert_eq!(queue(&m), [b, a], "then the rest by priority");
    }

    #[test]
    fn a_task_in_a_worktree_is_in_its_repositorys_area() {
        let (mut m, a, b) = two();
        m.repos = vec![crate::repos::Repo {
            name: "app".into(),
            path: "/code/app".into(),
            branch: None,
            worktrees: vec![crate::repos::Worktree { name: "w".into(), path: "/wt/app/w".into(), branch: None }],
        }];
        if let Some(task) = m.tasks.list.iter_mut().find(|t| t.id == a) {
            task.repo = Some("/wt/app/w".into());
        }
        m.update(Action::SetWeek { week: "2026-W48".into() }.sent_by(Origin::System));
        let area = crate::focus::Area { name: "App".into(), projects: vec![], repos: vec!["/code/app".into()] };
        m.focus.set(&[area], crate::focus::SetBy::User, 1);
        assert_eq!(queue(&m), [a, b]);
    }

    #[test]
    fn a_sorted_decision_ranks_in_its_areas_tier_and_counts_only_what_was_accepted() {
        use crate::filed::{Role, Triage};
        use crate::rank::Card;
        let (mut m, a, _) = two();
        in_focus(&mut m, a, "atlas");
        let mut f = crate::filed::tests::filing();
        (f.suggest.projects, f.suggest.role) = (vec!["atlas".into()], Some(Role::Core));
        (f.claims.person_waits, f.claims.due) = (true, Some(2_000 + 5 * 3_600_000));
        m.update(Action::DecisionFile { filing: Box::new(f), now: 2_000 }.sent_by(Origin::Socket));
        let id = m.filed.list[0].id;
        let filed = |m: &Model| m.ranked().into_iter().find(|e| e.card == Card::Filed(id));
        assert!(filed(&m).is_none(), "in triage: in no ranked queue");
        // Sorted under atlas, core, its claims not counted: in focus, C × U = 2.
        let d = &mut m.filed.list[0];
        (d.triage, d.accepted.project, d.accepted.role) = (Triage::Sorted, Some("atlas".into()), Some(Role::Core));
        let why = filed(&m).map(|e| e.why).unwrap_or_default();
        assert_eq!((why.area, why.person_waits, why.due, why.score(2_000)), (Some(0), false, None, 2));
        // Counted (u): someone waits, due within the day: 2 × 3.
        m.filed.list[0].accepted.claims = true;
        assert_eq!(filed(&m).map(|e| e.why.score(2_000)), Some(6));
        // Still after the question in its tier: an agent waits there.
        let order: Vec<Card> = m.ranked().into_iter().map(|e| e.card).collect();
        assert_eq!(&order[..2], &[Card::Ask { task: a, ticket: 1 }, Card::Filed(id)]);
        // Taken now: first of its tier, before the question.
        m.filed.list[0].accepted.now_until = Some(u64::MAX);
        assert_eq!(m.ranked()[0].card, Card::Filed(id));
    }

    #[test]
    fn a_blocked_task_counts_only_while_its_agent_asks() {
        use crate::filed::Triage;
        use crate::rank::Card;
        let (mut m, a, _) = two();
        let idle = live(&mut m, "idle", 5);
        let mut f = crate::filed::tests::filing();
        f.claims.blocks = vec![idle];
        m.update(Action::DecisionFile { filing: Box::new(f), now: 2_000 }.sent_by(Origin::Socket));
        m.filed.list[0].triage = Triage::Skipped;
        let waits =
            |m: &Model| m.ranked().into_iter().find(|e| matches!(e.card, Card::Filed(_))).map(|e| e.why.agent_waits);
        assert_eq!(waits(&m), Some(false), "its agent asks nothing");
        m.filed.list[0].claims.blocks = vec![a];
        assert_eq!(waits(&m), Some(true));
    }
}
