//! The week's focus in the model (`decision_sources`): the week the shell says it is, and the
//! editor the user sets it with by hand (⌘P › This Week's Focus…): the Projects entries, up to
//! four ticked. The rules: `crate::focus`.

use serde::Serialize;

use super::{Action, Effect, Model};
use crate::focus::{Area, MAX_AREAS, SetBy, project_name};

/// Why a client's step on the focus editor is refused: the week's focus is the user's.
pub const USERS_ALONE: &str = "only the user sets the week's focus";
/// Why a client may not say what week it is.
pub const SHELLS_ALONE: &str = "only ottospace says what week it is";

/// A row of the editor: a Projects entry (its path), or an area of this week's that is no entry's
/// (a name the user took from a proposal), kept as it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Project(String),
    Kept(Area),
}

/// The editor while it is open. In the state: how many rows, which are ticked, the cursor; never
/// a project's title or path, nor an area's name.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Edit {
    #[serde(skip)]
    pub rows: Vec<Row>,
    pub count: usize,
    pub ticked: Vec<usize>,
    pub cursor: usize,
}

impl Model {
    pub(super) fn focus_act(&mut self, action: Action, effects: &mut Vec<Effect>) {
        match action {
            Action::SetWeek { week } if crate::focus::valid_week(&week) => self.focus.week = week,
            Action::RestoreFocus { kept } => self.focus.restore(kept),
            Action::FocusEditOpen => self.focus_edit = Some(self.focus_editor()),
            Action::FocusEditClose => self.focus_edit = None,
            Action::FocusEditMove { delta } => {
                if let Some(edit) = self.focus_edit.as_mut().filter(|edit| edit.count > 0) {
                    let at = edit.cursor as i64 + i64::from(delta);
                    edit.cursor = at.clamp(0, edit.count as i64 - 1) as usize;
                }
            }
            Action::FocusEditToggle { row } => {
                if let Some(edit) = self.focus_edit.as_mut() {
                    let row = row.unwrap_or(edit.cursor);
                    match edit.ticked.iter().position(|r| *r == row) {
                        Some(at) => {
                            edit.ticked.remove(at);
                        }
                        // A fifth is not ticked: four areas at most.
                        None if row < edit.count && edit.ticked.len() < MAX_AREAS => edit.ticked.push(row),
                        None => {}
                    }
                }
            }
            Action::FocusEditSave { at } => {
                let Some(edit) = self.focus_edit.take() else { return };
                let mut ticked = edit.ticked.clone();
                ticked.sort_unstable();
                let areas: Vec<Area> = ticked.iter().filter_map(|r| self.edit_area(edit.rows.get(*r)?)).collect();
                if self.focus.set(&areas, SetBy::User, at) {
                    effects.push(Effect::SaveFocus { kept: self.focus.kept() });
                }
            }
            _ => {}
        }
    }

    /// After every step, while the editor is open: its rows follow the Projects list as it is read
    /// (the folder is read while the editor is open), the user's ticks kept on the rows they ticked.
    pub(super) fn settle_focus_edit(&mut self) {
        let Some(edit) = self.focus_edit.as_ref() else { return };
        let fresh = self.focus_editor();
        if fresh.rows == edit.rows {
            return;
        }
        let ticked: Vec<Row> = edit.ticked.iter().filter_map(|r| edit.rows.get(*r).cloned()).collect();
        let cursor = edit.rows.get(edit.cursor).cloned();
        // A row as it is now: the same, or for an area kept as is, the entry that now stands for it.
        let at = |row: &Row| {
            let same = fresh.rows.iter().position(|r| r == row);
            same.or_else(|| match row {
                Row::Kept(area) => fresh.rows.iter().position(|r| match r {
                    Row::Project(path) => area.projects.iter().any(|q| q.eq_ignore_ascii_case(project_name(path))),
                    Row::Kept(_) => false,
                }),
                Row::Project(_) => None,
            })
        };
        let mut ticked: Vec<usize> = ticked.iter().filter_map(at).collect();
        ticked.dedup();
        let cursor = cursor.as_ref().and_then(at).unwrap_or(0);
        self.focus_edit = Some(Edit { ticked, cursor, ..fresh });
    }

    /// The editor's rows: this week's areas that no entry stands for, then the Projects entries in
    /// their order; ticked, the week's areas.
    fn focus_editor(&self) -> Edit {
        let ordered = self.projects.ordered();
        let entry = |area: &Area| {
            ordered.iter().position(|p| area.projects.iter().any(|q| q.eq_ignore_ascii_case(project_name(&p.path))))
        };
        let kept: Vec<Row> =
            self.focus.areas().iter().filter(|a| entry(a).is_none()).map(|a| Row::Kept(a.clone())).collect();
        let first = kept.len();
        let mut ticked: Vec<usize> = (0..first).collect();
        ticked.extend(self.focus.areas().iter().filter_map(|a| entry(a).map(|at| first + at)));
        ticked.dedup();
        let rows: Vec<Row> = kept.into_iter().chain(ordered.iter().map(|p| Row::Project(p.path.clone()))).collect();
        Edit { count: rows.len(), rows, ticked, cursor: 0 }
    }

    /// The area a ticked row sets: an entry's, named by its title and linked by its name.
    fn edit_area(&self, row: &Row) -> Option<Area> {
        match row {
            Row::Kept(area) => Some(area.clone()),
            Row::Project(path) => {
                let project = self.projects.listed.iter().find(|p| &p.path == path)?;
                let name = project_name(&project.path).to_string();
                Some(Area { name: project.title.clone(), projects: vec![name], repos: Vec::new() })
            }
        }
    }

    /// A key while the focus editor is open: ↑↓ (J K) move, space ticks, ↵ sets the week's
    /// focus, Esc closes. `None`: not the editor's.
    pub fn focus_edit_key(&self, key: &str, now: u64) -> Option<Action> {
        self.focus_edit.as_ref()?;
        Some(match key {
            "up" | "k" => Action::FocusEditMove { delta: -1 },
            "down" | "j" => Action::FocusEditMove { delta: 1 },
            "space" => Action::FocusEditToggle { row: None },
            "enter" => Action::FocusEditSave { at: now },
            "escape" => Action::FocusEditClose,
            _ => return None,
        })
    }

    /// The words of the editor's row `at` and whether it is an entry, for its view: drawn only.
    pub fn focus_row(&self, at: usize) -> Option<(String, bool)> {
        match self.focus_edit.as_ref()?.rows.get(at)? {
            Row::Kept(area) => Some((area.name.clone(), false)),
            Row::Project(path) => {
                let project = self.projects.listed.iter().find(|p| &p.path == path)?;
                Some((project.title.clone(), true))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::focus::{Area, SetBy};
    use crate::model::{Action, Effect, Model, Origin};
    use crate::projects::Project;

    fn projects(m: &mut Model, names: &[&str]) {
        let listed = names
            .iter()
            .map(|n| Project {
                path: format!("/notes/projects/{n}"),
                title: format!("The {n}"),
                note: None,
                modified: 0,
            })
            .collect();
        m.update(Action::SetProjects { on: true, listed }.sent_by(Origin::System));
    }

    #[test]
    fn the_user_ticks_up_to_four_projects_and_sets_the_weeks_focus() {
        let mut m = Model::default();
        projects(&mut m, &["atlas", "beacon", "cobalt", "dune", "echo"]);
        m.update(Action::SetWeek { week: "2026-W48".into() }.sent_by(Origin::System));
        // A client neither opens it nor says the week.
        m.update(Action::FocusEditOpen.sent_by(Origin::Socket));
        assert!(m.focus_edit.is_none());
        m.update(Action::SetWeek { week: "2027-W01".into() }.sent_by(Origin::Socket));
        assert_eq!(m.focus.week, "2026-W48");
        m.update(Action::FocusEditOpen);
        assert_eq!(m.focus_edit.as_ref().map(|e| e.count), Some(5));
        for row in 0..5 {
            m.update(Action::FocusEditToggle { row: Some(row) });
        }
        assert_eq!(m.focus_edit.as_ref().map(|e| e.ticked.clone()), Some(vec![0, 1, 2, 3]), "four at most");
        m.update(Action::FocusEditToggle { row: Some(1) });
        let effects = m.update(Action::FocusEditSave { at: 7 });
        let names: Vec<&str> = m.focus.areas().iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["The atlas", "The cobalt", "The dune"]);
        assert_eq!(m.focus.area_of(Some("COBALT"), None), Some(1));
        assert!(effects.contains(&Effect::SaveFocus { kept: m.focus.kept() }));
        assert!(m.focus_edit.is_none());
        // Opened again: ticked as set; an area of no entry kept at the top, ticked.
        let free = Area { name: "Hiring".into(), ..Area::default() };
        let mut areas = m.focus.areas().to_vec();
        areas.insert(0, free);
        m.focus.set(&areas, SetBy::Proposal, 8);
        m.update(Action::FocusEditOpen);
        let edit = m.focus_edit.clone().unwrap();
        assert_eq!((edit.count, edit.ticked.clone()), (6, vec![0, 1, 3, 4]));
        assert_eq!(m.focus_row(0), Some(("Hiring".to_string(), false)));
        m.update(Action::FocusEditClose);
        assert_eq!(m.focus.areas().len(), 4, "closing sets nothing");
    }

    #[test]
    fn rows_read_after_it_opened_keep_the_ticks() {
        let mut m = Model::default();
        m.update(Action::SetWeek { week: "2026-W48".into() }.sent_by(Origin::System));
        projects(&mut m, &["atlas", "beacon"]);
        m.update(Action::FocusEditOpen);
        m.update(Action::FocusEditToggle { row: Some(1) });
        m.update(Action::FocusEditSave { at: 1 });
        // Opened again before the folder is read: the area kept as is, ticked; read, it is its entry.
        m.update(Action::SetProjects { on: true, listed: Vec::new() }.sent_by(Origin::System));
        m.update(Action::FocusEditOpen);
        assert_eq!(m.focus_edit.as_ref().map(|e| (e.count, e.ticked.clone())), Some((1, vec![0])));
        projects(&mut m, &["atlas", "beacon", "cobalt"]);
        assert_eq!(m.focus_edit.as_ref().map(|e| (e.count, e.ticked.clone())), Some((3, vec![1])));
    }

    #[test]
    fn the_flag_off_opens_no_editor() {
        let mut m = Model::default();
        m.update(Action::SetFeatures {
            features: crate::features::Features::all().with(crate::features::Feature::DecisionSources, false),
        });
        m.update(Action::FocusEditOpen);
        assert!(m.focus_edit.is_none());
    }
}
