//! The new-task box. `new_task_content` is what it holds: the line, what it reads, the hints.
//! It knows nothing of where it is: no size, no position, no backdrop. `render_compose` is the
//! host of the day, a modal panel over a dimmed workspace; another host (a side bar) can show
//! the same content.

use gpui::{Context, Div, KeyDownEvent, MouseButton, Window, black, div, prelude::*, px, rgb};
use ottospace_core::{Action, theme};

use crate::{KEYS_HINT, TAG_HINT, Workspace, dialog};

impl Workspace {
    /// The box's keys: ↵ adds the task, ⌘↵ adds and starts it, Tab completes, Esc closes. The
    /// rest edit the line. ⌘ shortcuts the line does not use still reach their bindings; pane
    /// ones wait.
    pub fn compose_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        let m = key.modifiers;
        let action = match key.key.as_str() {
            "enter" if event.is_held => return,
            "enter" if m.platform => Action::ComposeSubmit { open: true },
            "enter" => Action::ComposeSubmit { open: false },
            "escape" => Action::CloseCompose,
            "tab" => Action::ComposeComplete,
            // What types text is not stopped: the input method inserts it.
            _ => {
                if self.field_key(event, window, cx) {
                    cx.stop_propagation();
                }
                return;
            }
        };
        self.dispatch(action, self.input, window, cx);
        cx.stop_propagation();
    }

    /// The new-task box's content, for any host: the line, what ↵ would add, and names that
    /// finish the tag being typed. As wide as the host makes it.
    pub fn new_task_content(&self, cx: &mut Context<Self>) -> Div {
        let draft = self.model.draft(self.model.compose_line().map_or("", String::as_str));
        let chip = |label: String, color: u32| {
            div().px_1p5().rounded_sm().bg(rgb(theme::SELECTED)).text_xs().text_color(rgb(color)).child(label)
        };
        // What ↵ would add.
        let reads = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .children(draft.priority.map(|p| chip(format!("P{p}"), theme::TEXT)))
            .children(draft.project.map(|p| chip(format!("#{p}"), theme::COMPOSE)))
            .children(draft.repo.map(|r| chip(format!(">{r}"), theme::PRIMARY)))
            .children(draft.agent.map(|a| chip(format!("@{a}"), theme::VISUAL)))
            .children(draft.paths.into_iter().map(|path| chip(path, theme::TEXT)))
            .children(draft.skills.into_iter().map(|s| chip(format!("${s}"), theme::SNEAK)));
        let suggestions = self.model.compose_suggestions();
        let hint =
            if suggestions.is_empty() { TAG_HINT.to_string() } else { format!("tab: {}", suggestions.join("  ·  ")) };
        let muted = |text: String| div().text_xs().text_color(rgb(theme::MUTED)).child(text);
        let line = div()
            .min_h(px(34.))
            .px_2()
            .flex()
            .items_center()
            .rounded(px(dialog::INNER_RADIUS))
            .bg(rgb(theme::BACKGROUND))
            .text_base()
            .child(self.render_field("What needs doing?", cx));
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_xs().text_color(rgb(theme::COMPOSE)).child("NEW TASK"))
            .child(line)
            .child(reads)
            .child(muted(hint))
            .child(muted(KEYS_HINT.to_string()))
    }

    /// The modal host: the content in a panel near the top, the workspace dimmed behind it. A
    /// click outside the panel closes the box.
    pub fn render_compose(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let panel = dialog::panel(theme::COMPOSE)
            .w(px(560.))
            .max_w_full()
            .p_3()
            // A click inside the box must not reach the backdrop, which closes it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(self.new_task_content(cx));
        div()
            .id("compose-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .justify_center()
            .items_start() // the box is as tall as its lines, not the window
            .pt(px(120.))
            .px_4()
            .bg(black().opacity(0.4))
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.dispatch(Action::CloseCompose, this.input, window, cx);
                }),
            )
            .child(panel)
    }
}
