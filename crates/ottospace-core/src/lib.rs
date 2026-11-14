//! Functional core: workspace state and the rules that change it. No I/O, no UI, no clock.
//! The shell (`ottospace`) turns input into [`Action`]s and carries out the [`Effect`]s.

pub mod agents;
pub mod app_wheel;
pub mod apps;
pub mod ax;
pub mod calendar;
pub mod capture;
pub mod capture_model;
pub mod commands;
pub mod complete;
pub mod compose;
pub mod edit;
#[cfg(test)]
mod fuzz;
pub mod home;
pub mod hosted;
pub mod ics;
pub mod inspect;
pub mod keys;
pub mod layout;
pub mod layout_file;
pub mod list;
pub mod marks;
pub mod model;
pub mod palette;
pub mod progress;
pub mod repos;
pub mod restore;
pub mod rules;
pub mod scrollbar;
pub mod secrets;
pub mod sessions;
pub mod settings;
pub mod sneak;
pub mod sneak_apps;
pub mod talk;
pub mod tasks;
pub mod terminal;
pub mod theme;
pub mod titlebar;
pub mod workbook;

pub use layout::{Axis, Dir, Node, Rect, Zone};

/// A file's text as JSON: indented, with a newline at the end. Empty if it does not serialise.
pub fn pretty_json(value: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default() + "\n"
}
pub use model::{Action, Effect, Envelope, Kind, Mode, Model, Origin, Pane, PaneId, PickItem, Picker, Section, Tab};
