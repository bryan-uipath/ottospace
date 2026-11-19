//! The bug report's rules in the model: who starts one, what it covers, and what Send files.
//! The marks and the brief are `crate::bug`'s. See `docs/ai/bug-reporter.md`.

use super::worktrees::Making;
use super::{Effect, Model, PaneId, Subject};
use crate::bug::{self, Report};
use crate::model::Kind;
use crate::tasks::State;

impl Model {
    /// The bug button shows: the setting names a git checkout that is there.
    pub fn bug_icon(&self) -> bool {
        self.bug_repo.is_some()
    }

    /// The user's bug button, key or menu item, its picture taken: the report is drawn over the
    /// window. One at a time; none without the setting, or with a word that cannot name a folder.
    pub(crate) fn bug_start(
        &mut self,
        word: String,
        size: [f64; 2],
        (controls, panes): (Vec<(String, [f64; 4])>, Vec<(PaneId, [f64; 4])>),
        picture: bool,
        effects: &mut Vec<Effect>,
    ) {
        if self.bug.is_some() || self.bug_repo.is_none() || crate::tasks::materials_dir(0, &word).is_none() {
            return;
        }
        // What lies over the workspace gives way; a live app is parked, under the report.
        (self.keys, self.picker, self.palette) = (false, None, None);
        self.leave(effects);
        let kind = |id: PaneId| {
            let word = match &self.pane(id)?.kind {
                Kind::Terminal => "terminal",
                Kind::App { .. } => "app",
                Kind::File { .. } => "file",
                Kind::Empty => "empty",
                Kind::Slot { .. } => "slot",
            };
            Some(word.to_string())
        };
        let panes = panes.into_iter().filter_map(|(id, rect)| Some((id, kind(id)?, rect))).collect();
        let mut report = Report::new(word, size, controls, panes);
        report.picture = picture;
        self.bug = Some(report);
    }

    /// Esc or Cancel, or the setting gone: the report goes, and its picture.
    pub(crate) fn bug_drop(&mut self, effects: &mut Vec<Effect>) {
        if let Some(report) = self.bug.take() {
            effects.push(Effect::BugDropped { word: report.word });
        }
    }

    /// Send: the report becomes a task of the repository, with its brief, and its agent starts
    /// in a new worktree once git makes it, as the new-task box's "new worktree" does. Nothing
    /// is filed without a mark, or without a folder for worktrees (that is said).
    pub(crate) fn bug_send(&mut self, build: &str, os: &str, log: &[String], effects: &mut Vec<Effect>) {
        let (Some(report), Some(repo)) = (self.bug.as_ref(), self.bug_repo.clone()) else { return };
        if report.marks.is_empty() {
            return;
        }
        let Some(root) = self.repo_dirs.worktrees.clone() else {
            return effects.push(Effect::Notice { text: bug::NO_WORKTREES.into() });
        };
        let Some(report) = self.bug.take() else { return };
        // The report's own word after its branch: a repository not in Repos lists no branch, so
        // "Bug report" twice would ask git for one it has (`bug-report-k3f9`).
        let tag = report.word.chars().take(4).collect::<String>().to_ascii_lowercase();
        let (title, generated) = report.title();
        let id = self.file(title.clone(), 2, State::Captured);
        if generated {
            self.tasks.untitled(id);
        }
        self.tasks.tag(id, None, Some(repo.clone()));
        let Some(dir) = crate::tasks::materials_dir(id, &report.word) else {
            return effects.push(Effect::BugDropped { word: report.word });
        };
        self.tasks.attach(id, vec![format!("{dir}/1.json")]);
        let brief = bug::brief(id, &report, build, os, log);
        let blank = report.blanked();
        effects.push(Effect::BugFiled { word: report.word, dir, blank, brief: Box::new(brief) });
        let subject = Subject::Task(id);
        // No agent to start (no runner): the task is filed, its conversation shown.
        if self.agent_start(&subject).is_none() {
            return self.start(id, true, effects);
        }
        let listed = self.repos.iter().find(|r| r.path == repo);
        let taken = |branch: &str| {
            let dir = crate::repos::worktree_dir(&root, &repo, &self.repos, branch);
            listed.is_some_and(|r| r.checkouts().into_iter().any(|checkout| checkout.branch == branch))
                || self.repo_listed(&dir)
                || self.making.iter().any(|making| making.asked == dir)
        };
        let base = crate::repos::branch_for(&title, |base| taken(&format!("{base}-{tag}")));
        let branch = format!("{base}-{tag}");
        let path = crate::repos::worktree_dir(&root, &repo, &self.repos, &branch);
        self.making.push(Making { task: id, asked: path.clone(), branch: branch.clone(), bug: true });
        effects.push(Effect::AddWorktree { repo, branch, path, root, fresh: true });
        // Its own workspace, its conversation open: it says it waits for the worktree.
        self.talk_to(subject, true, effects);
    }

    /// A worktree asked for at `asked` is a bug report's: its repository need not be listed.
    pub fn bug_worktree(&self, asked: &str) -> bool {
        self.making.iter().any(|making| making.bug && making.asked == asked)
    }

    /// What must hold after every action: a report only while there is a repository to file it in.
    pub(crate) fn settle_bug(&mut self, effects: &mut Vec<Effect>) {
        if self.bug_repo.is_none() {
            self.bug_drop(effects);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bug::RepoFacts;
    use crate::edit::Edit;
    use crate::layout::Axis::Right;
    use crate::model::{Action, Mode, Origin};

    const WORD: &str = "k3f9a2c41b7d";
    const REPO: &str = "/code/ottospace";

    fn facts(exists: bool, git: bool) -> Action {
        Action::BugRepo { facts: Some(RepoFacts { path: REPO.into(), exists, git }) }
    }

    fn start() -> Action {
        let controls =
            vec![("top:fill".into(), [900.0, 0.0, 28.0, 22.0]), ("repo:/secret".into(), [0.0, 100.0, 100.0, 20.0])];
        Action::BugStart {
            word: WORD.into(),
            size: [1000.0, 500.0],
            controls,
            panes: vec![(1, [300.0, 40.0, 350.0, 400.0])],
            picture: true,
        }
    }

    fn send() -> Action {
        Action::BugSend { build: "ottospace 0.1.0 (local build)".into(), os: "macOS 26.1".into(), log: vec![] }
    }

    /// A terminal, the repository set and a folder for worktrees.
    fn model() -> Model {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        m.update(facts(true, true).sent_by(Origin::System));
        let dirs =
            crate::repos::RepoDirs { worktrees: Some("/home/.ottospace/worktrees".into()), ..Default::default() };
        m.update(Action::SetRepoDirs { dirs });
        m
    }

    #[test]
    fn the_button_shows_only_for_a_git_checkout_that_is_there() {
        let mut m = Model::default();
        assert!(!m.bug_icon());
        for (exists, git, shows) in
            [(false, false, false), (true, false, false), (false, true, false), (true, true, true)]
        {
            m.update(facts(exists, git).sent_by(Origin::System));
            assert_eq!(m.bug_icon(), shows, "exists {exists}, git {git}");
        }
        let relative = RepoFacts { path: "code/ottospace".into(), exists: true, git: true };
        m.update(Action::BugRepo { facts: Some(relative) }.sent_by(Origin::System));
        assert!(!m.bug_icon(), "a relative path is no repository");
        // Without the setting, the user's button does nothing.
        m.update(Action::BugRepo { facts: None }.sent_by(Origin::System));
        assert_eq!((m.update(start()), m.bug.is_some()), (vec![], false));
        // A client says nothing about the setting.
        let told = facts(true, true);
        assert!(m.refusal(&told, Origin::Socket).is_some());
    }

    #[test]
    fn only_the_user_starts_marks_and_sends_a_report() {
        let mut m = model();
        for origin in [Origin::Socket, Origin::System] {
            for action in [start(), Action::BugWhole, Action::BugCancel, send(), Action::BugApps { on: Some(true) }] {
                assert_eq!(m.refusal(&action, origin), Some(bug::USERS_ALONE), "{origin:?} {action:?}");
            }
        }
        assert_eq!(m.update(start().sent_by(Origin::Socket)), []);
        assert!(m.bug.is_none());
        assert_eq!((m.update(start()), m.bug.as_ref().map(|report| report.picture)), (vec![], Some(true)));
        assert_eq!(m.mode(), Mode::Annotate);
        // A client's keys do not type in a comment, nor take the keyboard from it.
        m.update(Action::BugWhole);
        m.update(Action::FieldEdit { edit: Edit::Insert("theirs".into()) }.sent_by(Origin::Socket));
        let open = Action::OpenCompose { text: "x".into() };
        assert_eq!(m.refusal(&open, Origin::Socket), Some(bug::OPEN));
        m.update(Action::FieldEdit { edit: Edit::Insert("the tabs flicker".into()) });
        assert_eq!(m.field().map(|(of, f)| (of, f.text())), Some(("bug", "the tabs flicker")));
        // The user's own pane keys wait behind it: the picture is of the window as it was.
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        assert_eq!((m.panes.len(), m.mode()), (1, Mode::Annotate));
        // A second start is no second report.
        assert_eq!(m.update(start()), []);
        // Esc: it goes.
        assert_eq!(m.update(Action::BugCancel), [Effect::BugDropped { word: WORD.into() }]);
        assert_eq!((m.bug.is_none(), m.mode()), (true, Mode::Normal));
    }

    #[test]
    fn a_live_app_is_parked_under_the_report() {
        let mut m = model();
        let app = Kind::App { window: 7, title: "#incidents".into(), bundle: None, name: "Chat".into() };
        m.update(Action::Open { kind: app, axis: Right, beside: None });
        m.update(Action::Enter { pane: 2 });
        assert_eq!(m.mode(), Mode::App);
        let effects = m.update(start());
        assert_eq!((effects, m.mode()), (vec![Effect::Parked(2)], Mode::Annotate));
    }

    #[test]
    fn send_files_a_task_in_a_new_worktree_of_the_repository_with_its_brief() {
        let mut m = model();
        m.update(start());
        // Nothing marked: nothing to send.
        assert_eq!(m.update(send()), []);
        m.update(Action::BugPress { at: [0.3, 0.1] });
        m.update(Action::BugDrag { to: [0.6, 0.5] });
        m.update(Action::BugRelease { at: [0.6, 0.5] });
        m.update(Action::FieldEdit { edit: Edit::Insert("The tab row flickers".into()) });
        let effects = m.update(send());
        let task = m.tasks.get(1).expect("the task").clone();
        assert_eq!((task.title.as_str(), task.repo.as_deref()), ("Bug: The tab row flickers", Some(REPO)));
        assert_eq!(task.materials, [format!("1-{WORD}/1.json")]);
        let Some(Effect::BugFiled { dir, blank, brief, .. }) =
            effects.iter().find(|e| matches!(e, Effect::BugFiled { .. }))
        else {
            panic!("no brief: {effects:?}")
        };
        assert_eq!((dir.as_str(), blank.len(), brief.marks.len()), ("1-k3f9a2c41b7d", 0, 1));
        let path = "/home/.ottospace/worktrees/ottospace/bug-the-tab-row-k3f9";
        let asked = Effect::AddWorktree {
            repo: REPO.into(),
            branch: "bug-the-tab-row-k3f9".into(),
            path: path.into(),
            root: "/home/.ottospace/worktrees".into(),
            fresh: true,
        };
        assert!(effects.contains(&asked), "{effects:?}");
        assert!(m.bug_worktree(path) && !m.bug_worktree("/elsewhere"));
        assert!(!effects.iter().any(|e| matches!(e, Effect::StartAgent { .. })), "it starts once the worktree is made");
        assert!(m.bug.is_none());
        // Made: its agent starts there, with the brief.
        let made = Action::WorktreeAdded {
            repo: REPO.into(),
            path: path.into(),
            asked: path.into(),
            fresh: true,
            repos: vec![],
        };
        let effects = m.update(made.sent_by(Origin::System));
        let started = effects.iter().find_map(|e| match e {
            Effect::StartAgent { cwd, brief, .. } => Some((cwd.clone(), brief.clone())),
            _ => None,
        });
        assert_eq!(started, Some((Some(path.into()), Some(format!("1-{WORD}")))));
    }

    #[test]
    fn with_no_folder_for_worktrees_nothing_is_filed_and_it_is_said() {
        let mut m = model();
        m.update(Action::SetRepoDirs { dirs: Default::default() });
        m.update(start());
        m.update(Action::BugWhole);
        assert_eq!(m.update(send()), [Effect::Notice { text: bug::NO_WORKTREES.into() }]);
        assert!(m.bug.is_some() && m.tasks.list.is_empty());
        // The setting gone: the report goes.
        let effects = m.update(Action::BugRepo { facts: None }.sent_by(Origin::System));
        assert_eq!((effects, m.bug.is_none()), (vec![Effect::BugDropped { word: WORD.into() }], true));
    }

    #[test]
    fn app_panes_are_blanked_in_what_is_filed_unless_shown() {
        let mut m = model();
        let app = Kind::App { window: 7, title: "#incidents".into(), bundle: None, name: "Chat".into() };
        m.update(Action::Open { kind: app, axis: Right, beside: None });
        let panes = vec![(1, [0.0, 40.0, 500.0, 400.0]), (2, [500.0, 40.0, 500.0, 400.0])];
        m.update(Action::BugStart {
            word: WORD.into(),
            size: [1000.0, 500.0],
            controls: vec![],
            panes,
            picture: false,
        });
        m.update(Action::BugWhole);
        let blank = |effects: Vec<Effect>| {
            effects.into_iter().find_map(|e| match e {
                Effect::BugFiled { blank, .. } => Some(blank),
                _ => None,
            })
        };
        let mut shown = model();
        shown.bug = m.bug.clone();
        assert_eq!(blank(m.update(send())), Some(vec![[500.0, 40.0, 500.0, 400.0]]));
        shown.update(Action::BugApps { on: None });
        assert_eq!(blank(shown.update(send())), Some(vec![]));
    }
}
