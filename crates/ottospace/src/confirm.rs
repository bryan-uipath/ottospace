//! The quit question: ⌘Q asks before it ends a running program or agent, or loses a draft. And
//! the other questions the dialog asks (paste, dismiss, unlist, close, delete a worktree).

use gpui::{Context, Keystroke, Window, div, prelude::*, px, relative, rgb};
use ottospace_core::clip::PasteAsk;
use ottospace_core::keys::Mods;
use ottospace_core::model::{Confirm, Owner, Running, quit};
use ottospace_core::{Action, Kind, Origin, theme};

use crate::{Workspace, dialog, ui};

impl Workspace {
    /// ⌘Q, Quit in the menu or the window's close button: the core quits, or asks if a terminal
    /// runs something.
    pub fn ask_quit(&mut self, window: &Window, cx: &mut Context<Self>) {
        let running = self.running();
        self.dispatch(Action::QuitAsked { running }, self.input, window, cx);
        // Asked from the menu while minimised or behind: the question must be seen. Not in a
        // test run, which never comes forward.
        if self.model.confirm.is_some() && self.hosting {
            cx.activate(true);
            window.activate_window();
        }
    }

    /// The terminals with a program in front of their shell, or a job of it, now.
    fn running(&self) -> Vec<Running> {
        let running = |pane: &ottospace_core::Pane| {
            let work = self.terminals.get(&pane.id)?.running()?;
            Some(Running { pane: pane.id, program: work.program, background: work.background })
        };
        self.model.panes.iter().filter_map(running).collect()
    }

    /// × or ⌘⌃W on the space at `index` (default: the selected one): the core asks first if it
    /// is a temporary workspace whose terminals run something.
    pub fn hide_space(&mut self, index: Option<usize>, window: &Window, cx: &mut Context<Self>) {
        let running = self.running();
        self.dispatch(Action::SpaceHideAsked { index, running }, self.input, window, cx);
    }

    /// ⌘⇧W, a tab's × or its label on the tab at `index` (default: the one on show): the core asks
    /// first if it is the last tab of a temporary workspace whose terminals run something.
    pub fn ask_close_tab(&mut self, index: Option<usize>, window: &Window, cx: &mut Context<Self>) {
        let running = self.running();
        self.dispatch(Action::CloseTabAsked { index, running }, self.input, window, cx);
    }

    /// What `key` from `origin` answers the question with, as the core says (`Confirm::answer`).
    /// `None`: the question is not up, or the key is a socket client's and not an answer.
    pub fn confirm_answer(&self, key: &Keystroke, origin: Origin) -> Option<Action> {
        let confirm = self.model.confirm.as_ref()?;
        let m = key.modifiers;
        // Fn counts as a modifier the core has no name for: with it no key is a yes.
        let mods = Mods { shift: m.shift || m.function, ..crate::mods(m) };
        match confirm.paste {
            Some(_) => PasteAsk::answer(&key.key, mods, origin == Origin::User),
            None if confirm.later.is_some() => Confirm::answer_later(&key.key, mods, origin),
            None if confirm.delete.is_some() => Confirm::answer_delete(&key.key, mods, origin),
            None => Confirm::answer(&key.key, mods, origin),
        }
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
            // Where each shell is now: the core keeps a worktree a terminal went into since.
            if self.model.confirm.as_ref().is_some_and(|c| c.delete.is_some()) {
                self.tell_terminals();
            }
            self.dispatch(answer, self.input, window, cx);
        }
        // Answered: the text a paste waited with goes.
        if self.model.confirm.is_none() {
            self.pasting = None;
        }
        true
    }

    /// Where the quit question's draft rows are drawn, in its list's order. Empty: no such question.
    pub fn quit_rows(&self) -> Vec<[f32; 4]> {
        let rows = self.model.confirm.as_ref().filter(|c| c.quits()).map_or(0, |c| c.unsent.len());
        let marks = self.marks.borrow();
        (0..rows).filter_map(|at| marks.get(&crate::Mark::QuitRow(at)).copied().map(crate::rect)).collect()
    }

    /// "Paste 4 lines?" over its pane: the first lines of the text, and what ↵ does.
    pub fn render_paste_ask(&self, ask: &PasteAsk) -> impl IntoElement + use<> {
        let text = self.pasting.as_ref().map_or("", |(_, text)| text.as_str());
        let lines = text.lines().take(3).map(|line| div().truncate().child(line.to_string()));
        let more = (ask.amount.lines > 3).then(|| div().child(format!("and {} more", ask.amount.lines - 3)));
        let panel = dialog::panel(theme::BORDER_STRONG)
            .w(px(320.))
            .max_w(relative(0.9))
            .flex()
            .flex_col()
            .child(div().px_3().pt_3().text_sm().text_color(rgb(theme::TEXT)).child(ask.title()))
            .child(
                div()
                    .p_3()
                    .flex()
                    .flex_col()
                    .text_xs()
                    .font_family(crate::terminal::font_family())
                    .text_color(rgb(theme::MUTED))
                    .children(lines)
                    .children(more),
            )
            .child(dialog::hint("↵ paste · esc cancel"));
        div().absolute().size_full().flex().items_center().justify_center().child(panel)
    }

    /// The question, centred over the workspace. A click outside it cancels.
    pub fn render_confirm(&self, confirm: &Confirm, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        // "Paste 4 lines?" is drawn in its pane (`render_paste_ask`); a click anywhere cancels it.
        if confirm.paste.is_some() {
            return dialog::backdrop("confirm-backdrop", 0., Action::QuitCancel, cx).flex();
        }
        // "Stop its agent and dismiss?": the task, and what ↵ does.
        if let Some(task) = confirm.dismiss.and_then(|id| self.model.tasks.get(id)) {
            let panel = dialog::floating(320.)
                .child(div().px_3().pt_3().text_sm().text_color(rgb(theme::TEXT)).child(confirm.title()))
                .child(div().p_3().text_xs().text_color(rgb(theme::MUTED)).child(task.title.clone()))
                .child(dialog::hint("↵ stop and dismiss · esc cancel"));
            return dialog::backdrop("confirm-backdrop", 0.5, Action::QuitCancel, cx)
                .flex()
                .items_center()
                .justify_center()
                .child(panel);
        }
        // "Stop "<task>" and put it away?": Later on a task at work. Its conversation is kept.
        if confirm.later.is_some() {
            let said = "It is working. Its conversation is kept; you can resume it from Tasks.";
            let panel = dialog::floating(320.)
                .child(div().px_3().pt_3().text_sm().text_color(rgb(theme::TEXT)).child(confirm.title()))
                .child(div().p_3().text_xs().text_color(rgb(theme::MUTED)).child(said))
                .child(dialog::hint("↵ stop, resume later · esc keep running"));
            return dialog::backdrop("confirm-backdrop", 0.5, Action::QuitCancel, cx)
                .flex()
                .items_center()
                .justify_center()
                .child(panel);
        }
        // "Close <title>?": an owned window whose app says it has unsaved changes.
        if let Some(pane) = confirm.edited {
            let title = match self.model.pane(pane).map(|pane| &pane.kind) {
                Some(Kind::App { title, .. }) if !title.is_empty() => title.clone(),
                Some(Kind::App { name, .. }) => format!("this {name} window"),
                _ => "this window".into(),
            };
            let panel = dialog::floating(320.)
                .child(div().px_3().pt_3().text_sm().text_color(rgb(theme::TEXT)).child(format!("Close {title}?")))
                .child(div().p_3().text_xs().text_color(rgb(theme::MUTED)).child("It has unsaved changes"))
                .child(dialog::hint("↵ close · esc cancel"));
            return dialog::backdrop("confirm-backdrop", 0.5, Action::QuitCancel, cx)
                .flex()
                .items_center()
                .justify_center()
                .child(panel);
        }
        // "Unlist this repository?": which, that a task works there, and that no file goes.
        if let Some(path) = &confirm.unlist {
            let name = ottospace_core::repos::name_of(path);
            let panel = dialog::floating(320.)
                .child(div().px_3().pt_3().text_sm().text_color(rgb(theme::TEXT)).child(format!("Unlist {name}?")))
                .child(
                    div()
                        .p_3()
                        .text_xs()
                        .text_color(rgb(theme::MUTED))
                        .child("A task works in it. It leaves the Repos list; nothing on disk changes."),
                )
                .child(dialog::hint("↵ unlist · esc cancel"));
            return dialog::backdrop("confirm-backdrop", 0.5, Action::QuitCancel, cx)
                .flex()
                .items_center()
                .justify_center()
                .child(panel);
        }
        // "Delete this worktree?": its branch and full path, and that nothing brings it back.
        if let Some(path) = &confirm.delete {
            let branch = self.model.linked_worktree(path).and_then(|(_, w)| w.branch.clone());
            let branch = branch.unwrap_or_else(|| "(no branch)".into());
            let said = format!("Delete the worktree of {branch} at {path}? Its branch is kept.");
            let panel = dialog::floating(360.)
                .child(div().px_3().pt_3().text_sm().text_color(rgb(theme::CONFIRM)).child(confirm.title()))
                .child(
                    div()
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .text_xs()
                        .text_color(rgb(theme::MUTED))
                        .child(said)
                        .child("Its folder goes, and cannot be brought back. git keeps one with changes."),
                )
                .child(dialog::hint("⌘⌫ delete · ↵ esc keep"));
            return dialog::backdrop("confirm-backdrop", 0.5, Action::QuitCancel, cx)
                .flex()
                .items_center()
                .justify_center()
                .child(panel);
        }
        // "Close this workspace?": which, and the terminals that end once its Undo runs out.
        if let Some(id) = confirm.close {
            let name = self.model.space_title(&Owner::Workbook { id });
            let lines = confirm.lines().into_iter().map(|line| div().child(line));
            let panel = dialog::floating(320.)
                .child(div().px_3().pt_3().text_sm().text_color(rgb(theme::TEXT)).child(format!("Close {name}?")))
                .child(
                    div()
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .text_xs()
                        .text_color(rgb(theme::MUTED))
                        .child("It is not kept. Once its Undo runs out, closing ends these:")
                        .children(lines),
                )
                .child(dialog::hint("↵ close · esc cancel"));
            return dialog::backdrop("confirm-backdrop", 0.5, Action::QuitCancel, cx)
                .flex()
                .items_center()
                .justify_center()
                .child(panel);
        }
        // What quitting ends, then the drafts it loses: a section only with rows. A draft's row
        // cancels and goes to it.
        let section = |head: &'static str| div().pt_1().text_color(rgb(theme::TEXT)).child(head);
        let sections = confirm.sections().into_iter().map(|(head, rows)| {
            let drafts = head == quit::NOT_SENT;
            let row = |(at, line): (usize, String)| match drafts {
                // A list row, its words in line with the section's.
                true => ui::list_row(("quit-row", at), ui::Row::Normal, ui::State::Rest)
                    .mx(px(-theme::SPACE_2))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.dispatch(Action::QuitGo { row: at }, this.input, window, cx);
                    }))
                    .child(crate::mark(&self.marks, crate::Mark::QuitRow(at)))
                    .child(line)
                    .into_any_element(),
                false => div().child(line).into_any_element(),
            };
            div().flex().flex_col().gap_1().child(section(head)).children(rows.into_iter().enumerate().map(row))
        });
        let panel = dialog::floating(320.)
            .child(div().px_3().pt_3().text_sm().text_color(rgb(theme::CONFIRM)).child(confirm.title()))
            .child(div().p_3().flex().flex_col().gap_2().text_xs().text_color(rgb(theme::MUTED)).children(sections))
            .child(dialog::hint("↵ quit · esc cancel"));
        dialog::backdrop("confirm-backdrop", 0.5, Action::QuitCancel, cx)
            .flex()
            .items_center()
            .justify_center()
            .child(panel)
    }
}
