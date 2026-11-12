//! Sneak mode: each control of ottospace's own UI gets a short label, and typing a label does what
//! a click on the control does. The shell says where the controls are; the rules are here.

use serde::Serialize;

use crate::ax::{self, Labelled, Source, Target};

/// What a key does in sneak mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SneakKey {
    /// A letter: of a label, or of none, which leaves.
    Type(char),
    /// Take back the last letter.
    Back,
    Leave,
}

/// Sneak-mode keys: a letter narrows the labels, ⌫ takes one back, Esc leaves. Capitals count as
/// their letter. Any other key (a number, an arrow, ↵, a modifier alone) is `None`: it does nothing.
///
/// `key` is GPUI's `Keystroke::key`: on a layout with no Latin letters (Russian, Thai) that is the
/// letter the key has under ⌘, so the labels can be typed there. A letter no label has (`ö`)
/// leaves the mode, as any other wrong letter does.
pub fn sneak_key(key: &str) -> Option<SneakKey> {
    let mut chars = key.chars();
    match (key, chars.next(), chars.next()) {
        ("escape", ..) => Some(SneakKey::Leave),
        ("backspace", ..) => Some(SneakKey::Back),
        (_, Some(c), None) if c.is_alphabetic() => Some(SneakKey::Type(c.to_ascii_lowercase())),
        _ => None,
    }
}

/// A control that ends something: a pane's or a tab's ×. Its label always has two letters, so one
/// stray key cannot close a shell. `id` is the shell's name for the control, e.g. `tab-close:2`.
pub fn destructive(id: &str) -> bool {
    id.starts_with("close:") || id.starts_with("tab-close:")
}

fn own_id(target: &Target) -> Option<&str> {
    match &target.source {
        Source::Own { id } => Some(id),
        Source::App { .. } => None,
    }
}

/// Labels in reading order, none a prefix of another. As many one-letter labels as can be, home
/// row first, and never on a destructive control; the letters left over start the two-letter ones.
/// e.g. a tab, its × and a pane: `a`, `da`, `s`.
fn assign(mut targets: Vec<Target>) -> Vec<Labelled> {
    ax::reading_order(&mut targets);
    let letters: Vec<char> = ax::ALPHABET.chars().collect();
    let safe = |target: &Target| !own_id(target).is_some_and(destructive);
    let (all, most, plain) = (targets.len(), letters.len(), targets.iter().filter(|t| safe(t)).count());
    // `short` one-letter labels leave `most - short` letters to start the two-letter ones.
    let fits = |short: &usize| (most - short) * most >= all - short;
    let short = match plain == all && all <= most {
        true => Some(all),
        false => (0..most.min(plain + 1)).rev().find(fits),
    };
    // More controls than two letters can name (hundreds): three letters, and none has one.
    let Some(short) = short else {
        let labels = ax::labels(all, ax::ALPHABET);
        return targets.into_iter().zip(labels).map(|(target, label)| Labelled { label, target }).collect();
    };
    let mut one = letters[..short].iter().map(char::to_string);
    let mut two = letters[short..].iter().flat_map(|a| letters.iter().map(move |b| format!("{a}{b}")));
    let label = |target: Target| {
        let short = safe(&target).then(|| one.next()).flatten();
        Some(Labelled { label: short.or_else(|| two.next())?, target })
    };
    targets.into_iter().filter_map(label).collect()
}

/// What a typed letter did.
#[derive(Clone, Debug, PartialEq)]
pub enum Typed {
    /// Some labels go on with it: they stay, the others go.
    Narrowed,
    /// The letters now spell this control's label.
    Chosen(String),
    /// No label goes on with it.
    NoLabel,
}

/// The labels on show, and the letters typed so far.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Sneak {
    /// Every label, in reading order. `typed` narrows which are on show (`visible`).
    pub targets: Vec<Labelled>,
    pub typed: String,
    /// Starting the mode hid the accessibility overlay: its chips use the same letters.
    #[serde(skip)]
    pub hid_overlay: bool,
}

impl Sneak {
    /// Labels for ottospace's own controls. A hosted app's nodes get none: pressing another app's
    /// control is not built. `None`: there is nothing to label.
    pub fn new(targets: Vec<Target>) -> Option<Self> {
        let own = targets.into_iter().filter(|t| own_id(t).is_some()).collect();
        let targets = assign(own);
        (!targets.is_empty()).then(|| Self { targets, typed: String::new(), hid_overlay: false })
    }

    /// The labels still in play: those that start with what was typed.
    pub fn visible(&self) -> impl Iterator<Item = &Labelled> {
        self.targets.iter().filter(|t| t.label.starts_with(&self.typed))
    }

    /// Whether a frame that drew `drawn` moved or dropped a labelled control.
    pub fn is_stale(&self, drawn: &[Target]) -> bool {
        self.targets.iter().any(|t| !drawn.contains(&t.target))
    }

    /// A new frame was drawn. A control still drawn keeps its letters, at its new place; one no
    /// longer drawn loses its label, so its letters cannot reach what took its place; a new
    /// control gets none until the mode starts again.
    pub fn redraw(&mut self, drawn: &[Target]) {
        self.targets.retain_mut(|t| match drawn.iter().find(|d| d.source == t.target.source) {
            Some(now) => {
                t.target.frame = now.frame;
                true
            }
            None => false,
        });
    }

    /// One more letter. Labels are prefix-free, so a spelled label is chosen with no wait and no ↵.
    pub fn type_letter(&mut self, letter: char) -> Typed {
        let typed = format!("{}{letter}", self.typed);
        if !self.targets.iter().any(|t| t.label.starts_with(&typed)) {
            return Typed::NoLabel;
        }
        self.typed = typed;
        let chosen = self.targets.iter().find(|t| t.label == self.typed).and_then(|t| own_id(&t.target));
        chosen.map_or(Typed::Narrowed, |id| Typed::Chosen(id.to_string()))
    }

    pub fn backspace(&mut self) {
        self.typed.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ax::Frame;

    fn own(id: &str, x: f64, y: f64) -> Target {
        Target { frame: Frame { x, y, w: 20., h: 20. }, source: Source::Own { id: id.into() } }
    }

    /// `n` controls in a row, left to right: `c0`, `c1`, ...
    fn row(n: usize) -> Vec<Target> {
        (0..n).map(|i| own(&format!("c{i}"), i as f64 * 30., 0.)).collect()
    }

    fn label_of<'a>(sneak: &'a Sneak, id: &str) -> &'a str {
        let own = Source::Own { id: id.into() };
        &sneak.targets.iter().find(|t| t.target.source == own).expect("a label").label
    }

    fn assert_prefix_free(sneak: &Sneak) {
        let labels: Vec<&str> = sneak.targets.iter().map(|t| t.label.as_str()).collect();
        for (i, a) in labels.iter().enumerate() {
            assert!(labels.iter().enumerate().all(|(j, b)| i == j || !b.starts_with(a)), "{a} is a prefix");
        }
    }

    #[test]
    fn labels_go_out_in_reading_order_from_the_home_row() {
        let sneak = Sneak::new(vec![own("low", 0., 50.), own("right", 40., 0.), own("left", 0., 0.)]).unwrap();
        assert_eq!((label_of(&sneak, "left"), label_of(&sneak, "right"), label_of(&sneak, "low")), ("a", "s", "d"));
        assert!(Sneak::new(Vec::new()).is_none(), "nothing to label");
    }

    #[test]
    fn a_hosted_apps_nodes_get_no_label() {
        let node = Target {
            frame: Frame { x: 0., y: 0., w: 9., h: 9. },
            source: Source::App { pane: 1, path: vec![0], role: "AXButton".into() },
        };
        let sneak = Sneak::new(vec![node.clone(), own("rail:apps", 0., 40.)]).unwrap();
        assert_eq!(sneak.targets.len(), 1);
        assert_eq!(label_of(&sneak, "rail:apps"), "a");
        assert!(Sneak::new(vec![node]).is_none());
    }

    #[test]
    fn one_letter_chooses_while_the_alphabet_is_enough() {
        let mut sneak = Sneak::new(row(ax::ALPHABET.len())).unwrap();
        assert!(sneak.targets.iter().all(|t| t.label.len() == 1));
        assert_eq!(sneak.type_letter('s'), Typed::Chosen("c1".into()));
    }

    #[test]
    fn more_targets_than_letters_get_prefix_free_labels_that_narrow_then_choose() {
        let mut sneak = Sneak::new(row(30)).unwrap();
        // All but one letter keep their one-letter labels: `p` alone starts the other twelve.
        let count = |len: usize| sneak.targets.iter().filter(|t| t.label.len() == len).count();
        assert_eq!((count(1), count(2)), (18, 12));
        assert_eq!((label_of(&sneak, "c0"), label_of(&sneak, "c17"), label_of(&sneak, "c18")), ("a", "o", "pa"));
        assert_prefix_free(&sneak);
        // The first letter of a two-letter label narrows; the second chooses.
        assert_eq!(sneak.type_letter('p'), Typed::Narrowed);
        assert_eq!((sneak.typed.as_str(), sneak.visible().count()), ("p", 12));
        assert_eq!(sneak.type_letter('s'), Typed::Chosen("c19".into()));
    }

    #[test]
    fn a_destructive_control_always_gets_two_letters() {
        // A tab bar and a pane, as drawn: the × of each among them.
        let ids = ["tab:1", "tab-close:1", "tab:2", "tab-close:2", "header:1", "close:1", "pane:1"];
        let targets = ids.iter().enumerate().map(|(i, id)| own(id, i as f64 * 30., 0.)).collect();
        let sneak = Sneak::new(targets).unwrap();
        let labels: Vec<&str> = ids.iter().map(|id| label_of(&sneak, id)).collect();
        assert_eq!(labels, ["a", "ga", "s", "gs", "d", "gd", "f"]);
        assert_prefix_free(&sneak);
        // Alone, too: one stray key never closes anything.
        let alone = Sneak::new(vec![own("close:1", 0., 0.)]).unwrap();
        assert_eq!(label_of(&alone, "close:1"), "aa");
        // And among more controls than letters.
        let mut many = row(40);
        many.push(own("tab-close:9", 5., 0.));
        let many = Sneak::new(many).unwrap();
        assert_eq!(label_of(&many, "tab-close:9").len(), 2);
        assert_prefix_free(&many);
        assert!(many.targets.iter().all(|t| t.label.len() <= 2));
        // Hundreds of controls: three letters, still none with one.
        let crowd = Sneak::new(row(400)).unwrap();
        assert_eq!(crowd.targets.len(), 400);
        assert!(crowd.targets.iter().all(|t| t.label.len() >= 2));
        assert_prefix_free(&crowd);
    }

    #[test]
    fn a_letter_no_label_goes_on_with_is_no_label() {
        let mut sneak = Sneak::new(row(3)).unwrap();
        assert_eq!((sneak.type_letter('z'), sneak.typed.as_str()), (Typed::NoLabel, ""));
        // Also after a first letter: `pa` is a label of 30, `pz` is not.
        let mut sneak = Sneak::new(row(30)).unwrap();
        sneak.type_letter('p');
        assert_eq!((sneak.type_letter('z'), sneak.typed.as_str()), (Typed::NoLabel, "p"));
    }

    #[test]
    fn a_new_frame_moves_a_label_with_its_control_and_drops_the_label_of_one_not_drawn() {
        let mut sneak = Sneak::new(row(3)).unwrap();
        assert!(!sneak.is_stale(&row(3)));
        // c1 is gone, c2 slid into its place, and c9 is new.
        let drawn = vec![own("c0", 0., 0.), own("c2", 30., 0.), own("c9", 60., 0.)];
        assert!(sneak.is_stale(&drawn));
        sneak.redraw(&drawn);
        let labels: Vec<(&str, f64)> = sneak.targets.iter().map(|t| (t.label.as_str(), t.target.frame.x)).collect();
        assert_eq!(labels, [("a", 0.), ("d", 30.)], "c2 keeps `d`; `s` names nothing; c9 has no label");
        assert!(!sneak.is_stale(&drawn), "a new control alone is no reason to draw again");
        assert_eq!(sneak.type_letter('s'), Typed::NoLabel);
        assert_eq!(sneak.type_letter('d'), Typed::Chosen("c2".into()));
    }

    #[test]
    fn backspace_shows_every_label_again() {
        let mut sneak = Sneak::new(row(30)).unwrap();
        sneak.type_letter('p');
        sneak.backspace();
        assert_eq!((sneak.typed.as_str(), sneak.visible().count()), ("", 30));
        sneak.backspace(); // nothing typed: nothing to take back
        assert_eq!(sneak.typed, "");
    }

    #[test]
    fn keys_are_letters_backspace_and_escape() {
        assert_eq!(sneak_key("a"), Some(SneakKey::Type('a')));
        assert_eq!(sneak_key("A"), Some(SneakKey::Type('a')));
        assert_eq!(sneak_key("backspace"), Some(SneakKey::Back));
        assert_eq!(sneak_key("escape"), Some(SneakKey::Leave));
        // A letter no label has is still a letter: it leaves the mode, it does not trap.
        assert_eq!((sneak_key("ö"), sneak_key("ф")), (Some(SneakKey::Type('ö')), Some(SneakKey::Type('ф'))));
        // Not letters: they do nothing.
        for key in ["1", "enter", "f1", "", "left", "tab", "space", ";", "shift"] {
            assert_eq!(sneak_key(key), None, "{key}");
        }
    }
}
