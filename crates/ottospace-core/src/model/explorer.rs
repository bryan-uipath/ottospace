//! The file explorer pane in the model (`crate::explorer`): which checkout it opens on, its keys
//! (arrows, ↵, letters typed to a name), clicks, and what a row's menu does. The shell lists a
//! folder when asked (`Effect::ListDir`) and says git's marks; never a name in the state.

use super::{Action, Effect, Kind, Mode, Model, PaneId};
use crate::explorer::{Entry, Explorer, Listing, MAX_ENTRIES, Mark, Row, TYPED_MS, listable};
use crate::layout::Axis;

/// Why a client's listing or marks are refused: only the shell reads the folders.
pub(crate) const SHELLS_ALONE: &str = "only ottospace lists an explorer's folders";
/// Said when ⌘⇧X finds no checkout to show.
pub(crate) const NO_CHECKOUT: &str = "No checkout on show: open one from Repos or ⌘P, then the explorer";

impl Model {
    /// The explorer of pane `pane`, if it is one.
    pub fn explorer(&self, pane: PaneId) -> Option<&Explorer> {
        self.explorers.get(&pane)
    }

    /// The focused explorer, while ottospace has the keyboard: its keys move it.
    pub fn keyboard_explorer(&self) -> Option<PaneId> {
        let id = self.focused.filter(|_| self.mode() == Mode::Normal && !self.keys)?;
        matches!(self.pane(id)?.kind, Kind::Explorer { .. }).then_some(id)
    }

    /// The row selected in explorer `pane`: its full path, and whether it is a folder. The pane's
    /// fact for what may follow it (the code city of the folder selected).
    pub fn explorer_selection(&self, pane: PaneId) -> Option<(String, bool)> {
        let Kind::Explorer { root } = &self.pane(pane)?.kind else { return None };
        let explorer = self.explorers.get(&pane)?;
        let rows = explorer.rows();
        let row = rows.get(explorer.selected_at(&rows)?)?;
        Some((format!("{}/{}", root.trim_end_matches('/'), row.path), row.dir))
    }

    /// ⌘⇧X, the palette or a Repos row: an explorer of `root` (default: the checkout on show),
    /// beside the focused pane, or in it if it is empty. Only a checkout the Repos section lists.
    pub(crate) fn explorer_open(&mut self, root: Option<String>, axis: Axis, effects: &mut Vec<Effect>) {
        let tab = self.tabs[self.main_at()].id;
        let root = root.or_else(|| self.empty_folder(super::city::At::Tab(tab)).map(String::from));
        let Some(root) = root.filter(|root| crate::viewer::sound(root) && self.repo_listed(root)) else {
            if self.by_user {
                effects.push(Effect::Notice { text: NO_CHECKOUT.into() });
            }
            return;
        };
        self.open(Kind::Explorer { root }, axis, None, effects);
    }

    /// The shell listed folder `dir` of explorer `pane` (`None`: it could not). Only names that
    /// are one plain path part are kept, at most `MAX_ENTRIES`.
    pub(super) fn explorer_listed(&mut self, pane: PaneId, dir: String, entries: Option<Vec<Entry>>, cut: bool) {
        let Some(explorer) = self.explorers.get_mut(&pane) else { return };
        if !matches!(explorer.listings.get(&dir), Some(Listing::Reading)) {
            return;
        }
        let listing = match entries {
            Some(mut entries) => {
                entries.retain(|entry| listable(&entry.name));
                let cut = cut || entries.len() > MAX_ENTRIES;
                entries.truncate(MAX_ENTRIES);
                Listing::Read { entries, cut }
            }
            None => Listing::Unread,
        };
        explorer.listings.insert(dir, listing);
        if explorer.selected.is_none() {
            explorer.selected = explorer.rows().first().map(|row| row.path.clone());
        }
    }

    /// Git's marks for explorer `pane`'s files, as the shell read them.
    pub(super) fn explorer_marks(&mut self, pane: PaneId, marks: Vec<(String, Mark)>) {
        if let Some(explorer) = self.explorers.get_mut(&pane) {
            explorer.marks = marks.into_iter().filter(|(path, _)| path.split('/').all(listable)).collect();
        }
    }

    /// A key in the focused explorer: ↑ ↓ move, → opens a folder (or goes into it), ← closes it
    /// (or goes to its parent), ↵ opens the file or turns the folder, a letter goes to the next
    /// name that starts with what was typed. True: it was the explorer's.
    pub(super) fn explorer_key(&mut self, key: &str, effects: &mut Vec<Effect>) {
        let Some(id) = self.keyboard_explorer() else { return };
        let now = self.now;
        let Some(explorer) = self.explorers.get_mut(&id) else { return };
        let rows = explorer.rows();
        let at = explorer.selected_at(&rows);
        let row = at.and_then(|at| rows.get(at)).cloned();
        let go = |to: usize| rows.get(to).map(|row| row.path.clone());
        let to = match key {
            "down" => go(at.map_or(0, |at| (at + 1).min(rows.len().saturating_sub(1)))),
            "up" => go(at.map_or(0, |at| at.saturating_sub(1))),
            "right" => match &row {
                Some(row) if row.dir && !row.open => return self.explorer_turn(id, &row.path, effects),
                Some(row) if row.dir => go(at.map_or(0, |at| at + 1)).filter(|next| next.starts_with(&row.path)),
                _ => None,
            },
            "left" => match &row {
                Some(row) if row.dir && row.open => return self.explorer_turn(id, &row.path, effects),
                Some(row) => row.path.rsplit_once('/').map(|(parent, _)| parent.to_string()),
                None => None,
            },
            "enter" => {
                if let Some(row) = row {
                    self.explorer_choose(id, &row, Axis::Right, effects);
                }
                return;
            }
            typed if typed.chars().count() == 1 && !typed.chars().any(char::is_control) => {
                let fresh = now.saturating_sub(explorer.typed_at) > TYPED_MS;
                if fresh {
                    explorer.typed.clear();
                }
                explorer.typed.push_str(&typed.to_lowercase());
                explorer.typed_at = now;
                // From the selected row on (the next one when a fresh letter repeats), wrapping.
                let from = at.map_or(0, |at| if fresh { at + 1 } else { at });
                let typed = explorer.typed.clone();
                let found = (0..rows.len())
                    .map(|n| (from + n) % rows.len().max(1))
                    .find(|at| rows.get(*at).is_some_and(|row| row.name().to_lowercase().starts_with(&typed)));
                found.and_then(go)
            }
            _ => None,
        };
        if let Some(to) = to
            && let Some(explorer) = self.explorers.get_mut(&id)
        {
            explorer.selected = Some(to);
        }
    }

    /// Whether `key` is the focused explorer's: else it is a shortcut's.
    pub fn explorer_takes(&self, key: &str) -> bool {
        self.keyboard_explorer().is_some()
            && (matches!(key, "up" | "down" | "left" | "right" | "enter")
                || (key.chars().count() == 1 && !key.chars().any(char::is_control) && key != " "))
    }

    /// A click on row `at` of explorer `pane`: selected; a folder turns, a file opens in the
    /// viewer beside (the same one for the next file).
    pub(super) fn explorer_click(&mut self, pane: PaneId, at: usize, effects: &mut Vec<Effect>) {
        let Some(explorer) = self.explorers.get_mut(&pane) else { return };
        let Some(row) = explorer.rows().get(at).cloned() else { return };
        explorer.selected = Some(row.path.clone());
        self.focus(pane, effects);
        self.explorer_choose(pane, &row, Axis::Right, effects);
    }

    /// Ignored files shown, or not.
    pub(super) fn explorer_ignored(&mut self, pane: PaneId) {
        if let Some(explorer) = self.explorers.get_mut(&pane) {
            explorer.show_ignored = !explorer.show_ignored;
            let rows = explorer.rows();
            if explorer.selected_at(&rows).is_none() {
                explorer.selected = rows.first().map(|row| row.path.clone());
            }
        }
    }

    /// A folder opened (listed when it is first) or closed.
    fn explorer_turn(&mut self, pane: PaneId, path: &str, effects: &mut Vec<Effect>) {
        let Some(Kind::Explorer { root }) = self.pane(pane).map(|p| p.kind.clone()) else { return };
        let Some(explorer) = self.explorers.get_mut(&pane) else { return };
        if !explorer.open.remove(path) {
            explorer.open.insert(path.to_string());
            if !explorer.listings.contains_key(path) {
                explorer.listings.insert(path.to_string(), Listing::Reading);
                effects.push(Effect::ListDir { pane, root, dir: path.to_string() });
            }
        }
    }

    /// ↵ or a click on a row: a folder turns; a file opens in a viewer `axis` of the explorer,
    /// the one it opened before reused. A link is never read through.
    fn explorer_choose(&mut self, pane: PaneId, row: &Row, axis: Axis, effects: &mut Vec<Effect>) {
        let Some(Kind::Explorer { root }) = self.pane(pane).map(|p| p.kind.clone()) else { return };
        if row.link {
            return;
        }
        if row.dir {
            return self.explorer_turn(pane, &row.path, effects);
        }
        let path = format!("{}/{}", root.trim_end_matches('/'), row.path);
        let reuse = axis == Axis::Right;
        self.act(Action::OpenFile { path, axis, beside: Some(pane), reuse, at: None }, effects);
    }

    /// An item of a row's menu (`row_menu`): open, open below, reveal in Finder, copy its path,
    /// or a terminal in a folder.
    pub(crate) fn explorer_act(
        &mut self,
        pane: PaneId,
        path: &str,
        item: crate::row_menu::Item,
        effects: &mut Vec<Effect>,
    ) {
        use crate::row_menu::Item;
        let Some(Kind::Explorer { root }) = self.pane(pane).map(|p| p.kind.clone()) else { return };
        let Some(row) = self.explorers.get(&pane).and_then(|e| e.rows().into_iter().find(|row| row.path == path))
        else {
            return;
        };
        let full = format!("{}/{}", root.trim_end_matches('/'), row.path);
        match item {
            Item::Open => self.explorer_choose(pane, &row, Axis::Right, effects),
            Item::OpenBelow => self.explorer_choose(pane, &row, Axis::Down, effects),
            Item::Reveal if !row.link => effects.push(Effect::Reveal { path: full }),
            Item::CopyPath => effects.push(Effect::CopyText { text: full }),
            Item::TerminalHere if row.dir => {
                self.show_tab_of(pane, effects);
                self.open_terminal_at(Axis::Right, Some(full), effects);
            }
            _ => {}
        }
    }

    /// After every step: one explorer for each explorer pane, its root listed when it is new.
    pub(super) fn settle_explorers(&mut self, effects: &mut Vec<Effect>) {
        let panes: Vec<(PaneId, String)> = self
            .panes
            .iter()
            .filter_map(|pane| match &pane.kind {
                Kind::Explorer { root } => Some((pane.id, root.clone())),
                _ => None,
            })
            .collect();
        self.explorers.retain(|id, _| panes.iter().any(|(pane, _)| pane == id));
        for (id, root) in panes {
            if !self.explorers.contains_key(&id) {
                let mut explorer = Explorer::default();
                explorer.listings.insert(String::new(), Listing::Reading);
                self.explorers.insert(id, explorer);
                effects.push(Effect::ListDir { pane: id, root: root.clone(), dir: String::new() });
                effects.push(Effect::ExplorerMarks { pane: id, root });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Origin, Repo};
    use crate::row_menu::Item;

    const ROOT: &str = "/code/app";

    fn entry(name: &str, dir: bool) -> Entry {
        Entry { name: name.into(), dir, link: false, ignored: false }
    }

    /// An explorer of a listed checkout, its root listed: `src/` and two files.
    fn opened() -> (Model, PaneId) {
        let mut m = Model::default();
        let repo = Repo { name: "app".into(), path: ROOT.into(), branch: Some("main".into()), worktrees: vec![] };
        m.update(Action::SetRepos { repos: vec![repo] });
        m.update(Action::Open { kind: Kind::Terminal, axis: Axis::Right, beside: None });
        let effects = m.update(Action::ExplorerOpen { root: Some(ROOT.into()), axis: Axis::Right });
        let id = m.focused.unwrap_or_default();
        assert!(effects.contains(&Effect::ListDir { pane: id, root: ROOT.into(), dir: String::new() }));
        let entries = vec![entry("src", true), entry("README.md", false), entry("build.rs", false), entry("..", true)];
        let listed = Action::ExplorerListed { pane: id, dir: String::new(), entries: Some(entries), cut: false };
        m.update(listed.sent_by(Origin::System));
        (m, id)
    }

    fn names(m: &Model, id: PaneId) -> Vec<String> {
        m.explorer(id).map(|e| e.rows().into_iter().map(|row| row.path).collect()).unwrap_or_default()
    }

    fn key(m: &mut Model, key: &str) -> Vec<Effect> {
        m.update(Action::ExplorerKey { key: key.into() })
    }

    #[test]
    fn an_explorer_opens_on_a_listed_checkout_only_and_lists_its_root() {
        let (m, id) = opened();
        assert_eq!(names(&m, id), ["src", "build.rs", "README.md"], "`..` is never listed");
        assert_eq!(m.explorer_selection(id), Some(("/code/app/src".into(), true)));
        // Elsewhere: a notice, no pane.
        let mut home = Model::default();
        let said = home.update(Action::ExplorerOpen { root: None, axis: Axis::Right });
        assert_eq!((said, home.panes.len()), (vec![Effect::Notice { text: NO_CHECKOUT.into() }], 0));
        let mut other = Model::default();
        other.update(Action::ExplorerOpen { root: Some("/etc".into()), axis: Axis::Right });
        assert!(other.panes.is_empty(), "a folder Repos does not list");
    }

    #[test]
    fn keys_move_open_folders_and_open_files_in_one_viewer() {
        let (mut m, id) = opened();
        // → opens `src`, listed once.
        let asked = key(&mut m, "right");
        assert_eq!(asked, [Effect::ListDir { pane: id, root: ROOT.into(), dir: "src".into() }]);
        let listed = Action::ExplorerListed {
            pane: id,
            dir: "src".into(),
            entries: Some(vec![entry("main.rs", false)]),
            cut: false,
        };
        m.update(listed.sent_by(Origin::System));
        assert_eq!(names(&m, id), ["src", "src/main.rs", "build.rs", "README.md"]);
        key(&mut m, "right");
        assert_eq!(m.explorer_selection(id), Some(("/code/app/src/main.rs".into(), false)));
        // ← goes to its folder, ← again closes it, and it is not read again.
        key(&mut m, "left");
        key(&mut m, "left");
        assert_eq!(names(&m, id).len(), 3);
        assert!(key(&mut m, "right").is_empty());
        // Letters go to a name; ↵ opens it beside, the same viewer for the next file.
        key(&mut m, "r");
        assert_eq!(m.explorer_selection(id), Some(("/code/app/README.md".into(), false)));
        key(&mut m, "enter");
        let viewers: Vec<PaneId> = m.viewers.keys().copied().collect();
        assert_eq!(viewers.len(), 1);
        assert_eq!(m.focused, Some(id), "the keyboard stays in the explorer");
        key(&mut m, "up");
        key(&mut m, "enter");
        assert_eq!(m.viewers.keys().copied().collect::<Vec<_>>(), viewers);
    }

    #[test]
    fn a_rows_menu_reveals_copies_or_opens_a_terminal_and_a_link_is_never_followed() {
        let (mut m, id) = opened();
        let mut effects = Vec::new();
        m.explorer_act(id, "build.rs", Item::CopyPath, &mut effects);
        m.explorer_act(id, "build.rs", Item::Reveal, &mut effects);
        assert_eq!(
            effects,
            [
                Effect::CopyText { text: "/code/app/build.rs".into() },
                Effect::Reveal { path: "/code/app/build.rs".into() }
            ]
        );
        let panes = m.panes.len();
        m.explorer_act(id, "src", Item::TerminalHere, &mut Vec::new());
        assert_eq!(m.panes.len(), panes + 1);
        // A link: no read through it, nor a reveal.
        let mut link = entry("out", true);
        link.link = true;
        let (mut m, id) = opened();
        let listed = Action::ExplorerListed { pane: id, dir: String::new(), entries: Some(vec![link]), cut: false };
        // Listed once only: a second answer for the root changes nothing.
        m.update(listed.sent_by(Origin::System));
        assert_eq!(names(&m, id).len(), 3);
    }

    #[test]
    fn only_the_shell_lists_and_the_state_has_no_name() {
        let (mut m, id) = opened();
        key(&mut m, "right");
        let evil =
            Action::ExplorerListed { pane: id, dir: "src".into(), entries: Some(vec![entry("x", false)]), cut: false };
        m.update(evil.sent_by(Origin::Socket));
        assert_eq!(names(&m, id).len(), 3);
        let state = serde_json::to_string(&m).unwrap_or_default();
        assert!(!state.contains("README") && !state.contains("build.rs"), "{state}");
    }
}
