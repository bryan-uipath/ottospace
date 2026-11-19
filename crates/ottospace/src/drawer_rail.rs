//! The drawer's right rail in the shell: a narrow column at the window's right edge, always
//! shown. On top an item for each task of the workspace and + for a new task here; at the foot,
//! below a divider, the transcript, idle or recording; the drawer collapsed, ← to open it. Which
//! items, their order and which is selected are the core's (`ottospace_core::model::rail`).

use gpui::{AnyElement, AnyView, Context, Div, ElementId, Stateful, Window, canvas, div, prelude::*, px, rgb, svg};
use ottospace_core::listen::Status;
use ottospace_core::model::Owner;
use ottospace_core::model::rail::{RailItem, RailSection};
use ottospace_core::model::zones::RailFocus;
use ottospace_core::talk::Subject;
use ottospace_core::{Action, theme};

use crate::{RAIL_CELL, Tooltip, Workspace, focus_ring, raised_cell, rect};

/// The rail's width, and +'s, in points.
pub(crate) const WIDTH: f32 = 48.;
const ITEM: f32 = 36.;
/// Frames a pending scroll is asked for again before it gives up.
const TRIES: u8 = 4;
/// What the rail last drew: its space, the item keyed and the one selected.
pub(crate) type Drawn = (Owner, Option<RailItem>, Option<RailItem>);

/// The control id of an item: `drawer-rail:transcript`, `drawer-rail:3`.
fn control(item: RailItem) -> String {
    match item {
        RailItem::Transcript => "drawer-rail:transcript".into(),
        RailItem::Task { id } => format!("drawer-rail:{id}"),
    }
}

impl Workspace {
    /// Right of the panes: the drawer's place, `width` wide with its rail at the right edge, always
    /// shown. In the place, what the core says is there: the inspector, a capture's draft, the
    /// transcript, the new-task box, the conversation or a brief. Nothing: the drawer is collapsed,
    /// the rail alone. With the width drawn.
    pub(crate) fn render_right(&self, width: f32, cx: &mut Context<Self>) -> (AnyElement, f32) {
        let inner = (width - WIDTH).max(0.);
        let body = self.render_inspector(inner, cx).or_else(|| self.render_draft(cx));
        let body = body.or_else(|| self.render_listen_view(inner, cx)).or_else(|| self.render_drawer(inner, cx));
        let drawn = if body.is_some() { width } else { WIDTH };
        (div().flex_none().h_full().flex().children(body).child(self.drawer_rail(cx)).into_any_element(), drawn)
    }

    /// The column: the tasks and + scroll; at the foot a divider, then the transcript, always in view.
    fn drawer_rail(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let selected = self.model.rail_selected();
        // Lifted only while the drawer shows it: not collapsed, nor beside the inspector or the box.
        let lifted = self.model.rail_shown();
        let keyed = match self.model.rail_focus {
            Some(RailFocus::Right(item)) => Some(item),
            _ => None,
        };
        let items = self.model.rail();
        let column = div()
            .id("drawer-rail")
            .w(px(WIDTH))
            .flex_none()
            .h_full()
            // The selected item's shadow falls on its neighbours, never over the drawer.
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(rgb(theme::BACKGROUND));
        let item = |item: RailItem| {
            let face = self.rail_face(item)?;
            Some((item, self.rail_item(item, face, lifted == Some(item), keyed == Some(item), cx)))
        };
        let (top, foot): (Vec<RailItem>, Vec<RailItem>) =
            items.iter().partition(|it| it.section() == RailSection::Workspace);
        let (shown, top): (Vec<RailItem>, Vec<Stateful<Div>>) = top.into_iter().filter_map(item).unzip();
        // The item the keys or the selection moved to, scrolled into view once: a wheel may move it out again.
        // A space's own: drawn anew, or another space's, it starts at the top.
        // Keyed and selected apart: the keys entering the rail on the selected item scroll to it too.
        let key = (self.model.space().clone(), keyed, selected);
        let (frame, last) = self.rail_scrolled.replace((self.renders, Some(key.clone())));
        let anew = frame + 1 != self.renders;
        if anew || last.as_ref().map(|(space, ..)| space) != Some(&key.0) {
            self.rail_scroll.set_offset(gpui::point(px(0.), px(0.)));
        }
        let at = shown.iter().position(|it| Some(*it) == keyed.or(selected));
        if anew || last.as_ref() != Some(&key) {
            self.rail_pending.set(at.map(|at| (at, TRIES)));
        } else if let Some((_, tries)) = self.rail_pending.get() {
            // Still pending, the tasks reordered (one now needs you): its place now.
            self.rail_pending.set(at.map(|at| (at, tries)));
        }
        let new = div()
            .id("drawer-rail-new")
            .relative()
            .flex_none()
            .w(px(ITEM))
            .h(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .cursor_pointer()
            .text_color(rgb(theme::MUTED))
            .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
            .child(svg().path("icons/plus.svg").size(px(14.)).text_color(rgb(theme::MUTED)))
            .child(self.controls.mark("drawer-rail:new"))
            .tooltip(|_, cx| AnyView::from(cx.new(|_| Tooltip("New task in this workspace (⌘N)".into()))))
            .on_click(cx.listener(|this, _, window, cx| this.new_task(window, cx)));
        // Cells flush, as the left rail's; the line toward the drawer on all but the selected one.
        let tasks = div()
            .id("drawer-rail-tasks")
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .track_scroll(&self.rail_scroll)
            .children(top)
            .child(edged().flex_grow().pt(px(6.)).child(new));
        let foot = foot.into_iter().filter_map(item).map(|(_, it)| it).collect::<Vec<_>>();
        let divider = div().flex_none().w(px(20.)).h(px(1.)).bg(rgb(theme::BORDER));
        let foot = (!foot.is_empty())
            .then(|| div().flex_none().flex().flex_col().child(edged().py(px(8.)).child(divider)).children(foot));
        let tasks = self.bars.tracked_under("drawer-rail", tasks, &self.rail_scroll, theme::BACKGROUND).w_full();
        // Collapsed, a click on any item opens the drawer on it (the owner: no expand button).
        let column = column.child(tasks).child(self.rail_follow()).children(foot);
        // Its frame starts past the line toward the drawer, as when the line was the column's: the
        // top row's labels keep their spot under their buttons, off the rail.
        let frame = div().absolute().top_0().bottom_0().left(px(1.)).right_0().child(self.controls.mark("drawer-rail"));
        column.child(edged().h_2()).child(frame)
    }

    /// After the tasks are laid out: the pending item wholly in view, else scrolled to and checked
    /// next frame (the first frame's bounds are the last one's). A few tries: one taller than the rail never fits.
    fn rail_follow(&self) -> AnyElement {
        let (pending, handle) = (self.rail_pending.clone(), self.rail_scroll.clone());
        let follow = move |_, window: &mut Window, _: &mut gpui::App| {
            let Some((at, tries)) = pending.get() else { return };
            let (area, offset) = (handle.bounds(), handle.offset().y);
            let shown = handle.bounds_for_item(at).is_some_and(|item| {
                item.top() + offset >= area.top() - px(0.5) && item.bottom() + offset <= area.bottom() + px(0.5)
            });
            if (at == 0 && offset == px(0.)) || (at > 0 && shown) || tries == 0 {
                return pending.set(None);
            }
            // The first to the very top, its padding with it.
            match at {
                0 => handle.set_offset(gpui::point(px(0.), px(0.))),
                _ => handle.scroll_to_item(at),
            }
            pending.set(Some((at, tries - 1)));
            window.request_animation_frame();
        };
        canvas(follow, |_, _, _, _| {}).absolute().size_0().into_any_element()
    }

    /// What an item shows, and its hover: the transcript's waveform, grey while idle, in the
    /// accent while one is on, with a red dot while it records; a task's badge, whose dot is its
    /// state's colour.
    fn rail_face(&self, item: RailItem) -> Option<(AnyElement, gpui::SharedString)> {
        match item {
            RailItem::Transcript => {
                let status = self.model.listen.on.as_ref().map(|on| on.status);
                let recording = status == Some(Status::On);
                let colour = if status.is_some() { theme::PRIMARY } else { theme::MUTED };
                let wave = svg().path("icons/waveform.svg").size(px(20.)).text_color(rgb(colour));
                let dot = div()
                    .absolute()
                    .top(px(3.))
                    .right(px(3.))
                    .size(px(7.))
                    .rounded_full()
                    .bg(rgb(theme::RECORDING))
                    .border_1()
                    .border_color(rgb(theme::BACKGROUND));
                let face = div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(wave)
                    .when(recording, |d| d.child(dot));
                let tip = match status {
                    None => "Transcript: not listening",
                    Some(Status::On) => "Transcript: recording",
                    Some(Status::Paused) => "Transcript: paused",
                    Some(Status::Starting) => "Transcript: starting",
                };
                Some((face.into_any_element(), tip.into()))
            }
            RailItem::Task { id } => {
                let task = self.model.tasks.get(id)?;
                let badge = self.task_badge(id, 22.)?;
                let label = self.model.worktree_label(id);
                let tip = label.map_or(task.title.clone(), |label| format!("{} · {label}", task.title));
                let face = div().size_full().flex().items_center().justify_center().child(badge);
                Some((face.into_any_element(), tip.into()))
            }
        }
    }

    /// One item, a cell as the left rail's: while selected, raised with an accent bar at the window's
    /// edge and no line toward the drawer, joined to it; ringed while the keys are on it.
    fn rail_item(
        &self,
        item: RailItem,
        (face, tip): (AnyElement, gpui::SharedString),
        selected: bool,
        keyed: bool,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        // A task's dim × on hover: Later.
        let group = control(item);
        let later = match item {
            RailItem::Task { id } if self.model.finish(&Subject::Task(id)).is_some_and(|b| b.later) => {
                Some(self.rail_later(id, &group, cx))
            }
            RailItem::Task { .. } => None,
            RailItem::Transcript => None,
        };
        div()
            .id(ElementId::Name(control(item).into()))
            .group(group)
            .relative()
            .flex_none()
            .w_full()
            .h(px(RAIL_CELL))
            .cursor_pointer()
            .when(selected, |d| raised_cell(d, theme::PRIMARY, true))
            .when(!selected, |d| d.border_l_1().border_color(rgb(theme::BORDER)).hover(|s| s.bg(rgb(theme::SELECTED))))
            .when(keyed, focus_ring)
            .child(face)
            .child(self.controls.mark(control(item)))
            .children(later)
            .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone()))))
            .on_click(cx.listener(move |this, _, window, cx| this.rail_click(item, window, cx)))
    }

    /// A click on an item, or its sneak label: shown, as whoever acts (a client's is refused).
    fn rail_click(&mut self, item: RailItem, window: &Window, cx: &mut Context<Self>) {
        if let Some(why) = self.model.refusal(&Action::RailSelect { item }, self.input) {
            self.error = Some(why.into());
            return cx.notify();
        }
        self.dispatch(Action::RailSelect { item }, self.input, window, cx);
    }

    /// A sneak label on the rail: `drawer-rail:transcript`, `drawer-rail:3`, `drawer-rail:new`.
    /// False: not the rail's.
    pub(crate) fn click_drawer_rail(
        &mut self,
        kind: &str,
        name: &str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if kind != "drawer-rail" {
            return false;
        }
        match name {
            "new" => self.new_task(window, cx),
            "transcript" => self.rail_click(RailItem::Transcript, window, cx),
            later if let Some(id) = later.strip_prefix("later:").and_then(|id| id.parse().ok()) => {
                self.later(id, window, cx)
            }
            id => {
                if let Ok(id) = id.parse() {
                    self.rail_click(RailItem::Task { id }, window, cx);
                }
            }
        }
        true
    }

    /// The rail as the state shows it: each item's `kind`, `id` (a task's, else null),
    /// `section` (`workspace` on top, `everywhere` below the divider), `selected` (what the drawer shows: none while collapsed or beside the box) and where it
    /// was drawn (null: not drawn). Empty: no rail. Never a transcript's text.
    pub(crate) fn rail_json(&self) -> serde_json::Value {
        let selected = self.model.rail_shown();
        let row = |item: RailItem| {
            let (kind, id) = match item {
                RailItem::Transcript => ("transcript", None),
                RailItem::Task { id } => ("task", Some(id)),
            };
            let drawn = self.controls.bounds(&control(item)).map(rect);
            let section = item.section();
            serde_json::json!({"kind": kind, "id": id, "section": section, "selected": selected == Some(item), "rect": drawn})
        };
        serde_json::Value::Array(self.model.rail().into_iter().map(row).collect())
    }
}

/// A stretch of the rail with its line toward the drawer, centred: under the cells, the divider, the foot.
fn edged() -> Div {
    div().flex_none().w_full().flex().flex_col().items_center().border_l_1().border_color(rgb(theme::BORDER))
}
