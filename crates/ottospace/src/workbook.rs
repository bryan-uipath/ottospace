//! Workbooks in the shell: the Workbook section and the files in `~/.ottospace/workbooks`.
//! What one holds and what opening it does are the core's (`ottospace_core::workbook`).

use gpui::{AnyView, Context, ElementId, Window, div, prelude::*, px, rgb};
use ottospace_core::marks::MarkInput;
use ottospace_core::workbook::{self, WorkbookId, WorkbookRow};
use ottospace_core::{Action, Origin, theme};

use crate::{Tooltip, Workspace, config, list_button, marks, now_ms, rect};

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

impl Workspace {
    /// Reads the workbooks folder at start. A file that does not read is left as it is, and
    /// counted in the status bar: no other workbook's write touches it.
    pub(crate) fn restore_workbooks(&mut self) {
        let kept = config::load_workbooks();
        let read = |(uid, text): &(String, String)| workbook::parse(text, uid).ok();
        let workbooks: Vec<workbook::Workbook> = kept.files.iter().filter_map(read).collect();
        let unread = kept.unread + kept.files.len() - workbooks.len();
        let unread = (unread > 0).then(|| format!("workbooks: {unread} file(s) could not be read; left as they are"));
        self.kept_workbooks.notice = kept.error.or(unread);
        self.model.update(Action::RestoreWorkbooks { workbooks }.sent_by(Origin::System));
        self.kept_workbooks.stored = self.workbook_files(); // what was read needs no writing back
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
                    failed = Some(format!("workbooks: could not save one: {err}"));
                    stored.extend(was(uid).cloned()); // what the file still holds, if there is one
                }
            }
        }
        for (uid, text) in before.iter().filter(|(uid, _)| !now.iter().any(|(kept, _)| kept == uid)) {
            if let Err(err) = config::remove_workbook(uid) {
                failed = Some(format!("workbooks: could not delete one: {err}"));
                stored.push((uid.clone(), text.clone()));
            }
        }
        (self.kept_workbooks.stored, self.kept_workbooks.unsaved) = (stored, failed);
    }

    /// Sends a workbook action as whoever is acting. `false`: the core would drop it, for who
    /// sent it or behind what has the keyboard, and the status bar says which.
    fn workbook_act(&mut self, action: Action, window: &Window, cx: &mut Context<Self>) -> bool {
        if let Some(why) = self.model.refusal(&action, self.input).or(self.model.waits(&action)) {
            self.error = Some(why.into());
            cx.notify();
            return false;
        }
        // The offer to take a delete or an update back counts its seconds from now.
        self.model.update(Action::Tick { now: now_ms() }.sent_by(Origin::System));
        self.dispatch(action, self.input, window, cx);
        true
    }

    /// ⌘⇧S, the menu, the header's +: saves the tab on show, or updates the workbook it came from.
    pub(crate) fn save_workbook(&mut self, window: &Window, cx: &mut Context<Self>) {
        let refused = self.model.workbook_save_refusal();
        let action = Action::WorkbookSave { uid: marks::new_uid(), cwds: self.cwds() };
        if self.workbook_act(action, window, cx) {
            self.error = refused;
        }
    }

    /// A click on a workbook's row or on one of its words, by the control's name: `workbook`
    /// and `workbook-open` open it, `workbook-rename` types its name, `workbook-delete` deletes it.
    pub(crate) fn click_workbook(&mut self, control: &str, id: WorkbookId, window: &Window, cx: &mut Context<Self>) {
        match control {
            "workbook-rename" => drop(self.workbook_act(Action::WorkbookEdit { workbook: id }, window, cx)),
            "workbook-delete" => drop(self.workbook_act(Action::WorkbookRemove { workbook: id }, window, cx)),
            _ => self.open_workbook(id, window, cx),
        }
    }

    /// Opens a workbook, or selects its open tab. The status bar says which directory is
    /// gone (its terminal starts in the user's folder) and which window could not be hosted.
    fn open_workbook(&mut self, id: WorkbookId, window: &Window, cx: &mut Context<Self>) {
        let name = self.model.workbooks.get(id).map(|book| book.name.clone()).unwrap_or_default();
        let gone = self.model.workbook_gone(id);
        let home = Some(config::user_home()).filter(|home| home.starts_with('/'));
        self.error = None;
        if !self.workbook_act(Action::WorkbookOpen { workbook: id, home }, window, cx) || gone.is_empty() {
            return;
        }
        let gone = format!("{name}: {} is gone; its terminal starts in your home folder", gone.join(", "));
        // A window that could not be hosted: `carry_out` left the reason in `error`.
        self.error = Some(self.error.take().map_or(gone.clone(), |unhosted| format!("{gone}; {unhosted}")));
    }

    /// A click on "Updated · Undo" or "Workbook deleted · Undo", or its label.
    pub(crate) fn undo_workbook(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.workbook_act(Action::WorkbookUndo, window, cx);
    }

    /// The workbooks as the state lists them: each row, and where it and its words were drawn.
    pub(crate) fn workbooks_json(&self) -> Vec<serde_json::Value> {
        let drawn = |id: String| self.controls.bounds(&id).map(rect);
        let row = |row: WorkbookRow| {
            let id = row.id;
            let mut value = serde_json::json!(row);
            value["row"] = serde_json::json!(drawn(format!("workbook:{id}")));
            value["buttons"] = serde_json::json!({
                "open": drawn(format!("workbook-open:{id}")),
                "rename": drawn(format!("workbook-rename:{id}")),
                "delete": drawn(format!("workbook-delete:{id}")),
            });
            value
        };
        self.model.workbook_rows().into_iter().map(row).collect()
    }

    /// The Workbook section: each workbook's name and what its tab holds. A click opens it;
    /// hover (or list mode's selection) shows `Open`, `Rename` and `Delete`.
    pub(crate) fn render_workbooks(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.model.workbook_rows();
        let empty = rows.is_empty().then(|| {
            let save = div()
                .id("workbook-save")
                .relative()
                .cursor_pointer()
                .text_color(rgb(theme::PRIMARY))
                .child("Save this tab")
                .child(self.controls.mark("workbook-save"))
                .on_click(cx.listener(|this, _, window, cx| this.save_workbook(window, cx)));
            let line = div().px_2().flex().flex_wrap().gap_1().text_xs().text_color(rgb(theme::MUTED));
            line.child("No workbooks yet —").child(save)
        });
        let word =
            |name: &'static str, label: &'static str, tip: &'static str, id: WorkbookId, cx: &mut Context<Self>| {
                div()
                    .id(ElementId::Name(format!("{name}:{id}").into()))
                    .relative()
                    .child(self.controls.mark(format!("{name}:{id}")))
                    .px_1()
                    .rounded_sm()
                    .text_xs()
                    .text_color(rgb(theme::TEXT))
                    .hover(|s| s.bg(rgb(theme::BORDER_STRONG)))
                    .child(label)
                    .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.into()))))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation(); // not a click on the row
                        this.click_workbook(name, id, window, cx);
                    }))
            };
        let rows = rows.into_iter().enumerate().map(|(index, row)| {
            let id = row.id;
            let selected = self.model.list == Some(index);
            let renaming =
                matches!(&self.model.mark_input, Some(MarkInput::Workbook { workbook, .. }) if *workbook == id);
            let name = match renaming {
                // The name being typed: the text field.
                true => div().flex().text_color(rgb(theme::MARK)).child(self.render_field("name", cx)),
                false => div().truncate().child(row.name.clone()),
            };
            // A dot while its tab is open.
            let dot = div().flex_none().size(px(6.)).rounded_full().when(row.open, |d| d.bg(rgb(theme::PICK)));
            let title = div().w_full().flex().items_center().gap_2().child(name.flex_1().min_w_0()).child(dot);
            let summary = div().w_full().truncate().text_xs().text_color(rgb(theme::MUTED)).child(row.summary);
            // Over the end of the row, so the name keeps its room while they are hidden.
            let words = div()
                .absolute()
                .right(px(6.))
                .top_0()
                .bottom_0()
                .flex()
                .items_center()
                .gap_0p5()
                .pl_1()
                .bg(rgb(theme::SELECTED))
                .when(!selected, |d| d.invisible().group_hover("workbook-row", |s| s.visible()))
                .child(word("workbook-open", "Open", "Open it in a tab (↵)", id, cx))
                .child(word("workbook-rename", "Rename", "Type its name (R)", id, cx))
                .child(word("workbook-delete", "Delete", "Delete it (X)", id, cx));
            list_button(("workbook", id as usize), selected)
                .group("workbook-row")
                .relative()
                .flex_col()
                .items_start()
                .gap_0p5()
                .child(title)
                .child(summary)
                .child(self.controls.mark(format!("workbook:{id}")))
                .when(!renaming, |d| d.child(words))
                .on_click(cx.listener(move |this, _, window, cx| this.click_workbook("workbook", id, window, cx)))
        });
        let rows: Vec<_> = rows.collect();
        // After a delete or an update, a line at the top offers to take it back, as Tasks does.
        let notice = self.model.workbooks.offered().map(|change| {
            self.undo_line("workbook-undo", change.title())
                .on_click(cx.listener(|this, _, window, cx| this.undo_workbook(window, cx)))
        });
        let list = div().id("workbooks").flex_1().flex().flex_col().gap_1().p_2().overflow_y_scroll();
        let list = list.track_scroll(&self.list_scroll).children(notice).children(empty).children(rows);
        self.bars.tracked("workbooks", list, &self.list_scroll)
    }
}
