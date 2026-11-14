//! Feature flags in the shell: which are on at start, and the user's Features… choice written to
//! the settings and applied. The registry and the rule are the core's (`ottospace_core::features`).

use gpui::{App, actions};
use ottospace_core::features::{Feature, Features};
use ottospace_core::settings::Settings;
use ottospace_core::{Action, Origin};

use crate::*;

// The palette's and the app menu's Features….
actions!(ottospace, [ShowFeatures]);

/// The features on at start.
pub fn startup() -> Features {
    of(&config::get().settings)
}

/// The features on as `settings` say, or in a test run as `OTTOSPACE_FEATURES` lists them
/// (`marks,apps_section`), which no other run reads, and which wins over the file.
fn of(settings: &Settings) -> Features {
    match listed().map(|list| Features::parse_list(&list)) {
        Some(Ok(features)) => features,
        // A test that names a flag no feature has must not pass for one with every flag off.
        Some(Err(why)) => panic!("OTTOSPACE_FEATURES: {why}"),
        None => settings.features(),
    }
}

/// `OTTOSPACE_FEATURES`, in a test run only.
fn listed() -> Option<String> {
    std::env::var("OTTOSPACE_FEATURES").ok().filter(|_| std::env::var_os("OTTOSPACE_E2E").is_some())
}

impl Workspace {
    /// Carries out `Effect::SaveFeature`: `features.<name>` in the settings file, then live. A
    /// test run that named no folder writes none, and the flag turns for the run.
    pub fn save_feature(&mut self, feature: Feature, on: bool, cx: &mut App) {
        if let Err(why) = config::save_feature(feature.name(), on) {
            self.error = Some(format!("{}{why}", Self::NOT_WRITTEN));
            return;
        }
        self.set_features(self.model.features.with(feature, on), cx);
    }

    /// `settings.json` read again and right (`Effect::ApplySettings`): its flags apply at once.
    /// Not in a test run that named its flags: they were set at start, and Features… since.
    pub fn apply_features(&mut self, settings: &Settings, cx: &mut App) {
        let features = of(settings);
        if listed().is_none() && features != self.model.features {
            self.set_features(features, cx);
        }
    }

    fn set_features(&mut self, features: Features, cx: &mut App) {
        self.model.update(Action::SetFeatures { features }.sent_by(Origin::System));
        cx.set_menus(menus::app_menus(&self.model.features));
    }

    /// Whether `feature` is on: a key of one that is off is no key of ottospace's.
    pub fn on(&self, feature: Feature) -> bool {
        self.model.features.on(feature)
    }
}
