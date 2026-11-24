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

/// What the editor starts ticked with: this week's areas, an agent's proposal (E on its card), or
/// last week's (E on "Keep last week's focus?").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Seed {
    Current,
    Proposal(u32),
    Last,
}

/// The card Decisions shows for the week's focus: an agent's proposal (its revision), or "keep last
/// week's focus?" at a new week.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WeekCard {
    Proposal(u32),
    Carry,
}

/// A row of the editor: a Projects entry (its path), or an area that is no entry's (a name a
/// proposal brought), kept as it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Project(String),
    Kept(Area),
}

/// The editor while it is open. In the state: how many rows, which are ticked, the cursor, what it
/// started from; never a project's title or path, nor an area's name.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Edit {
    #[serde(skip)]
    pub rows: Vec<Row>,
    pub count: usize,
    pub ticked: Vec<usize>,
    pub cursor: usize,
    pub from: Seed,
}

impl Model {
    pub(super) fn focus_act(&mut self, action: Action, effects: &mut Vec<Effect>) {
        match action {
            Action::SetWeek { week } if crate::focus::valid_week(&week) => self.focus.week = week,
            Action::RestoreFocus { kept } => self.focus.restore(kept),
            Action::FocusEditOpen { from } => {
                // A seed that is not there (a proposal answered meanwhile) opens nothing.
                if let Some(areas) = self.seed_areas(from) {
                    self.focus_edit = Some(self.focus_editor(&areas, from));
                }
            }
            Action::FocusPropose { week, areas, provenance, now } => {
                let refused = self.focus_refusal(&week, &areas, now).is_some();
                self.focus.attempt(now);
                if !refused {
                    self.focus.propose(&areas, provenance, now);
                }
            }
            // The week's card answers only while it is on show, at the revision named.
            Action::FocusAccept { revision, at } if self.week_on_show(revision) => {
                if self.focus.accept(revision, at) {
                    effects.push(Effect::SaveFocus { kept: self.focus.kept() });
                }
            }
            Action::FocusDecline { revision } if self.week_on_show(revision) => {
                self.focus.decline(revision);
            }
            Action::FocusCarry { keep, at } if self.week_on_show(0) => {
                if self.focus.carry(keep, at) && keep {
                    effects.push(Effect::SaveFocus { kept: self.focus.kept() });
                }
            }
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
                let seed = self.seed_areas(edit.from).unwrap_or_default();
                let areas: Vec<Area> =
                    ticked.iter().filter_map(|r| self.edit_area(edit.rows.get(*r)?, &seed)).collect();
                if self.focus.set(&areas, SetBy::User, at) {
                    effects.push(Effect::SaveFocus { kept: self.focus.kept() });
                    // Set from a proposal or last week's: that question is answered too.
                    match edit.from {
                        Seed::Proposal(revision) => {
                            self.focus.pending.take_if(|p| p.revision == revision);
                        }
                        Seed::Last => self.focus.carried = Some(self.focus.week.clone()),
                        Seed::Current => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// Decisions shows the week's card at `revision` (0: keep last week's?).
    fn week_on_show(&self, revision: u32) -> bool {
        self.on_show() == Some((super::decisions::On::Week, revision))
    }

    /// The Projects entries' names: what an area's projects link to.
    pub fn project_names(&self) -> Vec<String> {
        self.projects.listed.iter().map(|p| project_name(&p.path).to_string()).collect()
    }

    /// Why a `focus_set` is refused now, or `None`. The socket's reply says it.
    pub fn focus_refusal(&self, week: &str, areas: &[Area], now: u64) -> Option<String> {
        self.focus.refusal(week, areas, now)
    }

    /// The card for the week's focus Decisions has to show, if any: a proposal waiting first, else
    /// "keep last week's focus?".
    pub fn week_card(&self) -> Option<WeekCard> {
        if !self.features.on(crate::features::Feature::DecisionSources) {
            return None;
        }
        match &self.focus.pending {
            Some(p) => Some(WeekCard::Proposal(p.revision)),
            None => self.focus.wants_carry().then_some(WeekCard::Carry),
        }
    }

    /// The areas a seed ticks: none there (a proposal gone) is `None`.
    fn seed_areas(&self, from: Seed) -> Option<Vec<Area>> {
        match from {
            Seed::Current => Some(self.focus.areas().to_vec()),
            Seed::Proposal(revision) => {
                self.focus.pending.as_ref().filter(|p| p.revision == revision).map(|p| p.areas.clone())
            }
            Seed::Last => self.focus.last().map(|w| w.areas.clone()),
        }
    }

    /// After every step, while the editor is open: its rows follow the Projects list as it is read
    /// (the folder is read while the editor is open), the user's ticks kept on the rows they ticked.
    pub(super) fn settle_focus_edit(&mut self) {
        self.focus.settle();
        // The palette or the picker over it takes the keys: the editor closes, setting nothing.
        if self.picker.is_some() || self.palette.is_some() {
            self.focus_edit = None;
        }
        let Some(edit) = self.focus_edit.as_ref() else { return };
        let Some(areas) = self.seed_areas(edit.from) else { return self.focus_edit = None };
        let fresh = self.focus_editor(&areas, edit.from);
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
                // An entry gone from the folder: the area kept as is that it stood for.
                Row::Project(path) => fresh.rows.iter().position(|r| match r {
                    Row::Kept(area) => area.projects.iter().any(|q| q.eq_ignore_ascii_case(project_name(path))),
                    Row::Project(_) => false,
                }),
            })
        };
        let mut ticked: Vec<usize> = ticked.iter().filter_map(at).collect();
        ticked.sort_unstable();
        ticked.dedup();
        let cursor = cursor.as_ref().and_then(at).unwrap_or(0);
        self.focus_edit = Some(Edit { ticked, cursor, ..fresh });
    }

    /// The editor's rows: the seed's areas that no entry stands for, then the Projects entries in
    /// their order; ticked, the seed's areas.
    fn focus_editor(&self, areas: &[Area], from: Seed) -> Edit {
        let ordered = self.projects.ordered();
        let entry = |area: &Area| {
            ordered.iter().position(|p| area.projects.iter().any(|q| q.eq_ignore_ascii_case(project_name(&p.path))))
        };
        let kept: Vec<Row> = areas.iter().filter(|a| entry(a).is_none()).map(|a| Row::Kept(a.clone())).collect();
        let first = kept.len();
        let mut ticked: Vec<usize> = (0..first.min(MAX_AREAS)).collect();
        for at in areas.iter().filter_map(|a| entry(a).map(|at| first + at)) {
            if !ticked.contains(&at) && ticked.len() < MAX_AREAS {
                ticked.push(at);
            }
        }
        let rows: Vec<Row> = kept.into_iter().chain(ordered.iter().map(|p| Row::Project(p.path.clone()))).collect();
        Edit { count: rows.len(), rows, ticked, cursor: 0, from }
    }

    /// The area a ticked row sets: the seed's own area that links the entry, as it is (its name and
    /// repositories kept); else a new one, named by the entry's title and linked by its name.
    fn edit_area(&self, row: &Row, seed: &[Area]) -> Option<Area> {
        match row {
            Row::Kept(area) => Some(area.clone()),
            Row::Project(path) => {
                let name = project_name(path);
                let own = seed.iter().find(|a| a.projects.iter().any(|q| q.eq_ignore_ascii_case(name)));
                if let Some(own) = own {
                    return Some(own.clone());
                }
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
    use super::Seed;
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
        m.update(Action::FocusEditOpen { from: Seed::Current }.sent_by(Origin::Socket));
        assert!(m.focus_edit.is_none());
        m.update(Action::SetWeek { week: "2027-W01".into() }.sent_by(Origin::Socket));
        assert_eq!(m.focus.week, "2026-W48");
        m.update(Action::FocusEditOpen { from: Seed::Current });
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
        m.update(Action::FocusEditOpen { from: Seed::Current });
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
        m.update(Action::FocusEditOpen { from: Seed::Current });
        m.update(Action::FocusEditToggle { row: Some(1) });
        m.update(Action::FocusEditSave { at: 1 });
        // Opened again before the folder is read: the area kept as is, ticked; read, it is its entry.
        m.update(Action::SetProjects { on: true, listed: Vec::new() }.sent_by(Origin::System));
        m.update(Action::FocusEditOpen { from: Seed::Current });
        assert_eq!(m.focus_edit.as_ref().map(|e| (e.count, e.ticked.clone())), Some((1, vec![0])));
        projects(&mut m, &["atlas", "beacon", "cobalt"]);
        assert_eq!(m.focus_edit.as_ref().map(|e| (e.count, e.ticked.clone())), Some((3, vec![1])));
    }

    fn propose(m: &mut Model, areas: &[(&str, &[&str])], now: u64) {
        let areas = areas
            .iter()
            .map(|(name, projects)| Area {
                name: (*name).into(),
                projects: projects.iter().map(|p| p.to_string()).collect(),
                repos: vec![],
            })
            .collect();
        m.update(
            Action::FocusPropose { week: "2026-W48".into(), areas, provenance: None, now }.sent_by(Origin::Socket),
        );
    }

    fn key(m: &mut Model, key: &str) {
        if let crate::model::chat::Key::Act(action) = m.decision_key(key, crate::keys::Mods::default(), 50) {
            m.update(action);
        }
    }

    #[test]
    fn a_proposal_sets_nothing_until_the_users_enter_on_the_revision_shown() {
        let mut m = Model::in_workspace();
        projects(&mut m, &["atlas", "beacon"]);
        m.update(Action::SetWeek { week: "2026-W48".into() }.sent_by(Origin::System));
        let refused = |m: &Model, week: &str, areas: Vec<Area>| m.focus_refusal(week, &areas, 1).is_some();
        let area =
            |name: &str, project: &str| Area { name: name.into(), projects: vec![project.into()], repos: vec![] };
        assert!(refused(&m, "2026-W47", vec![area("Login", "atlas")]), "another week");
        // A link to nothing listed is no refusal: the agent learns nothing of what exists.
        assert!(!refused(&m, "2026-W48", vec![area("Login", "nope")]), "kept, shown as not listed");
        assert!(refused(&m, "2026-W48", vec![area(&"x".repeat(41), "atlas")]), "a name too long");
        assert!(refused(&m, "2026-W48", vec![area("a", "atlas"); 5]), "five areas");
        propose(&mut m, &[("Login rewrite", &["ATLAS"]), ("Hiring", &[])], 1);
        assert_eq!(m.week_card(), Some(super::WeekCard::Proposal(1)));
        assert!(m.focus.areas().is_empty(), "nothing set yet");
        m.update(Action::DecisionsOpen);
        assert_eq!(m.decisions.as_ref().and_then(|r| r.week), Some(super::WeekCard::Proposal(1)));
        // A client's ↵ is refused; a newer proposal takes the card's place, and a ↵ of the older one does nothing.
        m.update(Action::FocusAccept { revision: 1, at: 2 }.sent_by(Origin::Socket));
        propose(&mut m, &[("Login rewrite", &["atlas"]), ("Beacon", &["beacon"])], 3);
        assert_eq!(m.decisions.as_ref().and_then(|r| r.week), Some(super::WeekCard::Proposal(2)));
        m.update(Action::FocusAccept { revision: 1, at: 4 });
        assert!(m.focus.areas().is_empty(), "the older one is not what was shown");
        // Replaced while on show: marked changed; the first ↵ only says it was seen, the next sets it.
        assert!(m.decisions.as_ref().is_some_and(|r| r.changed));
        key(&mut m, "enter");
        assert!(m.focus.areas().is_empty() && m.decisions.as_ref().is_some_and(|r| !r.changed));
        // A key the shell gives for the revision it drew before the swap takes nothing.
        let stale = m.decision_key_drawn(
            "enter",
            crate::keys::Mods::default(),
            50,
            Some((super::super::decisions::On::Week, 1)),
        );
        assert_eq!(stale, crate::model::chat::Key::Swallow);
        key(&mut m, "enter");
        let names: Vec<&str> = m.focus.areas().iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            (names, m.focus.current().map(|w| w.set_by)),
            (vec!["Login rewrite", "Beacon"], Some(SetBy::Proposal))
        );
        assert!(m.decisions.is_none(), "nothing else waits");
        // The same again: no card. Declined, the same is not shown again; another is.
        propose(&mut m, &[("Login rewrite", &["atlas"]), ("Beacon", &["beacon"])], 5);
        assert_eq!(m.week_card(), None);
        propose(&mut m, &[("Hiring", &[])], 6);
        m.update(Action::DecisionsOpen);
        key(&mut m, "n");
        assert_eq!((m.week_card(), m.focus.areas().len()), (None, 2), "declined: the focus stays");
        propose(&mut m, &[("Hiring", &[])], 7);
        assert_eq!(m.week_card(), None, "declined this week");
        propose(&mut m, &[("Hiring loop", &[])], 8);
        assert!(matches!(m.week_card(), Some(super::WeekCard::Proposal(_))));
        // E: the editor with the proposal ticked; ↵ there sets it and answers the proposal.
        let revision = m.focus.pending.as_ref().map(|p| p.revision).unwrap();
        m.update(Action::FocusEditOpen { from: Seed::Proposal(revision) });
        assert_eq!(m.focus_edit.as_ref().map(|e| e.ticked.clone()), Some(vec![0]));
        m.update(Action::FocusEditToggle { row: Some(1) });
        m.update(Action::FocusEditSave { at: 9 });
        assert_eq!((m.week_card(), m.focus.areas().len()), (None, 2));
    }

    #[test]
    fn a_new_week_asks_once_whether_to_keep_last_weeks() {
        let mut m = Model::in_workspace();
        projects(&mut m, &["atlas"]);
        m.update(Action::SetWeek { week: "2026-W47".into() }.sent_by(Origin::System));
        m.update(Action::FocusEditOpen { from: Seed::Current });
        m.update(Action::FocusEditToggle { row: Some(0) });
        m.update(Action::FocusEditSave { at: 1 });
        assert_eq!(m.week_card(), None);
        m.update(Action::SetWeek { week: "2026-W48".into() }.sent_by(Origin::System));
        assert!(m.focus.areas().is_empty(), "a new week starts with nothing in focus");
        assert_eq!(m.week_card(), Some(super::WeekCard::Carry));
        m.update(Action::DecisionsOpen);
        m.update(Action::FocusCarry { keep: true, at: 2 }.sent_by(Origin::Socket));
        assert!(m.focus.areas().is_empty(), "a client keeps nothing");
        key(&mut m, "enter");
        assert_eq!((m.focus.areas().len(), m.week_card()), (1, None));
        // Next week: N, none; not asked again.
        m.update(Action::SetWeek { week: "2026-W49".into() }.sent_by(Origin::System));
        m.update(Action::DecisionsOpen);
        key(&mut m, "n");
        assert_eq!((m.focus.areas().len(), m.week_card()), (0, None));
    }

    #[test]
    fn a_save_keeps_an_areas_own_name_and_links_and_ticks_a_row_once() {
        let mut m = Model::default();
        projects(&mut m, &["atlas", "beacon"]);
        m.update(Action::SetWeek { week: "2026-W48".into() }.sent_by(Origin::System));
        let area = |name: &str, project: &str, repos: Vec<String>| Area {
            name: name.into(),
            projects: vec![project.into()],
            repos,
        };
        let own = area("Hiring push", "atlas", vec!["/code/ats".into()]);
        m.focus.set(&[own.clone(), area("B", "beacon", vec![]), area("C", "atlas", vec![])], SetBy::Proposal, 1);
        m.update(Action::FocusEditOpen { from: Seed::Current });
        assert_eq!(m.focus_edit.as_ref().map(|e| e.ticked.clone()), Some(vec![0, 1]), "atlas once");
        m.update(Action::FocusEditSave { at: 2 });
        assert_eq!(m.focus.areas()[0], own, "its own name and repositories, unchanged");
        // The palette over it closes it, setting nothing.
        m.update(Action::FocusEditOpen { from: Seed::Current });
        m.update(Action::PaletteOpen);
        assert!(m.focus_edit.is_none());
    }

    #[test]
    fn the_flag_off_opens_no_editor() {
        let mut m = Model::default();
        m.update(Action::SetFeatures {
            features: crate::features::Features::all().with(crate::features::Feature::DecisionSources, false),
        });
        m.update(Action::FocusEditOpen { from: Seed::Current });
        assert!(m.focus_edit.is_none());
    }
}
