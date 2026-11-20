//! Functional core: workspace state and the rules that change it. No I/O, no UI, no clock.
//! The shell (`ottospace`) turns input into [`Action`]s and carries out the [`Effect`]s.

pub mod activity;
pub mod agent_mode;
pub mod agents;
pub mod app_wheel;
pub mod apps;
pub mod ask;
pub mod atlas;
pub mod ax;
pub mod bug;
pub mod cadence;
pub mod calendar;
pub mod capture;
pub mod capture_model;
pub mod clip;
pub mod clone;
pub mod commands;
pub mod complete;
pub mod compose;
pub mod diff;
pub mod edit;
pub mod features;
pub mod fill;
pub mod fly;
#[cfg(test)]
mod fuzz;
pub mod gist;
pub mod home;
pub mod hosted;
pub mod ics;
pub mod inspect;
pub mod key_use;
pub mod keys;
pub mod layout;
pub mod layout_file;
pub mod links;
pub mod list;
pub mod listen;
pub mod marks;
pub mod model;
pub mod motion;
pub mod new_window;
pub mod notice;
pub mod palette;
pub mod progress;
pub mod prs;
pub mod repos;
pub mod restore;
pub mod row_menu;
pub mod rules;
pub mod runner;
pub mod scene;
pub mod scrollbar;
pub mod seal;
pub mod secrets;
pub mod selection;
pub mod sessions;
pub mod settings;
pub mod slot;
pub mod sneak;
pub mod sneak_apps;
pub mod sneak_place;
pub mod status_bar;
pub mod talk;
pub mod tasks;
pub mod terminal;
pub mod theme;
pub mod titlebar;
pub mod transcript;
pub mod viewer;
pub mod vim;
pub mod workbook;
pub mod working;

pub use layout::{Axis, Dir, Node, Rect, Zone};

/// A file's text as JSON: indented, with a newline at the end. Empty if it does not serialise.
pub fn pretty_json(value: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default() + "\n"
}
pub use model::{Action, Effect, Envelope, Kind, Mode, Model, Origin, Pane, PaneId, PickItem, Picker, Section, Tab};
