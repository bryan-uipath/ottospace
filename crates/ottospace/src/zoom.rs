//! Zoom-out in the shell (`zoom_out`, `ottospace_core::model::zoom`): the space's tabs as tiles
//! over the panes area, each a live miniature of its tab, and the zoom between a tile and the whole
//! area. The model has moved before the zoom plays: on the way in it is drawn only, and takes no
//! click. Design: `docs/ai/zoom-out.md`.

use gpui::{
    AnyElement, Bounds, BoxShadow, Context, Div, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels,
    Point, ScrollDelta, ScrollWheelEvent, Window, actions, black, canvas, div, prelude::*, px, rgb, surface,
};
use ottospace_core::model::canvas::{Camera, CanvasStep, KEY_STEP, in_view};
use ottospace_core::model::zoom::{ZOOM_MS, between};
use ottospace_core::{Action, Dir, Kind, Pane, Rect, theme};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::overlays::{Inert, Slide};
use crate::{PANE_HEADER, Workspace, cut_text, terminal};

// ⌘⇧\: every tab at once, or back in. Zoomed out, ⌘= ⌘- ⌘0: nearer, farther, every tile.
actions!(ottospace, [ZoomToggle, CanvasNearer, CanvasFarther, CanvasFit]);

/// A tile at least this wide (points) shows its terminals' text; at least `BLOCKS`, their rows as
/// bars; narrower, the tab's name alone. Far out nothing is shaped.
const TEXT: f32 = 360.;
const BLOCKS: f32 = 120.;
/// A mouse wheel's line, and a trackpad's point with ⌘: how much nearer.
const PER_LINE: f32 = 1.12;
const PER_POINT: f32 = 1.006;

/// The zoom's state between frames, and what it drew.
#[derive(Default)]
pub(crate) struct Zooming {
    /// How far out it is: a side overlay's slide, `ZOOM_MS` long.
    slide: Slide,
    /// The panes area as last drawn, in window points: the tiles are laid out in it.
    pub(crate) area: Rc<Cell<Bounds<Pixels>>>,
    /// Each tile as drawn this frame, by tab id: the state's `zoom_tiles`.
    pub(crate) tiles: Rc<RefCell<BTreeMap<u32, Bounds<Pixels>>>>,
    /// Frames the zoom asked for (`counts.zoom_frames`): none at rest, none under Reduce Motion.
    pub(crate) frames: u64,
    /// The camera last drawn: the zoom back into a tab plays from it.
    camera: Option<Camera>,
    /// A drag of the empty canvas: where the pointer was last.
    drag: Option<Point<Pixels>>,
}

/// The room around a tile's label, above it.
const LABEL: f32 = 20.;

impl Workspace {
    /// Over the panes area: the overview while zoomed out, or the zoom back into a tab, drawn only.
    /// `true` with it: the overview covers the area at rest, so the tab under it is not drawn.
    pub(crate) fn render_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) -> (Option<AnyElement>, bool) {
        self.zooming.tiles.borrow_mut().clear();
        let shown = self.model.zoom.is_some();
        // How far out (0 the tab, 1 the grid); a toggle mid-way turns it, Reduce Motion cuts.
        let (t, moving) = self.zooming.slide.step(shown, crate::dial::calm(), ZOOM_MS);
        if moving {
            self.zooming.frames += 1;
            window.request_animation_frame();
        }
        if !shown && !moving {
            return (None, false);
        }
        let area = self.zooming.area.get();
        let whole = Rect { x: 0., y: 0., w: area.size.width.into(), h: area.size.height.into() };
        let spots = self.model.zoom_spots();
        // The tile that grows or shrinks: the tab it opened on, or the one gone into.
        let main = self.model.tabs[self.model.main_at()].id;
        let grows = self.model.zoom.as_ref().map_or(main, |zoom| zoom.from);
        let camera = self.model.zoom.as_ref().map(|zoom| zoom.camera).or(self.zooming.camera);
        let Some(camera) = camera else { return (None, false) };
        self.zooming.camera = Some(camera);
        let tiles: Vec<AnyElement> = spots
            .iter()
            .enumerate()
            .filter_map(|(place, (id, spot))| {
                // On screen, in the area's points; off it, not drawn.
                let f = camera.on_screen(*spot);
                let cell = Rect { x: f.x * whole.w, y: f.y * whole.h, w: f.w * whole.w, h: f.h * whole.h };
                if !in_view(f) && *id != grows {
                    return None;
                }
                let rect = if *id == grows { between(whole, cell, t) } else { cell };
                let alpha = if *id == grows { 1. } else { t };
                // Where it is drawn, its border included: a probe inside it would sit inside the border.
                let at = area.origin + gpui::point(px(rect.x), px(rect.y));
                self.zooming.tiles.borrow_mut().insert(*id, Bounds::new(at, gpui::size(px(rect.w), px(rect.h))));
                Some(self.render_tile(place, *id, rect, whole, (alpha, t), cx))
            })
            .collect();
        let backdrop = gpui::Hsla::from(rgb(theme::BACKGROUND)).opacity(t);
        let layer = div()
            .id("zoom-canvas")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(backdrop)
            .children(tiles)
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, window, cx| this.canvas_wheel(e, window, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    // The press is the canvas's: nothing under it takes it.
                    cx.stop_propagation();
                    this.zooming.drag = Some(e.position);
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| this.canvas_drag(e, window, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.zooming.drag = None));
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
        // Narrow: the tab's name alone, nothing of its panes.
        let panes = tree.filter(|_| rect.w >= BLOCKS).map(|tree| tree.rects(whole)).unwrap_or_default();
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
            .when(rect.w < BLOCKS, |tile| {
                let name = div().size_full().flex().items_center().justify_center().px_1().text_xs();
                tile.child(name.text_color(rgb(theme::MUTED)).child(cut_text(name_of(place, &title))))
            })
            // Out on another display: its window shows it; the tile says so.
            .when(out, |tile| tile.child(div().absolute().inset_0().bg(black().opacity(0.45))))
            .child(self.controls.mark(format!("zoom-tile:{id}")));
        // Its place on the tab row and its name, above it; where it opened, in the accent.
        let words = name_of(place, &title);
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
            // A small tile: its rows as bars, nothing shaped.
            let blocks = scale * f32::from(self.zooming.area.get().size.width) < TEXT;
            let paint = move |bounds, _, window: &mut Window, cx: &mut gpui::App| match blocks {
                true => {
                    terminal::paint_blocks(&screen, bounds, gpui::size(cell.width * scale, cell.height * scale), window)
                }
                false => terminal::paint_scaled(&screen, bounds, (cell, scale), false, &[], window, cx),
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

    /// The wheel over the canvas: a mouse's lines and a trackpad's scroll with ⌘ zoom about the
    /// pointer; a trackpad's scroll pans, the canvas following the fingers (GPUI 0.2.2 has no pinch).
    fn canvas_wheel(&mut self, e: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let area = self.zooming.area.get();
        let (w, h) = (f32::from(area.size.width), f32::from(area.size.height));
        if w <= 0. || h <= 0. {
            return;
        }
        let at = e.position - area.origin;
        let about = [f32::from(at.x) / w, f32::from(at.y) / h];
        let step = match e.delta {
            ScrollDelta::Lines(d) => CanvasStep::Zoom { factor: PER_LINE.powf(d.y), about },
            ScrollDelta::Pixels(d) if e.modifiers.platform => {
                CanvasStep::Zoom { factor: PER_POINT.powf(f32::from(d.y)), about }
            }
            ScrollDelta::Pixels(d) => CanvasStep::Pan { dx: -f32::from(d.x) / w, dy: -f32::from(d.y) / h },
        };
        cx.stop_propagation();
        self.dispatch(Action::ZoomCamera { step }, self.input, window, cx);
    }

    /// A drag of the empty canvas pans it: the canvas follows the pointer.
    fn canvas_drag(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(last) = self.zooming.drag.filter(|_| e.pressed_button == Some(MouseButton::Left)) else {
            self.zooming.drag = None;
            return;
        };
        let area = self.zooming.area.get();
        let (w, h) = (f32::from(area.size.width), f32::from(area.size.height));
        let d = e.position - last;
        self.zooming.drag = Some(e.position);
        cx.stop_propagation();
        if w > 0. && h > 0. {
            let step = CanvasStep::Pan { dx: -f32::from(d.x) / w, dy: -f32::from(d.y) / h };
            self.dispatch(Action::ZoomCamera { step }, self.input, window, cx);
        }
    }

    /// ⌘= ⌘- ⌘0 while zoomed out: nearer, farther (about the middle), every tile. Else not ours.
    pub(crate) fn canvas_key(&mut self, step: CanvasStep, window: &mut Window, cx: &mut Context<Self>) {
        match self.model.zoom.is_some() {
            true => drop(self.dispatch(Action::ZoomCamera { step }, self.input, window, cx)),
            false => cx.propagate(),
        }
    }

    /// Zoomed out, the user's plain keys: arrows and H J K L select, ↵ goes in, Esc goes back. No
    /// other key reaches the tab under it (⌃C, ⌥B: nothing); a ⌘ shortcut runs its binding.
    pub(crate) fn zoom_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        let modified = key.modifiers.platform || key.modifiers.control || key.modifiers.alt;
        let action = match key.key.as_str() {
            _ if modified => None,
            // Held, it goes in or back once; an arrow held moves on.
            "escape" | "enter" if event.is_held => None,
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
            // A held ↵ or Esc: its repeats are not for the tab it went into.
            self.zoom_spent = self.model.zoom.is_none();
        }
    }
}

/// A tile's words: its place on the tab row, and its name.
fn name_of(place: usize, title: &str) -> String {
    match title.is_empty() {
        true => format!("{}", place + 1),
        false => format!("{} · {title}", place + 1),
    }
}

/// ⌘= and ⌘-: one key step nearer or farther, about the area's middle.
pub(crate) fn key_step(nearer: bool) -> CanvasStep {
    CanvasStep::Zoom { factor: if nearer { KEY_STEP } else { KEY_STEP.recip() }, about: [0.5, 0.5] }
}
