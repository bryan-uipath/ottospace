//! Shell half of "btw" (`ottospace_core::model::btw`): the column of side questions left of the
//! conversation, inside the drawer, and the composer's "btw" control. Answers are what a model
//! wrote: drawn as a reply is, never in the state. The core decides; this draws.

use gpui::{AnyElement, Context, Div, MouseButton, Stateful, div, prelude::*, px, rgb};
use ottospace_core::model::btw::{Btw, Reply};
use ottospace_core::talk::Subject;
use ottospace_core::{Action, theme};

use crate::Workspace;
use crate::conversation::Links;
use crate::markdown;

/// The column's width: the drawer widens by it while it shows.
pub(crate) const WIDTH: f32 = 300.;

/// What the state says of a conversation's btw: whether it is offered, and its thread's shape.
/// Never a question or an answer.
pub(crate) fn snapshot(model: &ottospace_core::Model, subject: &Subject) -> serde_json::Value {
    let btw = model.chat(subject).and_then(|chat| chat.btw.as_ref());
    let count = |of: fn(&Reply) -> bool| btw.map_or(0, |btw| btw.pairs.iter().filter(|p| of(&p.reply)).count());
    serde_json::json!({
        "offered": model.btw_offered(subject),
        "open": btw.is_some(),
        "keys": btw.is_some_and(|btw| btw.keys),
        "asked": btw.map_or(0, |btw| btw.pairs.len()),
        "answered": count(|reply| matches!(reply, Reply::Answer { synthetic: false, .. })),
        "synthetic": count(|reply| matches!(reply, Reply::Answer { synthetic: true, .. })),
        "failed": count(|reply| *reply == Reply::Failed),
        "waiting": btw.is_some_and(|btw| btw.asking().is_some()),
    })
}

impl Workspace {
    /// The btw column is drawn: open, and the conversation shows (not its brief, nor the editor).
    pub(crate) fn btw_drawn(&self) -> bool {
        let talk = self.model.talk.as_ref();
        let brief = self.model.brief.is_some() && self.model.brief == talk.and_then(|talk| talk.task.task());
        self.model.shown_btw().is_some() && !brief && !self.model.editor_shown()
    }

    /// The composer's "btw": only where the conversation offers one and its column is closed.
    pub(crate) fn btw_button(&self, subject: &Subject, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let open = self.model.chat(subject).is_some_and(|chat| chat.btw.is_some());
        (self.model.btw_offered(subject) && !open).then(|| {
            self.talk_button("chat-btw", "btw", "Ask on the side · ⌘⇧K").on_click(
                cx.listener(|this, _, window, cx| drop(this.dispatch(Action::BtwOpen, this.input, window, cx))),
            )
        })
    }

    /// The btw column of the shown conversation, if open: its header, the thread, its input.
    pub(crate) fn btw_column(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let btw = self.model.shown_btw().filter(|_| self.btw_drawn())?;
        let subject = self.model.shown_chat()?.subject.clone();
        let close = self
            .talk_button("btw-close", "×", "Close")
            .on_click(cx.listener(|this, _, window, cx| drop(this.dispatch(Action::BtwClose, this.input, window, cx))));
        let header = div()
            .h(px(crate::TOP_BAR))
            .flex_none()
            .pl_3()
            .pr_1()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .child(div().flex_1().text_sm().text_color(rgb(theme::MUTED)).child("btw · read-only"))
            .child(close);
        let pairs = btw.pairs.iter().map(|pair| self.btw_pair(&subject, pair, cx));
        let list = div()
            .id("btw-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap_3()
            .children(pairs);
        let column = div()
            .id("btw")
            .flex_none()
            .w(px(WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .child(header)
            .child(list)
            .child(self.btw_input(btw, cx));
        Some(column.into_any_element())
    }

    /// One question, and its answer as markdown with "Use as my answer"; or that it waits or failed.
    fn btw_pair(&self, subject: &Subject, pair: &ottospace_core::model::btw::Pair, cx: &mut Context<Self>) -> Div {
        let question = div()
            .rounded_md()
            .bg(rgb(theme::SELECTED))
            .px_2()
            .py_1()
            .text_sm()
            .child(ottospace_core::transcript::clean(&pair.question));
        let muted = |text: &'static str| div().text_xs().text_color(rgb(theme::MUTED)).child(text);
        let answer = match &pair.reply {
            Reply::Waiting => muted("…"),
            Reply::Failed => muted("No answer"),
            Reply::Answer { text, synthetic, .. } => {
                let blocks = markdown::parse(text.text(), self.repo_of(subject).as_deref());
                let (at, of) = (format!("btw:{}:{}", crate::talk::key(subject), pair.id), Links::Chat(subject.clone()));
                let id = pair.id;
                let used = self
                    .word(
                        format!("talk:btw-use:{id}"),
                        ("Use as my answer", "Into the open question's Other…, else the composer; not sent"),
                        move |this, window, cx| drop(this.dispatch(Action::BtwUse { id }, this.input, window, cx)),
                        cx,
                    )
                    .text_color(rgb(theme::PRIMARY));
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .text_sm()
                    .children(synthetic.then(|| muted("From the CLI, not the model")))
                    .children(blocks.iter().enumerate().map(|(n, block)| self.block(&of, &at, n, block, cx)))
                    .child(div().flex().child(used))
            }
        };
        div().flex().flex_col().gap_1().child(question).child(answer)
    }

    /// The column's input: a press gives it the keyboard; ↵ asks, one question at a time.
    fn btw_input(&self, btw: &Btw, cx: &mut Context<Self>) -> Stateful<Div> {
        let placeholder = if btw.asking().is_some() { "Waiting for the answer…" } else { "Ask on the side" };
        div()
            .id("btw-input")
            .flex_none()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(rgb(theme::BORDER))
            .text_sm()
            .child(self.controls.mark("talk:btw-input".to_string()))
            .child(self.render_field_of("btw", placeholder, cx))
            // Before the field's own press, so the press lands in this field, not the composer.
            .capture_any_mouse_down(cx.listener(|this, _, window, cx| {
                // The keys leave the transcript: Esc and ⌘A are the input's.
                if let Some(subject) = this.model.shown_chat().map(|chat| chat.subject.clone()) {
                    this.transcript_leave(&subject);
                }
                this.dispatch(Action::Leave, this.input, window, cx);
                this.dispatch(Action::BtwKeys { on: true }, this.input, window, cx);
            }))
            // Not the drawer's press, which gives the composer the keyboard.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    }
}
