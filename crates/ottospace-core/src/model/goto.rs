//! Go to (⌘P) in the model: the places listed, and what a choice, Tab, `/` and Back do. Every
//! choice goes as its section's own row does (`ListChoose`). Design: `crate::goto`.

use super::{Action, Effect, MAX_FILTER, Model, Owner, TaskPlace, short};
use crate::edit::Field;
use crate::goto::{Drill, Files, GoTo, Place, PlaceRow};
use crate::layout::Axis;
use crate::talk::Subject;
use crate::tasks::Heading;

/// Why a place's folder has no files listed: git runs only where the Repos section lists a checkout.
pub const NOT_LISTED: &str = "not a checkout listed in Repos";
/// Why a client's `GoToFiles` is refused: only the shell's own read fills the list.
pub const SHELLS_READ: &str = "only the shell lists a folder's files";
/// Why a listed checkout has no files listed: git failed or took too long.
pub const NOT_READ: &str = "git could not list its files";

impl Model {
    /// The places, most recently selected first (`last_spaces`), the one on show last. Then, of
    /// none selected yet, as their sections list them: the workspaces, each listed checkout, the
    /// tasks not done (needs-you first).
    pub(crate) fn goto_places(&self) -> GoTo {
        let home = self.home.as_deref();
        let current = self.tabs[self.main_at()].space.clone();
        let mut rows: Vec<(PlaceRow, Option<Owner>)> = Vec::new();
        for book in self.workbooks.listed() {
            let of = self.workbooks.space_of(&Owner::Workbook { id: book.id });
            let detail = book.worktree.as_deref().map(|folder| short(home, folder)).unwrap_or_default();
            let (name, folder) = (book.name.clone(), book.worktree.clone());
            rows.push((PlaceRow { place: Place::Workspace(book.id), name, detail, folder, current: false }, Some(of)));
        }
        for repo in &self.repos {
            for checkout in repo.checkouts() {
                let at = if checkout.default { short(home, &checkout.path) } else { repo.name.clone() };
                let detail = format!("{at} · {}", checkout.branch);
                let of = self.repo_space(&checkout.path, true).map(|(of, _)| of);
                let (place, folder) = (Place::Checkout(checkout.path.clone()), Some(checkout.path));
                rows.push((PlaceRow { place, name: checkout.label, detail, folder, current: false }, of));
            }
        }
        for TaskPlace { heading, task, .. } in self.task_places() {
            if matches!(heading, Heading::Done | Heading::DoneToday) || !task.undone() {
                continue;
            }
            let (detail, folder) = (heading.title().to_string(), self.task_worktree(task.id).map(String::from));
            let row =
                PlaceRow { place: Place::Task(task.id), name: task.title.clone(), detail, folder, current: false };
            rows.push((row, Some(self.task_space(task.id))));
        }
        for (row, of) in &mut rows {
            row.current = of.as_ref() == Some(&current);
        }
        rows.sort_by_key(|(row, of)| (row.current, std::cmp::Reverse(of.as_ref().map_or(0, |of| self.picked(of)))));
        let places = rows.into_iter().map(|(row, _)| row).collect();
        GoTo { places, recent: self.recent_files.clone(), ..GoTo::default() }
    }

    /// The folder a place's files come from now: a workspace's default, a checkout, a task's checkout.
    fn place_folder(&self, place: &Place) -> Option<String> {
        match place {
            Place::Workspace(id) => self.workbooks.get(*id).and_then(|book| book.worktree.clone()),
            Place::Checkout(path) => Some(path.clone()),
            Place::Task(id) => self.task_worktree(*id).map(String::from),
        }
    }

    /// Whether a place is still there to go to.
    pub(crate) fn place_exists(&self, place: &Place) -> bool {
        match place {
            Place::Workspace(id) => self.workbooks.listed().any(|book| book.id == *id),
            Place::Checkout(path) => self.repo_listed(path),
            Place::Task(id) => self.tasks.live(*id).is_some_and(|task| task.undone()),
        }
    }

    /// Goes to `place` as its section's row does: a workspace opens, a checkout's row is
    /// clicked, a task's conversation opens.
    fn goto_go(&mut self, place: Place, effects: &mut Vec<Effect>) {
        match place {
            Place::Workspace(id) => self.workbook_open(id, None, self.home.clone(), effects),
            Place::Checkout(path) => self.act(Action::RepoClick { path, axis: Axis::Right }, effects),
            Place::Task(id) => self.talk_to(Subject::Task(id), false, effects),
        }
    }

    /// ⌘P opens on the files of the checkout on show (the tab's, else its space's: a task's
    /// worktree, a workspace's default), when the Repos section lists one; ⌫ goes to the places.
    /// Elsewhere it opens on the places.
    pub(crate) fn goto_here(&mut self, effects: &mut Vec<Effect>) {
        let tab = self.tabs[self.main_at()].id;
        let Some(folder) = self.empty_folder(crate::model::city::At::Tab(tab)).map(String::from) else { return };
        let Some(goto) = self.palette.as_ref().and_then(|palette| palette.goto.as_ref()) else { return };
        // Named as its Repos row is; on show, so a file opens here without going anywhere.
        let listed = goto.places.iter().find(|row| row.place == Place::Checkout(folder.clone()));
        let (name, detail) = listed.map_or_else(
            || (crate::marks::directory_name(&folder).unwrap_or_default().to_string(), String::new()),
            |row| (row.name.clone(), row.detail.clone()),
        );
        let row =
            PlaceRow { place: Place::Checkout(folder.clone()), name, detail, folder: Some(folder), current: true };
        self.drill_into(row, String::new(), "", effects);
        if let Some(drill) = self.palette.as_mut().and_then(|p| p.goto.as_mut()).and_then(|goto| goto.drill.as_mut()) {
            drill.here = true;
        }
    }

    /// ↵ or a click on row `at` of go-to's places or files. Nothing there: it stays open.
    pub(crate) fn goto_choose(&mut self, at: usize, effects: &mut Vec<Effect>) {
        self.goto_choose_at(at, Axis::Right, effects);
    }

    /// Row `at` chosen; a file opens in a viewer `axis` of the focused pane (`Right`: in it, if
    /// it is empty).
    pub(crate) fn goto_choose_at(&mut self, at: usize, axis: Axis, effects: &mut Vec<Effect>) {
        let Some(palette) = &self.palette else { return };
        let Some(goto) = &palette.goto else { return };
        let chosen = match &goto.drill {
            Some(drill) => goto.rows.get(at).map(|file| (drill.row.clone(), Some(file.clone()))),
            None => goto.place_matches(palette.query.text()).get(at).map(|(_, row)| ((*row).clone(), None)),
        };
        let Some((row, file)) = chosen else { return };
        self.palette = None;
        self.palette_closed(effects);
        if file.is_none() || !row.current {
            self.goto_go(row.place, effects);
        }
        if let (Some(file), Some(folder)) = (file, row.folder) {
            let path = format!("{}/{file}", folder.trim_end_matches('/'));
            let open = Action::OpenFile { path, axis, beside: None, reuse: false, at: None };
            self.act(open, effects);
        }
    }

    /// Tab: into the highlighted place's files, if it has a folder.
    pub(crate) fn goto_drill(&mut self, effects: &mut Vec<Effect>) {
        let Some(palette) = &self.palette else { return };
        let Some(goto) = palette.goto.as_ref().filter(|goto| goto.drill.is_none()) else { return };
        let query = palette.query.text().to_string();
        let row = goto.place_matches(&query).get(palette.selected).map(|(_, row)| (*row).clone());
        if let Some(row) = row {
            self.drill_into(row, query, "", effects);
        }
    }

    /// After the user's edit of go-to's query (it was `before`, its highlight `selected`): a
    /// `/` typed after a place query goes into the highlighted place; `name/part` in one go,
    /// into `name`'s best match if it is clearly best. Drilled in: the files are ranked again.
    pub(crate) fn goto_typed(&mut self, before: &str, selected: usize, effects: &mut Vec<Effect>) {
        let Some(palette) = &mut self.palette else { return };
        let Some(goto) = &mut palette.goto else { return };
        let text = palette.query.text().to_string();
        // `>` first lists the commands, from the files too (⌘P opens on them).
        if goto.drill.is_some() && text.starts_with('>') {
            (goto.drill, goto.rows, palette.selected) = (None, Vec::new(), 0);
            return;
        }
        if goto.drill.is_some() {
            return goto.refresh(&text);
        }
        let Some((place, file)) = text.split_once('/').filter(|_| !text.starts_with('>')) else { return };
        let found = goto.place_matches(place);
        // Clearly best: no other row scores as high, or those that do share its folder (a
        // workspace and its checkout).
        let best = found.first().filter(|(score, row)| {
            found.iter().filter(|(other, _)| other == score).all(|(_, other)| other.folder == row.folder)
        });
        let row = if before == place && file.is_empty() { found.get(selected) } else { best };
        if let Some(row) = row.map(|(_, row)| (*row).clone()).filter(|row| row.folder.is_some()) {
            self.drill_into(row, place.to_string(), file, effects);
        }
    }

    /// The place's files listed, `query` typed over them; asked of the shell once a palette.
    fn drill_into(&mut self, mut row: PlaceRow, back: String, query: &str, effects: &mut Vec<Effect>) {
        // As it is now: a task's agent may have moved since the list was made.
        row.folder = self.place_folder(&row.place);
        let Some(folder) = row.folder.clone() else { return };
        let listed = self.repo_listed(&folder);
        let Some(palette) = &mut self.palette else { return };
        let Some(goto) = &mut palette.goto else { return };
        if goto.files_of(&folder).is_none() {
            let files = if listed { Files::Reading } else { Files::Unread(NOT_LISTED) };
            if listed {
                effects.push(Effect::ListFiles { folder: folder.clone() });
            }
            goto.files.push((folder, files));
        }
        goto.drill = Some(Drill { row, back, here: false });
        palette.query = Field::new(query, MAX_FILTER);
        palette.selected = 0;
        goto.refresh(query);
    }

    /// Backspace on an empty query, or Esc, while drilled in: the places again, as typed.
    pub(crate) fn goto_back(&mut self) {
        let Some(palette) = &mut self.palette else { return };
        let Some(goto) = &mut palette.goto else { return };
        if let Some(drill) = goto.drill.take() {
            palette.query = Field::new(&drill.back, MAX_FILTER);
            (palette.selected, goto.rows) = (0, Vec::new());
        }
    }

    /// The shell read a folder's files: kept while the palette is open. Only an answer to the
    /// read it waits for: a later one (a read of an earlier palette) changes nothing.
    pub(crate) fn goto_files(&mut self, folder: String, paths: Option<Vec<String>>, cut: bool) {
        let Some(palette) = &mut self.palette else { return };
        let Some(goto) = &mut palette.goto else { return };
        if let Some((_, files)) = goto.files.iter_mut().find(|(of, files)| *of == folder && *files == Files::Reading) {
            *files = paths.map_or(Files::Unread(NOT_READ), |paths| Files::Read { paths, cut });
            goto.refresh(palette.query.text());
        }
    }

    /// After every step: a place that went is no longer listed, and a drill into one ends, as
    /// does one whose place's folder changed. A folder that left the Repos list lists no files.
    pub(crate) fn settle_goto(&mut self) {
        let Some(goto) = self.palette.as_ref().and_then(|palette| palette.goto.as_ref()) else { return };
        let gone: Vec<Place> =
            goto.places.iter().map(|row| row.place.clone()).filter(|place| !self.place_exists(place)).collect();
        let unlisted: Vec<String> = goto
            .files
            .iter()
            .filter(|(folder, files)| *files != Files::Unread(NOT_LISTED) && !self.repo_listed(folder))
            .map(|(folder, _)| folder.clone())
            .collect();
        let moved = |drill: &Drill| drill.row.folder != self.place_folder(&drill.row.place);
        if goto.drill.as_ref().is_some_and(|drill| gone.contains(&drill.row.place) || moved(drill)) {
            self.goto_back();
        }
        let Some(palette) = &mut self.palette else { return };
        if let Some(goto) = &mut palette.goto {
            goto.places.retain(|row| !gone.contains(&row.place));
            for (_, files) in goto.files.iter_mut().filter(|(folder, _)| unlisted.contains(folder)) {
                *files = Files::Unread(NOT_LISTED);
            }
            if !unlisted.is_empty() {
                goto.refresh(palette.query.text());
            }
        }
        palette.selected = palette.selected.min(palette.count().saturating_sub(1));
    }
}
