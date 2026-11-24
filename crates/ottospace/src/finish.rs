//! Done and Later in the shell: the header's buttons, the quiet offer under the last
//! reply and the × on a rail item's hover. What each shows and does is the core's
//! (`ottospace_core::model::finish`); a click is dispatched as whoever made it.

use gpui::{AnyView, Context, Div, ElementId, MouseButton, Stateful, Window, div, prelude::*, px, rgb};
use ottospace_core::talk::Subject;
use ottospace_core::tasks::TaskId;
use ottospace_core::{Action, theme};

use crate::{Tooltip, Workspace};

/// Done's green: never mistaken for Later.
pub(crate) const DONE: u32 = 0x34c38f;
const DONE_EDGE: u32 = 0x2c6e55;

impl Workspace {
    /// A bordered button of the header: `talk:<name>`.
    fn finish_button(&self, name: &'static str, tip: &'static str, (text, edge): (u32, u32)) -> Stateful<Div> {
        div()
            .id(name)
            .relative()
            .flex_none()
            .h(px(22.))
            .px_2()
            .flex()
            .items_center()
            .gap_1()
            .rounded_md()
            .border_1()
            .border_color(rgb(edge))
            .cursor_pointer()
            .text_xs()
            .text_color(rgb(text))
            .hover(|s| s.bg(rgb(theme::SELECTED)))
            .child(self.controls.mark(format!("talk:{name}")))
            .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.into()))))
            // The press must not reach the drawer, which takes the keyboard on a press.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    }

    /// Later, then ✓ Done, at the header's right, as the core says for its agent now.
    pub(crate) fn finish_buttons(&self, subject: &Subject, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let (Some(buttons), Some(id)) = (self.model.finish(subject), subject.task()) else { return Vec::new() };
        let mut out = Vec::new();
        if buttons.later {
            let later = self.finish_button(
                "talk-later",
                "Stop the agent; resume it from Tasks (⌘W)",
                (theme::TEXT, theme::BORDER_STRONG),
            );
            out.push(later.child("Later").on_click(cx.listener(move |this, _, window, cx| this.later(id, window, cx))));
        }
        if buttons.done {
            let done = self.finish_button("talk-done", "The task is finished (⌘↵)", (DONE, DONE_EDGE));
            let done = done.font_weight(gpui::FontWeight::MEDIUM).child("✓").child("Done");
            out.push(done.on_click(cx.listener(move |this, _, window, cx| this.done(id, window, cx))));
        }
        out
    }

    /// "Got what you needed? ✓ Done · Later" under the last reply, while the core offers it.
    pub(crate) fn done_offer_line(&self, subject: &Subject, cx: &mut Context<Self>) -> Option<Div> {
        let id = subject.task().filter(|_| self.model.done_offered(subject))?;
        let link = |name: &'static str, text: &'static str, color: u32| {
            div()
                .id(name)
                .relative()
                .cursor_pointer()
                .text_color(rgb(color))
                .hover(|s| s.underline())
                .child(text)
                .child(self.controls.mark(format!("talk:{name}")))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let done = link("offer-done", "✓ Done", DONE)
            .on_click(cx.listener(move |this, _, window, cx| this.done(id, window, cx)));
        let later = link("offer-later", "Later", theme::MUTED)
            .on_click(cx.listener(move |this, _, window, cx| this.later(id, window, cx)));
        let line = div().flex().items_center().gap_2().text_xs().text_color(rgb(theme::MUTED));
        Some(line.child("Got what you needed?").child(done).child("·").child(later))
    }

    /// The dim × on a rail item's hover: Later for that task. In `group`, the item's own.
    pub(crate) fn rail_later(&self, id: TaskId, group: &str, cx: &Context<Self>) -> Stateful<Div> {
        let control = format!("drawer-rail:later:{id}");
        div()
            .id(ElementId::Name(control.clone().into()))
            .absolute()
            .top(px(-3.))
            .right(px(-3.))
            .size(px(16.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(theme::RAISED))
            .text_size(px(10.))
            .text_color(rgb(theme::MUTED))
            .opacity(0.)
            .group_hover(group.to_string(), |s| s.opacity(1.))
            .hover(|s| s.text_color(rgb(theme::TEXT)))
            .child("×")
            .child(self.controls.mark(control))
            .tooltip(|_, cx| AnyView::from(cx.new(|_| Tooltip("Later: stop the agent, keep the task in Tasks".into()))))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.later(id, window, cx);
            }))
    }

    /// Done, as whoever acts: a client's is refused, and told why. The core is told first where
    /// each terminal is now: one in the task's own worktree keeps it.
    pub(crate) fn done(&mut self, id: TaskId, window: &Window, cx: &mut Context<Self>) {
        // Its six seconds of Undo count from now, not from the last tick.
        self.dispatch(Action::Tick { now: crate::now_ms() }, ottospace_core::Origin::System, window, cx);
        self.tell_terminals();
        self.dispatch(Action::TaskDone { id }, self.input, window, cx);
        cx.notify();
    }

    pub(crate) fn later(&mut self, id: TaskId, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::TaskLater { id }, self.input, window, cx);
        cx.notify();
    }

    /// The local day's start, when it turns: what Done today counts from.
    pub(crate) fn tell_day(&mut self, window: &Window, cx: &mut Context<Self>) {
        let now = crate::now_ms();
        let day = crate::day_start(now);
        if self.told_day != day {
            self.told_day = day;
            self.dispatch(Action::LocalDay { day }, ottospace_core::Origin::System, window, cx);
            // The ISO week of the local date: the week's focus is this week's.
            let week = ottospace_core::focus::iso_week(crate::local_days(now));
            self.dispatch(Action::SetWeek { week }, ottospace_core::Origin::System, window, cx);
        }
    }

    /// The header's buttons and the offer, for the state: what shows and where (null: not drawn).
    pub(crate) fn finish_snapshot(&self, subject: &Subject) -> serde_json::Value {
        let rect = |name: &str| self.controls.bounds(&format!("talk:{name}")).map(crate::rect);
        let buttons = self.model.finish(subject);
        serde_json::json!({
            "later": buttons.is_some_and(|b| b.later),
            "done": buttons.is_some_and(|b| b.done),
            "offered": self.model.done_offered(subject),
            "rects": {
                "later": rect("talk-later"),
                "done": rect("talk-done"),
                "offer_done": rect("offer-done"),
                "offer_later": rect("offer-later"),
            },
        })
    }
}
