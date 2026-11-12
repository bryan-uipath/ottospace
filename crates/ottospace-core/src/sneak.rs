//! Sneak mode: each control of ottospace's own UI gets a short label, and typing a label does what
//! a click on the control does. The shell says where the controls are; the rules are here.

use serde::Serialize;

use crate::ax::{self, Labelled, Source, Target};

/// What a key does in sneak mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SneakKey {
    /// A letter of a label.
    Type(char),
    /// Take back the last letter.
    Back,
    Leave,
}

/// Sneak-mode keys: a letter narrows the labels, ⌫ takes one back, Esc leaves. Capitals count as
/// their letter. Any other key is `None`.
pub fn sneak_key(key: &str) -> Option<SneakKey> {
    let mut chars = key.chars();
    match (key, chars.next(), chars.next()) {
        ("escape", ..) => Some(SneakKey::Leave),
        ("backspace", ..) => Some(SneakKey::Back),
        (_, Some(c), None) if c.is_alphabetic() => Some(SneakKey::Type(c.to_ascii_lowercase())),
        _ => None,
    }
}

/// The labels on show, and the letters typed so far.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Sneak {
    /// Every label, in reading order. `typed` narrows which are on show (`visible`).
    pub targets: Vec<Labelled>,
    pub typed: String,
}

impl Sneak {
    /// Labels for ottospace's own controls: home-row letters first, two letters when one is not
    /// enough. A hosted app's nodes get none: pressing another app's control is not built.
    /// `None`: there is nothing to label.
    pub fn new(targets: Vec<Target>) -> Option<Self> {
        let own = targets.into_iter().filter(|t| matches!(t.source, Source::Own { .. })).collect();
        let targets = ax::assign(own, ax::ALPHABET);
        (!targets.is_empty()).then(|| Self { targets, typed: String::new() })
    }

    /// The labels still in play: those that start with what was typed.
    pub fn visible(&self) -> impl Iterator<Item = &Labelled> {
        self.targets.iter().filter(|t| t.label.starts_with(&self.typed))
    }

    /// One more letter. A letter no label goes on with is ignored. `Some(id)`: the letters now
    /// spell that control's label. Labels are prefix-free, so there is no wait and no ↵.
    pub fn type_letter(&mut self, letter: char) -> Option<String> {
        let typed = format!("{}{letter}", self.typed);
        if !self.targets.iter().any(|t| t.label.starts_with(&typed)) {
            return None;
        }
        self.typed = typed;
        self.targets.iter().find(|t| t.label == self.typed).and_then(|t| match &t.target.source {
            Source::Own { id } => Some(id.clone()),
            Source::App { .. } => None,
        })
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
        assert_eq!(sneak.type_letter('s'), Some("c1".into()));
    }

    #[test]
    fn more_targets_than_letters_get_prefix_free_labels_that_narrow_then_choose() {
        let mut sneak = Sneak::new(row(30)).unwrap();
        let labels: Vec<&str> = sneak.targets.iter().map(|t| t.label.as_str()).collect();
        assert!(labels.iter().any(|l| l.len() == 1) && labels.iter().any(|l| l.len() == 2));
        for (i, a) in labels.iter().enumerate() {
            assert!(labels.iter().enumerate().all(|(j, b)| i == j || !b.starts_with(a)), "{a} is a prefix");
        }
        // The first letter of a two-letter label narrows; the second chooses.
        let two = sneak.targets.iter().find(|t| t.label.len() == 2).unwrap().clone();
        let Source::Own { id } = &two.target.source else { panic!("own") };
        let mut letters = two.label.chars();
        let first = letters.next().unwrap();
        assert_eq!(sneak.type_letter(first), None);
        assert_eq!(sneak.typed, first.to_string());
        assert!(sneak.visible().count() > 1 && sneak.visible().all(|t| t.label.starts_with(first)));
        assert!(sneak.visible().count() < sneak.targets.len());
        assert_eq!(sneak.type_letter(letters.next().unwrap()), Some(id.clone()));
    }

    #[test]
    fn a_letter_no_label_goes_on_with_is_ignored() {
        let mut sneak = Sneak::new(row(3)).unwrap();
        assert_eq!((sneak.type_letter('z'), sneak.typed.as_str()), (None, ""));
        assert_eq!(sneak.visible().count(), 3);
        // Also after a first letter: `ap` is a label of 30, `az` is not.
        let mut sneak = Sneak::new(row(30)).unwrap();
        sneak.type_letter('a');
        assert_eq!((sneak.type_letter('z'), sneak.typed.as_str()), (None, "a"));
    }

    #[test]
    fn backspace_shows_every_label_again() {
        let mut sneak = Sneak::new(row(30)).unwrap();
        sneak.type_letter('a');
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
        assert_eq!((sneak_key("1"), sneak_key("enter"), sneak_key("f1"), sneak_key("")), (None, None, None, None));
    }
}
