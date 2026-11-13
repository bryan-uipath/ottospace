//! Directional focus across the window's zones: left of the panes the section list, then the
//! left rail; right of them the drawer, then the right rail. The user's alone: a client's
//! focus move stops at the panes, as before.

use serde::Serialize;

use super::{Action, Effect, Model, Section};
use crate::layout::{Dir, Node};
use crate::model::PaneId;

/// The left rail's sections, top to bottom, as the shell draws them.
pub const SECTIONS: [Section; 5] = [Section::Apps, Section::Workbook, Section::Tasks, Section::Repos, Section::Marks];

/// A rail that has the keyboard, in list mode as the section list does.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RailFocus {
    /// A section's cell on the left rail.
    Left(Section),
    /// A cell of the right rail, by place: its conversations (`Model::rail`), then `+`.
    Right(usize),
}

/// Where the keyboard is, for the state reply and the property test.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Pane(PaneId),
    /// The open section's list, on this row.
    List(usize),
    Rail(Section),
    /// The drawer: the agent's terminal or the new task.
    Drawer,
    /// The accessibility inspector in the drawer's place: its rows.
    Inspector,
    Right(usize),
    /// No pane, and nothing else has it.
    Nothing,
}

/// The row a section's list was left on, however it was left: where ⌘⌃← comes back to. Not
/// part of what the model shows, so its `Debug` says nothing of it.
#[derive(Clone, Copy, Default)]
pub struct KeptRow(pub Option<(Section, usize)>);

impl std::fmt::Debug for KeptRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeptRow")
    }
}

/// Why a client's key on a rail does nothing.
pub const RAILS_USERS: &str = "only the user moves the keyboard over the rails";

impl Model {
    /// Where the keyboard is.
    pub fn focus_target(&self) -> Target {
        match self.rail_focus {
            Some(RailFocus::Left(section)) => Target::Rail(section),
            Some(RailFocus::Right(at)) => Target::Right(at),
            None if self.list.is_some() => Target::List(self.list.unwrap_or_default()),
            None if self.inspector.as_ref().is_some_and(|inspector| inspector.filtering) => Target::Inspector,
            None if self.talk.as_ref().is_some_and(|talk| talk.keys) || self.drawer_compose() => Target::Drawer,
            None => self.focused.map_or(Target::Nothing, Target::Pane),
        }
    }

    /// A user's move that crosses zones, or one over a rail. True: it was handled here.
    pub(super) fn zones(&mut self, action: &Action, effects: &mut Vec<Effect>) -> bool {
        // The Apps filter is a text field: its keys are its own.
        if self.app_lists.filtering {
            return false;
        }
        match (action, self.rail_focus) {
            (Action::FocusDir { dir }, Some(RailFocus::Left(section))) => match dir {
                Dir::Up | Dir::Down => self.rail_move(if *dir == Dir::Up { -1 } else { 1 }),
                Dir::Right => self.open_section(section, effects),
                Dir::Left => {}
            },
            (Action::FocusDir { dir }, Some(RailFocus::Right(at))) => match dir {
                Dir::Up | Dir::Down => self.rail_move(if *dir == Dir::Up { -1 } else { 1 }),
                Dir::Left => self.show_cell(at, effects),
                Dir::Right => {}
            },
            (Action::ListMove { delta }, Some(_)) => self.rail_move(*delta),
            (Action::ListChoose { .. }, Some(RailFocus::Left(section))) => self.open_section(section, effects),
            (Action::ListChoose { .. }, Some(RailFocus::Right(at))) => self.show_cell(at, effects),
            (Action::LeaveList, Some(focus)) => {
                let side = if matches!(focus, RailFocus::Right(_)) { Dir::Right } else { Dir::Left };
                self.back_to_pane(side, effects)
            }
            // Other list keys (Tab, s, h, x) do nothing on a rail.
            (Action::ListSection { .. } | Action::ListSnooze { .. } | Action::ListHold, Some(_)) => {}
            (Action::ListDismiss | Action::ListUndo | Action::ListPin | Action::ListReorder { .. }, Some(_)) => {}
            (Action::FocusDir { dir: Dir::Left }, None) if self.list.is_some() => {
                self.list = None;
                self.rail_focus = self.sidebar.map(RailFocus::Left);
            }
            (Action::FocusDir { dir: Dir::Right }, None) if self.list.is_some() => {
                self.back_to_pane(Dir::Left, effects)
            }
            (Action::FocusDir { dir: Dir::Right }, None) if self.in_right_zone() => {
                self.leave_right_zone(effects);
                self.rail_focus = Some(RailFocus::Right(self.shown_cell()));
            }
            (Action::FocusDir { dir: Dir::Left }, None) if self.in_right_zone() => {
                self.leave_right_zone(effects);
                self.back_to_pane(Dir::Right, effects);
            }
            // A pane at the edge: on into the zone beside it.
            (Action::FocusDir { dir: dir @ (Dir::Left | Dir::Right) }, None) if self.at_edge(*dir) => {
                if *dir == Dir::Left {
                    match self.sidebar {
                        Some(section) => self.open_section(section, effects),
                        None => self.rail_focus = Some(RailFocus::Left(Section::Apps)),
                    }
                } else if self.inspector.is_some() {
                    // The inspector has the drawer's place: its rows take the keys.
                    self.act(Action::InspectorFilter { on: true }, effects);
                } else if self.talk.as_ref().is_some_and(|talk| talk.open && self.agent_runs(&talk.task)) {
                    self.talk_keys(true, effects);
                } else {
                    self.rail_focus = Some(RailFocus::Right(self.shown_cell()));
                }
            }
            _ => return false,
        }
        true
    }

    /// No pane lies that way from the focused one (or there is none).
    fn at_edge(&self, dir: Dir) -> bool {
        match (self.focused, &self.layout) {
            (Some(from), Some(layout)) => layout.neighbor(from, dir).is_none(),
            _ => true,
        }
    }

    /// The keyboard is in what the drawer's place shows: the conversation, the new task, or the inspector.
    pub(super) fn in_right_zone(&self) -> bool {
        matches!(self.focus_target(), Target::Drawer | Target::Inspector)
    }

    /// The drawer's place gives up the keyboard: a new task's line is put aside.
    fn leave_right_zone(&mut self, effects: &mut Vec<Effect>) {
        self.talk_keys(false, effects);
        self.compose_aside();
        if let Some(inspector) = &mut self.inspector {
            inspector.filtering = false;
        }
    }

    /// The new task in the drawer has the keyboard. A capture's draft title is not the drawer's.
    fn drawer_compose(&self) -> bool {
        self.compose_keys() && self.compose_host() == Some(super::Host::Drawer)
    }

    /// The new task being typed in the drawer gives up the keyboard: its line is kept for the
    /// box's next opening.
    fn compose_aside(&mut self) {
        if self.drawer_compose() {
            self.put_compose_aside();
        }
    }

    /// The right rail's cell of the conversation the drawer shows, else its first.
    fn shown_cell(&self) -> usize {
        self.rail().iter().position(|cell| cell.shown).unwrap_or(0)
    }

    /// Up (negative) or down a rail, wrapping. The right rail's last place is its `+`.
    fn rail_move(&mut self, delta: i32) {
        let step = |at: usize, len: usize| (at as i64 + i64::from(delta)).rem_euclid(len as i64) as usize;
        self.rail_focus = match self.rail_focus {
            Some(RailFocus::Left(section)) => {
                let at = SECTIONS.iter().position(|s| *s == section).unwrap_or(0);
                Some(RailFocus::Left(SECTIONS[step(at, SECTIONS.len())]))
            }
            Some(RailFocus::Right(at)) => Some(RailFocus::Right(step(at, self.rail().len() + 1))),
            None => None,
        };
    }

    /// A section's list, with the keyboard on the row it was on when it was left, else its first.
    fn open_section(&mut self, section: Section, effects: &mut Vec<Effect>) {
        self.leave(effects);
        (self.rail_focus, self.sidebar) = (None, Some(section));
        let kept = self.list_kept.0.filter(|(of, _)| *of == section).map(|(_, row)| row);
        let rows = self.list_rows().len();
        self.list = Some(kept.filter(|row| *row < rows).unwrap_or_else(|| self.first_row()));
    }

    /// The right rail's cell at `at`: its conversation in the drawer, with the keyboard; `+`
    /// opens the new task.
    fn show_cell(&mut self, at: usize, effects: &mut Vec<Effect>) {
        self.rail_focus = None;
        match self.rail().get(at) {
            Some(cell) if cell.shown => {
                if let Some(talk) = &mut self.talk {
                    talk.open = true;
                }
                self.talk_keys(true, effects);
            }
            Some(cell) => self.act(Action::TalkPick { id: cell.task.clone() }, effects),
            None => self.act(Action::OpenCompose { text: String::new() }, effects),
        }
    }

    /// Back from a zone to the focused pane, the one it was entered from (nothing in a zone
    /// moves pane focus), if it is still on show; else to the pane at the workspace's edge on
    /// the zone's side (`side`).
    fn back_to_pane(&mut self, side: Dir, effects: &mut Vec<Effect>) {
        (self.rail_focus, self.list) = (None, None);
        let leaves = self.layout.as_ref().map(Node::leaves).unwrap_or_default();
        let back = self.focused.filter(|pane| leaves.contains(pane));
        let edge = || {
            leaves.iter().copied().find(|pane| self.layout.as_ref().is_some_and(|l| l.neighbor(*pane, side).is_none()))
        };
        if let Some(pane) = back.or_else(edge) {
            self.focus(pane, effects);
        }
    }

    /// A rail keeps the keyboard only while nothing else has taken it, and its place is there.
    pub(super) fn settle_zone(&mut self) {
        let taken = self.list.is_some()
            || self.holds_keyboard()
            || self.live.is_some()
            || self.talk.as_ref().is_some_and(|talk| talk.keys)
            || self.mark_input.is_some();
        if taken {
            self.rail_focus = None;
        }
        if let Some(RailFocus::Right(at)) = self.rail_focus {
            self.rail_focus = Some(RailFocus::Right(at.min(self.rail().len())));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Axis, Kind, Mode, Origin};

    fn user(m: &mut Model, dir: Dir) -> Target {
        m.update(Action::FocusDir { dir }.sent_by(Origin::User));
        m.focus_target()
    }

    #[test]
    fn left_goes_to_the_list_then_the_rail_and_right_comes_back() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        assert_eq!(m.focus_target(), Target::Pane(2));
        // A client's move stops at the panes, as before.
        m.update(Action::FocusDir { dir: Dir::Left }.sent_by(Origin::Socket));
        m.update(Action::FocusDir { dir: Dir::Left }.sent_by(Origin::Socket));
        assert_eq!(m.focus_target(), Target::Pane(1));
        // No section open: the rail, on its first cell. ↓ moves; → opens that section's list.
        assert_eq!((user(&mut m, Dir::Left), m.mode()), (Target::Rail(Section::Apps), Mode::List));
        m.update(Action::ListMove { delta: 1 }.sent_by(Origin::User));
        assert_eq!(m.focus_target(), Target::Rail(Section::Workbook));
        assert!(matches!(user(&mut m, Dir::Right), Target::List(_)));
        assert_eq!(m.sidebar, Some(Section::Workbook));
        // ← goes back to the rail; → the list again; → once more the pane it came from.
        assert_eq!(user(&mut m, Dir::Left), Target::Rail(Section::Workbook));
        assert!(matches!(user(&mut m, Dir::Right), Target::List(_)));
        assert_eq!(user(&mut m, Dir::Right), Target::Pane(1));
        // With the section open, left of the panes is its list.
        assert!(matches!(user(&mut m, Dir::Left), Target::List(_)));
        assert_eq!(user(&mut m, Dir::Right), Target::Pane(1));
        // A client's key on a rail does nothing, with a reason.
        user(&mut m, Dir::Left);
        user(&mut m, Dir::Left);
        let rail = m.focus_target();
        assert_eq!(m.refusal(&Action::ListMove { delta: 1 }, Origin::Socket), Some(RAILS_USERS));
        // Esc leaves to the pane.
        m.update(Action::LeaveList.sent_by(Origin::User));
        assert_eq!((rail, m.focus_target()), (Target::Rail(Section::Workbook), Target::Pane(1)));
    }

    #[test]
    fn right_goes_to_the_right_rail_and_left_comes_back() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        // No conversation runs: the right rail, on its `+`.
        assert_eq!(user(&mut m, Dir::Right), Target::Right(0));
        m.update(Action::ListMove { delta: 1 }.sent_by(Origin::User));
        assert_eq!(m.focus_target(), Target::Right(0), "one place: it wraps onto itself");
        // ← on `+`: the new task, in the drawer. ← again: back to the pane.
        assert_eq!(user(&mut m, Dir::Left), Target::Drawer);
        assert_eq!(user(&mut m, Dir::Left), Target::Pane(2));
        // → from the drawer: the right rail.
        m.update(Action::OpenCompose { text: "half".into() });
        assert_eq!(user(&mut m, Dir::Right), Target::Right(0));
        assert_eq!(m.compose, None, "the line is put aside");
    }

    /// A click on a pane while a rail had the keyboard left the keys with the rail.
    #[test]
    fn a_pane_taken_another_way_takes_the_keyboard_from_the_rail() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        user(&mut m, Dir::Right);
        m.update(Action::Focus { pane: 1 }.sent_by(Origin::User));
        assert_eq!((m.focus_target(), m.mode()), (Target::Pane(1), Mode::Normal));
        // A list left by Esc comes back on its row.
        m.update(Action::ToggleSection { section: Section::Tasks });
        m.update(Action::TaskAdd {
            title: "a".into(),
            priority: 3,
            state: crate::tasks::State::Captured,
            tab: None,
            assign: Default::default(),
        });
        m.update(Action::TaskAdd {
            title: "b".into(),
            priority: 3,
            state: crate::tasks::State::Captured,
            tab: None,
            assign: Default::default(),
        });
        let first = user(&mut m, Dir::Left);
        m.update(Action::ListMove { delta: 1 }.sent_by(Origin::User));
        m.update(Action::LeaveList.sent_by(Origin::User));
        assert_eq!((first, user(&mut m, Dir::Left)), (Target::List(1), Target::List(2)));
        m.update(Action::LeaveList.sent_by(Origin::User));
        m.update(Action::CloseSidebar.sent_by(Origin::User));
        // Esc on the right rail goes back to a pane on the right: the one focused.
        assert_eq!(user(&mut m, Dir::Left), Target::Rail(Section::Apps));
        m.update(Action::LeaveList.sent_by(Origin::User));
        assert_eq!(m.focus_target(), Target::Pane(1));
    }

    #[test]
    fn with_the_inspector_open_right_goes_into_its_rows_then_the_right_rail() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::InspectorOpen { pane: 2, app: "Notes".into() });
        assert_eq!((user(&mut m, Dir::Right), m.mode()), (Target::Inspector, Mode::Inspect));
        assert_eq!(user(&mut m, Dir::Right), Target::Right(0));
        assert!(m.inspector.as_ref().is_some_and(|inspector| !inspector.filtering));
        // Back: the inspector's rows, then the pane.
        user(&mut m, Dir::Right);
        m.update(Action::LeaveList.sent_by(Origin::User));
        assert_eq!(user(&mut m, Dir::Right), Target::Inspector);
        assert_eq!(user(&mut m, Dir::Left), Target::Pane(2));
    }
}
