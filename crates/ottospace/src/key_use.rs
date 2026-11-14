//! `keys.json` in the shell: how much the user uses each key, as the core counts it
//! (`ottospace_core::key_use`). Read at start, written a few seconds after a count and at quit.
//! Only the user's own keys count: never the palette, a menu or a socket client's key.

use std::time::{Duration, Instant};

use std::collections::HashMap;

use gpui::{App, Context, Div, Keymap, ParentElement, Styled, Window, div, px, rgb, rgba};
use ottospace_core::commands::{ALL, Command};
use ottospace_core::key_use::{self, Binding, KeyUse, Line, Reference, Unread};
use ottospace_core::{Action, Origin, theme};

use crate::{CopySelection, PasteText, RETURN_KEY, Workspace, collect, commands, config, now_ms};

/// A burst of keys makes one write.
const SETTLE: Duration = Duration::from_secs(5);

/// What `keys.json` last held, kept between frames.
#[derive(Default)]
pub struct Kept {
    /// False: not written this run (nothing is kept, or it is a newer build's).
    writing: bool,
    due: Option<Instant>,
    /// The text last read or written: the same text is not written again.
    stored: String,
}

/// The bindings whose action is no command's.
fn extras() -> [(Binding, Box<dyn gpui::Action>); 3] {
    [
        (Binding::Copy, Box::new(CopySelection)),
        (Binding::Paste, Box::new(PasteText)),
        (Binding::Palette, Box::new(commands::CommandPalette)),
    ]
}

/// The binding a key's action counts for. `None`: no key to learn (a debug key, a text edit).
pub fn binding_of(action: &dyn gpui::Action) -> Option<Binding> {
    let name = action.name();
    let command = |c: &&Command| commands::action(**c).name() == name;
    if let Some(command) = ALL.iter().find(command) {
        return Binding::of(*command);
    }
    extras().into_iter().find_map(|(binding, of)| (of.name() == name).then_some(binding))
}

/// A binding's keys as the key reference shows them, from the keymap: `⌘⌃←→↑↓`, `⌘] ⌘[`.
/// `None`: it has no key in this run (an unset return key).
pub fn keys_text(keymap: &Keymap, binding: Binding) -> Option<String> {
    if let Some(keys) = binding.keys() {
        return Some(keys.into());
    }
    match binding {
        Binding::ReturnKey => return RETURN_KEY.get().cloned(),
        Binding::CaptureKey => return collect::KEY.get().map(|key| collect::key_label(key)),
        _ => {}
    }
    let extra = extras().into_iter().filter(|(of, _)| *of == binding).map(|(_, action)| action);
    let actions = binding.commands().iter().map(|c| commands::action(*c)).chain(extra);
    // Each action's first key, as the menus show it.
    let first = |action: Box<dyn gpui::Action>| {
        let binding = keymap.bindings_for_action(action.as_ref()).next()?;
        binding.keystrokes().first().map(|stroke| commands::key_parts(stroke.inner()))
    };
    let strokes: Vec<(String, String)> = actions.filter_map(first).collect();
    let (mods, _) = strokes.first()?;
    // Arrows with one set of modifiers: the modifiers once, e.g. ⌘⌃←→↑↓.
    let arrows = strokes.iter().all(|(m, key)| m == mods && ["←", "→", "↑", "↓"].contains(&key.as_str()));
    Some(match arrows {
        true => mods.clone() + &strokes.iter().map(|(_, key)| key.as_str()).collect::<String>(),
        false => strokes.iter().map(|(m, key)| format!("{m}{key}")).collect::<Vec<_>>().join(" "),
    })
}

/// A row's bar of recent use: 2 by 24 points, filled to `fill`. None at 0: an unused key is a
/// plain row. Always its room, so rows line up.
pub fn bar(fill: f32) -> Div {
    let bar = div().w(px(24.)).h(px(2.)).rounded_full();
    if fill <= 0. {
        return bar;
    }
    // The primary colour at 60%.
    let filled = div().h_full().rounded_full().w(px(24. * fill.min(1.))).bg(rgba(theme::PRIMARY << 8 | 0x99));
    bar.bg(rgb(theme::BORDER)).child(filled)
}

/// `groups` in `n` columns, in order, each column about as long as the others.
pub fn columns<'a, T: Clone, L>(groups: &[(T, &'a [L])], n: usize) -> Vec<Vec<(T, &'a [L])>> {
    // A heading counts as a line.
    let total: usize = groups.iter().map(|(_, lines)| lines.len() + 1).sum();
    let per = total.div_ceil(n.max(1));
    let mut columns: Vec<Vec<(T, &'a [L])>> = vec![Vec::new()];
    let mut length = 0;
    for (title, lines) in groups {
        if length >= per && columns.len() < n {
            columns.push(Vec::new());
            length = 0;
        }
        length += lines.len() + 1;
        if let Some(column) = columns.last_mut() {
            column.push((title.clone(), *lines));
        }
    }
    columns
}

/// The key reference for a test run's state: binding ids and notes' keys, no scores.
pub fn snapshot(reference: &Reference, known_at: Option<[f32; 4]>) -> serde_json::Value {
    let id = |line: &Line| match line {
        Line::Key { binding, .. } => binding.id(),
        Line::Note { keys, .. } => keys.to_string(),
    };
    let groups: Vec<serde_json::Value> = reference
        .groups
        .iter()
        .map(|(group, lines)| serde_json::json!({ "group": group, "rows": lines.iter().map(id).collect::<Vec<_>>() }))
        .collect();
    serde_json::json!({
        "groups": groups,
        "known": reference.known.iter().map(id).collect::<Vec<_>>(),
        "known_open": reference.known_open,
        "known_header": known_at,
    })
}

impl Workspace {
    /// Reads `keys.json` at start. One that does not read starts the counts afresh; a newer
    /// build's is left as it is, and not written this run.
    pub(crate) fn restore_key_use(&mut self) {
        self.kept_keys.writing = config::get().home.is_some();
        let Some(text) = config::read_keys() else { return };
        let uses = match KeyUse::parse(&text) {
            Ok(uses) => uses,
            Err(Unread::Newer(_)) => return self.kept_keys.writing = false,
            Err(Unread::Bad(_)) => return,
        };
        self.model.update(Action::RestoreKeyUse { uses }.sent_by(Origin::System));
        self.kept_keys.stored = text;
    }

    /// A key ran `binding`: one use, if the key was the user's. Not through `dispatch`: a count
    /// is no input, and stays out of the trace.
    pub(crate) fn key_used(&mut self, binding: Binding) {
        // A held ⌘ shortcut reaches GPUI as presses: only its first counts.
        if self.input != Origin::User || ottospace_mac::window::key_repeating() {
            return;
        }
        self.model.update(Action::KeyUsed { binding, at: now_ms() }.sent_by(Origin::User));
        self.key_use_changed();
    }

    /// Reset Key Counts: every key unused again.
    pub(crate) fn reset_key_use(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::ResetKeyUse, self.input, window, cx);
        self.key_use_changed();
    }

    /// The key reference as the core lays it out, and each listed binding's keys.
    pub(crate) fn key_reference(&self, cx: &App) -> (Reference, HashMap<Binding, String>) {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let texts: HashMap<Binding, String> =
            key_use::ALL.iter().filter_map(|b| Some((*b, keys_text(&keymap, *b)?))).collect();
        (self.model.key_reference(|binding| texts.contains_key(&binding)), texts)
    }

    /// The ids of the bindings the user knows now, for a test run's state.
    pub(crate) fn keys_known(&self) -> Vec<String> {
        let now = now_ms();
        let known = ottospace_core::key_use::ALL.iter().filter(|b| self.model.key_use.known(**b, now));
        known.map(|binding| binding.id()).collect()
    }

    fn key_use_changed(&mut self) {
        if self.kept_keys.writing {
            self.kept_keys.due.get_or_insert_with(|| Instant::now() + SETTLE);
        }
    }

    /// Writes the file when a write is due, or with `quitting` at once.
    pub(crate) fn store_key_use(&mut self, quitting: bool) {
        let due = self.kept_keys.due.is_some_and(|at| at <= Instant::now());
        if !self.kept_keys.writing || !(due || quitting && self.kept_keys.due.is_some()) {
            return;
        }
        self.kept_keys.due = None;
        let text = self.model.key_use.write();
        // A failed write is tried at the next count: a count is not worth a word in the status bar.
        if text != self.kept_keys.stored && config::save_keys(&text).is_ok() {
            self.kept_keys.stored = text;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command with a key counts for a binding, and every binding's command has a key:
    /// the bindings follow the keymap. The debug key alone is not one to learn.
    #[test]
    fn the_bindings_follow_the_keymap() {
        let keymap = gpui::Keymap::new(crate::keymap());
        for command in ALL {
            let keyed = commands::key_of(&keymap, *command).is_some();
            let counted = Binding::of(*command).is_some();
            assert_eq!(keyed && *command != Command::AxOverlay, counted, "{command:?}");
        }
    }

    #[test]
    fn a_bindings_keys_come_from_the_keymap() {
        let keymap = gpui::Keymap::new(crate::keymap());
        let keys = |binding| keys_text(&keymap, binding);
        assert_eq!(keys(Binding::FocusDir).as_deref(), Some("⌘⌃←→↑↓"));
        assert_eq!(keys(Binding::FocusNext).as_deref(), Some("⌘] ⌘["));
        assert_eq!(keys(Binding::PickPane).as_deref(), Some("⌘E ⌘⇧E"));
        assert_eq!(keys(Binding::Copy).as_deref(), Some("⌘C"));
        assert_eq!(keys(Binding::TabNumber).as_deref(), Some("⌘1–9"));
        // Every other binding has a key; the return and capture keys only once set.
        for binding in key_use::ALL.iter().filter(|b| ![Binding::ReturnKey, Binding::CaptureKey].contains(b)) {
            assert!(keys(*binding).is_some(), "{binding:?}");
        }
    }

    #[test]
    fn groups_are_cut_into_columns_of_about_one_length_in_order() {
        let (a, b, c): (&[u8], &[u8], &[u8]) = (&[0; 6], &[0; 2], &[0; 3]);
        let cut = columns(&[("a", a), ("b", b), ("c", c)], 2);
        let titles: Vec<Vec<&str>> = cut.iter().map(|column| column.iter().map(|(t, _)| *t).collect()).collect();
        assert_eq!(titles, [vec!["a"], vec!["b", "c"]]);
        assert_eq!(columns::<&str, u8>(&[], 3).len(), 1);
    }

    #[test]
    fn a_keys_action_names_its_binding() {
        assert_eq!(binding_of(&crate::FocusUp), Some(Binding::FocusDir));
        assert_eq!(binding_of(&crate::SplitRight), Some(Binding::SplitRight));
        assert_eq!(binding_of(&CopySelection), Some(Binding::Copy));
        assert_eq!(binding_of(&commands::CommandPalette), Some(Binding::Palette));
        // A debug key, and a text field's own edit, are none to learn.
        assert_eq!(binding_of(&crate::AxOverlay), None);
        assert_eq!(binding_of(&crate::Undo), None);
    }
}
