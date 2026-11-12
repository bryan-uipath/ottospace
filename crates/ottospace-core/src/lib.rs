//! Functional core: workspace state and the rules that change it. No I/O, no UI, no clock.
//! The shell (`ottospace`) turns input into [`Action`]s and carries out the [`Effect`]s.

pub mod keys;
pub mod layout;
pub mod model;
pub mod palette;
pub mod terminal;
pub mod theme;
pub mod titlebar;

pub use layout::{Axis, Dir, Node, Rect};
pub use model::{Action, Effect, Kind, Mode, Model, Pane, PaneId, PickItem, Picker, Section, Tab};
