//! List mode: the secondary bar has the keyboard. Keys move a selection over its rows and act on it.

use crate::marks::MarkId;
use crate::tasks::{Heading, TaskId};
use crate::workbook::WorkbookId;

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
    /// A project of the Project view (`None`: no project), and its "Done" line. Opening one folds or unfolds it.
    Project(Option<String>),
    ProjectDone(Option<String>),
    /// A session no task has adopted, by runner and id.
    Session(String, String),
    Mark(MarkId),
    Workbook(WorkbookId),
    /// A tab of an unfolded workbook, by its place in the workbook.
    WorkbookTab(WorkbookId, usize),
    /// An app that is not running, by its bundle path. Opening it starts it.
    Closed(String),
    /// "and N more" under the suggested apps, and the "All apps" heading. Opening one folds or unfolds it.
    MoreApps,
    AllApps,
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
    /// Move the selected mark down (positive) or up the list.
    Reorder(i32),
    /// Type the selected mark's name, or its letter.
    Rename,
    Letter,
    /// Pin or unpin the selected app.
    Pin,
    /// Type in the filter of "All apps".
    Filter,
    /// Add the tab on show to the selected workbook.
    Add,
    /// ← or →: across zones, from a rail (`model::zones`).
    Side(i32),
    Leave,
}

/// List-mode keys: arrows or `j`/`k` move, ↵ opens (⇧↵ splits down; space opens too, for a
/// heading that folds), Tab changes section, `s` `h` `x` `u` act on a task, Esc leaves. On a mark: `x` removes it, `r` renames
/// it, `m` gives it a letter, ⇧J ⇧K (or ⇧↓ ⇧↑) move it. On an app that is not running: `p` pins or
/// unpins it, `x` hides a suggestion, `u` takes that back; `/` types in the filter of "All apps".
/// On a workbook: `a` adds the tab on show to it.
pub fn list_key(key: &str, shift: bool) -> Option<ListKey> {
    Some(match key {
        "down" | "j" if shift => ListKey::Reorder(1),
        "up" | "k" if shift => ListKey::Reorder(-1),
        "down" | "j" => ListKey::Move(1),
        "up" | "k" => ListKey::Move(-1),
        "enter" => ListKey::Open { down: shift },
        "space" => ListKey::Open { down: false },
        "tab" => ListKey::Section(if shift { -1 } else { 1 }),
        "s" => ListKey::Snooze,
        "h" => ListKey::Hold,
        "x" => ListKey::Dismiss,
        "u" => ListKey::Undo,
        "r" => ListKey::Rename,
        "m" => ListKey::Letter,
        "p" => ListKey::Pin,
        "/" => ListKey::Filter,
        "a" => ListKey::Add,
        "escape" => ListKey::Leave,
        "left" => ListKey::Side(-1),
        "right" => ListKey::Side(1),
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
        assert_eq!(
            (list_key("j", true), list_key("up", true)),
            (Some(ListKey::Reorder(1)), Some(ListKey::Reorder(-1)))
        );
        assert_eq!((list_key("r", false), list_key("m", false)), (Some(ListKey::Rename), Some(ListKey::Letter)));
        assert_eq!((list_key("p", true), list_key("/", false)), (Some(ListKey::Pin), Some(ListKey::Filter)));
        assert_eq!(list_key("q", false), None);
    }
}
