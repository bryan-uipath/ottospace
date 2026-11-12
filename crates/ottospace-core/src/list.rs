//! List mode: the secondary bar has the keyboard. Keys move a selection over its rows and act on it.

use crate::tasks::{Heading, TaskId};

/// What a row of the secondary bar stands for.
#[derive(Clone, Debug, PartialEq)]
pub enum ListRow {
    /// A running app, by name.
    App(String),
    /// A repository or worktree, by path.
    Repo(String),
    Task(TaskId),
    /// A heading of the Tasks section. Opening it folds or unfolds it.
    Heading(Heading),
    /// A session no task has adopted, by runner and id.
    Session(String, String),
    /// "N older" under Sessions. Opening it shows or hides the older sessions.
    OlderSessions,
}

/// What a key does in list mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ListKey {
    /// Down (positive) or up.
    Move(i32),
    /// Open the selected row. `down`: split down, not right.
    Open {
        down: bool,
    },
    /// The next (positive) or previous section.
    Section(i32),
    /// The four task moves. They do nothing on a row that is not a task.
    Snooze,
    Hold,
    Dismiss,
    Undo,
    Leave,
}

/// List-mode keys: arrows or `j`/`k` move, ↵ opens (⇧↵ splits down; space opens too, for a
/// heading that folds), Tab changes section, `s` `h` `x` `u` act on a task, Esc leaves.
pub fn list_key(key: &str, shift: bool) -> Option<ListKey> {
    Some(match key {
        "down" | "j" => ListKey::Move(1),
        "up" | "k" => ListKey::Move(-1),
        "enter" => ListKey::Open { down: shift },
        "space" => ListKey::Open { down: false },
        "tab" => ListKey::Section(if shift { -1 } else { 1 }),
        "s" => ListKey::Snooze,
        "h" => ListKey::Hold,
        "x" => ListKey::Dismiss,
        "u" => ListKey::Undo,
        "escape" => ListKey::Leave,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_move_open_and_act() {
        assert_eq!((list_key("j", false), list_key("up", false)), (Some(ListKey::Move(1)), Some(ListKey::Move(-1))));
        assert_eq!(list_key("enter", true), Some(ListKey::Open { down: true }));
        assert_eq!(list_key("space", false), Some(ListKey::Open { down: false }));
        assert_eq!(
            (list_key("tab", false), list_key("tab", true)),
            (Some(ListKey::Section(1)), Some(ListKey::Section(-1)))
        );
        assert_eq!(list_key("x", false), Some(ListKey::Dismiss));
        assert_eq!(list_key("q", false), None);
    }
}
