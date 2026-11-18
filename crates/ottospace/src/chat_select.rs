//! Selecting text in a native transcript: the shell half of `ottospace_core::selection`. Each
//! text the transcript draws notes its layout as it is laid out; a press, a drag, ⌘A and ⌘C ask
//! the core's `Selection` over them. Only the user's ⌘C or Copy puts text on the clipboard.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, ClickEvent, Context, DispatchPhase, HighlightStyle, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, SharedString, StyledText, TextLayout, Window, canvas, div, prelude::*, rgb,
};
use ottospace_core::selection::{self, Selection};
use ottospace_core::talk::Subject;
use ottospace_core::{Origin, theme};

use crate::Workspace;

/// Why a client's ⌘C or Copy in a conversation puts nothing on the clipboard.
pub(crate) const USERS_COPY: &str = "only the user copies from a conversation";

/// One text as the last frame laid it out.
struct Laid {
    key: String,
    text: SharedString,
    layout: TextLayout,
}

/// The texts of the conversation drawn last, in order: what a press finds, and ⌘C copies from.
#[derive(Default)]
struct Drawn {
    subject: Option<Subject>,
    laid: Vec<Laid>,
}

#[derive(Clone, Default)]
pub struct Texts(Rc<RefCell<Drawn>>);

impl Texts {
    /// A new frame of `subject`'s transcript: what is not drawn again is gone.
    pub(crate) fn begin(&self, subject: &Subject) {
        let mut drawn = self.0.borrow_mut();
        drawn.subject = Some(subject.clone());
        drawn.laid.clear();
    }

    /// The texts drawn of `subject`, as the core's `Selection` takes them.
    fn of(&self, subject: &Subject) -> Option<Vec<(String, SharedString)>> {
        let drawn = self.0.borrow();
        (drawn.subject.as_ref() == Some(subject))
            .then(|| drawn.laid.iter().map(|laid| (laid.key.clone(), laid.text.clone())).collect())
    }
}

/// What the drawer keeps of a transcript's selection: view state.
#[derive(Default)]
pub struct Selecting {
    pub(crate) selection: Selection,
    /// The transcript has the keys: the last press in the drawer was in it. ⌘A and ⌘C are its.
    pub(crate) keys: bool,
    /// The mouse went down in it and is still down.
    held: bool,
}

/// A click that came at the end of a drag is a selection, not a click on what is under it.
pub(crate) fn dragged(event: &ClickEvent) -> bool {
    match event {
        ClickEvent::Mouse(click) => {
            let moved = click.up.position - click.down.position;
            f32::from(moved.x).abs() > 3. || f32::from(moved.y).abs() > 3.
        }
        ClickEvent::Keyboard(_) => false,
    }
}

impl Workspace {
    /// `text` drawn as the transcript of `subject` draws it, selectable: `styles` with the selected
    /// part over them, its layout noted for the mouse once laid out. `key` names it in the
    /// transcript (`in:12`); a text of no conversation (a viewed file) is drawn plain.
    pub(crate) fn picked(
        &self,
        of: Option<&Subject>,
        key: &str,
        text: String,
        styles: Vec<(Range<usize>, HighlightStyle)>,
    ) -> (StyledText, Option<AnyElement>) {
        let Some(subject) = of else { return (StyledText::new(text).with_highlights(styles), None) };
        let selected = self.drawer.looks.get(subject).and_then(|look| look.select.selection.range_of(key, &text));
        let text = SharedString::from(text);
        let styles = match selected {
            Some(range) => selection::overlay(styles, range, |style| HighlightStyle {
                background_color: Some(rgb(theme::SELECTION).into()),
                ..style.copied().unwrap_or_default()
            }),
            None => styles,
        };
        let styled = StyledText::new(text.clone()).with_highlights(styles);
        let (texts, layout, key) = (self.drawer.texts.clone(), styled.layout().clone(), key.to_string());
        // After the text beside it: its layout is laid out by then.
        let note = move |_, _: &mut Window, _: &mut gpui::App| {
            texts.0.borrow_mut().laid.push(Laid { key, text, layout });
        };
        (styled, Some(canvas(note, |_, _, _, _| {}).absolute().size_0().into_any_element()))
    }

    /// `text` selectable in `subject`'s transcript, cleaned as drawn (`transcript::clean`), in a
    /// div of its own to style.
    pub(crate) fn picked_text(&self, of: Option<&Subject>, key: &str, text: &str) -> gpui::Div {
        let (styled, note) = self.picked(of, key, ottospace_core::transcript::clean(text), Vec::new());
        div().relative().cursor_text().child(styled).children(note)
    }

    /// The window's own mouse handlers while `subject`'s transcript is drawn: a drag goes on past
    /// its edges and ends anywhere, as in a text field.
    pub(crate) fn transcript_listener(&self, subject: &Subject, cx: &Context<Self>) -> AnyElement {
        let (workspace, subject) = (cx.entity().downgrade(), subject.clone());
        let listen = move |_, _, window: &mut Window, _: &mut gpui::App| {
            let (moved, who) = (workspace.clone(), subject.clone());
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && event.pressed_button == Some(MouseButton::Left) {
                    let _ = moved.update(cx, |this, cx| this.transcript_drag(&who, event.position, window, cx));
                }
            });
            let who = subject.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble {
                    let _ = workspace.update(cx, |this, _| {
                        if let Some(look) = this.drawer.looks.get_mut(&who) {
                            look.select.held = false;
                        }
                    });
                }
            });
        };
        canvas(|_, _, _| {}, listen).absolute().size_0().into_any_element()
    }

    /// The texts drawn now go into `subject`'s selection; then `at` in window points as a text and
    /// a byte of it. `None`: nothing drawn.
    fn transcript_at(&mut self, subject: &Subject, at: Point<Pixels>) -> Option<(usize, usize)> {
        let texts = self.drawer.texts.of(subject)?;
        let look = self.drawer.looks.entry(subject.clone()).or_default();
        look.select.selection.set(texts.iter().map(|(key, text)| (key.as_str(), text.as_ref())));
        let drawn = self.drawer.texts.0.borrow();
        let rects: Vec<[f32; 4]> = drawn.laid.iter().map(|laid| crate::rect(laid.layout.bounds())).collect();
        let hit = selection::hit(&rects, (at.x.into(), at.y.into()))?;
        let offset = |n: usize| {
            let layout = &drawn.laid[n].layout;
            // Level with the text, before or past it: its nearest line's start or end.
            let bounds = layout.bounds();
            let x = at.x.clamp(bounds.left(), bounds.right());
            layout.index_for_position(gpui::point(x, at.y)).unwrap_or_else(|nearest| nearest)
        };
        Some(look.select.selection.at(hit, offset))
    }

    /// The mouse went down in `subject`'s transcript: a press of the core's `Selection`.
    pub(crate) fn transcript_press(&mut self, subject: &Subject, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some((n, offset)) = self.transcript_at(subject, event.position) else { return };
        let look = self.drawer.looks.entry(subject.clone()).or_default();
        look.select.selection.press(n, offset, event.click_count, event.modifiers.shift);
        (look.select.keys, look.select.held) = (true, true);
        cx.notify();
    }

    fn transcript_drag(&mut self, subject: &Subject, at: Point<Pixels>, _: &Window, cx: &mut Context<Self>) {
        if !self.drawer.looks.get(subject).is_some_and(|look| look.select.held) {
            return;
        }
        let Some((n, offset)) = self.transcript_at(subject, at) else { return };
        if let Some(look) = self.drawer.looks.get_mut(subject) {
            look.select.selection.drag(n, offset);
        }
        cx.notify();
    }

    /// A press in the drawer outside the transcript: the keys are no longer its. Its selection stays.
    pub(crate) fn transcript_leave(&mut self, subject: &Subject) {
        if let Some(look) = self.drawer.looks.get_mut(subject) {
            look.select.keys = false;
        }
    }

    /// The transcript drawn last, if it has the keys: the open drawer's, and the drawer has the
    /// keyboard (`Mode::Talk`), not a terminal or a box over it.
    fn transcript_keyed(&self) -> Option<Subject> {
        let subject = self.drawer.texts.0.borrow().subject.clone()?;
        let talk = self.model.talk.as_ref().filter(|talk| talk.open && talk.task == subject);
        let drawer = talk.is_some() && self.model.mode() == ottospace_core::model::Mode::Talk;
        let keys = self.drawer.looks.get(&subject).is_some_and(|look| look.select.keys);
        (drawer && keys).then_some(subject)
    }

    /// The transcript with the keys has text selected: ⌘C has something to copy.
    pub(crate) fn transcript_selected(&self) -> bool {
        let subject = self.transcript_keyed();
        subject.and_then(|s| self.drawer.looks.get(&s)).is_some_and(|look| look.select.selection.extent().is_some())
    }

    /// ⌘C while the transcript has the keys and a selection: its text, the user's only. False:
    /// another has the keys.
    pub(crate) fn transcript_copy(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.transcript_selected() {
            return false;
        }
        let Some(subject) = self.transcript_keyed() else { return false };
        if let Some(texts) = self.drawer.texts.of(&subject)
            && let Some(look) = self.drawer.looks.get_mut(&subject)
        {
            look.select.selection.set(texts.iter().map(|(key, text)| (key.as_str(), text.as_ref())));
        }
        let text = self.drawer.looks.get(&subject).map(|look| look.select.selection.selected().to_string());
        self.copy_untrusted(text.unwrap_or_default(), cx);
        true
    }

    /// ⌘A while the transcript has the keys: all of it. False: another has the keys.
    pub(crate) fn transcript_select_all(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(subject) = self.transcript_keyed() else { return false };
        let texts = self.drawer.texts.of(&subject).unwrap_or_default();
        if let Some(look) = self.drawer.looks.get_mut(&subject) {
            look.select.selection.set(texts.iter().map(|(key, text)| (key.as_str(), text.as_ref())));
            look.select.selection.select_all();
        }
        cx.notify();
        true
    }

    /// Esc while the transcript has the keys and a selection: it goes (`terminal::clears_selection`).
    pub(crate) fn transcript_escape(&mut self, key: &str, mods: ottospace_core::keys::Mods) -> bool {
        let selected = self.transcript_selected();
        let Some(subject) = self.transcript_keyed() else { return false };
        if !ottospace_core::terminal::clears_selection(key, mods, selected) {
            return false;
        }
        if let Some(look) = self.drawer.looks.get_mut(&subject) {
            look.select.selection.clear();
        }
        true
    }

    /// Copies what a model or a tool wrote, as every copy goes (`copy_text`): the user's act
    /// only. The state reply's `copied` shows only a test run's stand-in clipboard.
    pub(crate) fn copy_untrusted(&mut self, text: String, cx: &mut Context<Self>) {
        if self.input != Origin::User {
            self.error = Some(USERS_COPY.into());
            return cx.notify();
        }
        self.copy_text(text, None, ottospace_core::clip::Verb::Copied, cx);
    }

    /// The selection for the state: its extent (texts and characters, never its text), whether
    /// the transcript has the keys, and each selectable text's key and rect as last drawn.
    pub(crate) fn selection_snapshot(&self, subject: &Subject) -> serde_json::Value {
        let look = self.drawer.looks.get(subject);
        let extent = look.and_then(|look| look.select.selection.extent());
        let drawn = self.drawer.texts.0.borrow();
        let texts: Vec<serde_json::Value> = (drawn.subject.as_ref() == Some(subject))
            .then(|| drawn.laid.iter().map(|laid| serde_json::json!([laid.key, crate::rect(laid.layout.bounds())])))
            .into_iter()
            .flatten()
            .collect();
        serde_json::json!({ "extent": extent, "keys": look.is_some_and(|look| look.select.keys), "texts": texts })
    }
}
