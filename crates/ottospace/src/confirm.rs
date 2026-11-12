//! The quit question: ⌘Q asks before it ends a program running in a terminal.

use gpui::{Context, Keystroke, MouseButton, Window, black, div, prelude::*, px, rgb};
use ottospace_core::keys::Mods;
use ottospace_core::model::{Confirm, Running};
use ottospace_core::{Action, Origin, theme};

use crate::Workspace;

impl Workspace {
    /// ⌘Q, Quit in the menu or the window's close button: the core quits, or asks if a terminal
    /// runs something.
    pub fn ask_quit(&mut self, window: &Window, cx: &mut Context<Self>) {
        let running = |pane: &ottospace_core::Pane| {
            let work = self.terminals.get(&pane.id)?.running()?;
            Some(Running { pane: pane.id, program: work.program, background: work.background })
        };
        let running = self.model.panes.iter().filter_map(running).collect();
        self.dispatch(Action::QuitAsked { running }, self.input, window, cx);
        // Asked from the menu while minimised or behind: the question must be seen. Not in a
        // test run, which never comes forward.
        if self.model.confirm.is_some() && self.hosting {
            cx.activate(true);
            window.activate_window();
        }
    }

    /// What `key` from `origin` answers the question with, as the core says (`Confirm::answer`).
    /// `None`: the question is not up, or the key is a socket client's and not an answer.
    pub fn confirm_answer(&self, key: &Keystroke, origin: Origin) -> Option<Action> {
        self.model.confirm.as_ref()?;
        let m = key.modifiers;
        // Fn counts as a modifier the core has no name for: with it no key is a yes.
        let mods = Mods { ctrl: m.control, alt: m.alt, shift: m.shift || m.function, cmd: m.platform };
        Confirm::answer(&key.key, mods, origin)
    }

    /// Before any binding or key handler runs. While the question is up it takes every key: it
    /// answers or it does nothing, e.g. ⌘V pastes nothing. `false`: no question, the key goes on.
    pub fn confirm_intercepts(&mut self, key: &Keystroke, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.model.confirm.is_none() {
            return false;
        }
        // The user's held key answers nothing: neither the ⌘Q that asked nor ↵ held from the
        // terminal. (A client's key is no event of the keyboard's.)
        let held = self.input == Origin::User && ottospace_mac::window::key_repeating();
        if let Some(answer) = self.confirm_answer(key, self.input).filter(|_| !held) {
            self.dispatch(answer, self.input, window, cx);
        }
        true
    }

    /// The question, centred over the workspace. A click outside it cancels.
    pub fn render_confirm(&self, confirm: &Confirm, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let lines = confirm.lines().into_iter().map(|line| div().child(line));
        let panel = crate::dialog::panel(theme::BORDER_STRONG)
            .w(px(320.))
            .flex()
            .flex_col()
            // A click inside the panel must not reach the backdrop, which cancels.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px_3()
                    .pt_3()
                    .flex()
                    .gap_2()
                    .text_sm()
                    .child(div().text_color(rgb(theme::TEXT)).child(confirm.title()))
                    .child(div().text_color(rgb(theme::CONFIRM)).child("Quit?")),
            )
            .child(
                div()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .child("Quitting ends these:")
                    .children(lines),
            )
            .child(
                div()
                    .px_3()
                    .py_1p5()
                    .border_t_1()
                    .border_color(rgb(theme::BORDER))
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .child("↵ quit · esc cancel"),
            );
        div()
            .id("confirm-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(black().opacity(0.5))
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.dispatch(Action::QuitCancel, this.input, window, cx);
                }),
            )
            .child(panel)
    }
}
