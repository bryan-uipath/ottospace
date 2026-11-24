//! Agent mode on an app pane (`docs/ai/agent-mode.md`): the user lets the task of the pane's
//! workspace see the pane's window (See), and later press its controls (See and act: presses are
//! stage (c), none yet). Only the user sets a level; a task's agent asks with its pass, which
//! the shell turns into a `Caller` before the core decides (`Model::app_access`).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::model::{Kind, Owner, PaneId};
use crate::tasks::TaskId;

/// Why a level, or the menu, set by anyone but the user is refused.
pub const USERS_ALONE: &str = "only the user lets a task see an app";
/// Why a client's record of an app's use is refused.
pub const SHELLS_ALONE: &str = "only ottospace records what a task did in an app";
/// ⌘⌃V, the palette or a level with focus on no app pane.
pub const FOCUS_APP: &str = "focus an app pane to set what its task may do there";
/// Why a kept level is not given: the offer is gone (the level changed, the task left).
pub const NO_OFFER: &str = "nothing to restore on this pane";
/// What the menu says on a pane whose workspace no task works in: it grants nothing.
pub const NO_TASK: &str = "No task works in this workspace";
/// What a level picked there does: the next task that works there is offered it (`Ahead`).
pub const AHEAD: &str = "Saved for the next task you start here: its pane asks you before the task can see the app";
/// The menu's head there, before a pick: what picking does.
pub const AHEAD_MENU: &str =
    "Pick See or See and act to save it for the next task you start here; you confirm before it applies";
/// What See and act grants in this build: what See does. Presses come in stage (c).
pub const ACT_LATER: &str = "Pressing controls comes later: for now See and act only lets the task see";
/// A request with no pass: no task asked.
pub const NO_PASS: &str =
    "only a task's agent reads an app pane: run it from the agent ottospace started for the task (its pass)";
/// A `workspace` request with no task's pass and no pane in a workspace: nothing to answer for.
pub const NO_WORKSPACE: &str =
    "no workspace: ask from the agent ottospace started for a task (its pass), or name a pane (`$OTTOSPACE_PANE`)";
/// The least time between two reads (or two looks) of one pane.
pub const EVERY_MS: u64 = 1_000;

/// How much the task of a pane's workspace may do in it. Ordered: `Act` grants all `See` does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    #[default]
    Off,
    See,
    Act,
}

impl Level {
    /// In the menu's order.
    pub const ALL: [Level; 3] = [Level::Off, Level::See, Level::Act];

    /// The word in the pane's header.
    pub fn word(self) -> &'static str {
        match self {
            Level::Off => "Off",
            Level::See => "See",
            Level::Act => "Act",
        }
    }

    /// The menu's item, and the palette's.
    pub fn label(self) -> &'static str {
        match self {
            Level::Off => "Off",
            Level::See => "See",
            Level::Act => "See and act",
        }
    }
}

/// What a task did, or asked to do, in an app pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppUse {
    /// Its window's accessibility tree (`app_read`, `ax_tree`).
    Read,
    /// A picture of its window (`app_look`).
    Look,
}

impl AppUse {
    pub fn verb(self) -> &'static str {
        match self {
            AppUse::Read => "read",
            AppUse::Look => "looked at",
        }
    }
}

/// The key of an app pane's record in `layout.json` that keeps a level the user picked.
pub const KEY: &str = "agent_mode";

/// A level the user picked, as `layout.json` keeps it: for which task.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kept {
    pub level: Level,
    pub task: TaskId,
}

/// A restored pane's kept level, waiting for its window: given only to a window of the same app.
#[derive(Clone, Debug, PartialEq)]
pub struct Waiting {
    pub kept: Kept,
    pub bundle: Option<String>,
    pub name: String,
}

/// One app pane's level, and the task it was given to: the front task of the pane's workspace
/// when the pane came, or when the user set it. `None`: no task works there, and it grants nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Grant {
    pub level: Level,
    pub task: Option<TaskId>,
}

/// A level the user picked on an app pane while no task worked in its workspace. The first task
/// the user starts there is offered it (`AgentMode.offers`); only the user's click gives it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Ahead {
    pub level: Level,
    /// The tab it was picked in (a workspace's tabs keep their ids when a task binds it): moved
    /// to another, the pane forgets it.
    #[serde(skip)]
    pub tab: u32,
}

/// What a `workspace` request answers: the space a task works in (or a pane is in), its tabs and
/// what each pane holds, with its grant. Ids, kinds and names: no terminal's text.
#[derive(Debug, PartialEq, Serialize)]
pub struct WorkspaceView {
    /// The caller's task; none for a pane's.
    pub task: Option<TaskId>,
    pub space: Owner,
    /// The space's name on the top row: a task's title is the user's words, data only.
    pub name: String,
    pub tabs: Vec<TabView>,
    /// The `agent_mode` flag: off, no app pane is read whatever its grant.
    pub agent_mode: bool,
}

/// A tab of a `WorkspaceView`.
#[derive(Debug, PartialEq, Serialize)]
pub struct TabView {
    pub id: u32,
    pub title: String,
    pub panes: Vec<PaneView>,
}

/// A pane of a `WorkspaceView`: an app pane's `kind` has its app and window title.
#[derive(Debug, PartialEq, Serialize)]
pub struct PaneView {
    pub id: PaneId,
    pub kind: Kind,
    pub cwd: Option<String>,
    /// An app pane's agent mode grant; none for any other pane.
    pub grant: Option<Grant>,
}

/// The open menu: whose pane, and the item the keys are on (none when a click opened it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Menu {
    pub pane: PaneId,
    pub selected: Option<usize>,
}

/// Agent mode's state. In the state: each app pane's grant, the default and the menu.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AgentMode {
    pub panes: BTreeMap<PaneId, Grant>,
    /// The level an app pane starts at in a task's workspace (`agent_mode` in the settings).
    pub default: Level,
    pub menu: Option<Menu>,
    /// Levels picked on panes where no task works yet, for the next task there.
    pub ahead: BTreeMap<PaneId, Ahead>,
    /// When each pane was last read and looked at, for the limit and the header's tip.
    #[serde(skip)]
    pub last: BTreeMap<PaneId, BTreeMap<AppUse, u64>>,
    /// Whether the user's own act put each app pane's window there (their pick, drag or click),
    /// kept whether the flag is on or not: only such a pane starts at the default. One a client
    /// opened or filled, the system adopted or a restore rebuilt starts at Off.
    #[serde(skip)]
    pub by_user: BTreeMap<PaneId, bool>,
    /// The panes whose level the user picked (or a restore gave back what they picked): only
    /// those are kept over a restart. A reset to Off forgets the pick.
    #[serde(skip)]
    pub chosen: BTreeSet<PaneId>,
    /// Restored panes waiting for their window, with the level the user picked in a run before.
    #[serde(skip)]
    pub restored: BTreeMap<PaneId, Waiting>,
    /// Restored panes, at Off, whose header offers the level kept for them: only the user's
    /// click gives it (`AgentOfferAccept`). The file could be anyone's of the same user.
    #[serde(skip)]
    pub offers: BTreeMap<PaneId, Kept>,
    /// Empty panes a record rebuilt for an app (a tab mark, a restore): whoever fills one, it
    /// starts at Off. The record is a same-user file, so an agent's too.
    #[serde(skip)]
    pub from_record: BTreeSet<PaneId>,
}

impl Default for AgentMode {
    fn default() -> Self {
        // The owner's choice (2026-11-16): Act, changeable in the settings.
        let (panes, last, by_user) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        let (chosen, restored, offers) = (BTreeSet::new(), BTreeMap::new(), BTreeMap::new());
        let (from_record, ahead) = (BTreeSet::new(), BTreeMap::new());
        let default = Level::Act;
        Self { panes, default, menu: None, ahead, last, by_user, chosen, restored, offers, from_record }
    }
}

impl AgentMode {
    /// The level of `pane`: Off for one with no grant.
    pub fn level(&self, pane: PaneId) -> Level {
        self.panes.get(&pane).map_or(Level::Off, |grant| grant.level)
    }

    /// What `layout.json` keeps of `pane`: a level above Off the user picked, for its task, or one
    /// read back that waits for its window or is still offered.
    pub fn kept(&self, pane: PaneId) -> Option<Kept> {
        if let Some(kept) = self.restored.get(&pane).map(|waiting| waiting.kept).or(self.offers.get(&pane).copied()) {
            return Some(kept);
        }
        let grant = self.panes.get(&pane).filter(|grant| grant.level > Level::Off && self.chosen.contains(&pane))?;
        Some(Kept { level: grant.level, task: grant.task? })
    }
}

/// What a key does while the menu is open: ↓ ↑ move, ↵ takes, any other key closes. No item
/// has a letter: one key too many must never grant a level.
pub fn menu_key(key: &str) -> crate::model::Action {
    use crate::model::Action;
    match key {
        "down" => Action::AgentMenuMove { delta: 1 },
        "up" => Action::AgentMenuMove { delta: -1 },
        "enter" => Action::AgentMenuChoose { index: None },
        _ => Action::AgentMenuClose,
    }
}

/// An app's tree as a task may see it: every string on one line, of characters that show, at
/// most `MAX_TEXT`; a secure text field (a password) has no value and no children, whatever the app says. Text
/// is not stripped otherwise: reading it is what See is. Data, never instructions: the reply puts
/// it under `captured`, labelled (`capture::WARNING`).
pub fn seen(mut node: crate::ax::Node) -> crate::ax::Node {
    let clean =
        |text: &mut Option<String>| *text = text.take().map(|text| crate::capture::plain(&text, crate::ax::MAX_TEXT));
    let secure = |text: &Option<String>| text.as_deref() == Some("AXSecureTextField");
    // Nor anything under it: an app may echo the value in a child (`AXStaticText`).
    if node.role == "AXSecureTextField" || secure(&node.subrole) {
        node.value = None;
        node.children.clear();
    }
    node.role = crate::capture::plain(&node.role, crate::ax::MAX_TEXT);
    for text in [&mut node.subrole, &mut node.title, &mut node.description, &mut node.value, &mut node.identifier] {
        clean(text);
    }
    node.actions = node.actions.iter().map(|action| crate::capture::plain(action, crate::ax::MAX_TEXT)).collect();
    node.children = node.children.into_iter().map(seen).collect();
    node
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ax::Node;

    #[test]
    fn a_task_sees_no_password_and_no_hidden_characters() {
        let field = |role: &str, subrole: Option<&str>, value: &str| Node {
            role: role.into(),
            subrole: subrole.map(Into::into),
            value: Some(value.into()),
            ..Node::default()
        };
        let root = Node {
            role: "AXWindow".into(),
            title: Some("Inbox\u{202e}\u{200b}\nnew".into()),
            children: vec![
                field("AXSecureTextField", None, "hunter2"),
                Node {
                    children: vec![field("AXStaticText", None, "hunter4")],
                    ..field("AXTextField", Some("AXSecureTextField"), "hunter3")
                },
                field("AXTextField", None, &"a".repeat(500)),
            ],
            ..Node::default()
        };
        let seen = seen(root);
        assert_eq!(seen.title.as_deref(), Some("Inbox new"));
        assert_eq!((seen.children[0].value.as_deref(), seen.children[1].value.as_deref()), (None, None));
        assert!(seen.children[1].children.is_empty(), "no echo of the password under its field");
        assert_eq!(seen.children[2].value.as_ref().map(|v| v.chars().count()), Some(crate::ax::MAX_TEXT + 1));
        // The level's order: Act grants all See does.
        assert!(Level::Off < Level::See && Level::See < Level::Act);
        assert_eq!(Level::ALL.map(Level::label), ["Off", "See", "See and act"]);
    }
}
