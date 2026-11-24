//! The quit question: ⌘Q asks before it ends a running program or agent, or loses a draft. And
//! the other questions the dialog asks (dismiss, unlist, close, delete a worktree).

use gpui::{Context, Keystroke, SharedString, Window, div, prelude::*, px, rgb};
use ottospace_core::keys::Mods;
use ottospace_core::model::{Confirm, Look, Owner, Running, quit};
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
        match () {
            _ if confirm.later.is_some() => Confirm::answer_later(&key.key, mods, origin),
            _ if confirm.delete.is_some() => Confirm::answer_delete(&key.key, mods, origin),
            _ => Confirm::answer(&key.key, mods, origin),
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
        true
    }

    /// Where the quit question's draft rows are drawn, in its list's order. Empty: no such question.
    pub fn quit_rows(&self) -> Vec<[f32; 4]> {
        let rows = self.model.confirm.as_ref().filter(|c| c.quits()).map_or(0, |c| c.unsent.len());
        let marks = self.marks.borrow();
        (0..rows).filter_map(|at| marks.get(&crate::Mark::QuitRow(at)).copied().map(crate::rect)).collect()
    }

    /// The question, centred over the workspace (`ui::dialog`): its title, one sentence, what it
    /// acts on, then its answers as buttons, the core's (`Confirm::answers`). × , Esc and a click
    /// outside it cancel.
    pub fn render_confirm(&self, confirm: &Confirm, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted = || {
            div().flex().flex_col().gap(px(theme::SPACE_1)).text_size(px(theme::TEXT_XS)).text_color(rgb(theme::MUTED))
        };
        let (title, purpose, body): (String, &str, Option<gpui::Div>) = if confirm.dismiss.is_some() {
            // "Stop its agent and dismiss?": the task.
            let task = confirm.dismiss.and_then(|id| self.model.tasks.get(id)).map(|task| task.title.clone());
            (confirm.title(), "Its agent is still working.", task.map(|task| muted().child(task)))
        } else if confirm.later.is_some() {
            // "Stop "<task>" and put it away?": Later on a task at work. Its conversation is kept.
            (confirm.title(), "It is working. Its conversation is kept; resume it from Tasks.", None)
        } else if let Some(pane) = confirm.edited {
            // "Close <title>?": an owned window whose app says it has unsaved changes.
            let title = match self.model.pane(pane).map(|pane| &pane.kind) {
                Some(Kind::App { title, .. }) if !title.is_empty() => title.clone(),
                Some(Kind::App { name, .. }) => format!("this {name} window"),
                _ => "this window".into(),
            };
            (format!("Close {title}?"), "It has unsaved changes.", None)
        } else if let Some(path) = &confirm.unlist {
            // "Unlist <repo>?": that a task works there, and that no file goes.
            let name = ottospace_core::repos::name_of(path);
            (format!("Unlist {name}?"), "A task works in it. It leaves the Repos list; nothing on disk changes.", None)
        } else if let Some(path) = &confirm.delete {
            // "Delete the <branch> worktree?": its full path, and that nothing brings it back.
            let branch = self.model.linked_worktree(path).and_then(|(_, w)| w.branch.clone());
            let title = branch.map_or_else(|| confirm.title(), |branch| format!("Delete the {branch} worktree?"));
            let path = div()
                .px(px(9.))
                .py(px(7.))
                .rounded(px(dialog::INNER_RADIUS))
                .bg(rgb(theme::WELL))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .text_size(px(theme::TEXT_XS))
                .font_family(crate::terminal::font_family())
                .child(path.clone());
            (
                title,
                "Its folder goes for good. The branch stays, and git refuses if it has changes.",
                Some(div().child(path)),
            )
        } else if let Some(id) = confirm.close {
            // "Close <workspace>?": the terminals that end once its Undo runs out.
            let name = self.model.space_title(&Owner::Workbook { id });
            let lines = confirm.lines().into_iter().map(|line| div().child(line));
            (
                format!("Close {name}?"),
                "It is not kept. Once its Undo runs out, closing ends these:",
                Some(muted().children(lines)),
            )
        } else {
            // What quitting ends, then the drafts it loses: a section only with rows. A draft's
            // row cancels and goes to it.
            let head = |head: &'static str, count: usize| {
                let head = format!("{} · {count}", head.trim_end_matches(':'));
                div()
                    .pt(px(theme::SPACE_1))
                    .text_size(px(theme::TEXT_2XS))
                    .text_color(rgb(theme::MUTED))
                    .child(head.to_uppercase())
            };
            let sections = confirm.sections().into_iter().map(|(name, rows)| {
                let drafts = name == quit::NOT_SENT;
                let count = rows.len();
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
                    false => div().text_size(px(theme::TEXT_XS)).child(line).into_any_element(),
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::SPACE_1))
                    .child(head(name, count))
                    .children(rows.into_iter().enumerate().map(row))
            });
            (confirm.title(), "These end, or are lost, when it quits.", Some(div().children(sections)))
        };
        let answers = confirm.answers().into_iter().enumerate().map(|(at, answer)| {
            let kind = match answer.look {
                Look::Cancel => ui::Button::Secondary(answer.label.into()),
                Look::Primary => ui::Button::Primary(answer.label.into()),
                Look::Destroys => ui::Button::DangerPrimary(answer.label.into()),
            };
            let id = if at == 0 { "confirm:no" } else { "confirm:yes" };
            let action = answer.action;
            ui::dialog::action(id, kind, answer.keys, ui::State::Rest)
                .child(self.controls.mark(id.to_string()))
                .on_click(cx.listener(move |this, _, window, cx| this.confirm_click(action.clone(), window, cx)))
                .into_any_element()
        });
        let close = ui::dialog::close("confirm:close", ui::State::Rest)
            .child(self.controls.mark("confirm:close".to_string()))
            .on_click(cx.listener(|this, _, window, cx| this.confirm_click(Action::QuitCancel, window, cx)));
        let panel = ui::dialog::frame(
            ui::dialog::QUESTION,
            title,
            Some(SharedString::from(purpose)),
            close,
            body,
            ui::dialog::actions(None, None, answers.collect()),
        );
        dialog::backdrop("confirm-backdrop", ui::dialog::DIM, Action::QuitCancel, cx)
            .flex()
            .items_center()
            .justify_center()
            .child(panel)
    }

    /// A button of the question pressed: as its key would answer, where each shell is first known
    /// for a delete (the core keeps a worktree a terminal went into since).
    fn confirm_click(&mut self, answer: Action, window: &Window, cx: &mut Context<Self>) {
        if self.model.confirm.as_ref().is_some_and(|c| c.delete.is_some()) && answer == Action::QuitConfirm {
            self.tell_terminals();
        }
        self.dispatch(answer, self.input, window, cx);
    }
}
