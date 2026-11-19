//! How to ask a running app for a new window: one table, by bundle id. The picker's "New window"
//! picks the recipe; the shell carries it out and hosts the window that appears as owned (it
//! closes with its pane). An app the table does not name is offered one only if a menu of its
//! has an item titled exactly "New Window", which is then pressed: never a guess such as ⌘N,
//! which in Notes makes a note.

use serde::Serialize;

/// Why a client's answer to "Close <title>?" is refused.
pub const USERS_ALONE: &str = "only the user closes a window with unsaved changes";

/// Why a client's "New window" is refused: it presses in another app.
pub const PICK_USERS: &str = "only the user asks an app for a new window";

/// Why a client's word on a window is refused.
pub const SHELLS_ALONE: &str = "only ottospace says what a window did";

/// How long the picker waits for the new window before the notice bar says none came.
pub const WAIT_MS: u64 = 3_000;

/// One way to ask for a new window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "how", rename_all = "snake_case")]
pub enum Recipe {
    /// Started again with these arguments: `open -n -g -b <bundle> --args <args>`. The running
    /// instance takes them, e.g. Chrome's `--new-window`.
    Open { args: &'static [&'static str] },
    /// An Apple Event to the app: `tell application id "<bundle>" to <command>`.
    Tell { command: &'static str },
    /// The app's menu item titled `MENU_ITEM`, pressed.
    MenuItem,
}

/// The menu item an app without a recipe must have to be offered "New window". English only, for
/// now.
pub const MENU_ITEM: &str = "New Window";

const CHROMIUM: Recipe = Recipe::Open { args: &["--new-window"] };
/// VS Code and its forks: `code -n`.
const CODE: Recipe = Recipe::Open { args: &["-n"] };

/// Every app with a recipe of its own. The rest get `MenuItem`, if they have the item.
pub const RECIPES: &[(&str, Recipe)] = &[
    ("com.google.Chrome", CHROMIUM),
    ("com.google.Chrome.beta", CHROMIUM),
    ("com.google.Chrome.dev", CHROMIUM),
    ("com.google.Chrome.canary", CHROMIUM),
    ("org.chromium.Chromium", CHROMIUM),
    ("com.brave.Browser", CHROMIUM),
    ("com.microsoft.edgemac", CHROMIUM),
    ("com.vivaldi.Vivaldi", CHROMIUM),
    ("com.operasoftware.Opera", CHROMIUM),
    ("com.microsoft.VSCode", CODE),
    ("com.microsoft.VSCodeInsiders", CODE),
    ("com.vscodium", CODE),
    ("com.todesktop.230313mzl4w4u92", CODE),
    ("com.apple.Safari", Recipe::Tell { command: "make new document" }),
    ("com.apple.finder", Recipe::Tell { command: "make new Finder window" }),
    ("com.apple.Terminal", Recipe::Tell { command: "do script \"\"" }),
    ("com.googlecode.iterm2", Recipe::Tell { command: "create window with default profile" }),
];

/// The recipe for the app with this bundle id: its own, else its "New Window" menu item if it
/// has one (`menu_item`). `None`: the picker offers no new window of it.
pub fn recipe(bundle: Option<&str>, menu_item: bool) -> Option<Recipe> {
    let own = RECIPES.iter().find(|(id, _)| Some(*id) == bundle).map(|(_, recipe)| *recipe);
    own.or(menu_item.then_some(Recipe::MenuItem))
}

/// What the notice bar says when no window came in time.
pub fn late(app: &str) -> String {
    format!("{app} opened no new window in {} seconds", WAIT_MS / 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_app_has_one_recipe_and_the_rest_only_a_new_window_item() {
        assert_eq!(recipe(Some("com.google.Chrome"), false), Some(Recipe::Open { args: &["--new-window"] }));
        assert_eq!(recipe(Some("com.microsoft.VSCode"), true), Some(Recipe::Open { args: &["-n"] }));
        assert!(matches!(recipe(Some("com.apple.Safari"), false), Some(Recipe::Tell { .. })));
        // Notes: no recipe and no "New Window" item, so no row; its ⌘N would make a note.
        assert_eq!(recipe(Some("com.apple.Notes"), false), None);
        assert_eq!(
            (recipe(Some("com.tinyspeck.slackmacgap"), true), recipe(None, true)),
            (Some(Recipe::MenuItem), Some(Recipe::MenuItem))
        );
        let mut ids: Vec<&str> = RECIPES.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), RECIPES.len(), "a bundle id twice");
    }

    #[test]
    fn a_told_command_is_one_line_of_applescript_with_no_quote_to_end_the_id() {
        // The shell puts it after `tell application id "<bundle>" to`: fixed text, never input.
        for (id, recipe) in RECIPES {
            assert!(!id.contains('"') && !id.contains('\n'), "{id}");
            if let Recipe::Tell { command } = recipe {
                assert!(!command.contains('\n'), "{id}: {command}");
            }
        }
    }
}
