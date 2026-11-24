//! Overlay panels (`overlay_panels`, an experiment): the open sidebar and the open drawer float
//! over the panes instead of taking width from them. The panes are laid out as if both were
//! collapsed into their rails, so opening either moves and resizes nothing, hosted windows
//! included. Each is ephemeral: only the user opens one, and it closes after use.
//!
//! - The sidebar closes on its toggle (⌘⇧L, its rail cell, ←), Esc in its list, a click outside it,
//!   the keyboard leaving it, or a move it made: a tab, a space or a pane chosen from it.
//! - The drawer folds on its toggle (⌘⇧R, →|, its rail item), a click outside it, or the keyboard
//!   leaving it for a pane. Esc in a conversation stays the agent's.
//! - The user's opening of the sidebar gives its list the keyboard; Esc or the toggle gives it back
//!   to the conversation it came from, else to the focused pane (it never left it).
//! - No client or timer opens either: what one shows in the drawer's place waits behind the rail.

use super::zones::Target;
use super::{Action, Effect, Envelope, Mode, Model, Origin, Owner, PaneId, rail};
use crate::features::Feature;

/// Why a client's opening of a side overlay, or its dismissal, is refused.
pub const OVERLAY_USERS: &str = "only the user opens or dismisses a side overlay";

/// Why a client's pull of the drawer out over the panes, or back, is refused.
pub const WIDE_USERS: &str = "only the user pulls the drawer out or back";

/// How much of the panes a pulled-out drawer leaves showing at its left: it still reads as over them.
pub const PULLED_EDGE: f32 = 64.;

/// The pulled-out drawer's width, its rail included: the window less what lies left of the panes
/// (`left`) and `PULLED_EDGE` of them. Never narrower than `normal`, the drawer as it is otherwise.
pub fn pulled_width(window: f32, left: f32, normal: f32) -> f32 {
    (window - left - PULLED_EDGE).max(normal)
}

/// How long an overlay's slide in lasts, in milliseconds. Its close plays it backwards: drawn only,
/// the model has already closed it.
pub const SLIDE_MS: f32 = 140.;

/// How far in a slide of `ms` (an overlay's, `SLIDE_MS`; zoom-out's) is `at` milliseconds in: 0 out
/// of sight, 1 at rest. Eased out, so it is nearly there at once and never holds up a click.
pub fn slide(at: f32, ms: f32) -> f32 {
    crate::seal::ease_out((at / ms).clamp(0., 1.))
}

/// Where a slide of `ms` is `elapsed` milliseconds after it was `from` milliseconds in, moving in
/// (`opening`) or back out. A toggle mid-way turns it from where it is: 60 in, closed, 20 later is 40.
pub fn travel(from: f32, elapsed: f32, opening: bool, ms: f32) -> f32 {
    (if opening { from + elapsed } else { from - elapsed }).clamp(0., ms)
}

/// What an overlay rule compares a step with: the step's sender and action, and the view before it.
pub(super) struct Before {
    origin: Origin,
    action: Action,
    sidebar: bool,
    collapsed: bool,
    target: Target,
    view: (u32, Owner, Option<PaneId>),
}

impl Model {
    /// The sidebar and the drawer are overlays (`overlay_panels`).
    pub fn overlays(&self) -> bool {
        self.features.on(Feature::OverlayPanels)
    }

    /// The drawer is pulled out over the panes, an edge of them left showing: the user's choice,
    /// kept for every task (`layout.json`'s `drawer.wide`), and only as an overlay.
    pub fn talk_pulled(&self) -> bool {
        self.talk_wide && self.overlays()
    }

    /// The open sidebar takes its width from the panes: open, and no overlay.
    pub fn sidebar_takes_room(&self) -> bool {
        self.sidebar.is_some() && !self.overlays()
    }

    /// The open drawer takes its width from the panes: something in its place, and no overlay.
    pub fn drawer_takes_room(&self) -> bool {
        !self.drawer_collapsed() && !self.overlays()
    }

    /// A client's step that would open, switch or close an overlay: refused with `OVERLAY_USERS`.
    pub(super) fn overlay_refusal(&self, action: &Action) -> Option<&'static str> {
        let boxed = self.compose_host() == Some(super::Host::Drawer);
        let opens = match action {
            Action::ToggleSection { .. } | Action::OverlayDismiss => true,
            Action::FocusList => self.sidebar.is_none(),
            Action::CloseSidebar => self.sidebar.is_some(),
            Action::OpenCompose { .. } => self.drawer_collapsed(),
            // The user's box over the panes: a client's Esc or × would fold the drawer under them.
            Action::ComposeEscape | Action::CloseCompose => boxed,
            _ => false,
        };
        (self.overlays() && opens).then_some(OVERLAY_USERS)
    }

    /// The view an overlay rule compares the step with. None with the flag off: nothing to compare.
    pub(super) fn overlay_before(&self, sent: &Envelope) -> Option<Before> {
        if !self.overlays() {
            return None;
        }
        let view = (self.tabs[self.main_at()].id, self.shown_space().clone(), self.focused);
        Some(Before {
            origin: sent.origin,
            action: sent.action.clone(),
            sidebar: self.sidebar.is_some(),
            collapsed: self.drawer_collapsed(),
            target: self.focus_target(),
            view,
        })
    }

    /// A popup over everything (a row's menu, the palette, the picker, the quit question, the key
    /// reference): a click outside an overlay is its own, and dismisses neither.
    fn popped(&self) -> bool {
        let menu = self.row_menu.is_some() || self.agent.menu.is_some() || self.slot_dialog.is_some();
        menu || self.palette.is_some() || self.picker.is_some() || self.confirm.is_some() || self.keys
    }

    /// A click outside both overlays and both rails: each closes.
    pub(super) fn overlay_dismiss(&mut self, effects: &mut Vec<Effect>) {
        if self.popped() {
            return;
        }
        self.close_overlay_sidebar();
        self.fold_drawer(effects);
    }

    /// The drawer folds into its rail, whatever it shows (a brief, the inspector, the box's line put
    /// aside), but a capture's draft: the user's work in progress. What the toggle opens again is kept.
    pub(super) fn fold_drawer(&mut self, effects: &mut Vec<Effect>) {
        // Nor while the user's editor writes the open box's line (⌃G): its save would find no box.
        // Nor the box's own ✎ draft: its captures would be lost with a line put aside.
        if self.drawer_collapsed() || self.capture.is_some() || (self.overlays() && self.edits_compose()) {
            return;
        }
        // Folded: no Esc in the sidebar hands the keyboard back to it.
        self.overlay_back = false;
        self.folded = Some(match () {
            _ if self.listen.shown => rail::Folded::Transcript,
            _ if self.welcome() || self.compose_host() == Some(super::Host::Drawer) => rail::Folded::Box,
            _ => rail::Folded::Rest,
        });
        self.listen.shown = false;
        self.talk_keys(false, effects);
        if let Some(talk) = &mut self.talk {
            talk.open = false;
        }
        (self.brief, self.inspector, self.drawer_open) = (None, None, false);
        if self.compose_host() == Some(super::Host::Drawer) {
            self.put_compose_aside();
        }
    }

    fn close_overlay_sidebar(&mut self) {
        let left = matches!(self.focus_target(), Target::List(_) | Target::Rail(_));
        self.sidebar = None;
        if left {
            (self.list, self.rail_focus) = (None, None);
        }
        // The Apps filter's keyboard goes with its section: this may run after the step settled it.
        self.settle_app_filter();
    }

    /// After a step, with the flag on: what opened or closed an overlay by the rules above.
    pub(super) fn settle_overlays(&mut self, before: Option<Before>, effects: &mut Vec<Effect>) {
        let Some(before) = before.filter(|_| self.overlays()) else {
            self.overlay_back = false;
            return;
        };
        if before.origin != Origin::User {
            // A client's or a timer's step opens neither: what it put in the drawer waits behind the rail.
            if !before.sidebar && self.sidebar.is_some() {
                self.close_overlay_sidebar();
            }
            if before.collapsed && !self.drawer_collapsed() {
                self.fold_drawer(effects);
            }
            return;
        }
        let left = |target: Target| matches!(target, Target::List(_) | Target::Rail(_));
        let right = |target: Target| matches!(target, Target::Drawer | Target::Inspector | Target::DrawerRail(_));
        let target = self.focus_target();
        let own =
            matches!(before.action, Action::ToggleSection { .. } | Action::ListSection { .. } | Action::CloseSidebar);
        let free = matches!(self.mode(), Mode::Normal | Mode::Talk | Mode::List) && self.away.is_none();
        if !before.sidebar && self.sidebar.is_some() && free {
            // Opened by the user: its list takes the keyboard, from the conversation too. Not from
            // a box, a mode, an app or a tab out's window that has it.
            self.overlay_back = before.target == Target::Drawer;
            self.talk_keys(false, effects);
            if self.list.is_none() {
                (self.list, self.rail_focus) = (Some(self.first_row()), None);
            }
        } else if before.sidebar && self.sidebar.is_some() && !own {
            let view = (self.tabs[self.main_at()].id, self.shown_space().clone(), self.focused);
            let esc = matches!(before.action, Action::LeaveList) && left(before.target);
            // The drawer's own keys take the keyboard beside it: both may be open.
            let drawer = matches!(
                before.action,
                Action::TalkToggle | Action::TalkKeys { .. } | Action::RailSelect { .. } | Action::TalkCycle { .. }
            );
            let gone = left(before.target) && !left(target) && !drawer;
            // A row chosen in it that goes somewhere: a task's conversation, a repository, a workspace.
            let chose = matches!(
                before.action,
                Action::TaskTalk { .. }
                    | Action::TaskGo { .. }
                    | Action::TaskCard { .. }
                    | Action::SessionOpen { .. }
                    | Action::RepoClick { .. }
                    | Action::OpenRepo { .. }
                    | Action::MarkGo { .. }
                    | Action::WorkbookOpen { .. }
            );
            if esc || gone || chose || view != before.view {
                self.close_overlay_sidebar();
                if esc && std::mem::take(&mut self.overlay_back) {
                    self.talk_keys(true, effects);
                }
            }
        } else if before.sidebar && self.sidebar.is_none() && own && left(before.target) {
            // Its toggle by key or click: the keyboard back where it came from.
            if std::mem::take(&mut self.overlay_back) {
                self.talk_keys(true, effects);
            }
        }
        if self.sidebar.is_none() && !left(self.focus_target()) {
            self.overlay_back = false;
        }
        // The keyboard left the drawer for a pane: it folds.
        let moves = matches!(
            before.action,
            Action::Focus { .. }
                | Action::FocusDir { .. }
                | Action::FocusCycle { .. }
                | Action::LeaveList
                | Action::Enter { .. }
                | Action::TalkKeys { on: false }
        );
        if !before.collapsed && moves && right(before.target) && matches!(self.focus_target(), Target::Pane(_)) {
            self.fold_drawer(effects);
        }
    }
}

#[cfg(test)]
mod pulled {
    use super::*;
    use crate::features::Features;

    /// Pulled out, the drawer covers the panes but an edge; never narrower than it was; only as
    /// an overlay; only the user's; kept for every task.
    #[test]
    fn the_drawer_pulls_out_over_the_panes_leaving_an_edge_and_only_the_user_pulls_it() {
        assert_eq!(pulled_width(1400., 56., 420.), 1400. - 56. - PULLED_EDGE);
        assert_eq!(pulled_width(500., 56., 420.), 420., "a narrow window: as it was");
        let mut m = Model::default();
        m.update(Action::TalkWide);
        assert!(!m.talk_pulled(), "no overlay: nothing to pull out over");
        m.update(Action::SetFeatures { features: Features::all() }.sent_by(Origin::System));
        assert_eq!(m.refusal(&Action::TalkWide, Origin::Socket), Some(WIDE_USERS));
        m.update(Action::TalkWide);
        assert!(m.talk_pulled());
        m.update(Action::TalkWide);
        assert!(!m.talk_pulled());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Axis;
    use crate::features::Features;
    use crate::layout::Dir;
    use crate::model::{Kind, Section};

    /// Two terminals and a task with its conversation in the drawer, the flag on or off.
    fn workspace(on: bool) -> Model {
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::Focus { pane: 1 });
        m.update(Action::OpenCompose { text: "one".into() });
        m.update(Action::ComposeSubmit { open: true });
        m.update(Action::TalkKeys { on: false });
        let features = Features::all().with(Feature::OverlayPanels, on);
        m.update(Action::SetFeatures { features }.sent_by(Origin::System));
        assert_eq!((m.drawer_collapsed(), m.focus_target()), (false, Target::Pane(1)));
        m
    }

    #[test]
    fn the_slide_eases_out_to_rest() {
        let slide = |at| slide(at, SLIDE_MS);
        assert_eq!((slide(0.), slide(SLIDE_MS), slide(1e9), slide(-5.)), (0., 1., 1., 0.));
        assert!(slide(SLIDE_MS / 4.) > 0.5, "most of the way in the first quarter");
    }

    /// The close is the opening backwards, from where it is: a toggle mid-way turns it there.
    #[test]
    fn a_slide_travels_back_from_where_it_is() {
        let (slide, travel) =
            (|at| slide(at, SLIDE_MS), |from, elapsed, opening| travel(from, elapsed, opening, SLIDE_MS));
        assert_eq!((travel(0., 30., true), travel(SLIDE_MS, 30., false)), (30., SLIDE_MS - 30.));
        assert_eq!((travel(60., 20., false), travel(40., 1e9, true), travel(40., 1e9, false)), (40., SLIDE_MS, 0.));
        // As far out 30 ms into the close as 30 ms before the opening's end.
        assert_eq!(slide(travel(SLIDE_MS, 30., false)), slide(SLIDE_MS - 30.));
    }

    #[test]
    fn with_the_flag_on_neither_takes_room_and_off_both_do() {
        for on in [false, true] {
            let mut m = workspace(on);
            m.update(Action::ToggleSection { section: Section::Repos });
            assert!(m.sidebar.is_some() && !m.drawer_collapsed(), "flag {on}");
            assert_eq!((m.sidebar_takes_room(), m.drawer_takes_room()), (!on, !on), "flag {on}");
        }
    }

    /// Off: today's sidebar, which keeps the keyboard where it was and stays open after a click on a pane.
    #[test]
    fn with_the_flag_off_nothing_changes() {
        let mut m = workspace(false);
        m.update(Action::ToggleSection { section: Section::Repos });
        assert_eq!((m.list, m.sidebar), (None, Some(Section::Repos)));
        m.update(Action::Focus { pane: 2 });
        assert_eq!(m.sidebar, Some(Section::Repos));
        assert!(m.refusal(&Action::ToggleSection { section: Section::Tasks }, Origin::Socket).is_none());
        m.update(Action::OverlayDismiss);
        assert!(m.sidebar.is_some() && !m.drawer_collapsed(), "the dismissal is the flag's");
    }

    #[test]
    fn the_users_opening_gives_the_list_the_keyboard_and_esc_or_the_toggle_closes_it() {
        let mut m = workspace(true);
        for close in [Action::LeaveList, Action::ToggleSection { section: Section::Repos }, Action::CloseSidebar] {
            m.update(Action::ToggleSection { section: Section::Repos });
            assert_eq!((m.mode(), m.sidebar), (Mode::List, Some(Section::Repos)), "{close:?}");
            // ↓ in the list and another section keep it open.
            m.update(Action::ListMove { delta: 1 });
            m.update(Action::ToggleSection { section: Section::Tasks });
            m.update(Action::ToggleSection { section: Section::Repos });
            assert_eq!(m.sidebar, Some(Section::Repos), "{close:?}");
            m.update(close.clone());
            assert_eq!((m.sidebar, m.focus_target()), (None, Target::Pane(1)), "{close:?}");
        }
    }

    #[test]
    fn a_click_outside_or_a_move_from_it_closes_the_sidebar() {
        let mut m = workspace(true);
        let open = |m: &mut Model| {
            m.update(Action::ToggleSection { section: Section::Tasks });
            assert!(m.sidebar.is_some());
        };
        // A click on a pane, on what is outside both, or the keys back to the panes.
        for outside in [Action::Focus { pane: 2 }, Action::OverlayDismiss, Action::FocusDir { dir: Dir::Right }] {
            open(&mut m);
            m.update(outside.clone());
            assert!(m.sidebar.is_none() && m.list.is_none(), "{outside:?}");
        }
        // A tab or a task chosen from it: it closes, by click or by ↵.
        m.update(Action::NewTab);
        open(&mut m);
        m.update(Action::SelectTab { index: 0 });
        assert!(m.sidebar.is_none());
        open(&mut m);
        m.update(Action::TaskTalk { id: 1 });
        assert!(m.sidebar.is_none());
        open(&mut m);
        let row = m.list_rows().iter().position(|row| matches!(row, crate::list::ListRow::Task(1)));
        m.list = row;
        m.update(Action::ListChoose { axis: Axis::Right });
        assert!(m.sidebar.is_none() && row.is_some(), "{:?}", m.list_rows());
        // A menu over it: the click is the menu's.
        open(&mut m);
        m.update(Action::PaletteOpen);
        m.update(Action::OverlayDismiss);
        assert!(m.sidebar.is_some());
    }

    #[test]
    fn the_drawer_folds_on_a_click_outside_or_the_keys_leaving_it_and_esc_stays_the_agents() {
        let mut m = workspace(true);
        assert!(!m.drawer_collapsed());
        m.update(Action::OverlayDismiss);
        assert!(m.drawer_collapsed());
        // ⌘⇧R opens it with the keyboard; Esc is the agent's (the shell types it), the drawer stays.
        m.update(Action::TalkToggle);
        assert_eq!((m.drawer_collapsed(), m.focus_target()), (false, Target::Drawer));
        // ← to a pane: it folds, the keyboard on the pane it left.
        m.update(Action::FocusDir { dir: Dir::Left });
        assert_eq!((m.drawer_collapsed(), m.focus_target()), (true, Target::Pane(1)));
        // ⌘L gives the keyboard back to the pane: it folds too.
        m.update(Action::TalkToggle);
        m.update(Action::TalkKeys { on: false });
        assert_eq!((m.drawer_collapsed(), m.focus_target()), (true, Target::Pane(1)));
        // Open by its rail item, the keys on a pane: a move between panes keeps it.
        m.update(Action::RailSelect { item: rail::RailItem::Task { id: 1 } });
        m.update(Action::FocusDir { dir: Dir::Right });
        assert!(!m.drawer_collapsed(), "the keys were not in it");
    }

    #[test]
    fn both_open_at_once_and_the_sidebar_gives_the_keyboard_back_to_the_conversation() {
        let mut m = workspace(true);
        m.update(Action::TalkKeys { on: true });
        assert_eq!(m.focus_target(), Target::Drawer);
        m.update(Action::ToggleSection { section: Section::Repos });
        assert!(m.sidebar.is_some() && !m.drawer_collapsed());
        assert_eq!(m.mode(), Mode::List);
        m.update(Action::LeaveList);
        assert_eq!((m.sidebar, m.focus_target(), m.drawer_collapsed()), (None, Target::Drawer, false));
        // One click outside both closes both.
        m.update(Action::ToggleSection { section: Section::Repos });
        m.update(Action::OverlayDismiss);
        assert!(m.sidebar.is_none() && m.drawer_collapsed());
        // Review: the drawer folded while the sidebar had the keyboard; Esc there does not reopen it.
        m.update(Action::TalkToggle);
        m.update(Action::ToggleSection { section: Section::Repos });
        m.update(Action::TalkToggle);
        assert!(m.sidebar.is_some() && m.drawer_collapsed());
        m.update(Action::LeaveList);
        assert!(m.sidebar.is_none() && m.drawer_collapsed());
    }

    /// The box sent while its editor (⌃G) still runs: the editor writes no box, and keeps no drawer open.
    #[test]
    fn an_editor_on_a_sent_box_lets_the_drawer_fold() {
        let mut m = workspace(true);
        m.update(Action::OpenCompose { text: "two".into() });
        m.update(Action::EditorOpen);
        // On the open box: a click outside leaves it under the editor.
        m.update(Action::OverlayDismiss);
        assert!(m.compose.is_some() && m.editing.is_some());
        m.update(Action::ComposeSubmit { open: true });
        assert!(m.compose.is_none() && m.editing.is_some() && !m.drawer_collapsed());
        m.update(Action::OverlayDismiss);
        assert!(m.drawer_collapsed());
        // Nor does a client's step open it again.
        m.update(Action::TaskTalk { id: 2 }.sent_by(Origin::Socket));
        assert!(m.drawer_collapsed());
    }

    /// The Apps filter has the keyboard in the sidebar: closed by a click outside or a row chosen, it gives it up.
    #[test]
    fn the_apps_filter_goes_with_the_sidebar() {
        for close in [Action::OverlayDismiss, Action::OpenRepo { path: "/code/app".into(), axis: Axis::Right }] {
            let mut m = workspace(true);
            m.update(Action::ToggleSection { section: Section::Apps });
            m.update(Action::AppsFilter { on: true });
            assert!(m.app_lists.filtering, "{close:?}");
            m.update(close.clone());
            assert_eq!((m.sidebar, m.app_lists.filtering, m.field()), (None, false, None), "{close:?}");
        }
    }

    /// `g` typed in a viewer, then the sidebar opened: its list has the keyboard, the count goes.
    #[test]
    fn a_viewers_count_goes_with_the_keyboard_to_the_sidebar() {
        let mut m = workspace(true);
        let at = None;
        m.update(Action::OpenFile { path: "/w/main.rs".into(), axis: Axis::Right, beside: None, reuse: false, at });
        m.update(Action::ViewerKey { key: "g".into() });
        assert_eq!(m.viewer_pending().as_deref(), Some("g"));
        let pane = m.keyboard_viewer();
        m.update(Action::ToggleSection { section: Section::Marks });
        assert!(matches!(m.focus_target(), Target::List(_)));
        assert_eq!(pane.and_then(|id| m.viewer(id)).and_then(|v| v.pending.pending()), None);
    }

    /// The user's ⌘N floats the box centred over the panes; ↵ makes the task the drawer's. A client's
    /// box, one reopened with ⌘⇧R, one with nothing open, and the flag off: in the drawer, as before.
    #[test]
    fn the_users_box_is_centred_and_its_enter_leaves_the_drawer_on_the_task() {
        let mut m = workspace(true);
        m.update(Action::TalkToggle);
        m.update(Action::OpenCompose { text: "two".into() });
        assert!(m.compose_centred() && m.compose_host() == Some(super::super::Host::Drawer));
        m.update(Action::ComposeEnter);
        let two = m.tasks.list.last().map(|task| task.id);
        assert!(!m.compose_centred() && m.compose.is_none() && !m.drawer_collapsed());
        assert_eq!(m.talk.as_ref().and_then(|talk| talk.task.task()), two);
        // ⌃G: the editor has its line in the drawer; back from it, centred again.
        m.update(Action::OpenCompose { text: "edit".into() });
        m.update(Action::EditorOpen);
        assert!(m.compose.is_some() && !m.compose_centred());
        m.update(Action::EditorDone { saved: None, more: false }.sent_by(Origin::System));
        assert!(m.compose_centred());
        m.update(Action::ComposeEscape);
        // ⇧↵ and Esc close it too: nothing centred stays.
        for close in [Action::ComposeSubmit { open: false }, Action::ComposeEscape] {
            m.update(Action::OpenCompose { text: "three".into() });
            assert!(m.compose_centred(), "{close:?}");
            m.update(close.clone());
            assert!(!m.compose_centred() && m.compose.is_none(), "{close:?}");
        }
        // Folded by a click outside, ⌘⇧R reopens it in the drawer.
        m.update(Action::OpenCompose { text: "four".into() });
        m.update(Action::OverlayDismiss);
        assert!(m.compose.is_none() && m.drawer_collapsed());
        m.update(Action::TalkToggle);
        assert!(m.compose.is_some() && !m.compose_centred());
        m.update(Action::ComposeEscape);
        // ⌘⇧R twice: folded, then back in the drawer.
        m.update(Action::OpenCompose { text: "five".into() });
        m.update(Action::TalkToggle);
        assert!(m.compose.is_none() && m.drawer_collapsed());
        m.update(Action::TalkToggle);
        assert!(m.compose.is_some() && !m.compose_centred() && !m.drawer_collapsed());
        m.update(Action::ComposeEscape);
        // A client's box, where the drawer is open: in the drawer.
        let mut m = workspace(true);
        m.update(Action::OpenCompose { text: "theirs".into() }.sent_by(Origin::Socket));
        assert!(m.compose.is_some() && !m.compose_centred());
        m.update(Action::ComposeEscape);
        // The flag off: in the drawer.
        let mut m = workspace(false);
        m.update(Action::OpenCompose { text: "two".into() });
        assert!(m.compose.is_some() && !m.compose_centred());
        // Nothing open at all: the box is the drawer's own.
        let mut m = Model::default();
        m.update(Action::SetFeatures { features: Features::all() }.sent_by(Origin::System));
        m.update(Action::OpenCompose { text: String::new() });
        assert!(m.welcome() && m.compose.is_some() && !m.compose_centred());
    }

    #[test]
    fn no_client_or_timer_opens_either_or_dismisses_them() {
        let mut m = workspace(true);
        m.update(Action::TalkToggle);
        assert!(m.drawer_collapsed());
        for origin in [Origin::Socket, Origin::System] {
            for theirs in [
                Action::ToggleSection { section: Section::Repos },
                Action::FocusList,
                Action::OpenCompose { text: String::new() },
                Action::TaskTalk { id: 1 },
                Action::RailSelect { item: rail::RailItem::Task { id: 1 } },
            ] {
                m.update(theirs.clone().sent_by(origin));
                assert!(m.sidebar.is_none() && m.drawer_collapsed(), "{theirs:?} {origin:?}");
            }
            assert_eq!(m.refusal(&Action::ToggleSection { section: Section::Repos }, origin), Some(OVERLAY_USERS));
        }
        // Nor dismisses the user's: its click outside, its toggle of the open section, its close.
        m.update(Action::ToggleSection { section: Section::Repos });
        m.update(Action::TalkToggle);
        for theirs in [Action::OverlayDismiss, Action::ToggleSection { section: Section::Repos }, Action::CloseSidebar]
        {
            m.update(theirs.clone().sent_by(Origin::Socket));
            assert!(m.sidebar.is_some() && !m.drawer_collapsed(), "{theirs:?}");
        }
        // A needs-you change of the task in front, or its agent's news: the user's drawer stays as left.
        m.update(Action::TalkToggle);
        m.update(Action::TaskSetState { id: 1, state: crate::tasks::State::NeedsYou }.sent_by(Origin::System));
        assert!(m.drawer_collapsed());
    }
}
