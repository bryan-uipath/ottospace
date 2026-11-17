//! The model's side of agent mode (`crate::agent_mode`): each app pane's grant, kept or reset as
//! panes, workspaces and tasks change; the user's level and menu; and who may read a pane. Who may
//! send each action is `Model::refusal`.

use super::{Action, Effect, Kind, Model, Origin, PaneId, STALE_PASS};
use crate::agent_mode::{self, AppUse, Grant, Level, Menu};
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
    /// level: a client that opens Mail in its task's workspace cannot read it. One whose task no
    /// longer works there (it moved to another workspace, the task ended, was dismissed or unbound)
    /// is Off, for the task now there: a grant is never carried to a task the user did not give it
    /// to. A closed pane's goes. Grants are not kept over a restart: a restored pane is Off. They
    /// are kept, and kept true, while the flag is off.
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
            self.agent.by_user.entry(*pane).or_insert(origin == Origin::User);
        }
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
                    Some(_) => Grant { level: Level::Off, task: front },
                    None => {
                        let users = self.agent.by_user.get(&id).copied().unwrap_or(false);
                        let level = if front.is_some() && users { self.agent.default } else { Level::Off };
                        Grant { level, task: front }
                    }
                };
                panes.insert(id, grant);
            }
        }
        // The menu is for the task it showed: its pane gone, or given to another task, it closes.
        let before = std::mem::replace(&mut self.agent.panes, panes);
        let panes = &self.agent.panes;
        self.agent.last.retain(|pane, _| panes.contains_key(pane));
        let moved = |pane: &PaneId| panes.get(pane).map(|grant| grant.task) != before.get(pane).map(|grant| grant.task);
        if self.agent.menu.is_some_and(|menu| moved(&menu.pane) || !panes.contains_key(&menu.pane)) {
            self.agent.menu = None;
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
                "agent mode is off on pane {pane}: only the user turns it on, in the pane's header (ask them to)"
            ));
        }
        let last = self.agent.last.get(&pane).and_then(|uses| uses.get(&what));
        if last.is_some_and(|at| now.saturating_sub(*at) < agent_mode::EVERY_MS) {
            return Err(format!("pane {pane} was {} less than a second ago: try again in a moment", what.verb()));
        }
        Ok(task)
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
                let tasked = self.agent.panes.get(&pane).is_some_and(|grant| grant.task.is_some());
                if !open {
                    self.agent.menu = Some(Menu { pane, selected: (keys && tasked).then_some(0) });
                }
            }
            Action::AgentMenuMove { delta } => {
                let tasked = |menu: &Menu| self.agent.panes.get(&menu.pane).is_some_and(|grant| grant.task.is_some());
                if let Some(menu) = self.agent.menu.as_mut().filter(|menu| tasked(menu)) {
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
            Action::AppUsed { task, pane, what, refused, at } => self.app_used(task, pane, what, refused, at),
            _ => {}
        }
    }

    /// The user's level for a pane. Where no task works it grants nothing: the status bar says so.
    fn agent_level(&mut self, pane: Option<PaneId>, level: Level, effects: &mut Vec<Effect>) {
        self.agent.menu = None;
        let Some(pane) = self.agent_pane(pane) else {
            return effects.push(Effect::Notice { text: agent_mode::FOCUS_APP.into() });
        };
        match self.agent.panes.get_mut(&pane) {
            Some(grant) if grant.task.is_some() => grant.level = level,
            _ => effects.push(Effect::Notice { text: agent_mode::NO_TASK.into() }),
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
