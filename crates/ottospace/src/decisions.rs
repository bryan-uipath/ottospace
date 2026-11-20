//! The Decisions card pane (`decisions` flag): the task, why it is first, the ask card as the
//! drawer draws it, and Skip and Later. Its keys are the core's (`Model::decision_key`). The
//! conversation is the drawer's: ⌘L talks the card over.

use gpui::{AnyElement, Context, Div, FontWeight, KeyDownEvent, Window, div, prelude::*, px, rgb};
use ottospace_core::model::chat::Key;
use ottospace_core::model::decisions::LATER_MS;
use ottospace_core::talk::Subject;
use ottospace_core::{Action, theme};

use crate::Workspace;
use crate::ui::{self, Button, Face, State};

/// The card's column at most this wide, in the middle of its pane.
const WIDTH: f32 = 720.;

impl Workspace {
    /// The card pane's body: its header, the ask card, its foot.
    pub(crate) fn decision_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(run) = self.model.decisions.as_ref() else { return div().into_any_element() };
        let (task, ticket) = (run.task, run.ticket);
        let subject = Subject::Task(task);
        let chat = self.model.chat(&subject);
        let card = chat.and_then(|chat| Some((chat, chat.form()?)));
        let card = card.map(|(chat, form)| self.ask_card(&subject, chat, form, ticket, Some(task), cx));
        let title = self.model.tasks.get(task).map(|t| t.title.clone()).unwrap_or_default();
        // Why it is first: from the queue's inputs alone. Its own entry: a held card may not be first.
        let why = run.queue.iter().find(|w| (w.task, w.ticket) == (task, ticket)).map(|own| {
            let age = ottospace_core::tasks::age(own.since, crate::now_ms());
            let waiting = if age == "now" { "just asked".to_string() } else { format!("waiting {age}") };
            format!("P{} · {waiting} · its agent asks", own.priority)
        });
        let header = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .pt_3()
            .pb_2()
            .children(self.task_badge(task, 22.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(crate::cut_text(title).text_sm().font_weight(FontWeight::SEMIBOLD))
                    .children(why.map(|why| div().text_xs().text_color(rgb(theme::MUTED)).child(why))),
            )
            .child(div().text_xs().text_color(rgb(theme::MUTED)).child(format!("{} of {}", run.at, run.of)));
        let skip = ui::button("decision-skip", Button::Ghost(Face::Label("Skip (s)".into())), State::Rest)
            .child(self.controls.mark("decision:skip"))
            .on_click(cx.listener(move |this, _, window, cx| {
                drop(this.dispatch(Action::DecisionsSkip { task, ticket }, this.input, window, cx))
            }));
        let later = ui::button("decision-later", Button::Ghost(Face::Label("Later (l)".into())), State::Rest)
            .child(self.controls.mark("decision:later"))
            .on_click(cx.listener(move |this, _, window, cx| {
                let until = crate::now_ms().saturating_add(LATER_MS);
                drop(this.dispatch(Action::DecisionsLater { task, ticket, until }, this.input, window, cx))
            }));
        // Skip and Later first: in a narrow pane the hint wraps under them, never pushes them out.
        let foot = div()
            .flex_none()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .px_3()
            .pb_3()
            .child(skip)
            .child(later)
            .child(div().text_xs().text_color(rgb(theme::MUTED)).child("1–9 answer · ⌘L talk it over · Esc close"));
        div()
            .size_full()
            .flex()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(WIDTH))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(header)
                    .children(card.map(|card| card.flex_shrink().min_h_0()))
                    .child(foot),
            )
            .into_any_element()
    }

    /// A key while the card pane has the keyboard. True: it was the card's.
    pub(crate) fn decision_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) -> bool {
        let key = &event.keystroke;
        // Held, only a move repeats: a held 1, ↵ or S acts once, not on the next card too.
        if event.is_held && !matches!(key.key.as_str(), "up" | "down" | "j" | "k") {
            return true;
        }
        match self.model.decision_key(&key.key, crate::mods(key.modifiers), crate::now_ms()) {
            Key::Act(action) => {
                self.dispatch(action, self.input, window, cx);
                cx.notify();
                true
            }
            Key::Swallow | Key::Field | Key::Newline => true,
            Key::Pass => false,
        }
    }

    /// Decisions' progress on its row in the top row: a thin bar along its top, `at - 1` of `of` cards done.
    pub(crate) fn decisions_progress(&self) -> Option<Div> {
        let run = self.model.decisions.as_ref()?;
        let done = (run.at.saturating_sub(1)) as f32 / run.of.max(1) as f32;
        let track = div().absolute().top_0().left_0().right_0().h(px(2.)).bg(rgb(theme::BORDER));
        Some(track.child(div().h_full().w(gpui::relative(done)).bg(rgb(theme::PICK))))
    }
}
