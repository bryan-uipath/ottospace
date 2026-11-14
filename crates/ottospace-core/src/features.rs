//! Feature flags: parts that are not ready to be on for everyone. One registry: the settings'
//! `features`, the table in `docs/reference.md` and the palette's Features… read it. A flag is
//! temporary and about readiness; a preference is a normal setting.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// How ready a feature is. A stable one has no flag: its name goes to `RETIRED`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Off unless the settings turn it on.
    Experimental,
    /// On unless the settings turn it off.
    Preview,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Experimental => "Experimental",
            Stage::Preview => "Preview",
        }
    }
}

/// A part of ottospace behind a flag. Off, it is absent: no rail cell, section, key, command,
/// menu item or socket request of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    Marks,
    AppsSection,
}

/// Every flag, in the order the docs and the palette list them.
pub const ALL: [Feature; 2] = [Feature::Marks, Feature::AppsSection];

/// Flags whose feature is stable now: a setting of one is ignored, with a warning.
pub const RETIRED: &[&str] = &[];

impl Feature {
    /// Its key under `features` in `settings.json`.
    pub fn name(self) -> &'static str {
        match self {
            Feature::Marks => "marks",
            Feature::AppsSection => "apps_section",
        }
    }

    pub fn stage(self) -> Stage {
        match self {
            Feature::Marks | Feature::AppsSection => Stage::Experimental,
        }
    }

    /// On when the settings say nothing of it.
    pub fn default_on(self) -> bool {
        self.stage() == Stage::Preview
    }

    pub fn description(self) -> &'static str {
        match self {
            Feature::Marks => "Marks: name a pane or a tab, and go back to it with a letter",
            Feature::AppsSection => "The Apps section: the Dock's apps, pins, Not running and All apps",
        }
    }

    pub fn named(name: &str) -> Option<Self> {
        ALL.into_iter().find(|feature| feature.name() == name)
    }

    /// What a socket request of a feature that is off answers.
    pub fn off(self) -> &'static str {
        match self {
            Feature::Marks => "feature off: marks",
            Feature::AppsSection => "feature off: apps_section",
        }
    }
}

/// The features that are on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Features(Vec<Feature>);

impl Default for Features {
    /// Each feature as its stage says.
    fn default() -> Self {
        Self::from_settings(&BTreeMap::new())
    }
}

impl Features {
    pub fn all() -> Self {
        Self(ALL.to_vec())
    }

    /// As the settings' `features` say, the defaults for what they do not name.
    pub fn from_settings(set: &BTreeMap<String, bool>) -> Self {
        Self(ALL.into_iter().filter(|f| set.get(f.name()).copied().unwrap_or(f.default_on())).collect())
    }

    /// e.g. `OTTOSPACE_FEATURES=marks,apps_section`: these on, the others off. `Err`: a name no flag has.
    pub fn parse_list(list: &str) -> Result<Self, String> {
        let mut on = Vec::new();
        for name in list.split(',').map(str::trim).filter(|name| !name.is_empty()) {
            on.push(Feature::named(name).ok_or_else(|| unknown(name))?);
        }
        Ok(Self(ALL.into_iter().filter(|f| on.contains(f)).collect()))
    }

    pub fn on(&self, feature: Feature) -> bool {
        self.0.contains(&feature)
    }

    pub fn with(&self, feature: Feature, on: bool) -> Self {
        Self(ALL.into_iter().filter(|f| if *f == feature { on } else { self.on(*f) }).collect())
    }
}

/// What is wrong with `"name": value` under `features`: `None` if it is a flag and a bool.
/// `retired`: `RETIRED`, given so a test can name one.
pub fn wrong(name: &str, value: &serde_json::Value, retired: &[&str]) -> Option<String> {
    if retired.contains(&name) {
        return Some(format!("{name:?} is stable now and has no flag: the setting does nothing"));
    }
    if Feature::named(name).is_none() {
        return Some(unknown(name));
    }
    (!value.is_boolean()).then(|| format!("{name:?} must be true or false"))
}

fn unknown(name: &str) -> String {
    let names: Vec<&str> = ALL.into_iter().map(Feature::name).collect();
    format!("{name:?} is not a feature: the features are {}", names.join(", "))
}

/// The table of flags in `docs/reference.md`, written from the registry.
pub fn table() -> String {
    let mut table = String::from("| Feature | Stage | Default | What it is |\n| --- | --- | --- | --- |\n");
    for feature in ALL {
        let default = if feature.default_on() { "on" } else { "off" };
        let (name, stage, about) = (feature.name(), feature.stage().label(), feature.description());
        table += &format!("| `{name}` | {stage} | {default} | {about} |\n");
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_docs_table_is_the_registry() {
        let reference = include_str!("../../../docs/reference.md");
        let table = table();
        assert!(reference.contains(&table), "docs/reference.md must hold:\n{table}");
    }

    #[test]
    fn each_flag_has_one_name_and_its_stage_says_its_default() {
        for (at, feature) in ALL.iter().enumerate() {
            assert_eq!(Feature::named(feature.name()), Some(*feature));
            assert!(!ALL[..at].iter().any(|f| f.name() == feature.name()));
            assert_eq!(feature.default_on(), feature.stage() == Stage::Preview);
            assert_eq!(serde_json::to_value(feature).unwrap(), feature.name());
            assert!(feature.off().ends_with(feature.name()) && !RETIRED.contains(&feature.name()));
        }
        assert_eq!(Features::default(), Features(Vec::new()), "both are experimental today");
    }

    #[test]
    fn the_settings_turn_flags_on_and_off_and_the_rest_keep_their_default() {
        let set = |pairs: &[(&str, bool)]| pairs.iter().map(|(name, on)| (name.to_string(), *on)).collect();
        assert_eq!(Features::from_settings(&set(&[("marks", true)])), Features(vec![Feature::Marks]));
        assert_eq!(
            Features::from_settings(&set(&[("marks", false), ("apps_section", true)])).0,
            [Feature::AppsSection]
        );
        assert_eq!(Features::all().with(Feature::Marks, false), Features(vec![Feature::AppsSection]));
        assert_eq!(Features::default().with(Feature::Marks, true).with(Feature::AppsSection, true), Features::all());
        assert_eq!(Features::parse_list(" apps_section,marks ").unwrap(), Features::all());
        assert_eq!(Features::parse_list("").unwrap(), Features(Vec::new()));
        assert!(Features::parse_list("marks,tabs").unwrap_err().contains("marks, apps_section"));
    }

    /// What the model does with a feature off, and on again.
    mod model {
        use super::*;
        use crate::commands::Command;
        use crate::marks::{MarkOf, MarkRef};
        use crate::model::{Action, Effect, Kind, Mode, Model, Origin, Section};
        use crate::{Axis, list::ListRow};

        fn set(features: Features) -> crate::model::Envelope {
            Action::SetFeatures { features }.sent_by(Origin::System)
        }

        fn mark(key: char, uid: &str) -> Action {
            Action::MarkAdd { of: MarkOf::Pane(None), name: None, key: Some(key), uid: uid.into(), cwds: Vec::new() }
        }

        fn terminal() -> Model {
            let mut m = Model::default();
            m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
            m
        }

        #[test]
        fn off_a_feature_has_no_rail_cell_section_key_or_command() {
            let mut m = terminal();
            m.update(set(Features::default()));
            // Tasks, Workbook, Repos: no Apps, no Marks.
            assert_eq!(m.sections(), [Section::Tasks, Section::Workbook, Section::Repos]);
            for section in [Section::Marks, Section::Apps] {
                assert!(m.update(Action::ToggleSection { section }).is_empty());
                assert_eq!(m.sidebar, None);
            }
            // A mark's key or request does nothing, whoever sends it.
            for origin in [Origin::User, Origin::Socket, Origin::System] {
                assert!(m.update(mark('a', "u1").sent_by(origin)).is_empty());
                assert!(m.update(Action::MarkPrefix.sent_by(origin)).is_empty());
            }
            assert_eq!((m.marks.list.len(), m.mode()), (0, Mode::Normal));
            assert_eq!(m.disabled(&Action::MarkPrefix), Some(Feature::Marks));
            // ⌘⌃E drives the rail's first section, not the Apps section; a pin there does nothing.
            m.update(Action::FocusList);
            assert_eq!((m.mode(), m.sidebar), (Mode::List, Some(Section::Tasks)));
            assert!(m.update(Action::ListPin).is_empty());
            // The palette lists no mark command, and lists Features….
            m.update(Action::LeaveList);
            m.update(Action::PaletteOpen);
            let listed = m.palette.as_ref().map(|p| p.matches()).unwrap_or_default();
            assert!(!listed.contains(&Command::MarkPane) && listed.contains(&Command::Features), "{listed:?}");
        }

        #[test]
        fn marks_kept_while_off_come_back_when_turned_on() {
            let mut m = terminal();
            m.update(mark('a', "u1"));
            let stored = serde_json::to_string(&m.marks.list).unwrap();
            m.update(set(Features::default()));
            // Neither gone to, renamed nor removed; what is on disk stays as it was.
            m.update(Action::MarkRemove { mark: MarkRef::Key('a') });
            m.update(Action::MarkRename { mark: MarkRef::Key('a'), name: "x".into() });
            assert!(m.update(Action::MarkGo { mark: MarkRef::Key('a'), axis: Axis::Right }).is_empty());
            assert_eq!(serde_json::to_string(&m.marks.list).unwrap(), stored);
            // On again, live: the section lists the mark.
            m.update(set(Features::default().with(Feature::Marks, true)));
            m.update(Action::ToggleSection { section: Section::Marks });
            assert_eq!(m.list_rows(), [ListRow::Mark(m.marks.list[0].id)]);
        }

        #[test]
        fn a_feature_turned_off_takes_its_section_wait_and_filter_with_it() {
            let mut m = terminal();
            m.update(Action::ToggleSection { section: Section::Marks });
            m.update(Action::MarkPrefix);
            m.update(set(Features::default().with(Feature::AppsSection, true)));
            assert_eq!((m.sidebar, m.mark_input.clone(), m.mode()), (None, None, Mode::Normal));
            m.update(Action::ToggleSection { section: Section::Apps });
            m.update(Action::AppsFilter { on: true });
            assert!(m.app_lists.filtering);
            m.update(set(Features::default()));
            assert_eq!((m.sidebar, m.app_lists.filtering), (None, false));
        }

        #[test]
        fn features_lists_the_flags_and_the_users_choice_writes_one() {
            let mut m = terminal();
            m.update(set(Features::default()));
            // Its name finds it first in the palette. No client opens it.
            assert_eq!(crate::commands::rank("features")[0], Command::Features);
            assert!(m.update(Action::PaletteFeatures.sent_by(Origin::Socket)).is_empty());
            assert!(m.palette.is_none());
            m.update(Action::PaletteFeatures);
            let flags = m.palette.as_ref().map(|p| p.feature_matches().into_iter().cloned().collect::<Vec<_>>());
            let names: Vec<(&str, bool)> = flags.iter().flatten().map(|c| (c.feature.name(), c.on)).collect();
            assert_eq!(names, [("marks", false), ("apps_section", false)]);
            assert_eq!(m.palette.as_ref().map(|p| p.matches()), Some(Vec::new()), "flags, not commands");
            m.update(crate::model::Action::FieldEdit { edit: crate::edit::Edit::Insert("apps".into()) });
            let effects = m.update(Action::PaletteChoose { index: None });
            assert_eq!(effects, [Effect::SaveFeature { feature: Feature::AppsSection, on: true }]);
            assert!(m.palette.is_none());
        }
    }

    #[test]
    fn an_unknown_name_lists_the_flags_and_a_retired_one_only_warns() {
        let yes = serde_json::json!(true);
        assert_eq!(wrong("marks", &yes, RETIRED), None);
        assert_eq!(
            wrong("bookmarks", &yes, RETIRED).unwrap(),
            "\"bookmarks\" is not a feature: the features are marks, apps_section"
        );
        assert_eq!(wrong("marks", &serde_json::json!("on"), RETIRED).unwrap(), "\"marks\" must be true or false");
        assert!(wrong("tabs", &yes, &["tabs"]).unwrap().contains("stable now"));
    }
}
