//! `keys.json` in the shell: how much the user uses each key, as the core counts it
//! (`ottospace_core::key_use`). Read at start, written a few seconds after a count and at quit.
//! Only the user's own keys count: never the palette, a menu or a socket client's key.

use std::time::{Duration, Instant};

use gpui::{Context, Window};
use ottospace_core::commands::{ALL, Command};
use ottospace_core::key_use::{Binding, KeyUse, Unread};
use ottospace_core::{Action, Origin};

use crate::{CopySelection, PasteText, Workspace, commands, config, now_ms};

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

/// The binding a key's action counts for. `None`: no key to learn (a debug key, a text edit).
pub fn binding_of(action: &dyn gpui::Action) -> Option<Binding> {
    let name = action.name();
    let command = |c: &&Command| commands::action(**c).name() == name;
    if let Some(command) = ALL.iter().find(command) {
        return Binding::of(*command);
    }
    let named = |of: &dyn gpui::Action| of.name() == name;
    [
        (Binding::Copy, &CopySelection as &dyn gpui::Action),
        (Binding::Paste, &PasteText),
        (Binding::Palette, &commands::CommandPalette),
    ]
    .into_iter()
    .find_map(|(binding, of)| named(of).then_some(binding))
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
        if self.input != Origin::User {
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
