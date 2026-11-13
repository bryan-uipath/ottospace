//! Sneak labels inside a hosted app's window: which of its accessibility elements get one, the
//! letters they get beside ottospace's own, and whether the element is still the one labelled
//! when its label is typed. The shell reads the tree and presses; the rules are here.
//!
//! Only the user's own typed label presses (`Model::refusal`). No value is set, no menu opened,
//! nothing scrolled: a button is pressed (`AXPress`), a text field focused. See `docs/ai/sneak-apps.md`.

use std::collections::VecDeque;

use serde::Serialize;

use crate::ax::{self, Frame, Labelled, Node, Source, Target};
use crate::model::PaneId;
use crate::sneak::{self, Sneak};

/// Most labels an app pane gets: those nearest its top left.
pub const CAP: usize = 150;
/// The smallest side, in points, of a shown element that gets a label.
pub const MIN_SIDE: f64 = 4.0;
/// How far, in points, an element may have moved in its window since it was read and still be pressed.
pub const TOLERANCE: f64 = 2.0;

/// Why a client's letter does not choose an app's label.
pub const USERS_ALONE: &str =
    "a sneak label in another app is the user's alone: no client presses a control in another app";
/// The status bar, when the element is no longer the one labelled.
pub const MOVED: &str = "the control moved or changed: nothing was pressed";
/// The status bar, when the element is disabled now.
pub const DISABLED: &str = "the control is disabled now: nothing was pressed";

/// Roles a label focuses rather than presses, so the user can type in them. No value is set.
const FOCUS: &[&str] = &["AXTextField", "AXTextArea", "AXSearchField", "AXComboBox"];

/// A hosted window as read for sneak mode: its tree with no text, and where the pane shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct AppRead {
    /// The window's frame, global, read just before the tree.
    pub window: Frame,
    /// The title-bar height cut off the parked stream.
    pub crop: f64,
    /// The pane's content area, in ottospace window coordinates.
    pub content: Frame,
    pub root: Node,
}

/// What the state tells of the app's labels: counts, no text.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct AppLabels {
    pub pane: PaneId,
    /// Labels on its elements now.
    pub controls: usize,
    /// Elements that could take one when last read; more than `CAP` get none.
    pub found: usize,
}

/// The tree with every text of the app's left out (title, description, value): sneak mode places
/// labels by role and frame, and another app's text has no place in the model or the state.
pub fn without_text(mut node: Node) -> Node {
    (node.title, node.description, node.value) = (None, None, None);
    node.children = node.children.into_iter().map(without_text).collect();
    node
}

/// Whether a label on an element of this role focuses it (a text field) rather than presses it.
pub fn focuses(role: &str) -> bool {
    FOCUS.contains(&role)
}

/// What a label in an app does, once the element is checked again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Press {
    Press,
    Focus,
}

fn acts(node: &Node) -> Option<Press> {
    match () {
        _ if focuses(&node.role) => Some(Press::Focus),
        _ if node.actions.iter().any(|a| a == "AXPress") => Some(Press::Press),
        _ => None,
    }
}

/// The elements that get labels, nearest the pane's top left first, and how many could have:
/// enabled, pressable or a text field, shown in the pane at least `MIN_SIDE` each way, and the
/// innermost of nested ones (a button in a pressable row, not the row). Frames in ottospace
/// window coordinates, clipped to what shows; `at` is each frame in its window, as read.
pub fn targets(pane: PaneId, read: &AppRead) -> (Vec<Target>, usize) {
    let AppRead { window, crop, content, root } = read;
    let found: Vec<Target> = ax::flatten(root)
        .into_iter()
        .filter(|f| acts(f.node).is_some() && f.node.enabled != Some(false))
        .filter_map(|f| {
            let g = f.node.frame?;
            let clipped = ax::on_pane(g, *window, *crop, *content)?;
            let at = Frame { x: g.x - window.x, y: g.y - window.y, ..g };
            let source = Source::App { pane, path: f.path, role: f.node.role.clone(), at };
            (clipped.w >= MIN_SIDE && clipped.h >= MIN_SIDE).then_some(Target { frame: clipped, source })
        })
        .collect();
    let path = |t: &Target| match &t.source {
        Source::App { path, .. } => path.clone(),
        Source::Own { .. } => Vec::new(),
    };
    let paths: Vec<Vec<usize>> = found.iter().map(path).collect();
    let outer = |p: &[usize]| paths.iter().any(|q| q.len() > p.len() && q.starts_with(p));
    let mut inner: Vec<Target> = found.into_iter().filter(|t| !outer(&path(t))).collect();
    let total = inner.len();
    let far = |t: &Target| (t.frame.x - content.x).hypot(t.frame.y - content.y);
    inner.sort_by(|a, b| far(a).total_cmp(&far(b)));
    inner.truncate(CAP);
    (inner, total)
}

/// `n` labels that are not a label of `taken`, nor start one, nor start with one, and none of
/// which starts another: shortest first. e.g. `a s` taken, 2 more: `d f`.
fn more_labels(taken: &[&str], n: usize) -> Vec<String> {
    let letters: Vec<char> = ax::ALPHABET.chars().collect();
    let extend = |prefix: &str| letters.iter().map(|c| format!("{prefix}{c}")).collect::<Vec<_>>();
    // The labels free of `taken`, by length: walk down only where a taken label goes on.
    let (mut free, mut under) = (VecDeque::new(), VecDeque::from([String::new()]));
    while let Some(prefix) = under.pop_front() {
        for label in extend(&prefix) {
            match () {
                _ if taken.contains(&label.as_str()) => {}
                _ if taken.iter().any(|t| t.starts_with(&label)) => under.push_back(label),
                _ => free.push_back(label),
            }
        }
    }
    // Split the shortest until there are enough.
    while !free.is_empty() && free.len() < n {
        let Some(shortest) = free.pop_front() else { break };
        free.extend(extend(&shortest));
    }
    free.into_iter().take(n).collect()
}

/// The same element: its path, its role, and its place in its window within `TOLERANCE`. One
/// that moved further is another element as far as its label goes: the label never moves to it.
fn same(a: &Source, b: &Source) -> bool {
    match (a, b) {
        (Source::App { pane, path, role, at }, Source::App { pane: p, path: q, role: r, at: b }) => {
            let off = [at.x - b.x, at.y - b.y, at.w - b.w, at.h - b.h];
            pane == p && path == q && role == r && off.iter().all(|d| d.abs() <= TOLERANCE)
        }
        _ => false,
    }
}

impl Sneak {
    /// The app pane's elements, as just read (`targets`). The first read labels them with
    /// letters ottospace's own labels and what was typed leave free, so those stay as they are.
    /// A later read keeps each label on its element (`same`) and drops the label of one that is
    /// gone or moved; a new element gets none until the mode starts again. So a label is never
    /// handed from one element to another while the mode is on.
    pub fn read_app(&mut self, pane: PaneId, mut found: Vec<Target>, total: usize) {
        // A read that found nothing (an app still building its tree) labels nothing yet.
        let first = self.app.is_none_or(|app| app.pane != pane || app.controls == 0);
        let is_app = |t: &Labelled| matches!(t.target.source, Source::App { .. });
        if first {
            self.targets.retain(|t| !is_app(t));
            ax::reading_order(&mut found);
            // What the user typed counts as taken: a label that arrives after a letter never goes
            // on with it, so every letter of an app label is typed after the label was drawn.
            let typed = Some(self.typed.as_str()).filter(|t| !t.is_empty());
            let taken: Vec<&str> = self.targets.iter().map(|t| t.label.as_str()).chain(typed).collect();
            // None when own labels used every letter: those never change under the user's eyes.
            let labels = more_labels(&taken, found.len());
            if labels.len() == found.len() {
                let labelled = found.into_iter().zip(labels).map(|(target, label)| Labelled { label, target });
                self.targets.extend(labelled);
            }
        } else {
            self.targets.retain_mut(|t| match found.iter().find(|f| same(&f.source, &t.target.source)) {
                _ if !is_app(t) => true,
                Some(now) => {
                    t.target = now.clone();
                    true
                }
                None => false,
            });
        }
        let controls = self.targets.iter().filter(|t| is_app(t)).count();
        self.app = Some(AppLabels { pane, controls, found: total });
    }

    /// The app pane's window is gone or no longer shown: its labels go.
    pub fn drop_app(&mut self) {
        self.targets.retain(|t| matches!(t.target.source, Source::Own { .. }));
        self.app = None;
    }

    /// Whether these letters would choose a label in an app, or start one: only the user's may,
    /// so no client picks what the user's next letter presses there.
    pub fn reaches_app(&self, text: &str) -> bool {
        let mut sneak = self.clone();
        for letter in text.chars() {
            match sneak.type_letter(letter.to_ascii_lowercase()) {
                sneak::Typed::App(_) => return true,
                sneak::Typed::Narrowed => {}
                sneak::Typed::Chosen(_) | sneak::Typed::NoLabel => return false,
            }
        }
        let app = |t: &Labelled| matches!(t.target.source, Source::App { .. });
        !sneak.typed.is_empty() && sneak.targets.iter().any(|t| app(t) && t.label.starts_with(&sneak.typed))
    }
}

/// At press time: whether `now`, the element found again at the label's path, is still the one
/// labelled: the same role, not disabled, still pressable (or a text field), and where it was in
/// its window (`window`, read with it) within `TOLERANCE`. A stale element is never pressed.
pub fn check(role: &str, at: Frame, window: Frame, now: &Node) -> Result<Press, &'static str> {
    let press = acts(now).filter(|_| now.role == role).ok_or(MOVED)?;
    if now.enabled == Some(false) {
        return Err(DISABLED);
    }
    let f = now.frame.ok_or(MOVED)?;
    let off = [f.x - window.x - at.x, f.y - window.y - at.y, f.w - at.w, f.h - at.h];
    // NaN is never within: `<=` is false.
    off.iter().all(|d| d.abs() <= TOLERANCE).then_some(press).ok_or(MOVED)
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

    fn button(x: f64, y: f64) -> Node {
        node("AXButton", &["AXPress"], [x, y, 30., 20.], vec![])
    }

    /// A 400x300 window at (100, 50), no title bar cropped, shown 1:1 in a pane at (0, 0).
    fn read(children: Vec<Node>) -> AppRead {
        AppRead {
            window: Frame { x: 100., y: 50., w: 400., h: 300. },
            crop: 0.,
            content: Frame { x: 0., y: 0., w: 400., h: 300. },
            root: node("AXWindow", &["AXRaise"], [100., 50., 400., 300.], children),
        }
    }

    fn paths(targets: &[Target]) -> Vec<Vec<usize>> {
        targets
            .iter()
            .map(|t| match &t.source {
                Source::App { path, .. } => path.clone(),
                Source::Own { .. } => panic!("an own control"),
            })
            .collect()
    }

    #[test]
    fn pressable_enabled_shown_elements_get_labels_and_text_fields_focus() {
        let disabled = Node { enabled: Some(false), ..button(150., 60.) };
        let tiny = node("AXButton", &["AXPress"], [200., 60., 3., 20.], vec![]);
        let below = button(110., 400.); // scrolled below the window
        let menu_only = node("AXStaticText", &["AXShowMenu"], [250., 60., 30., 20.], vec![]);
        let field = node("AXTextField", &[], [110., 100., 200., 24.], vec![]);
        let read = read(vec![button(110., 60.), disabled, tiny, below, menu_only, field]);
        let (got, found) = targets(1, &read);
        assert_eq!((paths(&got), found), (vec![vec![0], vec![5]], 2));
        assert_eq!(got[0].frame, Frame { x: 10., y: 10., w: 30., h: 20. });
        let Source::App { at, role, .. } = &got[1].source else { panic!("an app's") };
        assert_eq!((*at, focuses(role)), (Frame { x: 10., y: 50., w: 200., h: 24. }, true));
    }

    #[test]
    fn nested_pressables_label_the_innermost() {
        // A pressable row holding a button; a pressable group holding nothing pressable.
        let row = node("AXRow", &["AXPress"], [100., 60., 400., 30.], vec![button(400., 65.)]);
        let group = node(
            "AXGroup",
            &["AXPress"],
            [100., 100., 400., 30.],
            vec![node("AXImage", &[], [110., 105., 20., 20.], vec![])],
        );
        let (got, _) = targets(1, &read(vec![row, group]));
        assert_eq!(paths(&got), [vec![1], vec![0, 0]], "nearest the top left first");
    }

    #[test]
    fn a_cropped_title_bar_hides_its_controls_and_shifts_the_rest() {
        let mut read = read(vec![button(108., 55.), button(110., 92.)]);
        read.crop = 32.;
        let (got, _) = targets(1, &read);
        assert_eq!(paths(&got), [vec![1]]);
        assert_eq!(got[0].frame, Frame { x: 10., y: 10., w: 30., h: 20. });
        let Source::App { at, .. } = got[0].source else { panic!("an app's") };
        assert_eq!(at, Frame { x: 10., y: 42., w: 30., h: 20. }, "in the window, bar and all");
    }

    #[test]
    fn at_most_cap_labels_nearest_the_top_left() {
        let grid: Vec<Node> =
            (0..200).map(|i| button(100. + (i % 10) as f64 * 35., 50. + (i / 10) as f64 * 14.)).collect();
        let (got, found) = targets(1, &read(grid));
        assert_eq!((got.len(), found), (CAP, 200));
        let far = |t: &Target| t.frame.x.hypot(t.frame.y);
        let kept = got.iter().map(far).fold(0., f64::max);
        assert!(got.iter().any(|t| t.frame == Frame { x: 0., y: 0., w: 30., h: 20. }));
        assert!(kept < 330., "the farthest kept is {kept}");
    }

    fn own(id: &str, x: f64) -> Target {
        Target { frame: Frame { x, y: 0., w: 20., h: 20. }, source: Source::Own { id: id.into() } }
    }

    fn assert_prefix_free(sneak: &Sneak) {
        let labels: Vec<&str> = sneak.targets.iter().map(|t| t.label.as_str()).collect();
        for (i, a) in labels.iter().enumerate() {
            assert!(
                labels.iter().enumerate().all(|(j, b)| i == j || !b.starts_with(a)),
                "{a} starts another: {labels:?}"
            );
        }
    }

    #[test]
    fn app_labels_take_letters_own_labels_leave_free_and_own_labels_stay() {
        let mut sneak = Sneak::new(vec![own("rail:apps", 0.), own("close:1", 30.), own("pane:1", 60.)]).unwrap();
        let before = sneak.targets.clone();
        let (found, n) = targets(1, &read((0..40).map(|i| button(100. + i as f64 * 9., 60.)).collect()));
        sneak.read_app(1, found, n);
        assert_eq!(&sneak.targets[..3], &before[..], "own labels are as they were");
        assert_eq!((sneak.targets.len(), sneak.app.map(|a| a.controls)), (43, Some(40)));
        assert_prefix_free(&sneak);
        // Every label names exactly one element.
        let mut labels: Vec<&str> = sneak.targets.iter().map(|t| t.label.as_str()).collect();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), 43);
    }

    #[test]
    fn own_labels_never_change_and_leave_room_when_asked() {
        // 19 controls take every letter: the app gets no label, and the own ones stay.
        let own: Vec<Target> = (0..19).map(|i| own(&format!("c{i}"), i as f64 * 30.)).collect();
        let mut full = Sneak::new(own.clone()).unwrap();
        let before = full.targets.clone();
        let (found, n) = targets(1, &read(vec![button(110., 60.)]));
        full.read_app(1, found.clone(), n);
        assert_eq!(full.targets, before);
        // With room left, as for an app pane in focus, the app's elements get labels.
        for count in [1, 19, 37, 400] {
            let own: Vec<Target> = (0..count).map(|i| self::own(&format!("c{i}"), i as f64 * 30.)).collect();
            let mut sneak = Sneak::leaving_room(own, true).unwrap();
            sneak.read_app(1, found.clone(), n);
            assert_eq!(sneak.targets.len(), count + 1, "{count} own controls");
            assert_prefix_free(&sneak);
        }
    }

    #[test]
    fn typing_an_app_label_chooses_its_element_and_a_later_read_keeps_or_drops_labels() {
        let mut sneak = Sneak::new(vec![own("pane:1", 0.)]).unwrap();
        let (found, n) = targets(1, &read(vec![button(110., 60.), button(150., 60.)]));
        sneak.read_app(1, found, n);
        let label = |sneak: &Sneak, path: usize| {
            let t =
                sneak.targets.iter().find(|t| matches!(&t.target.source, Source::App { path: p, .. } if p == &[path]));
            t.map(|t| t.label.clone())
        };
        let (first, second) = (label(&sneak, 0).unwrap(), label(&sneak, 1).unwrap());
        // Read again: the first moved a point down, the second is gone, a third is new.
        let (found, n) = targets(
            1,
            &read(vec![button(110., 61.), node("AXGroup", &[], [0., 0., 1., 1.], vec![]), button(200., 60.)]),
        );
        sneak.read_app(1, found, n);
        assert_eq!((label(&sneak, 0), label(&sneak, 1), label(&sneak, 2)), (Some(first.clone()), None, None));
        let moved = sneak.targets.iter().find(|t| t.label == first).unwrap().target.clone();
        assert_eq!(moved.frame.y, 11.);
        assert!(sneak.reaches_app(&first) && !sneak.reaches_app(&second));
        assert_eq!(sneak.type_letter(first.chars().next().unwrap()), sneak::Typed::App(moved));
        sneak.drop_app();
        assert_eq!((sneak.targets.len(), sneak.app), (1, None));
    }

    #[test]
    fn a_new_frame_of_own_controls_keeps_the_app_labels() {
        let mut sneak = Sneak::new(vec![own("pane:1", 0.)]).unwrap();
        let (found, n) = targets(1, &read(vec![button(110., 60.)]));
        sneak.read_app(1, found, n);
        assert!(!sneak.is_stale(&[own("pane:1", 0.)]), "an app's label is not a drawn control");
        sneak.redraw(&[own("pane:1", 5.)]);
        assert_eq!(sneak.targets.len(), 2);
    }

    #[test]
    fn only_the_users_own_letter_presses_in_an_app_and_only_the_focused_pane_is_read() {
        use crate::layout::Axis;
        use crate::model::{Action, Effect, Kind, Model, Origin};
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Empty });
        let pane = m.focused.expect("a pane");
        m.update(Action::Sneak { targets: vec![own("pane:1", 0.)] });
        // Another pane's window is not read.
        m.update(
            Action::SneakApp { pane: pane + 100, read: Some(read(vec![button(110., 60.)])) }.sent_by(Origin::System),
        );
        assert_eq!(m.sneak.as_ref().map(|s| s.targets.len()), Some(1));
        m.update(Action::SneakApp { pane, read: Some(read(vec![button(110., 60.)])) }.sent_by(Origin::System));
        let sneak = m.sneak.clone().expect("sneak mode");
        let label = sneak
            .targets
            .iter()
            .find(|t| matches!(t.target.source, Source::App { .. }))
            .expect("a label")
            .label
            .clone();
        // A client's letter is refused, and the labels stay as they were.
        let typed = Action::SneakType { text: label.clone() };
        assert_eq!(m.refusal(&typed, Origin::Socket), Some(USERS_ALONE));
        assert_eq!(m.update(typed.clone().sent_by(Origin::Socket)), []);
        assert_eq!(m.sneak.as_ref(), Some(&sneak));
        // The user's presses the element labelled, and the mode ends.
        let at = Frame { x: 10., y: 10., w: 30., h: 20. };
        let press = Effect::PressApp { pane, path: vec![0], role: "AXButton".into(), at, focus: false };
        assert_eq!(m.update(typed), [press]);
        assert!(m.sneak.is_none());
    }

    #[test]
    fn a_client_cannot_start_an_app_label_for_the_users_next_letter() {
        use crate::model::{Action, Origin};
        let mut sneak = Sneak::new(vec![own("pane:1", 0.)]).unwrap();
        let (found, n) = targets(1, &read((0..40).map(|i| button(100. + i as f64 * 9., 60.)).collect()));
        sneak.read_app(1, found, n);
        let two = sneak.targets.iter().find(|t| t.label.len() == 2).expect("a two-letter label").label.clone();
        let first = two[..1].to_string();
        assert!(sneak.reaches_app(&first), "a first letter of an app label");
        let mut m = crate::model::Model::default();
        m.sneak = Some(sneak);
        assert_eq!(m.refusal(&Action::SneakType { text: first.clone() }, Origin::Socket), Some(USERS_ALONE));
        assert_eq!(m.refusal(&Action::SneakType { text: first }, Origin::User), None);
        // ottospace's own label is still a client's to type.
        let own = m.sneak.as_ref().unwrap().targets[0].label.clone();
        assert_eq!(m.refusal(&Action::SneakType { text: own }, Origin::Socket), None);
    }

    #[test]
    fn labels_that_arrive_after_a_letter_never_go_on_with_it() {
        // The user typed the first letter of a ×'s two-letter label; the app's tree arrives then.
        let mut own: Vec<Target> = (0..5).map(|i| self::own(&format!("c{i}"), i as f64 * 30.)).collect();
        own.extend([self::own("close:1", 200.), self::own("close:2", 230.)]);
        let mut sneak = Sneak::leaving_room(own, true).unwrap();
        let first = sneak.targets.iter().find(|t| t.label.len() == 2).unwrap().label.chars().next().unwrap();
        assert_eq!(sneak.type_letter(first), sneak::Typed::Narrowed);
        let (found, n) = targets(1, &read((0..40).map(|i| button(100. + i as f64 * 9., 60.)).collect()));
        sneak.read_app(1, found, n);
        let app: Vec<&Labelled> =
            sneak.targets.iter().filter(|t| matches!(t.target.source, Source::App { .. })).collect();
        assert_eq!(app.len(), 40);
        assert!(app.iter().all(|t| !t.label.starts_with(first)), "the next letter reaches no app label");
        // Every letter taken but under what was typed: none now; the next read, once the user
        // took the letter back, labels them.
        let own: Vec<Target> = (0..30).map(|i| self::own(&format!("c{i}"), i as f64 * 30.)).collect();
        let mut sneak = Sneak::leaving_room(own, true).unwrap();
        sneak.type_letter('p');
        let (found, n) = targets(1, &read(vec![button(110., 60.)]));
        sneak.read_app(1, found.clone(), n);
        assert_eq!(sneak.app.map(|a| a.controls), Some(0));
        sneak.backspace();
        sneak.read_app(1, found, n);
        assert_eq!(sneak.app.map(|a| a.controls), Some(1));
        assert_prefix_free(&sneak);
    }

    #[test]
    fn a_label_never_moves_to_another_element() {
        let mut sneak = Sneak::new(vec![own("pane:1", 0.)]).unwrap();
        let (found, n) = targets(1, &read(vec![button(110., 60.)]));
        sneak.read_app(1, found, n);
        // The same path and role, somewhere else in the window: another element. Its label goes.
        let (found, n) = targets(1, &read(vec![button(110., 200.)]));
        sneak.read_app(1, found, n);
        assert_eq!(sneak.targets.len(), 1, "{:?}", sneak.targets);
        // Within the tolerance it is the same one, and keeps its label.
        let mut sneak = Sneak::new(vec![own("pane:1", 0.)]).unwrap();
        let (found, n) = targets(1, &read(vec![button(110., 60.)]));
        sneak.read_app(1, found, n);
        let (found, n) = targets(1, &read(vec![button(111., 61.)]));
        sneak.read_app(1, found, n);
        assert_eq!(sneak.targets.len(), 2);
    }

    #[test]
    fn more_labels_are_free_of_the_taken_and_of_each_other() {
        assert_eq!(more_labels(&["a", "s"], 2), ["d", "f"]);
        assert_eq!(more_labels(&[], 0), Vec::<String>::new());
        let taken: Vec<String> = ax::ALPHABET.chars().map(String::from).collect();
        let taken: Vec<&str> = taken.iter().map(String::as_str).collect();
        assert!(more_labels(&taken, 3).is_empty(), "every letter is a label");
        let got = more_labels(&["a", "sd", "sf"], 40);
        assert_eq!(got.len(), 40);
        let all: Vec<&str> = got.iter().map(String::as_str).chain(["a", "sd", "sf"]).collect();
        for (i, x) in all.iter().enumerate() {
            assert!(all.iter().enumerate().all(|(j, y)| i == j || !y.starts_with(x)), "{x} starts another");
        }
    }

    #[test]
    fn a_stale_element_is_never_pressed() {
        let window = Frame { x: 100., y: 50., w: 400., h: 300. };
        let at = Frame { x: 10., y: 10., w: 30., h: 20. };
        let now = button(110., 60.);
        assert_eq!(check("AXButton", at, window, &now), Ok(Press::Press));
        // The window moved, the element with it: still the same place in its window.
        let moved_window = Frame { x: 300., ..window };
        assert_eq!(check("AXButton", at, moved_window, &button(310., 60.)), Ok(Press::Press));
        assert_eq!(check("AXButton", at, window, &button(111.5, 61.)), Ok(Press::Press), "within the tolerance");
        assert_eq!(check("AXButton", at, window, &button(113., 60.)), Err(MOVED));
        assert_eq!(
            check("AXCheckBox", at, window, &Node { role: "AXCheckBox".into(), ..now.clone() }),
            Ok(Press::Press)
        );
        assert_eq!(check("AXButton", at, window, &Node { role: "AXCheckBox".into(), ..now.clone() }), Err(MOVED));
        assert_eq!(check("AXButton", at, window, &Node { enabled: Some(false), ..now.clone() }), Err(DISABLED));
        assert_eq!(check("AXButton", at, window, &Node { actions: vec![], ..now.clone() }), Err(MOVED));
        assert_eq!(check("AXButton", at, window, &Node { frame: None, ..now.clone() }), Err(MOVED));
        let nan = Frame { x: f64::NAN, y: 60., w: 30., h: 20. };
        assert_eq!(check("AXButton", at, window, &Node { frame: Some(nan), ..now.clone() }), Err(MOVED));
        let field = node("AXTextField", &[], [110., 60., 30., 20.], vec![]);
        assert_eq!(check("AXTextField", at, window, &field), Ok(Press::Focus));
    }
}
