//! The drawer's right rail in the shell: a narrow column at the drawer's right edge, an item for
//! the running transcript and for each task of the workspace, and + for a new task here. Which
//! items, their order and which is selected are the core's (`ottospace_core::model::rail`).

use gpui::{AnyElement, AnyView, Context, Div, ElementId, Stateful, Window, div, prelude::*, px, rgb, svg};
use ottospace_core::listen::Status;
use ottospace_core::model::rail::RailItem;
use ottospace_core::model::zones::RailFocus;
use ottospace_core::{Action, theme};

use crate::{Tooltip, Workspace, rect};

/// The rail's width, and an item's side, in points.
pub(crate) const WIDTH: f32 = 48.;
const ITEM: f32 = 36.;

/// The control id of an item: `drawer-rail:transcript`, `drawer-rail:3`.
fn control(item: RailItem) -> String {
    match item {
        RailItem::Transcript => "drawer-rail:transcript".into(),
        RailItem::Task { id } => format!("drawer-rail:{id}"),
    }
}

impl Workspace {
    /// The drawer, `width` wide, with its rail at the right edge when it has one: the transcript
    /// or the conversation in what is left of the width.
    pub(crate) fn render_railed_drawer(&self, width: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.model.rail_drawn() {
            return self.render_listen_view(width, cx).or_else(|| self.render_drawer(width, cx));
        }
        let inner = (width - WIDTH).max(0.);
        let body = self.render_listen_view(inner, cx).or_else(|| self.render_drawer(inner, cx))?;
        Some(div().flex_none().h_full().flex().child(body).child(self.drawer_rail(cx)).into_any_element())
    }

    /// The column: the transcript, a divider, the tasks, +.
    fn drawer_rail(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let selected = self.model.rail_selected();
        let keyed = match self.model.rail_focus {
            Some(RailFocus::Right(item)) => Some(item),
            _ => None,
        };
        let items = self.model.rail();
        let mut column = div()
            .id("drawer-rail")
            .w(px(WIDTH))
            .flex_none()
            .h_full()
            .pt_2()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .bg(rgb(theme::BACKGROUND))
            .border_l_1()
            .border_color(rgb(theme::BORDER));
        for (at, item) in items.iter().copied().enumerate() {
            let Some(face) = self.rail_face(item, selected == Some(item)) else { continue };
            column = column.child(self.rail_item(item, face, selected == Some(item), keyed == Some(item), cx));
            if item == RailItem::Transcript && at + 1 < items.len() {
                column = column.child(div().w(px(20.)).h(px(1.)).my(px(2.)).bg(rgb(theme::BORDER)));
            }
        }
        let new = div()
            .id("drawer-rail-new")
            .relative()
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
        column.child(new).child(self.controls.mark("drawer-rail"))
    }

    /// What an item shows, and its hover: the transcript's waveform (in the accent while
    /// selected) with a red dot while it records; a task's badge, whose dot is its state's colour.
    fn rail_face(&self, item: RailItem, selected: bool) -> Option<(AnyElement, gpui::SharedString)> {
        match item {
            RailItem::Transcript => {
                let on = self.model.listen.on.as_ref()?;
                let recording = on.status == Status::On;
                let colour = if selected { theme::PRIMARY } else { theme::MUTED };
                let wave = svg().path("icons/waveform.svg").size(px(18.)).text_color(rgb(colour));
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
                let tip = if recording { "The transcript, recording" } else { "The transcript, paused" };
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

    /// One item: lifted with an accent edge while selected, ringed while the keys are on it.
    fn rail_item(
        &self,
        item: RailItem,
        (face, tip): (AnyElement, gpui::SharedString),
        selected: bool,
        keyed: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let edge = div().absolute().left_0().top(px(6.)).bottom(px(6.)).w(px(2.)).rounded_sm().bg(rgb(theme::PRIMARY));
        div()
            .id(ElementId::Name(control(item).into()))
            .relative()
            .flex_none()
            .size(px(ITEM))
            .rounded_lg()
            .cursor_pointer()
            .when(selected, |d| d.bg(rgb(theme::SELECTED)).child(edge))
            .when(!selected, |d| d.hover(|s| s.bg(rgb(theme::RAISED))))
            .when(keyed, |d| d.border_1().border_color(rgb(theme::LIST)))
            .child(face)
            .child(self.controls.mark(control(item)))
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
            id => {
                if let Ok(id) = id.parse() {
                    self.rail_click(RailItem::Task { id }, window, cx);
                }
            }
        }
        true
    }

    /// The rail as the state shows it: each item's `kind`, `id` (a task's, else null),
    /// `selected` and where it was drawn (null: not drawn). Empty: no rail. Never a transcript's text.
    pub(crate) fn rail_json(&self) -> serde_json::Value {
        let selected = self.model.rail_selected();
        let row = |item: RailItem| {
            let (kind, id) = match item {
                RailItem::Transcript => ("transcript", None),
                RailItem::Task { id } => ("task", Some(id)),
            };
            let drawn = self.controls.bounds(&control(item)).map(rect);
            serde_json::json!({"kind": kind, "id": id, "selected": selected == Some(item), "rect": drawn})
        };
        serde_json::Value::Array(self.model.rail().into_iter().map(row).collect())
    }
}
