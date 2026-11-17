//! Directional focus across the window's zones: left of the panes the section list, then the
//! left rail; right of them the drawer, then the drawer's rail (`rail`). The user's alone: a client's focus move stops at the
//! panes, as before.

use serde::Serialize;

use super::rail::RailItem;
use super::{Action, Effect, Model, Section};
use crate::layout::{Dir, Node};
use crate::model::PaneId;

/// The left rail's sections, top to bottom, as the shell draws them. Calendar only while a feed is
/// set: `Model::sections`.
pub const SECTIONS: [Section; 6] =
    [Section::Tasks, Section::Calendar, Section::Apps, Section::Workbook, Section::Repos, Section::Marks];

/// A rail when it has the keyboard, in list mode as the section list does: a section's cell, or
/// an item of the drawer's rail.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RailFocus {
    Left(Section),
    Right(RailItem),
}

/// Where the keyboard is, for the state reply and the property test.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Pane(PaneId),
    /// The open section's list, on this row.
    List(usize),
    Rail(Section),
    /// An item of the drawer's rail.
    DrawerRail(RailItem),
    /// The drawer: the agent's terminal or the new task.
    Drawer,
    /// The accessibility inspector in the drawer's place: its rows.
    Inspector,
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
            Some(RailFocus::Right(item)) => Target::DrawerRail(item),
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
            // The drawer's rail: ↑ ↓ move, ← back to the panes; nothing lies right of it.
            (Action::FocusDir { dir }, Some(RailFocus::Right(_))) => match dir {
                Dir::Up | Dir::Down => self.rail_move(if *dir == Dir::Up { -1 } else { 1 }),
                Dir::Left => self.back_to_pane(Dir::Right, effects),
                Dir::Right => {}
            },
            (Action::ListMove { delta }, Some(_)) => self.rail_move(*delta),
            (Action::ListChoose { .. }, Some(RailFocus::Left(section))) => self.open_section(section, effects),
            // ↵ shows the item; the keys stay on the rail.
            (Action::ListChoose { .. }, Some(RailFocus::Right(item))) => self.rail_select(item, effects),
            (Action::LeaveList, Some(RailFocus::Left(_))) => self.back_to_pane(Dir::Left, effects),
            (Action::LeaveList, Some(RailFocus::Right(_))) => self.back_to_pane(Dir::Right, effects),
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
            // Right of the drawer its rail, while it shows; else the drawer is the last zone.
            (Action::FocusDir { dir: Dir::Right }, None) if self.in_right_zone() => self.enter_drawer_rail(effects),
            (Action::FocusDir { dir: Dir::Left }, None) if self.in_right_zone() => {
                self.leave_right_zone(effects);
                self.back_to_pane(Dir::Right, effects);
            }
            // A pane at the edge: on into the zone beside it.
            (Action::FocusDir { dir: dir @ (Dir::Left | Dir::Right) }, None) if self.at_edge(*dir) => {
                if *dir == Dir::Left {
                    match self.sidebar {
                        Some(section) => self.open_section(section, effects),
                        None => self.rail_focus = Some(RailFocus::Left(self.sections()[0])),
                    }
                } else if self.inspector.is_some() {
                    // The inspector has the drawer's place: its rows take the keys.
                    self.act(Action::InspectorFilter { on: true }, effects);
                } else if !self.listen.shown
                    && self.talk.as_ref().is_some_and(|talk| talk.open && self.agent_runs(&talk.task))
                {
                    self.talk_keys(true, effects);
                } else if self.listen.shown {
                    // The transcript takes no keys: on to its rail.
                    self.enter_drawer_rail(effects);
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

    /// Up (negative) or down the rail, wrapping.
    fn rail_move(&mut self, delta: i32) {
        let step = |at: usize, len: usize| (at as i64 + i64::from(delta)).rem_euclid(len as i64) as usize;
        self.rail_focus = match self.rail_focus {
            Some(RailFocus::Left(section)) => {
                let sections = self.sections();
                let at = sections.iter().position(|s| *s == section).unwrap_or(0);
                Some(RailFocus::Left(sections[step(at, sections.len())]))
            }
            Some(RailFocus::Right(item)) => {
                let items = self.rail();
                let at = items.iter().position(|it| *it == item).unwrap_or(0);
                items.get(step(at, items.len().max(1))).copied().map(RailFocus::Right)
            }
            None => None,
        };
    }

    /// The keyboard onto the drawer's rail, on the item shown, while the drawer and its rail show.
    fn enter_drawer_rail(&mut self, effects: &mut Vec<Effect>) {
        let open = self.listen.shown || self.talk.as_ref().is_some_and(|talk| talk.open);
        let Some(item) = self.rail_selected().or(self.rail().first().copied()).filter(|_| open) else { return };
        self.talk_keys(false, effects);
        self.compose_aside();
        self.rail_focus = Some(RailFocus::Right(item));
    }

    /// A section's list, with the keyboard on the row it was on when it was left, else its first.
    fn open_section(&mut self, section: Section, effects: &mut Vec<Effect>) {
        self.leave(effects);
        (self.rail_focus, self.sidebar) = (None, Some(section));
        let kept = self.list_kept.0.filter(|(of, _)| *of == section).map(|(_, row)| row);
        let rows = self.list_rows().len();
        self.list = Some(kept.filter(|row| *row < rows).unwrap_or_else(|| self.first_row()));
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
        // An item gone from the drawer's rail (or the rail with it) takes the keyboard off it.
        let open = self.listen.shown || self.talk.as_ref().is_some_and(|talk| talk.open);
        let gone = matches!(self.rail_focus, Some(RailFocus::Right(item)) if !open || !self.rail().contains(&item));
        if taken || gone {
            self.rail_focus = None;
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
        assert_eq!((user(&mut m, Dir::Left), m.mode()), (Target::Rail(Section::Tasks), Mode::List));
        m.update(Action::ListMove { delta: 1 }.sent_by(Origin::User));
        assert_eq!(m.focus_target(), Target::Rail(Section::Apps));
        assert!(matches!(user(&mut m, Dir::Right), Target::List(_)));
        assert_eq!(m.sidebar, Some(Section::Apps));
        // ← goes back to the rail; → the list again; → once more the pane it came from.
        assert_eq!(user(&mut m, Dir::Left), Target::Rail(Section::Apps));
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
        assert_eq!((rail, m.focus_target()), (Target::Rail(Section::Apps), Target::Pane(1)));
    }

    /// A click on a pane while a rail had the keyboard left the keys with the rail.
    #[test]
    fn a_pane_taken_another_way_takes_the_keyboard_from_the_rail() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        user(&mut m, Dir::Left);
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
        // Esc on the rail goes back to the pane focused.
        assert_eq!(user(&mut m, Dir::Left), Target::Rail(Section::Tasks));
        m.update(Action::LeaveList.sent_by(Origin::User));
        assert_eq!(m.focus_target(), Target::Pane(1));
    }

    #[test]
    fn with_the_inspector_open_right_goes_into_its_rows_and_left_comes_back() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::InspectorOpen { pane: 2, app: "Notes".into() });
        assert_eq!((user(&mut m, Dir::Right), m.mode()), (Target::Inspector, Mode::Inspect));
        // Nothing lies beyond it.
        assert_eq!(user(&mut m, Dir::Right), Target::Inspector);
        assert_eq!(user(&mut m, Dir::Left), Target::Pane(2));
    }
}
