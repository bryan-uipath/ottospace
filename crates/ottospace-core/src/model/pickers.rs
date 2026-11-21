//! The new-task box's pickers, in its field's bottom edge: project (`#`), repository (`>`) and
//! agent (`@`), each showing what ↵ will use. A click, or Tab and ⇧Tab, opens one's list; taking a
//! row writes its tag into the line, as typing it would: the line is the one place a choice is
//! kept. ⌥↵ on an agent makes it the default (`default_agent`), the user's act alone.

use serde::Serialize;

use super::{Effect, Model};
use crate::agents;
use crate::complete::{self, Dropdown, Row};
use crate::runner::RunnerId;

/// Why a client's ⌥↵ in the agent list is refused: it writes the user's settings.
pub const DEFAULT_USERS: &str = "only the user sets the default agent";

/// What the pickers show: the project (none: untagged), where ↵ starts the agent (`app · main`, a
/// folder; none: the home folder), and the agent, `default` when it is `default_agent`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Choices {
    pub project: Option<String>,
    pub place: Option<String>,
    pub agent: String,
    pub default: bool,
}

/// The kind of a list's first row, the one that takes the tag out of the line.
const NONE: &str = "none";

impl Model {
    /// The pickers the open box shows, in Tab's order. The quick note has none.
    pub fn compose_pickers(&self) -> &'static [char] {
        if self.compose.is_none() || self.quick.is_some() { &[] } else { &['#', '>', '@'] }
    }

    /// What the pickers show, while the box has them.
    pub fn compose_choices(&self) -> Option<Choices> {
        if self.compose_pickers().is_empty() {
            return None;
        }
        let draft = self.draft(self.compose_line()?);
        let plan = self.compose_plan()?;
        let place = plan.dir.map(|dir| match plan.branch {
            Some(branch) => format!("{dir} · {branch}"),
            None => dir,
        });
        let default = plan.agent.eq_ignore_ascii_case(self.default_agent());
        Some(Choices { project: draft.project, place, agent: plan.agent, default })
    }

    /// The name a picker's list has highlighted at first: the line's own (`""` for its none row).
    fn picker_current(&self, mark: char) -> String {
        let draft = self.compose_line().map(|line| self.draft(line));
        let draft = draft.as_ref();
        match mark {
            '#' => draft.and_then(|draft| draft.project.clone()),
            '>' => draft.and_then(|draft| draft.repo.clone()),
            _ => Some(draft.and_then(|draft| draft.agent.clone()).unwrap_or_else(|| self.default_agent().into())),
        }
        .unwrap_or_default()
    }

    /// A picker's rows: for a project or a repository first "none", which takes the tag out; the
    /// agent's default first, then each runner and its models, then the saved agents.
    pub(super) fn picker_rows(&self, mark: char) -> Vec<Row> {
        let row = |kind: &'static str, name: String, detail: String| Row { name, kind, detail, matched: Vec::new() };
        let none = |name: &str| row(NONE, name.into(), String::new());
        let named = |kind| self.candidates(mark).into_iter().map(move |c| row(kind, c.name, c.detail));
        match mark {
            '#' => {
                let mut rows: Vec<Row> = std::iter::once(none("no project")).chain(named("project")).collect();
                // A project typed that no task has yet: still the one chosen.
                let typed = self.picker_current('#');
                if !typed.is_empty() && !rows.iter().any(|row| row.name == typed) {
                    rows.push(row("project", typed, "new".into()));
                }
                rows
            }
            '>' => std::iter::once(none("no repository")).chain(named("repo")).collect(),
            '@' => {
                let mut names = self.compose_picks().whos;
                for saved in self.known.names.agents.iter() {
                    if !names.iter().any(|name| name.eq_ignore_ascii_case(saved)) {
                        names.push(saved.clone());
                    }
                }
                let default = self.default_agent();
                let detail = |name: &str| match name.eq_ignore_ascii_case(default) {
                    true => "default".into(),
                    false => agents::kind(name),
                };
                names.into_iter().map(|name| row("agent", name.clone(), detail(&name))).collect()
            }
            _ => Vec::new(),
        }
    }

    /// The open picker's list: its rows, the one highlighted (that ↑ ↓ went to, else the line's
    /// own). `None`: no picker is open, or the line does not have the keyboard.
    pub fn picker_menu(&self) -> Option<Dropdown> {
        let (mark, moved) = self.compose_picker.as_ref().filter(|_| self.composing())?;
        let rows = self.picker_rows(*mark);
        let current = self.picker_current(*mark);
        let at = |name: &str| rows.iter().position(|row| row.name.eq_ignore_ascii_case(name) && !row.name.is_empty());
        let none = rows.iter().position(|row| row.kind == NONE);
        let mine = if current.is_empty() { none } else { at(&current) };
        let selected = moved.as_deref().and_then(at).or(mine).or((!rows.is_empty()).then_some(0));
        Some(Dropdown { mark: *mark, typed: String::new(), start: 0, rows, selected, note: None, stale: false })
    }

    /// A click on a picker: its list opens, or closes if it is the one open. `None` closes any.
    pub(super) fn picker_toggle(&mut self, mark: Option<char>) {
        let open = self.compose_picker.as_ref().map(|(open, _)| *open);
        self.compose_picker = match mark {
            Some(mark) if self.composing() && self.compose_pickers().contains(&mark) && open != Some(mark) => {
                Some((mark, None))
            }
            _ => None,
        };
    }

    /// Tab (`delta` 1) or ⇧Tab (-1) with no tag to finish: the next picker's list, and after the
    /// last none (back to typing). One step, whatever the size of `delta`.
    pub(super) fn picker_step(&mut self, delta: i32) {
        let order = self.compose_pickers();
        if !self.composing() || order.is_empty() {
            return;
        }
        let at = self.compose_picker.as_ref().and_then(|(open, _)| order.iter().position(|mark| mark == open));
        // Positions 0..len are the pickers, len is the line.
        let len = order.len() as i32 + 1;
        let from = at.map_or(len - 1, |at| at as i32);
        let to = (from + delta.signum()).rem_euclid(len) as usize;
        self.compose_picker = order.get(to).map(|mark| (*mark, None));
    }

    /// ↑ or ↓ in the open list.
    pub(super) fn picker_move(&mut self, delta: i32) {
        let Some(menu) = self.picker_menu() else { return };
        let to = complete::moved(menu.selected, delta, menu.rows.len());
        let name = to.and_then(|at| menu.rows.get(at)).map(|row| row.name.clone());
        if let Some((_, moved)) = &mut self.compose_picker {
            *moved = name.filter(|name| !name.is_empty()).or(Some(String::new()));
        }
    }

    /// Row `index` of the open list taken: its tag in the line (none: out of it); the list closes.
    pub(super) fn picker_take(&mut self, index: usize) {
        let Some(menu) = self.picker_menu() else { return };
        let Some(row) = menu.rows.get(index) else { return };
        let name = (row.kind != NONE).then(|| row.name.clone());
        self.compose_picker = None;
        self.compose_pick(menu.mark, name);
    }

    /// Whether ⌥↵ would make the agent highlighted in the open list the default: a runner, or a
    /// runner and a model (a saved agent is never `default_agent`). Its name.
    pub fn picker_default(&self) -> Option<String> {
        let menu = self.picker_menu().filter(|menu| menu.mark == '@')?;
        let name = &menu.rows.get(menu.selected?)?.name;
        let built_in = RunnerId::named(name).is_some() || agents::typed(name).is_some();
        built_in.then(|| name.clone())
    }

    /// ⌥↵ in the agent list: the highlighted agent becomes `default_agent`, written to the settings
    /// as the user's own act. The line's `@agent` goes: the default needs none.
    pub(super) fn picker_make_default(&mut self, effects: &mut Vec<Effect>) {
        let Some(agent) = self.picker_default().filter(|_| self.by_user) else { return };
        self.compose_picker = None;
        self.compose_pick('@', None);
        if self.default_agent() != agent {
            self.default_agent = Some(agent.clone());
            effects.push(Effect::SaveDefaultAgent { agent });
        }
    }
}
