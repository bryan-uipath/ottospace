//! The accessibility inspector: a hosted pane's tree as an indented list to read. Read-only:
//! nothing here presses or sets anything. The app's text is for the screen only, cleaned of what
//! does not show (`capture::plain`) and cut; the state snapshot gets counts and roles, no text.

use serde::Serialize;

use crate::ax::{self, Frame, Limits, Node, Tree, Truncated};
use crate::capture::plain;
use crate::edit::Field;
use crate::model::PaneId;

/// How far a read goes: deep enough for a web view's controls, small and quick enough to read
/// again every few seconds. The reader also stops at the first call the app does not answer.
pub const LIMITS: Limits = Limits { depth: 30, nodes: 2000, millis: Some(750) };
/// Characters shown of a row's title, label and value together.
pub const MAX_SHOWN: usize = 60;
/// The most rows drawn; past them the panel says to filter.
pub const MAX_ROWS: usize = 400;
/// Fewer nodes than this, read whole: the app exposes almost nothing.
pub const FEW: usize = 25;
/// The most characters the filter holds.
pub const MAX_FILTER: usize = 80;
/// Why a client's request to the inspector is refused.
pub const USERS_ALONE: &str = "only the user opens and uses the accessibility inspector";

/// One read of the pane's window: its tree, the window's frame (global) and how long it took.
#[derive(Clone, Debug, PartialEq)]
pub struct Read {
    pub tree: Tree,
    pub window: Frame,
    pub millis: f64,
}

/// The open inspector, for one app pane. In the snapshot only what is not the app's: the tree,
/// the folds and the selection are skipped (see `Inspector::summary`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Inspector {
    pub pane: PaneId,
    /// The app's name, for the header.
    pub app: String,
    #[serde(skip)]
    pub read: Option<Read>,
    /// Why the last read failed, e.g. "the window is gone".
    pub error: Option<String>,
    /// What the user typed to narrow the rows.
    pub filter: Field,
    /// The filter has the keyboard.
    pub filtering: bool,
    /// Nodes whose children are hidden, by path (child indices from the root).
    #[serde(skip)]
    pub folded: Vec<Vec<usize>>,
    #[serde(skip)]
    pub selected: Option<Vec<usize>>,
}

/// A row as drawn. `text` is the app's, cleaned; it is never in the snapshot.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Row {
    pub path: Vec<usize>,
    pub depth: usize,
    /// e.g. `AXButton`, cleaned and cut.
    pub role: String,
    #[serde(skip)]
    pub text: String,
    pub pressable: bool,
    pub settable: bool,
    pub focused: bool,
    /// How many children it has, shown or not.
    pub children: usize,
    pub folded: bool,
}

/// What the header says: "Spotify: 3 nodes, 0 pressable".
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Counts {
    pub nodes: usize,
    pub pressable: usize,
    pub settable: usize,
    /// Each role and how many nodes have it, most first.
    pub roles: Vec<(String, usize)>,
    pub truncated: Truncated,
    /// Read whole, and fewer than `FEW` nodes: the app exposes almost nothing.
    pub sparse: bool,
}

/// The selected node's attributes, for the detail strip. Cleaned and cut, as rows are.
#[derive(Clone, Debug, PartialEq)]
pub struct Detail {
    pub role: String,
    pub subrole: Option<String>,
    pub identifier: Option<String>,
    pub frame: Option<Frame>,
    /// Their names only, e.g. `AXPress`. Nothing is ever performed.
    pub actions: Vec<String>,
}

fn clean(text: &str) -> String {
    plain(text, MAX_SHOWN)
}

/// A role as a row shows it: `Button` for `AXButton`.
pub fn short_role(role: &str) -> String {
    let role = clean(role);
    match role.strip_prefix("AX") {
        Some(rest) if !rest.is_empty() => rest.to_string(),
        _ if role.is_empty() => "?".into(),
        _ => role,
    }
}

fn node_at<'a>(root: &'a Node, path: &[usize]) -> Option<&'a Node> {
    path.iter().try_fold(root, |node, &i| node.children.get(i))
}

impl Inspector {
    pub fn new(pane: PaneId, app: &str) -> Self {
        Self {
            pane,
            app: clean(app),
            read: None,
            error: None,
            filter: Field::new("", MAX_FILTER),
            filtering: false,
            folded: Vec::new(),
            selected: None,
        }
    }

    /// A new read. Folds and the selection stay on the paths that are still there.
    pub fn take(&mut self, read: Read) {
        let root = &read.tree.root;
        self.folded.retain(|path| node_at(root, path).is_some());
        self.selected = self.selected.take().filter(|path| node_at(root, path).is_some());
        (self.read, self.error) = (Some(read), None);
    }

    pub fn failed(&mut self, why: &str) {
        self.error = Some(clean(why));
    }

    /// The rows, top to bottom. With a filter: the nodes whose role or text holds it (ignoring
    /// case) and the nodes above them, folds ignored. Else every node not under a fold.
    pub fn rows(&self) -> Vec<Row> {
        let Some(read) = &self.read else { return Vec::new() };
        let query = self.filter.text().to_lowercase();
        let mut out = Vec::new();
        self.walk(&read.tree.root, &mut Vec::new(), &query, &mut out);
        out
    }

    /// Adds `node`'s row and those under it; false (and nothing added) if none matches `query`.
    fn walk(&self, node: &Node, path: &mut Vec<usize>, query: &str, out: &mut Vec<Row>) -> bool {
        let at = out.len();
        let folded = self.folded.contains(path);
        let row = row(node, path.clone(), folded && query.is_empty());
        let hit = query.is_empty()
            || row.role.to_lowercase().contains(query)
            || node.role.to_lowercase().contains(query)
            || row.text.to_lowercase().contains(query);
        out.push(row);
        let mut below = false;
        if !folded || !query.is_empty() {
            for (i, child) in node.children.iter().enumerate() {
                path.push(i);
                below |= self.walk(child, path, query, out);
                path.pop();
            }
        }
        if !hit && !below {
            out.truncate(at);
        }
        hit || below
    }

    pub fn counts(&self) -> Counts {
        let Some(read) = &self.read else { return Counts::default() };
        let mut counts = Counts { truncated: read.tree.truncated, ..Counts::default() };
        for flat in ax::flatten(&read.tree.root) {
            let node = flat.node;
            counts.nodes += 1;
            counts.pressable += usize::from(ax::presses(node));
            counts.settable += usize::from(node.settable);
            let role = clean(&node.role);
            match counts.roles.iter_mut().find(|(r, _)| *r == role) {
                Some((_, n)) => *n += 1,
                None => counts.roles.push((role, 1)),
            }
        }
        counts.roles.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let t = counts.truncated;
        counts.sparse = counts.nodes < FEW && !(t.depth || t.nodes || t.timeout);
        counts
    }

    pub fn selected_node(&self) -> Option<&Node> {
        node_at(&self.read.as_ref()?.tree.root, self.selected.as_ref()?)
    }

    pub fn detail(&self) -> Option<Detail> {
        let node = self.selected_node()?;
        Some(Detail {
            role: clean(&node.role),
            subrole: node.subrole.as_deref().map(clean),
            identifier: node.identifier.as_deref().map(clean),
            frame: node.frame,
            actions: node.actions.iter().take(12).map(|a| clean(a)).collect(),
        })
    }

    /// Where the selected node shows on the pane (`crop` and `content` as `ax::on_pane` takes
    /// them), to outline it. None: nothing selected, no frame, or it does not show.
    pub fn outline(&self, crop: f64, content: Frame) -> Option<Frame> {
        let window = self.read.as_ref()?.window;
        ax::on_pane(self.selected_node()?.frame?, window, crop, content)
    }

    /// Fold or unfold the node at `path`; a fold over the selection moves it to that node.
    pub fn fold(&mut self, path: &[usize]) {
        let Some(read) = &self.read else { return };
        if node_at(&read.tree.root, path).is_none_or(|node| node.children.is_empty()) {
            return;
        }
        match self.folded.iter().position(|p| p == path) {
            Some(at) => drop(self.folded.remove(at)),
            None => {
                self.folded.push(path.to_vec());
                if self.selected.as_ref().is_some_and(|s| s.len() > path.len() && s.starts_with(path)) {
                    self.selected = Some(path.to_vec());
                }
            }
        }
    }

    /// Select the node at `path`, if there is one; `None` selects nothing.
    pub fn select(&mut self, path: Option<Vec<usize>>) {
        let root = self.read.as_ref().map(|read| &read.tree.root);
        self.selected = path.filter(|path| root.is_some_and(|root| node_at(root, path).is_some()));
    }

    /// Move the selection down (positive) or up the rows; it wraps. With none: the first row.
    pub fn step(&mut self, delta: i32) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let now = self.selected.as_ref().and_then(|path| rows.iter().position(|row| row.path == *path));
        let at = match now {
            Some(at) => (at as i64 + delta as i64).rem_euclid(rows.len() as i64) as usize,
            None => 0,
        };
        self.selected = Some(rows[at].path.clone());
    }
}

fn row(node: &Node, path: Vec<usize>, folded: bool) -> Row {
    let parts = [&node.title, &node.description, &node.value];
    let text: Vec<String> = parts.iter().filter_map(|t| t.as_deref()).map(clean).filter(|t| !t.is_empty()).collect();
    Row {
        depth: path.len(),
        path,
        role: short_role(&node.role),
        text: ax::truncate(&text.join(" · "), MAX_SHOWN),
        pressable: ax::presses(node),
        settable: node.settable,
        focused: node.focused,
        children: node.children.len(),
        folded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Edit;

    fn node(role: &str, title: &str, children: Vec<Node>) -> Node {
        Node {
            role: role.into(),
            title: (!title.is_empty()).then(|| title.into()),
            frame: Some(Frame { x: 10., y: 20., w: 30., h: 40. }),
            children,
            ..Node::default()
        }
    }

    fn fixture() -> Inspector {
        let mut press = node("AXButton", "Play", vec![]);
        press.actions = vec!["AXPress".into(), "AXShowMenu".into()];
        let mut field = node("AXTextField", "Search", vec![]);
        (field.settable, field.focused, field.identifier) = (true, true, Some("search".into()));
        let group = node("AXGroup", "", vec![press, field]);
        let root =
            node("AXWindow", "Fixture \u{202e}evil\u{7}", vec![group, node("AXStaticText", "Now playing", vec![])]);
        let tree = Tree { count: 5, root, truncated: Truncated::default() };
        let mut inspector = Inspector::new(1, "Fixture");
        inspector.take(Read { tree, window: Frame::default(), millis: 3.0 });
        inspector
    }

    fn shown(i: &Inspector) -> Vec<(usize, String, String)> {
        i.rows().into_iter().map(|r| (r.depth, r.role, r.text)).collect()
    }

    #[test]
    fn rows_are_the_tree_indented_with_the_apps_text_cleaned() {
        let i = fixture();
        let rows = shown(&i);
        assert_eq!(rows.len(), 5);
        // Bidi overrides and control characters are gone.
        assert_eq!(rows[0], (0, "Window".into(), "Fixture evil".into()));
        assert_eq!((rows[2].0, rows[2].1.as_str()), (2, "Button"));
        let flags = |at: usize| {
            let row = &i.rows()[at];
            (row.pressable, row.settable, row.focused)
        };
        assert_eq!((flags(2), flags(3)), ((true, false, false), (false, true, true)));
    }

    #[test]
    fn counts_answer_how_much_the_app_exposes() {
        let i = fixture();
        let counts = i.counts();
        assert_eq!((counts.nodes, counts.pressable, counts.settable, counts.sparse), (5, 1, 1, true));
        assert_eq!(counts.roles[0].1, 1);
        assert_eq!(counts.roles.len(), 5);
        // A truncated read is not "almost nothing", however few it holds.
        let mut cut = fixture();
        if let Some(read) = &mut cut.read {
            read.tree.truncated.timeout = true;
        }
        assert!(!cut.counts().sparse);
    }

    #[test]
    fn a_fold_hides_the_children_and_takes_the_selection_up() {
        let mut i = fixture();
        i.select(Some(vec![0, 1]));
        i.fold(&[0]);
        assert_eq!(shown(&i).len(), 3);
        assert_eq!(i.selected, Some(vec![0]));
        assert!(i.rows()[1].folded);
        // A leaf does not fold.
        i.fold(&[1]);
        assert_eq!(i.folded, vec![vec![0]]);
        i.fold(&[0]);
        assert_eq!(shown(&i).len(), 5);
    }

    #[test]
    fn a_filter_keeps_the_matches_and_what_is_above_them() {
        let mut i = fixture();
        i.fold(&[0]);
        i.filter.apply(Edit::Insert("PLAY".into()));
        // Folds do not hide a match.
        let rows = shown(&i);
        assert_eq!(rows.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), ["Window", "Group", "Button", "StaticText"]);
        i.filter.apply(Edit::Insert("zzz".into()));
        assert!(i.rows().is_empty());
        i.step(1);
        assert_eq!(i.selected, None);
    }

    #[test]
    fn the_selection_steps_over_the_rows_and_shows_its_detail() {
        let mut i = fixture();
        i.step(1);
        assert_eq!(i.selected, Some(vec![]));
        i.step(-1);
        assert_eq!(i.selected, Some(vec![1]));
        i.select(Some(vec![0, 1]));
        let detail = i.detail().expect("a node is selected");
        assert_eq!((detail.role.as_str(), detail.identifier.as_deref()), ("AXTextField", Some("search")));
        // A path that is not there selects nothing; a new read keeps what is still there.
        i.select(Some(vec![7]));
        assert_eq!(i.selected, None);
        i.select(Some(vec![0, 1]));
        i.fold(&[0]);
        let read = Read {
            tree: Tree { root: node("AXWindow", "", vec![]), count: 1, truncated: Truncated::default() },
            window: Frame::default(),
            millis: 1.0,
        };
        i.take(read);
        assert_eq!((i.selected.clone(), i.folded.clone()), (None, vec![]));
    }
}
