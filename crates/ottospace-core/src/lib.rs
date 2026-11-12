//! Functional core: workspace state and the rules that change it. No I/O, no UI, no clock.
//! The shell (`ottospace`) turns input into [`Action`]s and carries out the [`Effect`]s.

pub mod ax;
pub mod compose;
pub mod home;
pub mod hosted;
pub mod keys;
pub mod layout;
pub mod list;
pub mod marks;
pub mod model;
pub mod palette;
pub mod repos;
pub mod rules;
pub mod sessions;
pub mod settings;
pub mod sneak;
pub mod tasks;
pub mod terminal;
pub mod theme;
pub mod titlebar;

pub use layout::{Axis, Dir, Node, Rect, Zone};
pub use model::{Action, Effect, Envelope, Kind, Mode, Model, Origin, Pane, PaneId, PickItem, Picker, Section, Tab};
