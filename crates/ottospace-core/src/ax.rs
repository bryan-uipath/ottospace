//! Accessibility trees as plain data, and the rules sneak mode and agents build on: which nodes
//! a user can act on, and the keyboard labels they get. The shell reads trees; nothing here does I/O.

use serde::{Deserialize, Serialize};

use crate::model::PaneId;

/// A rect in points, top-left origin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Frame {
    fn intersects(&self, other: &Frame) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }

    fn clip(&self, to: &Frame) -> Frame {
        let (x, y) = (self.x.max(to.x), self.y.max(to.y));
        let (r, b) = ((self.x + self.w).min(to.x + to.w), (self.y + self.h).min(to.y + to.h));
        Frame { x, y, w: (r - x).max(0.0), h: (b - y).max(0.0) }
    }
}

/// One accessibility element. Strings are truncated (see [`MAX_TEXT`]); a missing or unreadable
/// attribute is `None`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// e.g. `AXButton`. Empty if the app did not say.
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subrole: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Global screen coordinates, as AX reports them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<Frame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// e.g. `AXPress`, `AXShowMenu`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

/// Bounds on a walk. Browser and Electron windows have trees of many thousands of nodes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    /// Nodes deeper than this are not read. The window is depth 0.
    pub depth: usize,
    pub nodes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { depth: 40, nodes: 3000 }
    }
}

/// Why a tree is incomplete. All false: it is the whole tree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Truncated {
    /// Some node at the depth limit had children.
    pub depth: bool,
    /// The node limit was reached.
    pub nodes: bool,
    /// The app did not answer in time (busy or hung); the walk stopped there.
    pub timeout: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tree {
    pub root: Node,
    /// Nodes read, root included.
    pub count: usize,
    pub truncated: Truncated,
}

/// Longest string kept per attribute, in characters.
pub const MAX_TEXT: usize = 120;

/// `text` cut to `max` characters, with `…` marking the cut. e.g. a text area's whole document.
pub fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// A node and where it is: child indices from the root (the root's path is empty).
#[derive(Debug, PartialEq)]
pub struct Flat<'a> {
    pub path: Vec<usize>,
    pub node: &'a Node,
}

/// Every node, in reading (pre-) order.
pub fn flatten(root: &Node) -> Vec<Flat<'_>> {
    fn walk<'a>(node: &'a Node, path: &mut Vec<usize>, out: &mut Vec<Flat<'a>>) {
        out.push(Flat { path: path.clone(), node });
        for (i, child) in node.children.iter().enumerate() {
            path.push(i);
            walk(child, path, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(root, &mut Vec::new(), &mut out);
    out
}

/// Actions that do what a click would. Not `AXShowMenu`: web views put it on nearly everything.
const PRESS: &[&str] = &["AXPress", "AXPick", "AXConfirm", "AXOpen"];
/// Roles that take typing or a value even without a press action.
const INPUT: &[&str] = &["AXTextField", "AXTextArea", "AXComboBox", "AXSearchField", "AXSlider"];

/// A node the user can act on: it presses like a button or takes input, is not disabled, and has
/// an area on screen to put a label on.
pub fn is_interactive(node: &Node) -> bool {
    let acts = node.actions.iter().any(|a| PRESS.contains(&a.as_str())) || INPUT.contains(&node.role.as_str());
    let visible = node.frame.is_some_and(|f| f.w > 0.0 && f.h > 0.0);
    acts && visible && node.enabled != Some(false)
}

/// Home row first, then the keys above it: the order labels are handed out in.
pub const ALPHABET: &str = "asdfghjklqwertyuiop";

/// `n` distinct labels from `alphabet`, none a prefix of another, so a label is chosen as soon
/// as it is typed. Short labels go first; e.g. 3 letters, 4 targets: `s d aa as`.
pub fn labels(n: usize, alphabet: &str) -> Vec<String> {
    let letters: Vec<char> = alphabet.chars().collect();
    if n == 0 || letters.is_empty() {
        return Vec::new();
    }
    // Breadth first: replace the shortest label with its one-letter extensions until there are
    // enough. One letter cannot make two prefix-free labels, so it labels one target.
    let mut queue = std::collections::VecDeque::from([String::new()]);
    while queue[0].is_empty() || (queue.len() < n && letters.len() > 1) {
        let prefix = queue.pop_front().unwrap_or_default();
        queue.extend(letters.iter().map(|c| format!("{prefix}{c}")));
    }
    queue.into_iter().take(n).collect()
}

/// What a label activates.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Source {
    /// A hosted app's accessibility node, by path from its window: activating it presses the node.
    App { pane: PaneId, path: Vec<usize>, role: String },
    /// One of ottospace's own controls, by an id the shell picks (e.g. `rail:apps`): activating it
    /// does what clicking it does.
    Own { id: String },
}

/// Something a label can go on, in ottospace window coordinates.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Target {
    pub frame: Frame,
    pub source: Source,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Labelled {
    pub label: String,
    #[serde(flatten)]
    pub target: Target,
}

/// A hosted window's interactive nodes as targets. `window` is its frame (global) when read;
/// `crop` the title-bar height cut off its stream; `content` the pane area showing the rest, in
/// ottospace window coordinates. The stream fills the pane, so a window that refused the pane's
/// size is scaled. Nodes the pane does not show (e.g. traffic lights in a cropped bar) are left out.
pub fn app_targets(pane: PaneId, root: &Node, window: Frame, crop: f64, content: Frame) -> Vec<Target> {
    let window = Frame { y: window.y + crop, h: window.h - crop, ..window };
    let sx = if window.w > 0.0 { content.w / window.w } else { 1.0 };
    let sy = if window.h > 0.0 { content.h / window.h } else { 1.0 };
    flatten(root)
        .into_iter()
        .filter(|f| is_interactive(f.node))
        .filter_map(|f| {
            let g = f.node.frame?;
            let (x, y) = (content.x + (g.x - window.x) * sx, content.y + (g.y - window.y) * sy);
            let frame = Frame { x, y, w: g.w * sx, h: g.h * sy };
            let source = Source::App { pane, path: f.path, role: f.node.role.clone() };
            // Clipped to the pane, so a partly scrolled-out control is labelled where it shows.
            frame.intersects(&content).then(|| Target { frame: frame.clip(&content), source })
        })
        .collect()
}

/// Labels for targets from any source, in one namespace for the whole window. Reading order (top
/// to bottom, then left to right) hands them out, so a control keeps its label while the targets
/// above and left of it stay the same.
pub fn assign(mut targets: Vec<Target>, alphabet: &str) -> Vec<Labelled> {
    targets.sort_by(|a, b| a.frame.y.total_cmp(&b.frame.y).then(a.frame.x.total_cmp(&b.frame.x)));
    let labels = labels(targets.len(), alphabet);
    targets.into_iter().zip(labels).map(|(target, label)| Labelled { label, target }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(role: &str, actions: &[&str], frame: [f64; 4], children: Vec<Node>) -> Node {
        Node {
            role: role.into(),
            actions: actions.iter().map(|a| a.to_string()).collect(),
            frame: Some(Frame { x: frame[0], y: frame[1], w: frame[2], h: frame[3] }),
            children,
            ..Node::default()
        }
    }

    /// A window at (100, 50): a toolbar with two buttons, a text field, and a label.
    fn window() -> Node {
        node(
            "AXWindow",
            &["AXRaise"],
            [100., 50., 400., 300.],
            vec![
                node(
                    "AXToolbar",
                    &[],
                    [100., 50., 400., 40.],
                    vec![
                        node("AXButton", &["AXPress"], [110., 55., 30., 30.], vec![]),
                        node("AXButton", &["AXPress", "AXShowMenu"], [150., 55., 30., 30.], vec![]),
                    ],
                ),
                node("AXTextField", &[], [110., 100., 200., 24.], vec![]),
                node("AXStaticText", &["AXShowMenu"], [110., 130., 200., 20.], vec![]),
            ],
        )
    }

    #[test]
    fn flatten_is_reading_order_with_paths() {
        let tree = window();
        let flat: Vec<(Vec<usize>, &str)> =
            flatten(&tree).into_iter().map(|f| (f.path, f.node.role.as_str())).collect();
        assert_eq!(
            flat,
            vec![
                (vec![], "AXWindow"),
                (vec![0], "AXToolbar"),
                (vec![0, 0], "AXButton"),
                (vec![0, 1], "AXButton"),
                (vec![1], "AXTextField"),
                (vec![2], "AXStaticText"),
            ]
        );
    }

    #[test]
    fn interactive_means_press_or_input_enabled_and_visible() {
        let press = node("AXGroup", &["AXPress"], [0., 0., 10., 10.], vec![]);
        assert!(is_interactive(&press), "a clickable group, as web views make them");
        assert!(is_interactive(&node("AXTextArea", &[], [0., 0., 10., 10.], vec![])));
        assert!(!is_interactive(&node("AXStaticText", &["AXShowMenu"], [0., 0., 10., 10.], vec![])));
        assert!(!is_interactive(&node("AXWindow", &["AXRaise"], [0., 0., 10., 10.], vec![])));
        assert!(!is_interactive(&Node { enabled: Some(false), ..press.clone() }), "disabled");
        assert!(!is_interactive(&Node { frame: None, ..press.clone() }), "no frame");
        assert!(!is_interactive(&node("AXButton", &["AXPress"], [5., 5., 0., 10.], vec![])), "no area");
    }

    #[test]
    fn labels_are_short_first_and_prefix_free() {
        assert_eq!(labels(0, "asd"), Vec::<String>::new());
        assert_eq!(labels(1, "asd"), ["a"]);
        assert_eq!(labels(3, "asd"), ["a", "s", "d"]);
        // One more than the alphabet: `a` becomes a prefix, so it goes.
        assert_eq!(labels(4, "asd"), ["s", "d", "aa", "as"]);
        assert_eq!(labels(9, "asd"), ["aa", "as", "ad", "sa", "ss", "sd", "da", "ds", "dd"]);
        for n in [5, 19, 20, 100, 400] {
            let got = labels(n, ALPHABET);
            assert_eq!(got.len(), n);
            for (i, a) in got.iter().enumerate() {
                for (j, b) in got.iter().enumerate() {
                    assert!(i == j || !b.starts_with(a.as_str()), "{a} is a prefix of {b} (n={n})");
                }
            }
            assert!(got.windows(2).all(|w| w[0].len() <= w[1].len()), "short labels first (n={n})");
        }
        assert_eq!(labels(3, "a"), ["a"], "one letter can label one target");
    }

    #[test]
    fn labels_are_stable_for_the_same_count() {
        assert_eq!(labels(25, ALPHABET), labels(25, ALPHABET));
        assert_eq!(&labels(19, ALPHABET)[..3], ["a", "s", "d"]);
    }

    fn own(id: &str, frame: [f64; 4]) -> Target {
        Target {
            frame: Frame { x: frame[0], y: frame[1], w: frame[2], h: frame[3] },
            source: Source::Own { id: id.into() },
        }
    }

    fn app_path(t: &Target) -> Option<&[usize]> {
        match &t.source {
            Source::App { path, .. } => Some(path),
            Source::Own { .. } => None,
        }
    }

    #[test]
    fn app_targets_move_from_the_app_window_into_the_pane() {
        // The window (global 100,50) shows in a pane whose content starts at (300, 60) in ottospace.
        let content = Frame { x: 300., y: 60., w: 400., h: 300. };
        let got = app_targets(7, &window(), Frame { x: 100., y: 50., w: 400., h: 300. }, 0., content);
        let at: Vec<[f64; 2]> = got.iter().map(|t| [t.frame.x, t.frame.y]).collect();
        assert_eq!(at, [[310., 65.], [350., 65.], [310., 110.]]);
        assert_eq!(got[1].source, Source::App { pane: 7, path: vec![0, 1], role: "AXButton".into() });
    }

    #[test]
    fn app_targets_scale_when_the_window_kept_its_own_size() {
        // A fixed-size 200x150 window streamed into a 400x300 pane: everything doubles.
        let tree = node(
            "AXWindow",
            &[],
            [0., 0., 200., 150.],
            vec![node("AXButton", &["AXPress"], [50., 20., 10., 10.], vec![])],
        );
        let got = app_targets(
            1,
            &tree,
            Frame { x: 0., y: 0., w: 200., h: 150. },
            0.,
            Frame { x: 10., y: 0., w: 400., h: 300. },
        );
        assert_eq!(got[0].frame, Frame { x: 110., y: 40., w: 20., h: 20. });
    }

    #[test]
    fn a_cropped_title_bar_shifts_targets_up_and_hides_its_own_controls() {
        // A 200x182 window with a 32pt bar: its 200x150 content fills a 400x300 pane at (60, 30).
        let tree = node(
            "AXWindow",
            &[],
            [100., 50., 200., 182.],
            vec![
                node("AXButton", &["AXPress"], [108., 58., 16., 16.], vec![]), // close: in the bar
                node("AXButton", &["AXPress"], [110., 92., 10., 10.], vec![]), // 10,10 into the content
                node("AXButton", &["AXPress"], [150., 75., 10., 20.], vec![]), // straddles the bar's edge
            ],
        );
        let window = Frame { x: 100., y: 50., w: 200., h: 182. };
        let got = app_targets(1, &tree, window, 32., Frame { x: 60., y: 30., w: 400., h: 300. });
        assert_eq!(got.iter().filter_map(app_path).collect::<Vec<_>>(), [[1], [2]]);
        assert_eq!(got[0].frame, Frame { x: 80., y: 50., w: 20., h: 20. });
        assert_eq!(got[1].frame, Frame { x: 160., y: 30., w: 20., h: 26. }, "the visible part");
    }

    #[test]
    fn app_targets_outside_the_pane_are_left_out() {
        let tree = node(
            "AXWindow",
            &[],
            [0., 0., 100., 100.],
            vec![
                node("AXButton", &["AXPress"], [10., 200., 20., 20.], vec![]), // scrolled below
                node("AXButton", &["AXPress"], [90., 90., 20., 20.], vec![]),  // partly inside
            ],
        );
        let window = Frame { x: 0., y: 0., w: 100., h: 100. };
        let got = app_targets(1, &tree, window, 0., window);
        assert_eq!(got.iter().filter_map(app_path).collect::<Vec<_>>(), [[1]]);
        assert_eq!(got[0].frame, Frame { x: 90., y: 90., w: 10., h: 10. }, "clipped to the pane");
    }

    #[test]
    fn one_namespace_for_own_controls_and_app_nodes_in_reading_order() {
        let content = Frame { x: 60., y: 30., w: 400., h: 300. };
        let mut all = vec![own("rail:apps", [8., 40., 44., 44.]), own("tab:1", [80., 0., 60., 28.])];
        all.extend(app_targets(1, &window(), Frame { x: 100., y: 50., w: 400., h: 300. }, 0., content));
        all.push(own("pane:1:close", [440., 30., 16., 16.]));
        let got = assign(all, "asdf");
        let order: Vec<(&str, String)> = got
            .iter()
            .map(|l| {
                let what = match &l.target.source {
                    Source::Own { id } => id.clone(),
                    Source::App { path, .. } => format!("app{path:?}"),
                };
                (l.label.as_str(), what)
            })
            .collect();
        // Rows: tab (y 0), close (30), the app's toolbar (35), rail (40), the app's text field (80).
        let expected = [
            ("s", "tab:1"),
            ("d", "pane:1:close"),
            ("f", "app[0, 0]"),
            ("aa", "app[0, 1]"),
            ("as", "rail:apps"),
            ("ad", "app[1]"),
        ];
        assert_eq!(order, expected.map(|(l, w)| (l, w.to_string())));
        // The same targets in another order get the same labels.
        let mut shuffled: Vec<Target> = got.iter().map(|l| l.target.clone()).collect();
        shuffled.reverse();
        assert_eq!(assign(shuffled, "asdf"), got);
    }

    /// Calculator's window as the reader returned it (values stripped, moved to (100, 50)).
    #[test]
    fn a_recorded_calculator_window_labels_its_buttons() {
        #[derive(Deserialize)]
        struct Recorded {
            window: Frame,
            tree: Tree,
        }
        let recorded: Recorded = serde_json::from_str(include_str!("fixtures/ax-calculator.json")).unwrap();
        assert_eq!(flatten(&recorded.tree.root).len(), recorded.tree.count);
        let w = recorded.window;
        let got = assign(app_targets(1, &recorded.tree.root, w, 0., w), ALPHABET);
        // 20 keys, 2 toolbar buttons, close and minimise. Zoom is disabled; the displays only scroll.
        assert_eq!(got.len(), 24);
        let node =
            |l: &Labelled| app_path(&l.target).map(|p| p.iter().fold(&recorded.tree.root, |n, &i| &n.children[i]));
        let nodes: Vec<&Node> = got.iter().filter_map(node).collect();
        assert!(nodes.iter().all(|n| n.role == "AXButton"));
        let names: Vec<&str> = nodes.iter().filter_map(|n| n.description.as_deref()).take(3).collect();
        assert_eq!(names, ["Show Sidebar", "Mode", "Delete"], "toolbar, then traffic lights, then keys");
        assert_eq!(nodes[2].subrole.as_deref(), Some("AXCloseButton"));
        assert!(!nodes.iter().any(|n| n.subrole.as_deref() == Some("AXZoomButton")));
        // More targets than letters: 18 single letters, then two-letter labels after `a`.
        assert_eq!((got[0].label.as_str(), got[17].label.as_str(), got[18].label.as_str()), ("s", "p", "aa"));
    }

    #[test]
    fn truncate_counts_characters_not_bytes() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("abcdef", 3), "abc…");
        assert_eq!(truncate("ééé", 2), "éé…");
    }

    #[test]
    fn a_tree_round_trips_through_json_without_empty_fields() {
        let tree = Tree { root: window(), count: 6, truncated: Truncated::default() };
        let json = serde_json::to_string(&tree).unwrap();
        assert!(!json.contains("subrole"), "absent fields are left out: {json}");
        assert_eq!(serde_json::from_str::<Tree>(&json).unwrap(), tree);
    }
}
