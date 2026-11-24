//! Decisions filed from any source (`decision_sources`): what the model checks a filing against,
//! and when one expires. The record and its rules: `crate::filed`.

use super::{Action, Effect, Model};
use crate::filed::{Ctx, Filing};
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
            Action::RestoreDecisions { kept } => {
                self.filed.restore(kept);
                None
            }
            _ => None,
        };
        self.save_filed(changed.into_iter().collect(), Vec::new(), effects);
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
        m.update(Action::RestoreDecisions { kept: kept.list }.sent_by(Origin::System));
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
        asked.source = Source::AgentSession;
        assert!(m.filing_refusal(&asked, 10).is_some(), "names no task");
        asked.task = Some(task);
        assert_eq!(m.filing_refusal(&asked, 10), None);
        m.update(Action::DecisionFile { filing: Box::new(asked), now: 10 }.sent_by(Origin::Socket));
        assert_eq!(m.filed.list[0].triage, Triage::Skipped);
        m.update(Action::TaskSetState { id: task, state: State::Done }.sent_by(Origin::Socket));
        assert_eq!(m.filed.list[0].status, Status::Expired, "no one to answer");
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
