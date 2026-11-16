//! Workbooks in the shell (the user reads "workspace"): the Workspaces section and the files in
//! `~/.ottospace/workbooks`.
//! What one holds and what opening it does are the core's (`ottospace_core::workbook`).

use gpui::{Context, Div, ElementId, Window, div, prelude::*, px, rgb};
use ottospace_core::list::ListRow;
use ottospace_core::marks::MarkInput;
use ottospace_core::model::Section;
use ottospace_core::workbook::{self, History, WorkbookId, WorkbookRow};
use ottospace_core::{Action, Origin, PaneId, theme};

use crate::{Workspace, config, marks, now_ms, rect};

/// What the workbooks folder and this run last said, kept between frames.
#[derive(Default)]
pub struct Kept {
    /// Each file as last read or written: id and text. Only these are ever written or removed.
    stored: Vec<(String, String)>,
    /// Why a workbook could not be written or removed, until that succeeds.
    pub unsaved: Option<String>,
    /// What was wrong with the folder at start.
    pub notice: Option<String>,
}

/// A control's id for a tab of a workbook: `workbook-tab-menu:3:1` is tab 1 of workbook 3.
fn tab_control(name: &str, id: WorkbookId, at: usize) -> String {
    format!("{name}:{id}:{at}")
}

impl Workspace {
    /// Reads the workbooks folder at start. A file that does not read is left as it is, and
    /// counted in the status bar: no other workbook's write touches it.
    pub(crate) fn restore_workbooks(&mut self) {
        let kept = config::load_workbooks();
        let read = |(uid, text): &(String, String)| workbook::parse(text, uid).ok();
        let workbooks: Vec<workbook::Workbook> = kept.files.iter().filter_map(read).collect();
        let unread = kept.unread + kept.files.len() - workbooks.len();
        let unread = (unread > 0).then(|| format!("workspaces: {unread} file(s) could not be read; left as they are"));
        self.kept_workbooks.notice = kept.error.or(unread);
        self.model.update(Action::RestoreWorkbooks { workbooks }.sent_by(Origin::System));
        // What was read needs no writing back: a stage 1 file is written as version 2 at its next change.
        self.kept_workbooks.stored = self.workbook_files();
    }

    /// Each terminal's last lines (`workbook_scrollback_lines`) and the command in front, for a
    /// workbook the user saves. Read only then: the text goes into the action, never the state.
    pub(crate) fn histories(&self) -> Vec<(PaneId, History)> {
        let (most, saved) = (config::workbook_lines(), now_ms());
        let history = |(id, terminal): (&PaneId, &crate::terminal::Terminal)| {
            // 0: no terminal text at all, the command line included.
            let command = (most > 0).then(|| terminal.foreground_command().or_else(|| terminal.offered())).flatten();
            (*id, History { command, ..terminal.history(saved, most) }.kept(most))
        };
        self.terminals.iter().map(history).collect()
    }

    /// Each workbook as its file would hold it now.
    fn workbook_files(&self) -> Vec<(String, String)> {
        self.model.workbooks.list.iter().map(|book| (book.uid.clone(), workbook::write(book))).collect()
    }

    /// Writes the files of the workbooks that changed, and removes those of the ones deleted.
    /// One that failed counts as changed still: the next change to anything tries it again.
    pub(crate) fn store_workbooks(&mut self) {
        let now = self.workbook_files();
        if now == self.kept_workbooks.stored {
            return;
        }
        let before = std::mem::take(&mut self.kept_workbooks.stored);
        let was = |uid: &str| before.iter().find(|(known, _)| known == uid);
        let (mut stored, mut failed) = (Vec::with_capacity(now.len()), None);
        for (uid, text) in &now {
            if was(uid).is_some_and(|(_, kept)| kept == text) {
                stored.push((uid.clone(), text.clone()));
                continue;
            }
            match config::save_workbook(uid, text) {
                Ok(()) => stored.push((uid.clone(), text.clone())),
                Err(err) => {
                    failed = Some(format!("workspaces: could not save one: {err}"));
                    stored.extend(was(uid).cloned()); // what the file still holds, if there is one
                }
            }
        }
        for (uid, text) in before.iter().filter(|(uid, _)| !now.iter().any(|(kept, _)| kept == uid)) {
            if let Err(err) = config::remove_workbook(uid) {
                failed = Some(format!("workspaces: could not delete one: {err}"));
                stored.push((uid.clone(), text.clone()));
            }
        }
        (self.kept_workbooks.stored, self.kept_workbooks.unsaved) = (stored, failed);
    }

    /// Sends a workbook action as whoever is acting. `false`: the core would drop it, for who
    /// sent it or behind what has the keyboard, and the status bar says which.
    fn workbook_act(&mut self, action: Action, window: &Window, cx: &mut Context<Self>) -> bool {
        if let Some(why) = self.model.refusal(&action, self.input).or(self.model.waits_for(&action, self.input)) {
            self.error = Some(why.into());
            cx.notify();
            return false;
        }
        // The offer to take a delete or an update back counts its seconds from now.
        self.model.update(Action::Tick { now: now_ms() }.sent_by(Origin::System));
        self.dispatch(action, self.input, window, cx);
        true
    }

    /// + on the top row, in the section's header, and the empty section's words: a new workbook
    /// of no tab, its space open with one empty tab. Never the tab on show.
    pub(crate) fn new_workspace(&mut self, window: &Window, cx: &mut Context<Self>) {
        let _ = self.workbook_act(Action::WorkbookNew { uid: marks::new_uid() }, window, cx);
    }

    /// ⌘⇧S and the menu: saves the tab on show, or updates the workbook it came from.
    pub(crate) fn save_workbook(&mut self, window: &Window, cx: &mut Context<Self>) {
        let refused = self.model.workbook_save_refusal();
        let action = Action::WorkbookSave { uid: marks::new_uid(), cwds: self.cwds(), histories: self.histories() };
        if self.workbook_act(action, window, cx) {
            self.error = refused.or_else(|| self.model.workbook_saved_note());
        }
    }

    /// File ▸ Save All Tabs as Workbook: the tabs on the bar, as one new workbook.
    pub(crate) fn save_all_workbook(&mut self, window: &Window, cx: &mut Context<Self>) {
        let refused = self.model.workbook_save_all_refusal();
        let (uid, cwds, histories) = (marks::new_uid(), self.cwds(), self.histories());
        let action = Action::WorkbookSaveAll { uid, cwds, histories };
        if self.workbook_act(action, window, cx) {
            self.error = refused.or_else(|| self.model.workbook_saved_note());
        }
    }

    /// File ▸ Add Tab to Workbook…: the section in list mode, to pick the workbook with A.
    pub(crate) fn pick_workbook(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.model.workbooks.list.is_empty() {
            return self.save_workbook(window, cx);
        }
        if self.model.sidebar != Some(Section::Workbook) {
            self.dispatch(Action::ToggleSection { section: Section::Workbook }, self.input, window, cx);
        }
        if self.model.list.is_none() {
            self.dispatch(Action::FocusList, self.input, window, cx);
        }
        self.error = Some("pick a workspace: A adds this tab to it".into());
        cx.notify();
    }

    /// A click on a workbook's row or one of its tabs' rows, by the control's name: `workbook`
    /// opens it, `workbook-fold` lists its tabs, `workbook-menu` opens its menu (Rename, Delete);
    /// `workbook-tab` and `workbook-tab-menu` do the same to its tab `tab`. `workbook-add` (`a`
    /// in list mode) adds the tab on show to it.
    pub(crate) fn click_workbook(
        &mut self,
        control: &str,
        id: WorkbookId,
        tab: Option<usize>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let tab = tab.filter(|_| control.starts_with("workbook-tab"));
        match control {
            "workbook-menu" | "workbook-tab-menu" => {
                // By a label or a key: the keys are on its first item.
                let row = tab.map_or(ListRow::Workbook(id), |at| ListRow::WorkbookTab(id, at));
                self.open_row_menu(row, true, window, cx);
            }
            "workbook-add" => {
                let refused = self.model.workbook_add_refusal(id);
                let (cwds, histories) = (self.cwds(), self.histories());
                let action = Action::WorkbookAddTab { workbook: id, cwds, histories };
                if self.workbook_act(action, window, cx) {
                    self.error = refused.or_else(|| self.model.workbook_saved_note());
                }
            }
            // Only what the bar lists changes: nothing written, nothing opened.
            "workbook-fold" => drop(self.dispatch(Action::WorkbookFold { workbook: id }, self.input, window, cx)),
            _ => self.open_workbook(id, tab, window, cx),
        }
    }

    /// Opens a workbook's tabs (`tab`: that one), or selects them. The status bar says which
    /// directory is gone (its terminal starts in the user's folder) and which window could not be hosted.
    fn open_workbook(&mut self, id: WorkbookId, tab: Option<usize>, window: &Window, cx: &mut Context<Self>) {
        let name = self.model.workbooks.get(id).map(|book| book.name.clone()).unwrap_or_default();
        let gone = self.model.workbook_gone(id, tab);
        let home = Some(config::user_home()).filter(|home| home.starts_with('/'));
        self.error = None;
        if !self.workbook_act(Action::WorkbookOpen { workbook: id, tab, home }, window, cx) || gone.is_empty() {
            return;
        }
        let gone = format!("{name}: {} is gone; its terminal starts in your home folder", gone.join(", "));
        // A window that could not be hosted: `carry_out` left the reason in `error`.
        self.error = Some(self.error.take().map_or(gone.clone(), |unhosted| format!("{gone}; {unhosted}")));
    }

    /// A click on "Updated · Undo" or "Workspace deleted · Undo", or its label.
    pub(crate) fn undo_workbook(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.workbook_act(Action::WorkbookUndo, window, cx);
    }

    /// The workbooks as the state lists them: each row, and where it, its tabs' rows and their
    /// words were drawn.
    pub(crate) fn workbooks_json(&self) -> Vec<serde_json::Value> {
        let drawn = |id: String| self.controls.bounds(&id).map(rect);
        let row = |row: WorkbookRow| {
            let id = row.id;
            let mut value = serde_json::json!(row);
            value["row"] = serde_json::json!(drawn(format!("workbook:{id}")));
            // `menu`: its …, null while it does not show (no pointer on the row, not selected, no menu open).
            let shown = |row: ListRow, control: String| drawn(control).filter(|_| self.dots_shown(&row));
            value["buttons"] = serde_json::json!({
                "fold": drawn(format!("workbook-fold:{id}")),
                "menu": shown(ListRow::Workbook(id), format!("workbook-menu:{id}")),
            });
            for (at, tab) in value["tabs"].as_array_mut().into_iter().flatten().enumerate() {
                tab["row"] = serde_json::json!(drawn(tab_control("workbook-tab", id, at)));
                let menu = shown(ListRow::WorkbookTab(id, at), tab_control("workbook-tab-menu", id, at));
                tab["buttons"] = serde_json::json!({ "menu": menu });
            }
            value
        };
        self.model.workbook_rows().into_iter().map(row).collect()
    }

    /// The name of the workbook open tab `tab` stands for: the tab bar marks it.
    pub(crate) fn workbook_of(&self, tab: u32) -> Option<String> {
        let (id, _) = self.model.workbooks.entry_of(tab)?;
        self.model.workbooks.get(id).map(|book| book.name.clone())
    }

    /// The pointer came onto a row, or left it: its … shows while it is there.
    fn row_hover(&mut self, row: ListRow, over: bool, cx: &mut Context<Self>) {
        if over {
            self.row_hovered = Some(row);
        } else if self.row_hovered.as_ref() == Some(&row) {
            self.row_hovered = None;
        }
        cx.notify();
    }

    /// Whether a row's … shows: the pointer is on the row, list mode's selection is, or its
    /// menu is open.
    fn dots_shown(&self, row: &ListRow) -> bool {
        let selected = self.model.list.and_then(|at| self.model.list_rows().into_iter().nth(at));
        let open = self.model.row_menu().is_some_and(|(menu, _)| menu.row == *row);
        self.row_hovered.as_ref() == Some(row) || selected.as_ref() == Some(row) || open
    }

    /// The name being typed, or `name` cut to fit.
    fn workbook_name(&self, renaming: bool, name: String, cx: &mut Context<Self>) -> Div {
        match renaming {
            // The name being typed: the text field.
            true => div().flex().text_color(rgb(theme::MARK)).child(self.render_field("name", cx)),
            false => crate::cut_text(name),
        }
    }

    /// The Workspaces section: each workbook's name and what its tabs hold, and the tabs of an
    /// unfolded one under it. A click opens; hover (or list mode's selection) shows its ….
    pub(crate) fn render_workbooks(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.model.workbook_rows();
        let empty = rows.is_empty().then(|| {
            let new = div()
                .id("workbook-new")
                .relative()
                .cursor_pointer()
                .text_color(rgb(theme::PRIMARY))
                .child("New workspace")
                .child(self.controls.mark("workbook-new"))
                .on_click(cx.listener(|this, _, window, cx| this.new_workspace(window, cx)));
            let line = div().px_2().flex().flex_wrap().gap_1().text_xs().text_color(rgb(theme::MUTED));
            line.child("No workspaces yet —").child(new)
        });
        let typing = |id: WorkbookId, at: Option<usize>| matches!(&self.model.mark_input, Some(MarkInput::Workbook { workbook, tab, .. }) if *workbook == id && *tab == at);
        // The leading mark: filled while it is open, a ring when it is not.
        let dot = |open: bool| {
            let dot = div().flex_none().size(px(8.)).rounded_full();
            if open { dot.bg(rgb(theme::PICK)) } else { dot.border_1().border_color(rgb(theme::BORDER_STRONG)) }
        };
        // What it holds, muted at the row's end, its first part (`3 tabs` of `3 tabs · 2 terminals`):
        // all of it on hover. Gone while the ⋯ shows in its place, or the name is typed.
        let meta = |summary: &str, hidden: bool| {
            let short = summary.split(" · ").next().unwrap_or_default().to_string();
            (!hidden).then(|| crate::row_meta(short))
        };
        let tip = |summary: String| {
            move |_: &mut gpui::Window, cx: &mut gpui::App| {
                gpui::AnyView::from(cx.new(|_| crate::Tooltip(summary.clone().into())))
            }
        };
        let mut children = Vec::new();
        let mut index = 0;
        for row in rows {
            let (id, selected) = (row.id, self.model.list == Some(index));
            index += 1;
            let renaming = typing(id, None);
            // ▸ or ▾ before a workbook of more than one tab: lists its tabs under it.
            let fold = (row.tabs.len() > 1).then(|| {
                let control = format!("workbook-fold:{id}");
                div()
                    .id(ElementId::Name(control.clone().into()))
                    .relative()
                    .flex_none()
                    .w(px(12.))
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .hover(|s| s.text_color(rgb(theme::TEXT)))
                    .child(if row.unfolded { "▾" } else { "▸" })
                    .child(self.controls.mark(control))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.click_workbook("workbook-fold", id, None, window, cx);
                    }))
            });
            let name = self.workbook_name(renaming, row.name.clone(), cx);
            // Bound to a task: the task's badge, where the dot is.
            let lead = match row.task.and_then(|task| self.task_badge(task, 16.)) {
                Some(badge) => badge.flex_none(),
                None => dot(row.open),
            };
            let shown = self.dots_shown(&ListRow::Workbook(id));
            let dots = self.row_dots(ListRow::Workbook(id), shown, cx);
            let book = crate::sidebar_row(ElementId::Name(format!("workbook:{id}").into()), selected)
                .tooltip(tip(row.summary.clone()))
                .on_hover(cx.listener(move |this, over: &bool, _, cx| this.row_hover(ListRow::Workbook(id), *over, cx)))
                .children(fold)
                .child(lead)
                .child(name.flex_1().min_w_0())
                .children(meta(&row.summary, shown || renaming))
                .child(self.controls.mark(format!("workbook:{id}")))
                .children(dots.filter(|_| !renaming))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.click_workbook("workbook", id, None, window, cx)),
                );
            children.push(book.into_any_element());
            if !row.unfolded {
                continue;
            }
            let mut tabs = Vec::new();
            for (at, tab) in row.tabs.into_iter().enumerate() {
                let selected = self.model.list == Some(index);
                index += 1;
                let renaming = typing(id, Some(at));
                let row = ListRow::WorkbookTab(id, at);
                let shown = self.dots_shown(&row);
                let dots = self.row_dots(row.clone(), shown, cx);
                let name = self.workbook_name(renaming, tab.title, cx);
                let control = tab_control("workbook-tab", id, at);
                let tab_row = crate::sidebar_row(ElementId::Name(control.clone().into()), selected)
                    .tooltip(tip(tab.summary.clone()))
                    .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                        this.row_hover(ListRow::WorkbookTab(id, at), *over, cx)
                    }))
                    .ml_4()
                    .child(dot(tab.open))
                    .child(name.flex_1().min_w_0())
                    .children(meta(&tab.summary, shown || renaming))
                    .child(self.controls.mark(control))
                    .children(dots.filter(|_| !renaming))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.click_workbook("workbook-tab", id, Some(at), window, cx)
                    }));
                tabs.push(tab_row.into_any_element());
            }
            children.extend(crate::well(tabs, 4.));
        }
        // After a delete or an update, a line at the top offers to take it back, as Tasks does.
        let notice = self.model.workbooks.offered().map(|change| {
            self.undo_line("workbook-undo", change.title())
                .on_click(cx.listener(|this, _, window, cx| this.undo_workbook(window, cx)))
        });
        let list = div().id("workbooks").flex_1().flex().flex_col().gap_1().p_2().overflow_y_scroll();
        let list = list.track_scroll(&self.list_scroll).children(notice).children(empty).children(children);
        self.bars.tracked("workbooks", list, &self.list_scroll)
    }
}
