//! Key caps: a key from the keymap as one small box per key, and what to do in an empty place
//! listed with them. One list on the home screen, in an empty tab and in an empty pane; which
//! rows each shows is the core's (`ottospace_core::empty`).

use gpui::{Context, Div, Keymap, Keystroke, Modifiers, SharedString, Stateful, Window, div, prelude::*, px, rgb};
use ottospace_core::commands::Command;
use ottospace_core::empty::Hint;
use ottospace_core::{Action, Axis, Kind, PaneId, Picker, theme};

use crate::{Workspace, commands, key_use, ui};

/// A key as caps, each with whether it is lit: ⌘E with ⌘ held is [("⌘", true), ("E", false)].
/// A modifier's cap is lit while it is held and the keys held are all the chord's; `fired`
/// (the whole chord was pressed) lights every cap.
fn caps(stroke: &Keystroke, held: Modifiers, fired: bool) -> Vec<(String, bool)> {
    let (signs, key) = commands::key_parts(stroke);
    let on = |m: Modifiers| [(m.platform, '⌘'), (m.control, '⌃'), (m.alt, '⌥'), (m.shift, '⇧')];
    let within = on(held).iter().zip(on(stroke.modifiers)).all(|(held, chord)| !held.0 || chord.0);
    let lit = |sign: char| fired || (within && on(held).contains(&(true, sign)));
    signs.chars().map(|sign| (sign.to_string(), lit(sign))).chain([(key, fired)]).collect()
}

/// A key's caps in a row, e.g. [⌘] [E]. `held`: the modifiers down now.
pub fn key_caps(stroke: &Keystroke, held: Modifiers, fired: bool) -> Div {
    div().flex().gap_1().children(
        caps(stroke, held, fired).into_iter().map(|(text, lit)| ui::pill(text, ui::Pill::Key { lit, small: false })),
    )
}

/// A command's first key in the keymap.
fn stroke(keymap: &Keymap, command: Command) -> Option<Keystroke> {
    key_use::first_stroke(keymap, commands::action(command).as_ref())
}

/// The modifiers whose caps are lit: those down, in the active window only (⌘ let go in another
/// app is never heard of). A test run's window is never active.
pub fn held(window: &Window) -> Modifiers {
    let listens = window.is_window_active() || std::env::var_os("OTTOSPACE_E2E").is_some();
    if listens { window.modifiers() } else { Modifiers::default() }
}

impl Workspace {
    /// A modifier changed: the list is drawn again only if it is on show and its lit caps differ
    /// from those drawn. ⇧ while typing beside an empty tab draws nothing.
    pub(crate) fn modifiers_changed(&mut self, window: &Window, cx: &mut Context<Self>) {
        // Only while an empty place is on show: the main window's tab, or one folded out.
        let tabs = std::iter::once(self.model.main_at()).chain(
            self.model.tabs_out().into_iter().filter_map(|(id, _)| self.model.tabs.iter().position(|tab| tab.id == id)),
        );
        let empty = |at: usize| {
            self.model.tree_at(at).is_none_or(|tree| {
                tree.leaves().iter().any(|id| self.model.pane(*id).is_some_and(|pane| pane.kind == Kind::Empty))
            })
        };
        if !tabs.into_iter().any(empty) {
            return;
        }
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let strokes = [Hint::OpenApp, Hint::Terminal, Hint::NewTask].map(|hint| stroke(&keymap, hint.command()));
        let lit = |held: Modifiers| strokes.each_ref().map(|s| s.as_ref().map(|s| caps(s, held, false)));
        if lit(held(window)) != lit(self.caps_drawn.get()) {
            cx.notify();
        }
    }

    /// What to do in an empty place, a row each: its key's caps, then what it does. A click does
    /// what the key does. `hints`: the rows, in order; `prefix`: their controls' (`home:open_app`);
    /// `pane`: the empty pane it is in, if one. `held`: the modifiers down now, whose caps are lit.
    pub(crate) fn hint_list(
        &self,
        hints: &[Hint],
        prefix: &str,
        pane: Option<PaneId>,
        held: Modifiers,
        cx: &mut Context<Self>,
    ) -> Div {
        self.caps_drawn.set(held);
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        // This row's picker open is its chord, pressed: its caps stay lit, in the pane it fills
        // only. A terminal fills the tab at once, so its row has nothing to show.
        let picking = matches!(self.model.picker, Some(Picker { axis: Axis::Right, new_tab: false, .. }))
            && pane.is_none_or(|pane| self.model.focused == Some(pane));
        let row = |hint: Hint| -> Stateful<Div> {
            let stroke = stroke(&keymap, hint.command());
            let fired = hint == Hint::OpenApp && picking;
            // Their room also with no key, so the labels line up.
            let keys = div().min_w(px(48.)).children(stroke.map(|stroke| key_caps(&stroke, held, fired)));
            let row = div()
                .id(SharedString::from(format!("{prefix}:{}", hint.id())))
                .relative()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .gap_3()
                .rounded_md()
                .text_sm()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .child(keys)
                .child(hint.label())
                .child(self.controls.mark(format!("{prefix}:{}", hint.id())));
            match hint {
                Hint::OpenApp => {
                    row.on_click(cx.listener(|this, _, window, cx| this.open_picker(Axis::Right, false, window, cx)))
                }
                Hint::Terminal => row.on_click(cx.listener(|this, _, window, cx| {
                    let split = Action::Split { axis: Axis::Right, kind: Kind::Terminal };
                    this.dispatch(split, this.input, window, cx);
                })),
                // ⌘N's box: in a workspace ↵ starts the task there, on Home in a space of its own.
                Hint::NewTask => row.on_click(cx.listener(|this, _, window, cx| this.new_task(window, cx))),
            }
        };
        div().flex().flex_col().gap_1().children(hints.iter().map(|hint| row(*hint)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(keys: &str, held: &str) -> Vec<(String, bool)> {
        let held = Keystroke::parse(&format!("{held}x")).unwrap().modifiers;
        caps(&Keystroke::parse(keys).unwrap(), held, false)
    }

    #[test]
    fn a_key_is_a_cap_each_and_a_held_modifier_of_the_chord_is_lit() {
        let cap = |text: &str, lit: bool| (text.to_string(), lit);
        assert_eq!(lit("cmd-e", ""), [cap("⌘", false), cap("E", false)]);
        assert_eq!(lit("cmd-e", "cmd-"), [cap("⌘", true), cap("E", false)]);
        assert_eq!(lit("cmd-shift-e", "cmd-"), [cap("⌘", true), cap("⇧", false), cap("E", false)]);
        // ⌘⇧ starts no ⌘E, and ⌥ no chord here.
        assert_eq!(lit("cmd-e", "cmd-shift-"), [cap("⌘", false), cap("E", false)]);
        assert_eq!(lit("cmd-e", "alt-"), [cap("⌘", false), cap("E", false)]);
        // The whole chord, pressed: every cap.
        let fired = caps(&Keystroke::parse("cmd-e").unwrap(), Modifiers::default(), true);
        assert_eq!(fired, [cap("⌘", true), cap("E", true)]);
    }
}
