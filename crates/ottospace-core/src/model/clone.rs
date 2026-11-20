//! The clone dialog: From GitHub… of the Repos section's +. A repository the user names is cloned
//! into `<clone_dir>/<name>` by the shell, and listed as the user's own add (`repo_add`).

use super::{Effect, Model, Repo};
use crate::clone::{self, Dialog, Failure, Job, Listed, Load, Provider};

impl Model {
    /// From GitHub…: the dialog, empty, or showing the clone on its way; the shell lists the
    /// user's repositories for it.
    pub(crate) fn clone_open(&mut self, effects: &mut Vec<Effect>) {
        effects.push(Effect::CloneList);
        self.leave(effects);
        self.list = None;
        let mut dialog = Dialog::new(Provider::GitHub);
        // Why the last clone failed while hidden: its toast named nothing.
        dialog.said = self.clone_last.take();
        self.clone = Some(dialog);
    }

    /// ↑ or ↓: the row the keys are on. It wraps.
    pub(super) fn clone_move(&mut self, delta: i32) {
        let Some(dialog) = &mut self.clone else { return };
        let count = dialog.rows().len() as i64;
        if count > 0 {
            dialog.selected = (dialog.selected as i64 + i64::from(delta)).rem_euclid(count) as usize;
        }
    }

    /// Where the row the keys are on would be cloned, in full: shown before ↵.
    pub fn clone_destination(&self) -> Option<String> {
        let dialog = self.clone.as_ref()?;
        let row = dialog.rows().into_iter().nth(dialog.selected)?;
        clone::destination(self.repo_dirs.clone.as_deref()?, &row.listed.remote.name)
    }

    /// ↵: the row cloned into `<clone_dir>/<name>`. One clone at a time; none with no folder
    /// to clone into; a line that names no repository stays to be fixed.
    pub(super) fn clone_choose(&mut self, index: Option<usize>, effects: &mut Vec<Effect>) {
        let Some(dialog) = &mut self.clone else { return };
        let rows = dialog.rows();
        let Some(row) = rows.get(index.unwrap_or(dialog.selected)).cloned() else {
            dialog.said = Some(clone::NOT_A_REPO.into());
            return;
        };
        // Already here: ↵ goes to it.
        if let Some(path) = row.here {
            self.clone = None;
            return self.repo_select(path, effects);
        }
        let remote = row.listed.remote;
        if self.cloning.is_some() {
            dialog.said = Some(clone::ONE_AT_A_TIME.into());
            return;
        }
        let dir = self.repo_dirs.clone.clone();
        let Some((dir, dest)) = dir.and_then(|dir| clone::destination(&dir, &remote.name).map(|dest| (dir, dest)))
        else {
            dialog.said = Some(clone::NO_DIR.into());
            return;
        };
        // Made only once the user's ↵ follows the line that said it would be.
        let make_dir = dialog.make_dir.as_deref() == Some(dir.as_str());
        dialog.said = None;
        self.clone_asks += 1;
        let id = self.clone_asks;
        let provider = dialog.provider;
        self.cloning =
            Some(Job { id, remote: remote.clone(), dest: dest.clone(), line: String::new(), cancelled: false });
        effects.push(Effect::Clone { id, provider, remote, dest, make_dir });
    }

    /// Esc stops a running clone (its folder goes) and closes; a click outside closes, and the
    /// clone goes on, its end said on a toast.
    pub(super) fn clone_close(&mut self, cancel: bool, effects: &mut Vec<Effect>) {
        if self.clone.take().is_none() {
            return;
        }
        if let Some(job) = self.cloning.as_mut().filter(|job| cancel && !job.cancelled) {
            job.cancelled = true;
            effects.push(Effect::CloneCancel { id: job.id });
        }
    }

    /// What gh listed, or why not, and each listed checkout's GitHub remote. Kept only while
    /// the dialog is open.
    pub(super) fn clone_listed(
        &mut self,
        result: Result<(Vec<Listed>, Vec<String>), String>,
        here: Vec<(String, String)>,
    ) {
        let Some(dialog) = &mut self.clone else { return };
        match result {
            Ok((listed, owners)) => (dialog.load, dialog.listed, dialog.owners) = (Load::Read, listed, owners),
            Err(why) => dialog.load = Load::Failed(clone::one_line(&why)),
        }
        dialog.here = here;
        dialog.selected = 0;
    }

    /// What the search `q` found: kept while it is still the one for what is typed.
    pub(super) fn clone_found(&mut self, q: String, result: Result<Vec<Listed>, String>) {
        let Some(dialog) = self.clone.as_mut().filter(|dialog| dialog.search().as_ref() == Some(&q)) else { return };
        let before = dialog.rows().get(dialog.selected).map(|row| row.listed.remote.clone());
        dialog.found = Some((q, result.unwrap_or_default()));
        // The keys stay on the row they were on.
        let rows = dialog.rows();
        dialog.selected = before.and_then(|r| rows.iter().position(|row| row.listed.remote == r)).unwrap_or(0);
    }

    /// gh's last line, for the dialog: cleaned, one line.
    pub(super) fn clone_progress(&mut self, id: u64, line: &str) {
        if let Some(job) = self.cloning.as_mut().filter(|job| job.id == id) {
            job.line = clone::one_line(line);
        }
    }

    /// The clone's end. Made: listed as the user's own add would list it, and said; its row is
    /// selected only if the dialog is still open (a hidden clone takes no keyboard when it ends).
    /// Not: said in the dialog if it is open, else on a toast. A cancel the runner was too late
    /// for ends made: it is there, so it is listed.
    pub(super) fn clone_done(&mut self, id: u64, result: Result<Vec<Repo>, Failure>, effects: &mut Vec<Effect>) {
        let Some(job) = self.cloning.take_if(|job| job.id == id) else { return };
        let result = match result {
            // Nothing read there: said without the path (`repo_add` would name it).
            Ok(found) if !found.iter().any(|repo| repo.path == job.dest) => {
                Err(Failure::Said("gh ended with no repository there".into()))
            }
            result => result,
        };
        match result {
            Ok(found) => {
                effects.push(Effect::Told { text: clone::CLONED.into() });
                match self.clone.take() {
                    Some(_) => self.repo_add(job.dest, found, effects),
                    None => drop(self.repo_list(job.dest, found, effects)),
                }
            }
            Err(failure) => {
                let text = failure.explain(&job.remote, &job.dest);
                match self.clone.as_mut() {
                    Some(dialog) => {
                        let dir = job.dest.rsplit_once('/').map(|(dir, _)| dir.to_string());
                        dialog.make_dir = dir.filter(|_| failure == Failure::DirMissing);
                        dialog.said = Some(text);
                    }
                    None if failure == Failure::Cancelled => effects.push(Effect::Told { text: failure.public() }),
                    None => {
                        self.clone_last = Some(text);
                        effects.push(Effect::Failed { text: failure.public() })
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Action, Effect, Model, Origin, Repo, Section};
    use crate::clone::{self, Failure, Provider, Remote};
    use crate::edit::Edit;
    use crate::list::ListRow;
    use crate::repos::RepoDirs;
    use crate::row_menu::{Item, MenuOf};

    const DIR: &str = "/Users/me/code";

    /// The Repos section open, `clone_dir` set.
    fn repos() -> Model {
        let mut m = Model::default();
        let dirs = RepoDirs { clone: Some(DIR.into()), ..RepoDirs::default() };
        m.update(Action::SetRepoDirs { dirs }.sent_by(Origin::System));
        m.update(Action::ToggleSection { section: Section::Repos });
        m
    }

    fn typed(m: &mut Model, text: &str) {
        m.update(Action::FieldEdit { edit: Edit::Insert(text.into()) });
    }

    /// The dialog open, `text` typed.
    fn dialog(text: &str) -> Model {
        let mut m = repos();
        m.update(Action::CloneOpen);
        typed(&mut m, text);
        m
    }

    fn said(m: &Model) -> Option<&str> {
        m.clone.as_ref().and_then(|dialog| dialog.said.as_deref())
    }

    fn hello() -> Remote {
        Remote::new("octo", "hello").unwrap()
    }

    fn made() -> Vec<Repo> {
        vec![Repo { path: format!("{DIR}/hello"), name: "hello".into(), branch: None, worktrees: Vec::new() }]
    }

    #[test]
    fn the_plus_offers_a_local_folder_first_and_from_github() {
        let mut m = repos();
        m.update(Action::RowMenu { row: MenuOf::ReposAdd, dots: None, keys: false });
        let (menu, items) = m.row_menu().unwrap();
        assert_eq!((items, menu.selected), (&[Item::LocalFolder, Item::FromGitHub][..], Some(0)));
        // ↵ at once: the folder picker, as the + was.
        assert_eq!(m.update(Action::RowMenuChoose { index: None }), [Effect::PickRepoFolder]);
        m.update(Action::RowMenu { row: MenuOf::ReposAdd, dots: None, keys: false });
        // What the item stands for waits for nothing: the menu closes as the dialog opens.
        assert_eq!(m.row_menu_action(Some(1)).and_then(|stood| m.waits(&stood)), None);
        m.update(Action::RowMenuChoose { index: Some(1) });
        assert_eq!((m.clone.is_some(), m.field().map(|(of, _)| of)), (true, Some("clone")));
        // Not while `OTTOSPACE_REPOS` names the list, nor with the section closed.
        let mut m = repos();
        let fixed = RepoDirs { fixed: true, clone: Some(DIR.into()), ..RepoDirs::default() };
        m.update(Action::SetRepoDirs { dirs: fixed }.sent_by(Origin::System));
        m.update(Action::RowMenu { row: MenuOf::ReposAdd, dots: None, keys: false });
        assert!(m.row_menu().is_none());
    }

    #[test]
    fn a_pasted_repository_is_one_row_and_its_destination_shows_before_enter() {
        let mut m = dialog("https://github.com/octo/hello.git");
        let rows: Vec<Remote> = m.clone.as_ref().unwrap().rows().into_iter().map(|row| row.listed.remote).collect();
        assert_eq!(rows, [hello()]);
        assert_eq!(m.clone_destination().as_deref(), Some("/Users/me/code/hello"));
        let effects = m.update(Action::CloneChoose { index: None });
        let clone = Effect::Clone {
            id: 1,
            provider: Provider::GitHub,
            remote: hello(),
            dest: "/Users/me/code/hello".into(),
            make_dir: false,
        };
        assert_eq!(effects, [clone]);
        assert_eq!(m.cloning.as_ref().map(|job| job.dest.as_str()), Some("/Users/me/code/hello"));
        // A second ↵ starts none.
        assert!(m.update(Action::CloneChoose { index: None }).is_empty());
        assert_eq!(said(&m), Some(clone::ONE_AT_A_TIME));
        // gh's line, cleaned; then the end: listed, its row selected, the dialog closed, said.
        m.update(
            Action::CloneProgress { id: 1, line: "Cloning\x1b[1m into 'hello'...\r".into() }.sent_by(Origin::System),
        );
        assert_eq!(m.cloning.as_ref().unwrap().line, "Cloning into 'hello'...");
        let effects = m.update(Action::CloneDone { id: 1, result: Ok(made()) }.sent_by(Origin::System));
        assert!(effects.contains(&Effect::Told { text: clone::CLONED.into() }));
        assert!(
            effects.iter().any(|e| matches!(e, Effect::SaveRepos { dirs } if dirs[0].path == "/Users/me/code/hello"))
        );
        assert_eq!((m.clone.is_none(), m.cloning.is_none()), (true, true));
        assert_eq!(m.list_rows().get(m.list.unwrap()), Some(&ListRow::Repo("/Users/me/code/hello".into())));
    }

    /// A line that is no repository, or no folder to clone into: nothing runs, the dialog says why.
    #[test]
    fn a_hostile_or_unclonable_line_runs_nothing() {
        for text in ["../x", "a/b/c", "o/-rf", "o/x y", "hello", "o/..", "https://evil.com/o/x"] {
            let mut m = dialog(text);
            assert!(m.update(Action::CloneChoose { index: None }).is_empty(), "{text:?}");
            assert_eq!((said(&m), m.cloning.is_none(), m.clone_destination()), (Some(clone::NOT_A_REPO), true, None));
        }
        let mut m = dialog("octo/hello");
        m.update(Action::SetRepoDirs { dirs: RepoDirs::default() }.sent_by(Origin::System));
        assert!(m.update(Action::CloneChoose { index: None }).is_empty());
        assert_eq!(said(&m), Some(clone::NO_DIR));
    }

    #[test]
    fn a_failure_is_said_in_the_dialog_and_a_missing_folder_is_made_only_after_the_users_next_enter() {
        let mut m = dialog("octo/hello");
        m.update(Action::CloneChoose { index: None });
        let done = |id, failure| Action::CloneDone { id, result: Err(failure) }.sent_by(Origin::System);
        m.update(done(1, Failure::DirMissing));
        assert_eq!(said(&m), Some("/Users/me/code does not exist: ↵ makes it and clones"));
        let effects = m.update(Action::CloneChoose { index: None });
        assert!(matches!(&effects[..], [Effect::Clone { id: 2, make_dir: true, .. }]), "{effects:?}");
        m.update(done(2, Failure::Exists));
        assert_eq!(said(&m), Some("Already there: /Users/me/code/hello. Nothing was cloned"));
        // Another ↵ after another failure does not make the folder any more.
        let effects = m.update(Action::CloneChoose { index: None });
        assert!(matches!(&effects[..], [Effect::Clone { make_dir: false, .. }]), "{effects:?}");
        // An end of an old clone, or of none, does nothing.
        m.update(done(1, Failure::NoGh));
        assert!(m.cloning.is_some());
    }

    #[test]
    fn esc_while_cloning_stops_it_and_a_click_outside_leaves_it_running() {
        let mut m = dialog("octo/hello");
        m.update(Action::CloneChoose { index: None });
        assert_eq!(m.update(Action::CloneClose { cancel: true }), [Effect::CloneCancel { id: 1 }]);
        assert!(m.clone.is_none());
        // Stopped: said as information, nothing listed.
        let effects = m.update(Action::CloneDone { id: 1, result: Err(Failure::Cancelled) }.sent_by(Origin::System));
        let text = "Cancelled the clone: its folder was removed".to_string();
        assert_eq!(effects, [Effect::Told { text }]);
        assert!(m.repos.is_empty());
        // Too late to stop, or hidden: listed, said, and the keyboard stays where it is.
        for cancel in [true, false] {
            let mut m = dialog("octo/hello");
            m.update(Action::CloneChoose { index: None });
            m.update(Action::CloneClose { cancel });
            m.update(Action::ToggleSection { section: Section::Repos });
            let effects = m.update(Action::CloneDone { id: 1, result: Ok(made()) }.sent_by(Origin::System));
            assert!(effects.contains(&Effect::Told { text: clone::CLONED.into() }), "{effects:?}");
            assert_eq!((m.repos.len(), m.sidebar, m.list), (1, None, None), "{cancel}");
        }
        // Hidden: it goes on, and its end is a toast.
        let mut m = dialog("octo/hello");
        m.update(Action::CloneChoose { index: None });
        assert!(m.update(Action::CloneClose { cancel: false }).is_empty());
        let effects = m.update(
            Action::CloneDone { id: 1, result: Err(Failure::Said("x not found".into())) }.sent_by(Origin::System),
        );
        let text = "Not found on GitHub, or not yours to read".to_string();
        assert_eq!(effects, [Effect::Failed { text }]);
        // The dialog opened again says it in full.
        m.update(Action::CloneOpen);
        assert_eq!(said(&m), Some("Not found on GitHub, or not yours to read: octo/hello"));
    }

    #[test]
    fn the_dialog_is_the_users_alone_and_its_news_the_shells() {
        let m = dialog("octo/hello");
        let users = [
            Action::CloneOpen,
            Action::RepoPickFolder,
            Action::CloneMove { delta: 1 },
            Action::CloneChoose { index: None },
            Action::CloneClose { cancel: true },
            Action::FieldEdit { edit: Edit::Insert("x".into()) },
        ];
        for action in &users {
            for origin in [Origin::Socket, Origin::System] {
                assert!(m.refusal(action, origin).is_some(), "{action:?} {origin:?}");
            }
            assert_eq!(m.refusal(action, Origin::User), None, "{action:?}");
        }
        let news = [
            Action::CloneProgress { id: 1, line: String::new() },
            Action::CloneDone { id: 1, result: Err(Failure::NoGh) },
        ];
        for action in &news {
            assert_eq!(m.refusal(action, Origin::Socket), Some(clone::SHELLS_ALONE), "{action:?}");
        }
        // Nor opens the picker over it, whose keys would take its Esc.
        let picker = Action::OpenPicker { axis: crate::Axis::Right, new_tab: false, items: Vec::new() };
        assert_eq!(m.waits(&picker), Some(clone::OPEN));
        // A client's ↵ clones nothing, and its keys type nothing.
        let mut m = dialog("octo/hello");
        assert!(m.update(Action::CloneChoose { index: None }.sent_by(Origin::Socket)).is_empty());
        m.update(Action::FieldEdit { edit: Edit::Insert("x".into()) }.sent_by(Origin::Socket));
        assert_eq!((m.cloning.is_none(), m.clone.as_ref().unwrap().query.text()), (true, "octo/hello"));
    }

    /// A repository's name is the user's data (a private one's too): what is typed, the row, the
    /// destination and gh's line are drawn, never in the state every reply carries, nor in a toast.
    #[test]
    fn no_part_of_the_typed_name_reaches_the_state_or_a_toast() {
        let secret = "s3cr3t-app";
        let leaks = |m: &Model, effects: &[Effect]| {
            let state = serde_json::to_string(m).unwrap();
            let told = effects.iter().any(|e| matches!(e, Effect::Told { text } | Effect::Failed { text } | Effect::Notice { text } if text.contains(secret) || text.contains("octo")));
            state.contains(secret) || state.contains("octo/") || told
        };
        let mut m = dialog(&format!("https://github.com/octo/{secret}"));
        assert!(m.field_hidden() && !leaks(&m, &[]));
        let effects = m.update(Action::CloneChoose { index: None });
        assert!(!leaks(&m, &effects));
        let line = format!("Cloning into '{DIR}/{secret}'...");
        let effects = m.update(Action::CloneProgress { id: 1, line }.sent_by(Origin::System));
        assert!(!leaks(&m, &effects));
        // Hidden, then failed or cancelled: the toast names nothing.
        for (cancel, failure) in
            [(false, Failure::Exists), (false, Failure::Said(format!("x {secret} y"))), (true, Failure::Cancelled)]
        {
            let mut m = dialog(&format!("octo/{secret}"));
            m.update(Action::CloneChoose { index: None });
            let closed = m.update(Action::CloneClose { cancel });
            let effects = m.update(Action::CloneDone { id: 1, result: Err(failure.clone()) }.sent_by(Origin::System));
            assert!(!leaks(&m, &closed) && !leaks(&m, &effects), "{failure:?}");
        }
        // Made: the toast names nothing; the repository is then listed, as any the user adds. Made
        // with no repository read there: said without its path.
        let repo = Repo { path: format!("{DIR}/{secret}"), name: secret.into(), branch: None, worktrees: Vec::new() };
        let mut empty = dialog(&format!("octo/{secret}"));
        empty.update(Action::CloneChoose { index: None });
        let effects = empty.update(Action::CloneDone { id: 1, result: Ok(Vec::new()) }.sent_by(Origin::System));
        assert!(!leaks(&empty, &effects), "{effects:?}");
        let effects = m.update(Action::CloneDone { id: 1, result: Ok(vec![repo]) }.sent_by(Origin::System));
        let toasts = effects.iter().filter_map(|e| match e {
            Effect::Told { text } | Effect::Failed { text } | Effect::Notice { text } => Some(text),
            _ => None,
        });
        assert!(toasts.clone().all(|text| !text.contains(secret)), "{effects:?}");
    }

    #[test]
    fn the_dialog_lists_the_users_repositories_and_enter_on_one_already_here_goes_to_it() {
        let mut m = repos();
        let notes = "/code/notes".to_string();
        m.update(
            Action::SetRepos {
                repos: vec![Repo { path: notes.clone(), name: "notes".into(), branch: None, worktrees: Vec::new() }],
            }
            .sent_by(Origin::System),
        );
        assert_eq!(m.update(Action::CloneOpen), [Effect::CloneList]);
        let listed = |name: &str| clone::Listed {
            remote: Remote::new("me", name).unwrap(),
            description: "d".into(),
            private: true,
            fork: false,
            archived: false,
        };
        let result = Ok((vec![listed("app"), listed("notes")], vec!["me".into()]));
        m.update(
            Action::CloneListed { result, here: vec![("me/notes".into(), notes.clone())] }.sent_by(Origin::System),
        );
        assert_eq!(m.clone.as_ref().unwrap().load, clone::Load::Read);
        // A search for another line, or for none open, is not kept.
        typed(&mut m, "ap");
        m.update(
            Action::CloneFound { q: "zz in:name user:me".into(), result: Ok(vec![listed("zz")]) }
                .sent_by(Origin::System),
        );
        assert!(m.clone.as_ref().unwrap().found.is_none());
        m.update(
            Action::CloneFound { q: "ap in:name user:me".into(), result: Ok(vec![listed("apex")]) }
                .sent_by(Origin::System),
        );
        let rows: Vec<String> = m.clone.as_ref().unwrap().rows().iter().map(|r| r.listed.remote.slug()).collect();
        assert_eq!(rows, ["me/app", "me/apex"]);
        // ↵ on one already here: no clone, its row selected.
        m.update(Action::FieldEdit { edit: Edit::SelectAll });
        typed(&mut m, "notes");
        assert!(m.update(Action::CloneChoose { index: None }).iter().all(|e| !matches!(e, Effect::Clone { .. })));
        assert_eq!((m.clone.is_none(), m.list_rows().get(m.list.unwrap_or(99))), (true, Some(&ListRow::Repo(notes))));
    }
}
