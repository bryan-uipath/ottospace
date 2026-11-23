//! Zoom-out in the shell (`zoom_out`, `ottospace_core::model::zoom`): the space's tabs as tiles
//! over the panes area, each a live miniature of its tab, and the zoom between a tile and the whole
//! area. The model has moved before the zoom plays: on the way in it is drawn only, and takes no
//! click. Design: `docs/ai/zoom-out.md`.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    AnyElement, Bounds, BoxShadow, Context, Div, KeyDownEvent, MouseButton, Pixels, Window, actions, black, canvas,
    div, prelude::*, px, rgb, surface,
};
use ottospace_core::model::zoom::{ZOOM_MS, between, cells, progress, travel};
use ottospace_core::{Action, Dir, Kind, Pane, Rect, theme};

use crate::{PANE_HEADER, Workspace, cut_text, overlays::Inert, terminal};

// ⌘⇧\: every tab at once, or back in.
actions!(ottospace, [ZoomToggle]);

/// The zoom's state between frames, and what it drew.
#[derive(Default)]
pub(crate) struct Zooming {
    /// How far out (milliseconds in), since when, and whether it goes out.
    from: f32,
    since: Option<Instant>,
    out: bool,
    /// The panes area as last drawn, in window points: the tiles are laid out in it.
    pub(crate) area: Rc<Cell<Bounds<Pixels>>>,
    /// Each tile as drawn this frame, by tab id: the state's `zoom_tiles`.
    pub(crate) tiles: Rc<RefCell<BTreeMap<u32, Bounds<Pixels>>>>,
    /// Frames the zoom asked for (`counts.zoom_frames`): none at rest, none under Reduce Motion.
    pub(crate) frames: u64,
}

impl Zooming {
    /// This frame's way out (0 the tab, 1 the grid), and whether it still moves. A toggle mid-way
    /// turns it from where it is; `calm` (Reduce Motion): a cut.
    fn step(&mut self, shown: bool, calm: bool) -> (f32, bool) {
        let now = Instant::now();
        let at = |z: &Self| z.since.map_or(z.from, |since| travel(z.from, (now - since).as_secs_f32() * 1000., z.out));
        if shown != self.out {
            (self.from, self.since, self.out) = (at(self), Some(now), shown);
        }
        if calm {
            (self.from, self.since) = (if shown { ZOOM_MS } else { 0. }, None);
        }
        let at = at(self);
        (progress(at), if shown { at < ZOOM_MS } else { at > 0. })
    }
}

/// The room around a tile's label, above it.
const LABEL: f32 = 20.;

impl Workspace {
    /// Over the panes area: the overview while zoomed out, or the zoom back into a tab, drawn only.
    /// `true` with it: the overview covers the area at rest, so the tab under it is not drawn.
    pub(crate) fn render_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) -> (Option<AnyElement>, bool) {
        self.zooming.tiles.borrow_mut().clear();
        let shown = self.model.zoom.is_some();
        let (t, moving) = self.zooming.step(shown, crate::dial::calm());
        if moving {
            self.zooming.frames += 1;
            window.request_animation_frame();
        }
        if !shown && !moving {
            return (None, false);
        }
        let area = self.zooming.area.get();
        let whole = Rect { x: 0., y: 0., w: area.size.width.into(), h: area.size.height.into() };
        let tabs = self.model.zoom_tabs();
        // The tile that grows or shrinks: the tab it opened on, or the one gone into.
        let main = self.model.tabs[self.model.main_at()].id;
        let grows = self.model.zoom.as_ref().map_or(main, |zoom| zoom.from);
        let tiles: Vec<AnyElement> = tabs
            .iter()
            .zip(cells(tabs.len(), whole))
            .enumerate()
            .map(|(place, (id, cell))| {
                let rect = if *id == grows { between(whole, cell, t) } else { cell };
                let alpha = if *id == grows { 1. } else { t };
                self.render_tile(place, *id, rect, whole, (alpha, t), cx)
            })
            .collect();
        let backdrop = gpui::Hsla::from(rgb(theme::BACKGROUND)).opacity(t);
        let layer = div().absolute().top_0().left_0().size_full().bg(backdrop).children(tiles);
        let layer = match shown {
            true => layer.occlude().into_any_element(),
            false => Inert(layer.into_any_element()).into_any_element(),
        };
        (Some(layer), shown && !moving)
    }

    /// Notes the panes area each frame; zoomed out, a new size draws again with tiles laid out in it.
    pub(crate) fn zoom_area(&self) -> impl IntoElement + use<> {
        let (area, zoomed) = (self.zooming.area.clone(), self.model.zoom.is_some());
        let note = move |bounds: Bounds<Pixels>, window: &mut Window, _: &mut gpui::App| {
            if area.replace(bounds) != bounds && zoomed {
                window.refresh();
            }
        };
        canvas(note, |_, _, _, _| {}).absolute().top_0().left_0().size_full()
    }

    /// Tab `id`, the `place`th tile, at `rect` in the panes area (`whole`): its panes as they lie,
    /// each a miniature of what it shows. `alpha`: the tile's, then its label's.
    fn render_tile(
        &self,
        place: usize,
        id: u32,
        rect: Rect,
        whole: Rect,
        (alpha, label_alpha): (f32, f32),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let at = self.model.tabs.iter().position(|tab| tab.id == id);
        let tree = at.and_then(|at| self.model.tree_at(at));
        let scale = if whole.w > 0. { rect.w / whole.w } else { 0. };
        let panes = tree.map(|tree| tree.rects(whole)).unwrap_or_default();
        let panes = panes.into_iter().filter_map(|(pane, r)| {
            let pane = self.model.pane(pane)?;
            let r = Rect { x: r.x * scale, y: r.y * scale, w: r.w * scale, h: r.h * scale };
            Some(self.render_mini(pane, r, scale))
        });
        let zoom = self.model.zoom.as_ref();
        let (from, selected) = (zoom.is_some_and(|z| z.from == id), zoom.is_some_and(|z| z.selected == id));
        let out = self.model.is_out(id);
        let title = at.map(|at| self.model.tabs[at].title.clone()).unwrap_or_default();
        let ring = if selected { theme::PRIMARY } else { theme::BORDER_STRONG };
        let probe = {
            let tiles = self.zooming.tiles.clone();
            canvas(move |bounds, _, _| _ = tiles.borrow_mut().insert(id, bounds), |_, _, _, _| {})
                .absolute()
                .top_0()
                .left_0()
                .size_full()
        };
        let tile = div()
            .id(("zoom-tile", id))
            .absolute()
            .left(px(rect.x))
            .top(px(rect.y))
            .w(px(rect.w))
            .h(px(rect.h))
            .opacity(alpha)
            .rounded(px(6. * label_alpha))
            .overflow_hidden()
            .bg(rgb(theme::BACKGROUND))
            .border(px(if selected { 2. } else { 1. }))
            .border_color(rgb(ring))
            // Each tile lifts off the ground on its own shadow.
            .shadow(vec![BoxShadow {
                color: black().opacity(0.6 * label_alpha),
                offset: gpui::point(px(0.), px(10.)),
                blur_radius: px(24.),
                spread_radius: px(0.),
            }])
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.dispatch(Action::ZoomEnter { tab: Some(id) }, this.input, window, cx);
                }),
            )
            .children(panes)
            // Out on another display: its window shows it; the tile says so.
            .when(out, |tile| tile.child(div().absolute().inset_0().bg(black().opacity(0.45))))
            .child(probe)
            .child(self.controls.mark(format!("zoom-tile:{id}")));
        // Its place on the tab row and its name, above it; where it opened, in the accent.
        let words = match title.is_empty() {
            true => format!("{}", place + 1),
            false => format!("{} · {title}", place + 1),
        };
        let words = if out { format!("{words} · out") } else { words };
        let words = if from { format!("{words} · you were here") } else { words };
        let label = div()
            .absolute()
            .left(px(rect.x))
            .top(px(rect.y - LABEL))
            .w(px(rect.w))
            .h(px(LABEL - 4.))
            .flex()
            .items_end()
            .opacity(label_alpha)
            .text_xs()
            .text_color(rgb(if from { theme::PRIMARY } else { theme::MUTED }))
            .child(cut_text(words));
        div().child(label).child(tile).into_any_element()
    }

    /// One pane `rect` in its tile, `scale` of its size: a header with its name, and what it shows.
    /// A terminal's screen, live; an app's last frame (every window not live is parked and
    /// streamed already: nothing new is captured); anything else its name alone.
    fn render_mini(&self, pane: &Pane, rect: Rect, scale: f32) -> Div {
        let id = pane.id;
        let title = match self.pane_title(pane) {
            title if !title.is_empty() => title,
            _ => match &pane.kind {
                Kind::App { name, .. } => name.clone(),
                Kind::Terminal => "Terminal".into(),
                Kind::Empty => "Empty pane".into(),
                _ => String::new(),
            },
        };
        let text = 12. * scale;
        // Too small to read: the header stays, its words go.
        let header = div()
            .flex_none()
            .h(px(PANE_HEADER * scale))
            .px(px(8. * scale))
            .flex()
            .items_center()
            .bg(rgb(theme::RAISED))
            .text_size(px(text))
            .text_color(rgb(theme::MUTED))
            .when(text >= 5., |header| header.child(cut_text(title.clone())));
        let body = div().relative().flex_1().min_h_0().overflow_hidden();
        let body = if let Some(terminal) = self.terminals.get(&id) {
            let (screen, cell) = (terminal.screen(), self.cell);
            let paint = move |bounds, _, window: &mut Window, cx: &mut gpui::App| {
                terminal::paint_scaled(&screen, bounds, (cell, scale), false, &[], window, cx)
            };
            body.child(canvas(|_, _, _| {}, paint).size_full())
        } else if let Some((buffer, _)) = self.hosted.get(&id).and_then(|hosted| hosted.held()) {
            // At its own size at the top left, as the pane draws it.
            let shown = self.hosted.get(&id).map(|hosted| hosted.shown);
            let (w, h) = shown.map_or((0., 0.), |s| (s.width as f32 * scale, s.height as f32 * scale));
            body.child(surface(buffer).absolute().top_0().left_0().w(px(w)).h(px(h)))
        } else {
            body.flex()
                .items_center()
                .justify_center()
                .text_size(px(text))
                .text_color(rgb(theme::MUTED))
                .when(text >= 5., |body| body.child(title))
        };
        let pane = div().size_full().flex().flex_col().border_1().border_color(rgb(theme::BORDER)).child(header);
        div()
            .absolute()
            .left(px(rect.x))
            .top(px(rect.y))
            .w(px(rect.w))
            .h(px(rect.h))
            .p(px(2. * scale))
            .child(pane.child(body))
    }

    /// Zoomed out, the user's plain keys: arrows and H J K L select, ↵ goes in, Esc goes back. No
    /// other key reaches the tab under it. A ⌘ shortcut goes on to its binding.
    pub(crate) fn zoom_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let key = &event.keystroke;
        if key.modifiers.platform || key.modifiers.control || key.modifiers.alt {
            return false;
        }
        let action = match key.key.as_str() {
            "escape" => Some(Action::ZoomOut),
            "enter" => Some(Action::ZoomEnter { tab: None }),
            "left" | "h" => Some(Action::ZoomMove { dir: Dir::Left }),
            "right" | "l" => Some(Action::ZoomMove { dir: Dir::Right }),
            "up" | "k" => Some(Action::ZoomMove { dir: Dir::Up }),
            "down" | "j" => Some(Action::ZoomMove { dir: Dir::Down }),
            _ => None,
        };
        if let Some(action) = action {
            self.dispatch(action, self.input, window, cx);
        }
        true
    }
}
