//! Decisions filed from any source (`decision_sources`): what the model checks a filing against,
//! and when one expires. The record and its rules: `crate::filed`.

use super::decisions::Undo;
use super::{Action, Effect, Model};
use crate::filed::{Ctx, Filing, Role};
use crate::tasks::{State, TaskId};

impl Model {
    /// Tasks there are and that are not done: an agent's own question may name one, and one that
    /// is no longer here expires its question.
    fn filing_live(&self) -> Vec<TaskId> {
        self.tasks.list.iter().filter(|t| !t.dismissed && t.state != State::Done).map(|t| t.id).collect()
    }

    /// The listed repositories and their worktrees: a file given as evidence lies in one.
    fn checkouts(&self) -> Vec<String> {
        let trees = self.repos.iter().flat_map(|repo| repo.worktrees.iter().map(|tree| tree.path.clone()));
        self.repos.iter().map(|repo| repo.path.clone()).chain(trees).collect()
    }

    /// Why `filing` would be refused now, or `None`. The socket's reply says it.
    pub fn filing_refusal(&self, filing: &Filing, now: u64) -> Option<String> {
        let (live, checkouts) = (self.filing_live(), self.checkouts());
        let tasks: Vec<TaskId> = self.tasks.list.iter().map(|t| t.id).collect();
        self.filed.refusal(filing, &Ctx { now, live: &live, tasks: &tasks, checkouts: &checkouts })
    }

    pub(super) fn filed_act(&mut self, action: Action, effects: &mut Vec<Effect>) {
        let changed = match action {
            Action::DecisionFile { filing, now } => {
                let (live, checkouts) = (self.filing_live(), self.checkouts());
                let tasks: Vec<TaskId> = self.tasks.list.iter().map(|t| t.id).collect();
                self.filed.file(&filing, &Ctx { now, live: &live, tasks: &tasks, checkouts: &checkouts })
            }
            Action::DecisionWithdraw { source, key, now } => self.filed.withdraw(source, &key, now),
            Action::RestoreDecisions { kept, trusted } => {
                self.filed.restore(kept, trusted);
                None
            }
            _ => None,
        };
        self.save_filed(changed.into_iter().collect(), Vec::new(), effects);
    }

    /// Triage's keys, the user's alone, on the item on show (`decisions::sort_key`).
    pub(super) fn sort_act(&mut self, action: Action, effects: &mut Vec<Effect>) {
        let now = self.now;
        let Some(sort) = self.decisions.as_ref().and_then(|run| run.sort.clone()) else { return };
        let shown = |id: u64| sort.item == Some(id);
        let mut next = sort.clone();
        let mut changed = Vec::new();
        match action {
            Action::SortStart if sort.item.is_none() => {
                next = self.sorting_at(self.filed.next_to_sort(now), sort.done, sort.undo.clone());
            }
            Action::SortSkip if sort.item.is_none() => {
                if let Some(run) = self.decisions.as_mut() {
                    (run.sort, run.sorted) = (None, true);
                }
                return;
            }
            // A key given to another revision of it does nothing: the card shows the new one first.
            Action::SortItem { id, .. } if self.filed.get(id).is_some_and(|d| d.revision != sort.revision) => return,
            Action::SortItem { id, how } if shown(id) => {
                if let Some(before) = self.filed.sort(id, how, sort.claims, sort.role, now) {
                    next.undo.push(Undo::Sorted(Box::new(before)));
                    next.done += 1;
                    changed.push(id);
                }
            }
            Action::SortToggle { id, claims: true } if shown(id) => next.claims = !sort.claims,
            Action::SortToggle { id, claims: false } if shown(id) => {
                let core = sort.role == Some(Role::Core);
                next.role = Some(if core { Role::Ancillary } else { Role::Core });
            }
            // Its scope only (the channel, the project, the repository, the routine), as its card
            // names it; a key with none promotes nothing.
            Action::SortPromote { id } if shown(id) => {
                let Some((source, key)) = self.filed.get(id).map(|d| (d.source, d.key.clone())) else { return };
                let Some(scope) = self.filed.promote(source, &key) else { return };
                let ids = self.filed.in_scope(source, &scope);
                next.done += ids
                    .iter()
                    .filter(|id| self.filed.get(**id).is_some_and(|d| d.triage == crate::filed::Triage::Skipped))
                    .count();
                changed.extend(ids);
                next.undo.push(Undo::Promoted { source, scope });
                effects.push(Effect::SaveTrust { trusted: self.filed.trusted.clone() });
            }
            Action::SortUndo => match next.undo.pop() {
                Some(Undo::Sorted(before)) => {
                    let id = before.id;
                    if self.filed.put_back(*before) {
                        changed.push(id);
                        let undo = std::mem::take(&mut next.undo);
                        next = self.sorting_at(Some(id), next.done.saturating_sub(1), undo);
                    }
                }
                Some(Undo::Promoted { source, scope }) => {
                    self.filed.demote(source, &scope);
                    let ids = self.filed.in_scope(source, &scope);
                    let back = ids.iter().filter(|id| self.filed.get(**id).is_some_and(|d| d.to_sort(now))).count();
                    next.done = next.done.saturating_sub(back);
                    let first = ids.iter().copied().find(|id| self.filed.get(*id).is_some_and(|d| d.to_sort(now)));
                    // Shown as any next item is: nothing left on it from the one before.
                    let undo = std::mem::take(&mut next.undo);
                    next = self.sorting_at(first.or(next.item), next.done, undo);
                    changed.extend(ids);
                    effects.push(Effect::SaveTrust { trusted: self.filed.trusted.clone() });
                }
                None => return,
            },
            _ => return,
        }
        next.left = self.filed.to_sort(now);
        if let Some(run) = self.decisions.as_mut() {
            run.sort = Some(next);
        }
        self.save_filed(changed, Vec::new(), effects);
    }

    /// After every step: an open decision past its time, or whose task is done or gone, expires;
    /// one closed two weeks ago goes. Whether the flag is on or not: kept ones still age.
    pub(super) fn settle_filed(&mut self, effects: &mut Vec<Effect>) {
        if self.filed.list.is_empty() {
            return;
        }
        let live = self.filing_live();
        let (changed, removed) = self.filed.settle(self.now, &live);
        self.save_filed(changed, removed, effects);
    }

    /// One write per decision a step: the last. A step that files and then expires one writes it once.
    fn save_filed(&self, changed: Vec<u64>, removed: Vec<u64>, effects: &mut Vec<Effect>) {
        let again = |e: &Effect| match e {
            Effect::SaveDecision { decision } => changed.contains(&decision.id) || removed.contains(&decision.id),
            _ => false,
        };
        effects.retain(|e| !again(e));
        for id in changed {
            if let Some(decision) = self.filed.get(id) {
                effects.push(Effect::SaveDecision { decision: Box::new(decision.clone()) });
            }
        }
        effects.extend(removed.into_iter().map(|id| Effect::RemoveDecision { id }));
    }
}

#[cfg(test)]
mod tests {
    use crate::filed::{Source, Status, Triage, tests::filing};
    use crate::model::{Action, Effect, Model, Origin};
    use crate::tasks::{Assign, State};

    #[test]
    fn a_client_files_updates_and_withdraws_and_each_change_is_saved() {
        let mut m = Model::default();
        let filing = Box::new(filing());
        let effects = m.update(Action::DecisionFile { filing: filing.clone(), now: 10 }.sent_by(Origin::Socket));
        let d = m.filed.list[0].clone();
        assert_eq!((d.source, d.triage, d.status), (Source::Slack, Triage::Pending, Status::Open));
        assert_eq!(effects, [Effect::SaveDecision { decision: Box::new(d.clone()) }]);
        // The same again changes nothing and writes nothing.
        assert!(m.update(Action::DecisionFile { filing, now: 11 }.sent_by(Origin::Socket)).is_empty());
        let effects = m.update(Action::DecisionWithdraw { source: Source::Slack, key: d.key.clone(), now: 12 });
        assert_eq!(m.filed.list[0].status, Status::Withdrawn);
        assert!(matches!(&effects[..], [Effect::SaveDecision { decision }] if decision.status == Status::Withdrawn));
    }

    #[test]
    fn the_flag_off_files_nothing_and_a_kept_one_still_expires() {
        let mut m = Model::default();
        m.update(Action::SetFeatures {
            features: crate::features::Features::all().with(crate::features::Feature::DecisionSources, false),
        });
        let filed = Box::new(filing());
        assert!(m.update(Action::DecisionFile { filing: filed, now: 10 }.sent_by(Origin::Socket)).is_empty());
        assert!(m.filed.list.is_empty());
        let mut kept = crate::filed::Filed::default();
        let ctx = crate::filed::Ctx { now: 10, live: &[], tasks: &[], checkouts: &[] };
        kept.file(&filing(), &ctx);
        m.update(Action::RestoreDecisions { kept: kept.list, trusted: Vec::new() }.sent_by(Origin::System));
        assert_eq!(m.filed.list.len(), 1, "read at start whatever the flag");
        let effects = m.update(Action::Tick { now: 10 + crate::filed::DEFAULT_EXPIRY_MS }.sent_by(Origin::System));
        assert_eq!(m.filed.list[0].status, Status::Expired);
        assert!(effects.iter().any(|e| matches!(e, Effect::SaveDecision { .. })));
    }

    #[test]
    fn an_agents_question_needs_its_live_task_and_expires_when_it_is_done() {
        let mut m = Model::default();
        m.update(Action::TaskAdd {
            title: "login".into(),
            priority: 3,
            state: State::Running,
            tab: None,
            assign: Assign::default(),
        });
        let task = m.tasks.list[0].id;
        let mut asked = filing();
        (asked.source, asked.key) = (Source::AgentSession, "agent_session:q1".into());
        assert!(m.filing_refusal(&asked, 10).is_some(), "names no task");
        asked.task = Some(task);
        assert_eq!(m.filing_refusal(&asked, 10), None);
        m.update(Action::DecisionFile { filing: Box::new(asked), now: 10 }.sent_by(Origin::Socket));
        assert_eq!(m.filed.list[0].triage, Triage::Skipped);
        m.update(Action::TaskSetState { id: task, state: State::Done }.sent_by(Origin::Socket));
        assert_eq!(m.filed.list[0].status, Status::Expired, "no one to answer");
    }

    fn file(m: &mut Model, source: Source, key: &str, now: u64) -> u64 {
        let mut f = filing();
        (f.source, f.key) = (source, key.into());
        m.update(Action::DecisionFile { filing: Box::new(f), now }.sent_by(Origin::Socket));
        m.filed.find(source, key).map(|d| d.id).unwrap_or_default()
    }

    fn key(m: &mut Model, key: &str) -> Vec<Effect> {
        let crate::model::chat::Key::Act(action) = m.decision_key(key, crate::keys::Mods::default(), 1_000) else {
            panic!("{key} does nothing");
        };
        m.update(action)
    }

    fn item(m: &Model) -> Option<Option<u64>> {
        m.decisions.as_ref().and_then(|run| run.sort.as_ref()).map(|sort| sort.item)
    }

    #[test]
    fn triage_is_one_key_an_item_and_z_takes_the_last_back() {
        let mut m = Model::in_workspace();
        let a = file(&mut m, Source::Slack, "slack:C1:1.0", 10);
        let b = file(&mut m, Source::Slack, "slack:C1:2.0", 20);
        let c = file(&mut m, Source::Jira, "jira:ABC-1", 30);
        m.update(Action::Tick { now: 1_000 }.sent_by(Origin::System));
        assert_eq!(m.filed.to_sort(1_000), 3);
        // No agent waits: ⌘J opens Decisions on triage's card.
        m.update(Action::DecisionsOpen);
        assert_eq!(item(&m), Some(None));
        assert_eq!(m.decisions.as_ref().map(|run| (run.task, run.at, run.of)), Some((None, 1, 1)));
        // A client's keys are refused.
        for action in [Action::SortStart, Action::SortSkip, Action::SortUndo] {
            m.update(action.sent_by(Origin::Socket));
            assert_eq!(item(&m), Some(None));
        }
        key(&mut m, "enter");
        assert_eq!(item(&m), Some(Some(a)), "the first filed first");
        // T: taken now, its claims counted with U; the next shows.
        key(&mut m, "u");
        let effects = key(&mut m, "t");
        let d = m.filed.get(a).unwrap();
        assert_eq!((d.triage, d.accepted.claims, d.accepted.now_until.is_some()), (Triage::Sorted, true, true));
        assert!(effects.iter().any(|e| matches!(e, Effect::SaveDecision { decision } if decision.id == a)));
        assert_eq!(item(&m), Some(Some(b)));
        // Z: back as it was, on show again.
        key(&mut m, "z");
        assert_eq!((m.filed.get(a).unwrap().triage, item(&m)), (Triage::Pending, Some(Some(a))));
        key(&mut m, "n");
        assert_eq!(m.filed.get(a).unwrap().status, Status::NotMine);
        // P: its channel skips triage from now, the rest of it with it; Z takes that back too.
        let effects = key(&mut m, "p");
        assert!(effects.iter().any(|e| matches!(e, Effect::SaveTrust { trusted } if trusted.len() == 1)));
        assert_eq!((m.filed.get(b).unwrap().triage, item(&m)), (Triage::Skipped, Some(Some(c))));
        assert_eq!(file(&mut m, Source::Slack, "slack:C1:3.0", 1_000), m.filed.list.last().unwrap().id);
        assert_eq!(m.filed.list.last().unwrap().triage, Triage::Skipped, "a promoted channel's next skips triage");
        key(&mut m, "z");
        assert_eq!((m.filed.get(b).unwrap().triage, m.filed.trusted.len()), (Triage::Pending, 0));
        // L puts one off a day; D dismisses: nothing left, triage ends, and Decisions with it.
        key(&mut m, "l");
        assert!(m.filed.get(b).unwrap().later.is_some());
        key(&mut m, "d");
        assert_eq!(m.filed.get(c).unwrap().status, Status::Dismissed);
        // The one filed under the channel while it was trusted waits again, the promotion taken back.
        key(&mut m, "n");
        // All sorted: Z may still take the last back; Esc closes.
        assert_eq!(item(&m), Some(None));
        key(&mut m, "escape");
        assert!(m.decisions.is_none(), "nothing left to sort");
        // S on the card: not in this run.
        file(&mut m, Source::Jira, "jira:ABC-2", 1_000);
        m.update(Action::DecisionsOpen);
        key(&mut m, "s");
        assert!(m.decisions.is_none());
    }

    #[test]
    fn evidence_lies_in_a_listed_checkout() {
        let mut m = Model::default();
        m.repos = vec![crate::repos::Repo {
            name: "app".into(),
            path: "/Users/me/code/app".into(),
            branch: None,
            worktrees: vec![crate::repos::Worktree {
                name: "w".into(),
                path: "/Users/me/code/app-w".into(),
                branch: None,
            }],
        }];
        let mut f = filing();
        let at = |path: &str| crate::filed::Evidence {
            kind: crate::filed::EvidenceKind::File,
            text: String::new(),
            at: Some(crate::filed::At::File { path: path.into(), lines: None }),
            provenance: None,
        };
        f.evidence = vec![at("/Users/me/code/app-w/src/lib.rs")];
        assert_eq!(m.filing_refusal(&f, 10), None);
        f.evidence = vec![at("/Users/me/.ssh/id_ed25519")];
        assert!(m.filing_refusal(&f, 10).is_some());
    }
}
