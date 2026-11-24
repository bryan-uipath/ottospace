//! Zoom-out (`zoom_out`): every tab of the space on show as a tile on a grid, with room between.
//! The user's alone: ⌘⇧\ opens it and goes back, the arrows select, ↵ or a click goes in. It adds,
//! closes and moves nothing: it is a view. Design: `docs/ai/zoom-out.md`.

use serde::Serialize;

use super::canvas::{self, Camera, CanvasStep};
use super::{Effect, Kind, Model, Owner, PaneId, spaces::HOME_TAB};
use crate::features::Feature;
use crate::layout::{Dir, Rect};

/// Why a client's zoom is refused: the overview covers the user's work and takes the keyboard.
pub const USERS_ALONE: &str = "only the user zooms out or in";
/// Why a client's step that would take the keyboard waits while the user is zoomed out.
pub const ZOOMED_OUT: &str = "the user is zoomed out";
/// ⌘⇧\ on Home.
pub const HOME_NONE: &str = "Home has no tabs to zoom out to";

/// How long the zoom lasts, in milliseconds (`overlay::slide`'s curve). The model has moved on
/// before it starts.
pub const ZOOM_MS: f32 = 220.;

/// The overview while it shows. In the state: the tab ids and the camera.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Zoom {
    /// The tab on show when it opened: where ⌘⇧\ and Esc go back to.
    pub from: u32,
    /// The tile the keys are on.
    pub selected: u32,
    /// Where the canvas is looked at from (`canvas`): kept per space when it closes.
    pub camera: Camera,
    /// The app pane that had the keyboard when it opened: it has it again on the way back.
    #[serde(skip)]
    pub(crate) live: Option<PaneId>,
}

/// Columns and rows for `n` tiles: the least square that holds them, the last row maybe short.
/// e.g. 1 → 1×1, 4 → 2×2, 7 → 3 columns of 3, 3, 1.
pub(crate) fn grid(n: usize) -> (usize, usize) {
    let cols = (1..=n).find(|c| c * c >= n).unwrap_or(0);
    (cols, if cols == 0 { 0 } else { n.div_ceil(cols) })
}

/// `a` moved `t` of the way to `b`.
pub fn between(a: Rect, b: Rect, t: f32) -> Rect {
    let at = |p: f32, q: f32| p + (q - p) * t;
    Rect { x: at(a.x, b.x), y: at(a.y, b.y), w: at(a.w, b.w), h: at(a.h, b.h) }
}

impl Model {
    /// The tiles, by tab id in tab order: the tabs of the main window's space, not Home's own.
    pub fn zoom_tabs(&self) -> Vec<u32> {
        let tabs = self.space_tabs(self.shown_space()).into_iter().map(|at| self.tabs[at].id);
        tabs.filter(|id| *id != HOME_TAB).collect()
    }

    /// Each tile's place on the canvas, by tab id, in tab order: where the user placed it, else
    /// the next free grid cell.
    pub fn zoom_spots(&self) -> Vec<(u32, Rect)> {
        let tabs = self.zoom_tabs();
        let placed: Vec<Option<[f32; 2]>> = tabs.iter().map(|id| self.placed.get(id).copied()).collect();
        tabs.into_iter().zip(canvas::spots(&placed)).collect()
    }

    /// Whether any tile of the space on show is placed by hand: Tidy has something to do.
    pub fn zoom_placed(&self) -> bool {
        self.zoom_tabs().iter().any(|id| self.placed.contains_key(id))
    }

    /// The user's drag put tile `tab` with its corner at `at` (snapped), zoomed out.
    pub(super) fn zoom_place(&mut self, tab: u32, at: [f32; 2]) {
        let Some(at) = canvas::snapped(at).filter(|_| self.zoom.is_some() && self.zoom_tabs().contains(&tab)) else {
            return;
        };
        self.placed.insert(tab, at);
        self.zoom_keep();
    }

    /// Tidy: every tile of the space on show back on the grid.
    pub(super) fn zoom_tidy(&mut self) {
        if self.zoom.is_none() {
            return;
        }
        for id in self.zoom_tabs() {
            self.placed.remove(&id);
        }
        self.zoom_keep();
    }

    /// The camera kept inside the tiles' new bounds, for the space.
    fn zoom_keep(&mut self) {
        let (tiles, of) = (self.zoom_tiles(), self.shown_space().clone());
        if let Some(zoom) = self.zoom.as_mut() {
            zoom.camera = zoom.camera.clamped(&tiles);
            self.canvases.insert(of, zoom.camera);
        }
    }

    /// Every tile's place on the canvas.
    fn zoom_tiles(&self) -> Vec<Rect> {
        self.zoom_spots().into_iter().map(|(_, r)| r).collect()
    }

    /// The camera of the space on show: as the user left it, else every tile fitted.
    fn kept_camera(&self) -> Camera {
        let tiles = self.zoom_tiles();
        let kept = self.canvases.get(self.shown_space());
        kept.map_or_else(|| Camera::fit(canvas::extent(&tiles)), |cam| cam.clamped(&tiles))
    }

    /// The camera zoomed, panned or fitted by the user, and kept for the space.
    pub(super) fn zoom_camera(&mut self, step: CanvasStep) {
        let extent = self.zoom_tiles();
        let of = self.shown_space().clone();
        let Some(zoom) = self.zoom.as_mut() else { return };
        zoom.camera = zoom.camera.stepped(step, &extent);
        self.canvases.insert(of, zoom.camera);
    }

    /// ⌘⇧\: the overview of the tab on show's space, or back to where it opened.
    pub(super) fn zoom_toggle(&mut self, effects: &mut Vec<Effect>) {
        if self.zoom.is_some() {
            return self.zoom_back(effects);
        }
        // The keyboard in a tab out's window: the overview is the main window's.
        if self.away.is_some() {
            return;
        }
        let from = self.tabs[self.main_at()].id;
        if *self.shown_space() == Owner::Home || !self.zoom_tabs().contains(&from) {
            effects.push(Effect::Notice { text: HOME_NONE.into() });
            return;
        }
        let live = self.live;
        // A live window would float over the tiles: parked, its picture is in its tile.
        self.leave(effects);
        // Where it was left, with the tab it opens on in view.
        let spot = self.zoom_spots().into_iter().find(|(id, _)| *id == from).map(|(_, r)| r);
        let camera = self.kept_camera();
        let camera = spot.map_or(camera, |spot| camera.showing(spot, &self.zoom_tiles()));
        self.canvases.insert(self.shown_space().clone(), camera);
        self.zoom = Some(Zoom { from, selected: from, camera, live });
    }

    /// Back to the tab it opened on; the app that had the keyboard has it again.
    fn zoom_back(&mut self, effects: &mut Vec<Effect>) {
        let Some(zoom) = self.zoom.take() else { return };
        if let Some(at) = self.tabs.iter().position(|tab| tab.id == zoom.from) {
            self.select_tab(at, effects);
        }
        let app = |id: &PaneId| self.pane(*id).is_some_and(|p| matches!(p.kind, Kind::App { .. }));
        let shown = self.main_panes();
        if let Some(pane) = zoom.live.filter(|id| app(id) && shown.contains(id) && self.away.is_none()) {
            self.live = Some(pane);
            self.focused = Some(pane);
            effects.push(Effect::Live(pane));
        }
    }

    /// The selection one tile `dir`: the nearest tile that way; it stops at the canvas's edge.
    pub(super) fn zoom_move(&mut self, dir: Dir) {
        let (tabs, spots) = (self.zoom_tabs(), self.zoom_spots());
        let tiles: Vec<Rect> = spots.iter().map(|(_, r)| *r).collect();
        let of = self.shown_space().clone();
        let Some(zoom) = self.zoom.as_mut() else { return };
        let at = tabs.iter().position(|id| *id == zoom.selected).unwrap_or(0);
        let Some(to) = canvas::neighbour(&tiles, at, dir) else { return };
        zoom.selected = tabs[to];
        // The camera follows the selection into view.
        zoom.camera = zoom.camera.showing(tiles[to], &tiles);
        self.canvases.insert(of, zoom.camera);
    }

    /// Into `tab`, or the selected tile: the tab it opened on is the way back; any other is
    /// selected as ⌘1 would.
    pub(super) fn zoom_enter(&mut self, tab: Option<u32>, effects: &mut Vec<Effect>) {
        let Some(zoom) = &self.zoom else { return };
        let tab = tab.unwrap_or(zoom.selected);
        if !self.zoom_tabs().contains(&tab) {
            return;
        }
        if tab == zoom.from {
            return self.zoom_back(effects);
        }
        self.zoom = None;
        if let Some(at) = self.tabs.iter().position(|t| t.id == tab) {
            self.select_tab(at, effects);
        }
    }

    /// After every step: it closes with its flag, when no tile or the tab it came from is left,
    /// or once an app is live or the keyboard is in a tab out's window. A tile gone: the selection
    /// goes back to where it opened.
    pub(super) fn settle_zoom(&mut self) {
        // A tab gone takes its place with it.
        if !self.placed.is_empty() {
            let tabs: Vec<u32> = self.tabs.iter().map(|tab| tab.id).collect();
            self.placed.retain(|id, _| tabs.contains(id));
        }
        if self.zoom.is_none() {
            return;
        }
        let tabs = self.zoom_tabs();
        let on = self.features.on(Feature::ZoomOut);
        // A tile added or gone: the camera stays inside the new bounds.
        let (tiles, spots) = (self.zoom_tiles(), self.zoom_spots());
        let Some(zoom) = self.zoom.as_mut() else { return };
        zoom.camera = zoom.camera.clamped(&tiles);
        if !on || !tabs.contains(&zoom.from) || self.live.is_some() || self.away.is_some() {
            self.zoom = None;
            return;
        }
        if !tabs.contains(&zoom.selected) {
            // Its tile gone: back to where it opened, in view.
            zoom.selected = zoom.from;
            if let Some((_, spot)) = spots.iter().find(|(id, _)| *id == zoom.from) {
                zoom.camera = zoom.camera.showing(*spot, &tiles);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Axis;
    use crate::features::Features;
    use crate::model::{Action, Origin};

    fn app(window: u32) -> Kind {
        Kind::App { window, title: format!("w{window}"), bundle: None, name: "app".into() }
    }

    /// `n` tabs of one terminal each, the first on show.
    fn tabs(n: usize) -> Model {
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        for _ in 1..n {
            m.update(Action::NewTab);
            m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        }
        m.update(Action::SelectVisibleTab { index: 0 });
        assert_eq!(m.zoom_tabs().len(), n);
        m
    }

    #[test]
    fn the_grid_is_the_least_square() {
        assert_eq!(
            [0, 1, 2, 4, 5, 7, 9, 10].map(grid),
            [(0, 0), (1, 1), (2, 1), (2, 2), (3, 2), (3, 3), (3, 3), (4, 3)]
        );
    }

    #[test]
    fn a_tile_moves_between_its_cell_and_the_whole_area() {
        let (a, b) = (Rect { x: 0., y: 0., w: 100., h: 50. }, Rect { x: 10., y: 20., w: 30., h: 10. });
        assert_eq!((between(a, b, 0.), between(a, b, 1.)), (a, b));
    }

    #[test]
    fn it_opens_on_the_tab_on_show_and_goes_back_there() {
        let mut m = tabs(3);
        let ids = m.zoom_tabs();
        m.update(Action::ZoomOut);
        assert_eq!(m.zoom.as_ref().map(|z| (z.from, z.selected)), Some((ids[0], ids[0])));
        let selected = |m: &Model| m.zoom.as_ref().map(|z| z.selected);
        m.update(Action::ZoomMove { dir: Dir::Right });
        m.update(Action::ZoomMove { dir: Dir::Down });
        assert_eq!(selected(&m), Some(ids[1]), "3 tiles: 2 columns, none under the second");
        m.update(Action::ZoomMove { dir: Dir::Left });
        m.update(Action::ZoomMove { dir: Dir::Down });
        assert_eq!(selected(&m), Some(ids[2]));
        m.update(Action::ZoomOut);
        assert_eq!((m.zoom.is_none(), m.tabs[m.active].id), (true, ids[0]));
        // ↵ goes into the selected tile; a click into its own.
        m.update(Action::ZoomOut);
        m.update(Action::ZoomMove { dir: Dir::Right });
        m.update(Action::ZoomEnter { tab: None });
        assert_eq!((m.zoom.is_none(), m.tabs[m.active].id), (true, ids[1]));
        m.update(Action::ZoomOut);
        m.update(Action::ZoomEnter { tab: Some(ids[2]) });
        assert_eq!(m.tabs[m.active].id, ids[2]);
    }

    #[test]
    fn the_selection_stops_at_the_edges() {
        let mut m = tabs(7);
        let ids = m.zoom_tabs();
        m.update(Action::ZoomOut);
        let selected = |m: &Model| m.zoom.as_ref().map(|z| z.selected);
        for dir in [Dir::Left, Dir::Up] {
            m.update(Action::ZoomMove { dir });
            assert_eq!(selected(&m), Some(ids[0]), "{dir:?}");
        }
        for _ in 0..5 {
            m.update(Action::ZoomMove { dir: Dir::Down });
        }
        assert_eq!(selected(&m), Some(ids[6]), "the short row's first");
        m.update(Action::ZoomMove { dir: Dir::Right });
        assert_eq!(selected(&m), Some(ids[6]), "nothing right of it");
        m.update(Action::ZoomMove { dir: Dir::Up });
        m.update(Action::ZoomMove { dir: Dir::Right });
        m.update(Action::ZoomMove { dir: Dir::Down });
        assert_eq!(selected(&m), Some(ids[4]), "nothing under the second column's last");
    }

    #[test]
    fn the_live_app_is_parked_and_live_again_on_the_way_back() {
        let mut m = tabs(2);
        m.update(Action::Open { kind: app(7), axis: Axis::Right, beside: None });
        let pane = m.focused.unwrap_or_default();
        m.update(Action::Enter { pane });
        assert_eq!(m.update(Action::ZoomOut), [Effect::Parked(pane)]);
        assert!(m.live.is_none() && m.zoom.is_some());
        assert_eq!(m.update(Action::ZoomOut), [Effect::Live(pane)]);
        assert_eq!(m.live, Some(pane));
        // Esc's way back and ↵ on the same tile are one.
        m.update(Action::ZoomOut);
        assert_eq!(m.update(Action::ZoomEnter { tab: None }), [Effect::Live(pane)]);
        // Into another tab: no app is live there.
        let other = m.zoom_tabs()[1];
        m.update(Action::ZoomOut);
        m.update(Action::ZoomEnter { tab: Some(other) });
        assert!(m.live.is_none());
        assert_eq!(m.tabs[m.active].id, other);
    }

    #[test]
    fn only_the_user_zooms_and_a_client_takes_no_keyboard_under_it() {
        let mut m = tabs(2);
        for origin in [Origin::Socket, Origin::System] {
            for theirs in [Action::ZoomOut, Action::ZoomMove { dir: Dir::Right }, Action::ZoomEnter { tab: None }] {
                assert_eq!(m.refusal(&theirs, origin), Some(USERS_ALONE), "{theirs:?} {origin:?}");
                m.update(theirs.sent_by(origin));
                assert!(m.zoom.is_none());
            }
        }
        m.update(Action::ZoomOut);
        let picker = Action::OpenPicker { axis: Axis::Right, new_tab: false, items: Vec::new() };
        assert_eq!(m.refusal(&picker, Origin::Socket), Some(ZOOMED_OUT));
        m.update(Action::ZoomOut.sent_by(Origin::Socket));
        assert!(m.zoom.is_some(), "nor closes the user's");
        // No terminal has the keyboard under it.
        assert_eq!(m.focus_terminal(true), None);
        // A client's split runs underneath: its tile redraws.
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal }.sent_by(Origin::Socket));
        assert!(m.zoom.is_some());
    }

    #[test]
    fn any_other_move_of_the_users_closes_it_and_sneak_does_not() {
        let mut m = tabs(2);
        m.update(Action::ZoomOut);
        m.update(Action::Sneak { targets: Vec::new() });
        assert!(m.zoom.is_some(), "sneak's own step");
        let shown = m.tabs[m.active].id;
        m.update(Action::NewTab);
        assert!(m.zoom.is_none());
        assert_ne!(m.tabs[m.active].id, shown, "the move did what it does");
    }

    #[test]
    fn home_has_none_and_a_tab_gone_moves_the_selection_or_closes_it() {
        let mut m = Model::default();
        assert_eq!(m.update(Action::ZoomOut), [Effect::Notice { text: HOME_NONE.into() }]);
        assert!(m.zoom.is_none());
        let mut m = tabs(3);
        let ids = m.zoom_tabs();
        m.update(Action::ZoomOut);
        m.update(Action::ZoomEnter { tab: Some(ids[1]) });
        m.update(Action::ZoomOut);
        m.update(Action::ZoomMove { dir: Dir::Right });
        let at = |m: &Model, id: u32| m.tabs.iter().position(|t| t.id == id);
        m.update(Action::CloseTab { index: at(&m, ids[0]) }.sent_by(Origin::Socket));
        assert_eq!(m.zoom.as_ref().map(|z| (z.from, z.selected)), Some((ids[1], ids[1])));
        m.update(Action::CloseTab { index: at(&m, ids[1]) }.sent_by(Origin::Socket));
        assert!(m.zoom.is_none(), "the tab it came from is gone");
    }

    #[test]
    fn off_it_does_nothing_and_turning_it_off_closes_it() {
        let mut m = tabs(2);
        m.update(
            Action::SetFeatures { features: Features::all().with(Feature::ZoomOut, false) }.sent_by(Origin::System),
        );
        m.update(Action::ZoomOut);
        assert!(m.zoom.is_none());
        m.update(Action::SetFeatures { features: Features::all() }.sent_by(Origin::System));
        m.update(Action::ZoomOut);
        m.update(
            Action::SetFeatures { features: Features::all().with(Feature::ZoomOut, false) }.sent_by(Origin::System),
        );
        assert!(m.zoom.is_none());
    }

    #[test]
    fn it_opens_fitted_and_the_camera_is_kept_per_space() {
        let mut m = tabs(4);
        m.update(Action::ZoomOut);
        let fitted = m.zoom.as_ref().map(|z| z.camera);
        assert_eq!(fitted, Some(Camera::fit(canvas::extent(&m.zoom_tiles()))));
        let step = CanvasStep::Zoom { factor: 0.5, about: [0.5, 0.5] };
        m.update(Action::ZoomCamera { step });
        let farther = m.zoom.as_ref().map(|z| z.camera);
        assert!(farther.is_some_and(|c| fitted.is_some_and(|f| c.scale < f.scale)), "{farther:?}");
        // Back in and out again: where it was left.
        m.update(Action::ZoomOut);
        m.update(Action::ZoomOut);
        assert_eq!(m.zoom.as_ref().map(|z| z.camera), farther);
        // ⌘0: fitted again.
        m.update(Action::ZoomCamera { step: CanvasStep::Fit });
        assert_eq!(m.zoom.as_ref().map(|z| z.camera), fitted);
        // Closed, a step does nothing; a client's is refused.
        m.update(Action::ZoomOut);
        m.update(Action::ZoomCamera { step });
        assert!(m.zoom.is_none());
        let theirs = Action::ZoomCamera { step: CanvasStep::Pan { dx: 1., dy: 0. } };
        assert_eq!(m.refusal(&theirs, Origin::Socket), Some(USERS_ALONE));
    }

    #[test]
    fn the_selection_brings_its_tile_into_view() {
        let mut m = tabs(9);
        m.update(Action::ZoomOut);
        // Near the first tile: the last is out of view.
        m.update(Action::ZoomCamera { step: CanvasStep::Zoom { factor: 4., about: [0., 0.] } });
        for dir in [Dir::Right, Dir::Right, Dir::Down, Dir::Down] {
            m.update(Action::ZoomMove { dir });
        }
        let zoom = m.zoom.clone().expect("zoomed out");
        let spot = m.zoom_spots().into_iter().find(|(id, _)| *id == zoom.selected).map(|(_, r)| r).expect("tile");
        let r = zoom.camera.on_screen(spot);
        assert!(r.x >= -1e-3 && r.y >= -1e-3 && r.x + r.w <= 1.001 && r.y + r.h <= 1.001, "{r:?}");
    }

    /// Reopened from a tab the kept camera does not show: that tile is brought into view.
    #[test]
    fn it_opens_with_the_tab_it_came_from_in_view() {
        let mut m = tabs(9);
        m.update(Action::ZoomOut);
        m.update(Action::ZoomCamera { step: CanvasStep::Zoom { factor: 4., about: [0., 0.] } });
        m.update(Action::ZoomOut);
        m.update(Action::SelectVisibleTab { index: 8 });
        m.update(Action::ZoomOut);
        let zoom = m.zoom.clone().expect("zoomed out");
        let spot = m.zoom_spots().into_iter().find(|(id, _)| *id == zoom.from).map(|(_, r)| r).expect("tile");
        let r = zoom.camera.on_screen(spot);
        assert!(r.x >= -1e-3 && r.y >= -1e-3 && r.x + r.w <= 1.001 && r.y + r.h <= 1.001, "{r:?}");
    }

    #[test]
    fn a_tile_placed_by_the_user_stays_and_tidy_puts_it_back() {
        let mut m = tabs(4);
        let ids = m.zoom_tabs();
        m.update(Action::ZoomPlace { tab: ids[3], at: [-1.3, 0.1] });
        assert!(m.placed.is_empty(), "only zoomed out");
        m.update(Action::ZoomOut);
        let place = Action::ZoomPlace { tab: ids[3], at: [-1.3, 0.1] };
        assert_eq!(m.refusal(&place, Origin::Socket), Some(USERS_ALONE));
        m.update(place);
        let spot = |m: &Model, id: u32| m.zoom_spots().into_iter().find(|(t, _)| *t == id).map(|(_, r)| (r.x, r.y));
        assert_eq!(spot(&m, ids[3]), Some((-1.25, 0.)), "snapped");
        assert!(m.zoom_placed());
        // Kept when it closes and opens again; a new tab takes the next free cell.
        m.update(Action::ZoomOut);
        m.update(Action::NewTab);
        m.update(Action::SelectVisibleTab { index: 0 });
        m.update(Action::ZoomOut);
        assert_eq!(spot(&m, ids[3]), Some((-1.25, 0.)));
        // Not a number: nothing moves. Tidy: back on the grid.
        m.update(Action::ZoomPlace { tab: ids[0], at: [f32::NAN, 0.] });
        assert_eq!(m.placed.len(), 1);
        m.update(Action::ZoomTidy);
        assert!(!m.zoom_placed() && spot(&m, ids[3]) == Some((canvas::grid_spot(3, 5).x, canvas::grid_spot(3, 5).y)));
        // A placed tab closed takes its place with it.
        m.update(Action::ZoomPlace { tab: ids[1], at: [5., 5.] });
        m.update(Action::ZoomOut);
        let at = m.tabs.iter().position(|t| t.id == ids[1]);
        m.update(Action::CloseTab { index: at });
        assert!(m.placed.is_empty());
    }
}
