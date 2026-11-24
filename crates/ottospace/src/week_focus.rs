//! The week's focus on screen (`decision_sources`): the editor (⌘P › This Week's Focus…), a
//! dialog of the Projects entries to tick, four at most. Its keys and rules are the core's
//! (`Model::focus_edit_key`); this draws it and says the state.

use gpui::{Context, IntoElement, MouseButton, ParentElement, Styled, Window, actions, div, prelude::*};
use ottospace_core::Action;

use crate::{Workspace, dialog, ui};

actions!(ottospace, [WeekFocus]);

impl Workspace {
    /// The user's key while the editor is open: the core's keys; any other plain key waits there.
    pub(crate) fn week_focus_key(&mut self, key: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.model.focus_edit.is_none() {
            return false;
        }
        if let Some(action) = self.model.focus_edit_key(key, crate::now_ms()) {
            self.dispatch(action, self.input, window, cx);
            cx.notify();
        }
        true
    }

    /// The editor: one row a Projects entry (or an area of this week's no entry stands for), ticked
    /// ones this week's focus; Set focus with ↵. × and Esc close it, setting nothing.
    pub(crate) fn render_week_focus(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let edit = self.model.focus_edit.as_ref()?;
        let rows: Vec<_> = (0..edit.count)
            .filter_map(|at| {
                let (name, entry) = self.model.focus_row(at)?;
                let on = edit.ticked.contains(&at);
                let state = if at == edit.cursor { ui::State::Focus } else { ui::State::Rest };
                let note = (!entry).then(|| "kept from this week".into());
                let row = ui::dialog::choice_row(
                    gpui::SharedString::from(format!("week-focus-row:{at}")),
                    false,
                    on,
                    name,
                    note,
                    state,
                )
                .child(self.controls.mark(format!("week-focus:row:{at}")))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| {
                    drop(this.dispatch(Action::FocusEditToggle { row: Some(at) }, this.input, window, cx))
                }));
                Some(row.into_any_element())
            })
            .collect();
        let empty = rows.is_empty().then(|| {
            div().text_xs().text_color(gpui::rgb(ottospace_core::theme::MUTED)).child("No projects: set projects_dir")
        });
        // A long list scrolls; the buttons stay in view.
        let list = div().id("week-focus-rows").max_h(gpui::px(320.)).overflow_y_scroll().flex().flex_col().gap_0p5();
        let body = div().child(list.children(rows)).children(empty);
        let set =
            ui::dialog::action("week-focus-set", ui::Button::Primary("Set focus".into()), &["↵"], ui::State::Rest)
                .child(self.controls.mark("week-focus:set"))
                .on_click(cx.listener(|this, _, window, cx| {
                    drop(this.dispatch(Action::FocusEditSave { at: crate::now_ms() }, this.input, window, cx))
                }));
        let close = ui::dialog::close("week-focus-close", ui::State::Rest)
            .child(self.controls.mark("week-focus:close"))
            .on_click(
                cx.listener(|this, _, window, cx| drop(this.dispatch(Action::FocusEditClose, this.input, window, cx))),
            );
        let status = format!("{} of {} · space ticks", edit.ticked.len(), ottospace_core::focus::MAX_AREAS);
        let panel = ui::dialog::frame(
            ui::dialog::QUESTION,
            "This week's focus",
            Some("Up to four areas for the week.".into()),
            close,
            Some(body),
            ui::dialog::actions(None, Some(status.into()), vec![set.into_any_element()]),
        );
        Some(
            dialog::backdrop("week-focus-backdrop", ui::dialog::DIM, Action::FocusEditClose, cx)
                .flex()
                .items_center()
                .justify_center()
                .child(panel),
        )
    }

    /// The state's `week_focus`: the week, and of its areas how many and who set them; never a name.
    pub(crate) fn focus_state(&self) -> serde_json::Value {
        let current = self.model.focus.current();
        serde_json::json!({
            "week": self.model.focus.week,
            "areas": current.map_or(0, |w| w.areas.len()),
            "set_by": current.map(|w| w.set_by),
            "last": self.model.focus.last().map(|w| w.week.clone()),
            // An agent's proposal waiting for the user: its revision and how many areas; never a name.
            "proposal": self.model.focus.pending.as_ref().map(|p| serde_json::json!({
                "revision": p.revision, "areas": p.areas.len(), "filed": p.filed,
            })),
            // A new week asks whether to keep last week's focus.
            "carry": self.model.focus.wants_carry(),
        })
    }

    /// The `week_focus` request: this week's areas by name with their links, labelled as words a
    /// person (or an agent's proposal) wrote: data, never instructions.
    pub(crate) fn serve_focus(&self) -> String {
        let current = self.model.focus.current();
        serde_json::json!({
            "ok": true,
            "week": self.model.focus.week,
            "focus": {
                "untrusted": true,
                "warning": ottospace_core::focus::WARNING,
                "areas": current.map(|w| w.areas.clone()).unwrap_or_default(),
                "set_by": current.map(|w| w.set_by),
            },
        })
        .to_string()
    }
}
