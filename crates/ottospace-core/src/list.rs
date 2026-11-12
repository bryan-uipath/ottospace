//! List mode: the secondary bar has the keyboard. Keys move a selection over its rows and act on it.

use crate::tasks::TaskId;

/// What a row of the secondary bar stands for.
#[derive(Clone, Debug, PartialEq)]
pub enum ListRow {
    /// A running app, by name.
    App(String),
    /// A repository or worktree, by path.
    Repo(String),
    Task(TaskId),
    /// A row of the Routines section, by its key.
    Routine(String),
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
    /// Run the selected routine now.
    Run,
    /// Open the report of the selected row's run.
    Report,
    Leave,
}

/// List-mode keys: arrows or `j`/`k` move, ↵ opens (⇧↵ splits down), Tab changes section,
/// `s` `h` `x` `u` act on a task, `r` `o` on a routine, Esc leaves.
pub fn list_key(key: &str, shift: bool) -> Option<ListKey> {
    Some(match key {
        "down" | "j" => ListKey::Move(1),
        "up" | "k" => ListKey::Move(-1),
        "enter" => ListKey::Open { down: shift },
        "tab" => ListKey::Section(if shift { -1 } else { 1 }),
        "s" => ListKey::Snooze,
        "h" => ListKey::Hold,
        "x" => ListKey::Dismiss,
        "u" => ListKey::Undo,
        "r" => ListKey::Run,
        "o" => ListKey::Report,
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
        assert_eq!(
            (list_key("tab", false), list_key("tab", true)),
            (Some(ListKey::Section(1)), Some(ListKey::Section(-1)))
        );
        assert_eq!(list_key("x", false), Some(ListKey::Dismiss));
        assert_eq!((list_key("r", false), list_key("o", false)), (Some(ListKey::Run), Some(ListKey::Report)));
        assert_eq!(list_key("q", false), None);
    }
}
