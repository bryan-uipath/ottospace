//! The model's side of agent mode (`crate::agent_mode`): each app pane's grant, kept or reset as
//! panes, workspaces and tasks change; the user's level and menu; and who may read a pane. Who may
//! send each action is `Model::refusal`.

use super::{Action, Effect, Kind, Model, Origin, PaneId, STALE_PASS};
use crate::agent_mode::{self, Ahead, AppUse, Grant, Kept, Level, Menu, PaneView, TabView, WorkspaceView};
use crate::features::Feature;
use crate::tasks::{Caller, State, TaskId};
use crate::transcript::Notice;

impl Model {
    /// The tasks working in the workspace of `pane`'s tab, its front one first: a task's own
    /// space and those that joined it, none done. Empty for Home, a workbook, a pane in no tab.
    pub(super) fn pane_tasks(&self, pane: PaneId) -> Vec<TaskId> {
        let Some(tab) = self.tab_of(pane).and_then(|id| self.tabs.iter().find(|tab| tab.id == id)) else {
            return Vec::new();
        };
        let done = |id: &TaskId| self.tasks.get(*id).is_none_or(|task| task.state == State::Done);
        let mut working: Vec<TaskId> = self.members(&tab.space).into_iter().filter(|id| !done(id)).collect();
        if let Some(front) = self.front(&tab.space).filter(|front| working.contains(front)) {
            working.retain(|id| *id != front);
            working.insert(0, front);
        }
        working
    }

    /// After every step (`origin`: whose it was): every app pane has a grant, to a task that works
    /// in its workspace or to none. A new pane starts at the default only where a task works and
    /// the user's own act put its window there (`by_user`); else at Off, until the user picks a
    /// level: a client that opens Mail in its task's workspace cannot read it; and none while the flag
    /// is off. One whose task no
    /// longer works there (it moved to another workspace, the task ended, was dismissed or unbound)
    /// is Off, for the task now there: a grant is never carried to a task the user did not give it
    /// to. A closed pane's goes. Over a restart only a level the user picked is kept
    /// (`AgentMode::kept`), and read back it is only an offer: a restored pane is Off, and its
    /// header offers the level once the same app's window fills it, from no client, while that
    /// task works there; the user's click gives it (`AgentOfferAccept`). Grants are kept, and kept true,
    /// while the flag is off.
    pub(super) fn settle_agent(&mut self, origin: Origin) {
        let apps: Vec<PaneId> = self
            .tab_panes()
            .into_iter()
            .flatten()
            .filter(|id| matches!(self.pane(*id).map(|pane| &pane.kind), Some(Kind::App { .. })))
            .collect();
        // Who filled each app pane: the step it first was one in. Emptied and filled again, anew.
        self.agent.by_user.retain(|pane, _| apps.contains(pane));
        for pane in &apps {
            let from_record = self.agent.from_record.contains(pane);
            self.agent.by_user.entry(*pane).or_insert(origin == Origin::User && !from_record);
        }
        let empty = |model: &Model, pane: &PaneId| model.pane(*pane).is_some_and(|pane| pane.kind == Kind::Empty);
        let from_record = std::mem::take(&mut self.agent.from_record);
        self.agent.from_record = from_record.into_iter().filter(|pane| empty(self, pane)).collect();
        // The flag off hides the robot and refuses every read; the grants are kept as they are,
        // so turning it off and on again (a settings write) raises no level.
        if !self.features.on(Feature::AgentMode) {
            self.agent.menu = None;
        }
        let mut panes = std::collections::BTreeMap::new();
        for ids in self.tab_panes() {
            for id in ids {
                if !matches!(self.pane(id).map(|pane| &pane.kind), Some(Kind::App { .. })) {
                    continue;
                }
                let working = self.pane_tasks(id);
                let front = working.first().copied();
                let grant = match self.agent.panes.get(&id) {
                    Some(grant) if grant.task.is_some_and(|task| working.contains(&task)) => *grant,
                    Some(_) => {
                        self.agent.chosen.remove(&id);
                        // Picked before any task worked here: offered to the first, if the user's
                        // own step brought it (their ↵ in the box). A client's task gets nothing.
                        let here = self.tab_of(id);
                        let ahead = self.agent.ahead.get(&id).filter(|ahead| Some(ahead.tab) == here);
                        let users = origin == Origin::User;
                        if let (Some(level), Some(task), true) = (ahead.map(|ahead| ahead.level), front, users) {
                            self.agent.ahead.remove(&id);
                            self.agent.offers.insert(id, Kept { level, task });
                        }
                        Grant { level: Level::Off, task: front }
                    }
                    None => {
                        // A restored pane with a kept level starts at Off whoever fills it: the
                        // file only offers. And none starts above Off while the flag is off: a
                        // settings write that turns it on must find nothing granted.
                        let kept = self.agent.restored.contains_key(&id);
                        self.offer_back(id, origin, &working);
                        let users = self.agent.by_user.get(&id).copied().unwrap_or(false) && !kept;
                        let on = self.features.on(Feature::AgentMode);
                        let level = if front.is_some() && users && on { self.agent.default } else { Level::Off };
                        Grant { level, task: front }
                    }
                };
                panes.insert(id, grant);
            }
        }
        // A level picked ahead stands while its pane is in the workspace it was picked in, no task there.
        let ahead = std::mem::take(&mut self.agent.ahead);
        let stays = |pane: &PaneId, ahead: &Ahead| {
            panes.get(pane).is_some_and(|grant| grant.task.is_none()) && self.tab_of(*pane) == Some(ahead.tab)
        };
        self.agent.ahead = ahead.into_iter().filter(|(pane, ahead)| stays(pane, ahead)).collect();
        // The menu is for the task it showed: its pane gone, or given to another task, it closes.
        let before = std::mem::replace(&mut self.agent.panes, panes);
        self.agent.chosen.retain(|pane| self.agent.panes.contains_key(pane));
        // An offer stands while its pane is at Off, for a task that still works there.
        let stands = |model: &Model, pane: &PaneId, kept: &Kept| {
            model.agent.panes.get(pane).is_some_and(|grant| grant.level == Level::Off)
                && model.pane_tasks(*pane).contains(&kept.task)
        };
        let offers = std::mem::take(&mut self.agent.offers);
        self.agent.offers = offers.into_iter().filter(|(pane, kept)| stands(self, pane, kept)).collect();
        // A kept level waits only while its pane does: empty, for its window.
        let waits = |model: &Model, pane: &PaneId| model.pane(*pane).is_some_and(|pane| pane.kind == Kind::Empty);
        let restored = std::mem::take(&mut self.agent.restored);
        self.agent.restored = restored.into_iter().filter(|(pane, _)| waits(self, pane)).collect();
        let panes = &self.agent.panes;
        self.agent.last.retain(|pane, _| panes.contains_key(pane));
        let moved = |pane: &PaneId| panes.get(pane).map(|grant| grant.task) != before.get(pane).map(|grant| grant.task);
        if self.agent.menu.is_some_and(|menu| moved(&menu.pane) || !panes.contains_key(&menu.pane)) {
            self.agent.menu = None;
        }
    }

    /// The level the user picked for each app pane of `record`, and for which task, on its pane
    /// (`agent_mode::KEY`); no other. Read back (a restart, a tab mark) it is only offered (`wait_for_kept`).
    pub(crate) fn keep_levels(&self, record: &mut crate::marks::TabRecord) {
        for pane in &mut record.panes {
            pane.extra.remove(agent_mode::KEY);
            let kept = self.agent.kept(pane.id).filter(|_| matches!(pane.kind, crate::marks::RecordKind::App { .. }));
            if let Some(kept) = kept.and_then(|kept| serde_json::to_value(kept).ok()) {
                pane.extra.insert(agent_mode::KEY.into(), kept);
            }
        }
    }

    /// A tab mark of `pane`'s tab keeps the level the user has for it now: one picked since is
    /// offered back, one reset to Off is not. Its pane by place, while the tab has as many as the mark
    /// and the pane there records the same app (a swap since leaves the mark as it was). Marks off:
    /// they stay as stored (seed 7205).
    fn mark_level(&mut self, pane: PaneId) {
        if !self.features.on(Feature::Marks) {
            return;
        }
        let Some(at) = self.tabs.iter().position(|tab| Some(tab.id) == self.tab_of(pane)) else { return };
        let (tab, leaves) = (self.tabs[at].id, self.tab_panes().swap_remove(at));
        let Some(place) = leaves.iter().position(|id| *id == pane) else { return };
        let Some(live) = self.pane(pane).map(|open| open.kind.clone()) else { return };
        let kept = self.agent.kept(pane).and_then(|kept| serde_json::to_value(kept).ok());
        for record in self.marks.tab_records(tab) {
            if record.panes.len() != leaves.len() {
                continue;
            }
            let Some(kept_pane) = record.panes.get_mut(place) else { continue };
            if matches!(kept_pane.kind, crate::marks::RecordKind::App { .. }) && kept_pane.kind.is(&live) {
                match &kept {
                    Some(kept) => drop(kept_pane.extra.insert(agent_mode::KEY.into(), kept.clone())),
                    None => drop(kept_pane.extra.remove(agent_mode::KEY)),
                }
            }
        }
    }

    /// The panes `panes` rebuilt from `record` (in `shown` order): each empty app pane whose record
    /// keeps a level, for a task still there, waits for its window, to offer it (`offer_back`).
    pub(crate) fn wait_for_kept(&mut self, panes: &[PaneId], record: &crate::marks::TabRecord) {
        for (pane, kept) in panes.iter().zip(record.shown()) {
            let picked = kept.extra.get(agent_mode::KEY).cloned();
            let picked = picked.and_then(|value| serde_json::from_value::<Kept>(value).ok());
            if let (Some(picked), crate::marks::RecordKind::App { bundle, name, .. }) = (picked, &kept.kind)
                && self.tasks.live(picked.task).is_some()
                && self.pane(*pane).is_some_and(|open| open.kind == Kind::Empty)
            {
                let waiting = agent_mode::Waiting { kept: picked, bundle: bundle.clone(), name: name.clone() };
                self.agent.restored.insert(*pane, waiting);
            }
        }
    }

    /// A restored pane's level the user picked in a run before, once its window is there: offered,
    /// never given (the file is the same user's, so an agent's too). Only for a window of the same
    /// app, filled by no client, while its task works there.
    fn offer_back(&mut self, pane: PaneId, origin: Origin, working: &[TaskId]) {
        let Some(waiting) = self.agent.restored.remove(&pane) else { return };
        let same = match self.pane(pane).map(|pane| &pane.kind) {
            Some(Kind::App { bundle, name, .. }) => {
                crate::marks::same_app((bundle.as_deref(), name), (waiting.bundle.as_deref(), &waiting.name))
            }
            _ => false,
        };
        if same && origin != Origin::Socket && working.contains(&waiting.kept.task) {
            self.agent.offers.insert(pane, waiting.kept);
        }
    }

    /// Who may read `pane` for `caller`, and as which task: the pane is an app pane, its grant is to
    /// the caller's task, and its level is See or more; at most one read (or look) a second. The
    /// shell asks before it reads and again before it delivers (Off since: dropped).
    pub fn app_access(&self, pane: PaneId, caller: Caller, what: AppUse, now: u64) -> Result<TaskId, String> {
        if !self.features.on(Feature::AgentMode) {
            return Err(Feature::AgentMode.off().into());
        }
        let task = match caller {
            Caller::Anyone => return Err(agent_mode::NO_PASS.into()),
            Caller::Unknown => return Err(STALE_PASS.into()),
            Caller::Task(task) => task,
        };
        let Some(grant) = self.agent.panes.get(&pane) else { return Err(format!("pane {pane} is not an app pane")) };
        if grant.task != Some(task) {
            return Err(format!(
                "pane {pane} is not in task {task}'s workspace: a task sees only its own workspace's apps"
            ));
        }
        if grant.level < Level::See {
            return Err(format!(
                "agent mode is off on pane {pane}: ask the user to choose See in its header (the robot, or ⌘⌃V)"
            ));
        }
        let last = self.agent.last.get(&pane).and_then(|uses| uses.get(&what));
        if last.is_some_and(|at| now.saturating_sub(*at) < agent_mode::EVERY_MS) {
            return Err(format!("pane {pane} was {} less than a second ago: try again in a moment", what.verb()));
        }
        Ok(task)
    }

    /// Whether `caller` may read terminal pane `pane` (`capture`, `wait`): a task's agent (its pass)
    /// only a pane of its own workspace, so its tools that run unasked read nothing of another
    /// task's or of the user's elsewhere; a made-up or dead pass nothing. With no pass, as before:
    /// any same-user client reads every terminal in `state` anyway.
    pub fn pane_access(&self, pane: PaneId, caller: Caller) -> Result<(), String> {
        match caller {
            Caller::Anyone => Ok(()),
            Caller::Unknown => Err(STALE_PASS.into()),
            Caller::Task(task) => {
                let space = self.tab_of(pane).and_then(|id| self.tabs.iter().find(|tab| tab.id == id));
                let own = space.is_some_and(|tab| tab.space == self.task_space(task));
                own.then_some(()).ok_or_else(|| {
                    format!("pane {pane} is not in task {task}'s workspace: a task's agent reads only its own panes")
                })
            }
        }
    }

    /// The workspace `caller`'s task works in, else the one `pane` is in: its tabs, their panes and
    /// each app pane's grant. A made-up or dead pass is refused, never answered as no task.
    pub fn workspace_view(&self, caller: Caller, pane: Option<PaneId>) -> Result<WorkspaceView, String> {
        let (task, space) = match caller {
            Caller::Task(task) => (Some(task), self.task_space(task)),
            Caller::Unknown => return Err(STALE_PASS.into()),
            Caller::Anyone => {
                let tab = pane.and_then(|pane| self.tab_of(pane)).and_then(|id| self.tabs.iter().find(|t| t.id == id));
                let tab = tab.filter(|tab| tab.space != super::Owner::Home);
                (None, tab.ok_or(agent_mode::NO_WORKSPACE)?.space.clone())
            }
        };
        let view = |id: PaneId| {
            let pane = self.pane(id)?;
            let grant = self.agent.panes.get(&id).copied();
            Some(PaneView { id, kind: pane.kind.clone(), cwd: pane.cwd.clone(), grant })
        };
        let tabs = self.tabs.iter().zip(self.tab_panes()).filter(|(tab, _)| tab.space == space);
        let tabs = tabs.map(|(tab, panes)| TabView {
            id: tab.id,
            title: tab.title.clone(),
            panes: panes.into_iter().filter_map(view).collect(),
        });
        let (tabs, name) = (tabs.collect(), self.space_title(&space));
        Ok(WorkspaceView { task, space, name, tabs, agent_mode: self.features.on(Feature::AgentMode) })
    }

    /// The pane the menu, a level or the key is for: the one named, else the focused one.
    fn agent_pane(&self, pane: Option<PaneId>) -> Option<PaneId> {
        pane.or(self.focused).filter(|pane| self.agent.panes.contains_key(pane))
    }

    pub(super) fn agent_act(&mut self, action: Action, effects: &mut Vec<Effect>) {
        match action {
            Action::AgentLevel { pane, level } => self.agent_level(pane, level, effects),
            Action::AgentMenu { pane, keys } => {
                let Some(pane) = self.agent_pane(pane) else {
                    return effects.push(Effect::Notice { text: agent_mode::FOCUS_APP.into() });
                };
                let open = self.agent.menu.take().is_some_and(|menu| menu.pane == pane);
                if !open {
                    self.agent.menu = Some(Menu { pane, selected: keys.then_some(0) });
                }
            }
            Action::AgentMenuMove { delta } => {
                if let Some(menu) = self.agent.menu.as_mut() {
                    let count = Level::ALL.len() as i64;
                    let from = menu.selected.map_or(if delta > 0 { -1 } else { count }, |at| at as i64);
                    menu.selected = Some((from + i64::from(delta)).rem_euclid(count) as usize);
                }
            }
            Action::AgentMenuChoose { index } => {
                let Some(menu) = self.agent.menu else { return };
                // ↵ with the keys on no item: the menu stays.
                let Some(level) = index.or(menu.selected).and_then(|at| Level::ALL.get(at).copied()) else { return };
                self.agent_level(Some(menu.pane), level, effects);
            }
            Action::AgentMenuClose => self.agent.menu = None,
            Action::AgentOfferAccept { pane } => self.agent_offer_accept(pane, effects),
            Action::AppUsed { task, pane, what, refused, at } => self.app_used(task, pane, what, refused, at),
            _ => {}
        }
    }

    /// The user's click on a restored pane's offer: the level kept for it, for its task.
    fn agent_offer_accept(&mut self, pane: PaneId, effects: &mut Vec<Effect>) {
        let Some(kept) = self.agent.offers.remove(&pane) else {
            return effects.push(Effect::Notice { text: agent_mode::NO_OFFER.into() });
        };
        if let Some(grant) = self.agent.panes.get_mut(&pane) {
            *grant = Grant { level: kept.level, task: Some(kept.task) };
            self.agent.chosen.insert(pane);
        }
        self.mark_level(pane);
    }

    /// The user's level for a pane. Where no task works it grants nothing: it is kept for the next
    /// task there (`Ahead`), and the status bar says so.
    fn agent_level(&mut self, pane: Option<PaneId>, level: Level, effects: &mut Vec<Effect>) {
        self.agent.menu = None;
        let Some(pane) = self.agent_pane(pane) else {
            return effects.push(Effect::Notice { text: agent_mode::FOCUS_APP.into() });
        };
        match self.agent.panes.get_mut(&pane) {
            Some(grant) if grant.task.is_some() => {
                grant.level = level;
                self.agent.chosen.insert(pane);
                self.agent.offers.remove(&pane);
                self.mark_level(pane);
            }
            Some(_) => {
                let Some(tab) = self.tab_of(pane) else { return };
                match level {
                    Level::Off => {
                        self.agent.ahead.remove(&pane);
                    }
                    level => {
                        self.agent.ahead.insert(pane, Ahead { level, tab });
                        effects.push(Effect::Notice { text: agent_mode::AHEAD.into() });
                    }
                }
            }
            None => {}
        }
    }

    /// ottospace served (or refused) a task's read of an app pane: its record. A row in the
    /// task's conversation and a use of the dial's Apps station; served, the time, for the limit.
    fn app_used(&mut self, task: TaskId, pane: PaneId, what: AppUse, refused: Option<String>, at: u64) {
        // The shell's own news, never the user's hand.
        if self.by_user {
            return;
        }
        if refused.is_none() && self.agent.panes.contains_key(&pane) {
            self.agent.last.entry(pane).or_default().insert(what, at);
        }
        let app = match self.pane(pane).map(|pane| &pane.kind) {
            Some(Kind::App { name, .. }) if !name.is_empty() => crate::capture::plain(name, 60),
            _ => "an app".to_string(),
        };
        let failed = refused.is_some();
        let label = format!("{} {app}", what.verb());
        if let Some(chat) = self.chat_mut(&crate::talk::Subject::Task(task)) {
            chat.transcript.app_used(Notice::AppUsed { what, app, pane, refused }, &label, failed, at);
        }
    }
}
