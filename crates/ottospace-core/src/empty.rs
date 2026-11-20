//! One empty state: what a place with nothing in it shows. The same hints, in the same order and
//! words, wherever their actions work there; one backdrop; one fallback for a small pane. Home
//! is a different thing by design (its tasks, its heading); a pane a mark could not fill says
//! why above the same hints; a slot that waits keeps its own text (not reconciled here).
//! Design: `docs/reference.md`, "Empty places".

use crate::commands::Command;

/// A place with nothing in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// The home screen, the first run's too.
    Home,
    /// A tab of a workspace or a task with no pane, in the main window.
    Tab,
    /// An empty pane: from a split, `NewEmptyTab`, or a pane whose content went.
    Pane,
    /// A tab with no pane, folded out to a window of its own.
    OutTab,
}

/// A way to fill it: its key's caps and what it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    /// ⌘E: the picker, whose choice fills the place.
    OpenApp,
    /// ⌘D: a terminal that fills a tab with no pane. Beside a pane it splits, never fills.
    Terminal,
    /// ⌘N: a new task, in the workspace on show.
    NewTask,
}

impl Hint {
    /// The command whose key its caps show and whose action its click does.
    pub fn command(self) -> Command {
        match self {
            Hint::OpenApp => Command::PickPane,
            Hint::Terminal => Command::SplitRight,
            Hint::NewTask => Command::NewTask,
        }
    }

    /// Its row's words, the same everywhere.
    pub fn label(self) -> &'static str {
        match self {
            Hint::OpenApp => "Open an app",
            Hint::Terminal => "Terminal",
            Hint::NewTask => "New task",
        }
    }

    /// Its row's control id, after the surface's prefix (`home:open_app`).
    pub fn id(self) -> &'static str {
        match self {
            Hint::OpenApp => "open_app",
            Hint::Terminal => "terminal",
            Hint::NewTask => "new_task",
        }
    }
}

/// What a small pane draws: its hints alone, no backdrop (the lattice there is a few lines).
pub const SMALL: (f32, f32) = (240., 160.);
/// The least pane that draws its folder's city, in points: smaller, a district is a few pixels
/// and a name finds no room, so it keeps the lattice.
pub const CITY_LEAST: (f32, f32) = (320., 220.);

/// What is drawn behind the hints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backdrop {
    None,
    Lattice,
    /// The city of the place's folder (`Model::city_at`), with the `code_city` flag.
    City,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Empty {
    /// In order, top to bottom.
    pub hints: &'static [Hint],
    pub backdrop: Backdrop,
}

const ALL: &[Hint] = &[Hint::OpenApp, Hint::Terminal, Hint::NewTask];

/// What `surface`, `size` points wide and tall, shows; `city`: it has a city (a folder, and the
/// flag on: `Model::city_at`). A hint only where its action fills that place: beside a pane ⌘D
/// splits it and ⌘N fills nothing, so a pane shows ⌘E alone; in a folded-out window the picker
/// opens in the main window and fills its tab, so a folded-out tab shows ⌘D (a terminal there)
/// and ⌘N (a task in its workspace). Home has no folder, so never a city.
pub fn empty(surface: Surface, size: (f32, f32), city: bool) -> Empty {
    let hints = match surface {
        Surface::Home | Surface::Tab => ALL,
        Surface::Pane => &ALL[..1],
        Surface::OutTab => &ALL[1..],
    };
    let under = |least: (f32, f32)| surface == Surface::Pane && (size.0 < least.0 || size.1 < least.1);
    let backdrop = match () {
        _ if city && surface != Surface::Home && !under(CITY_LEAST) => Backdrop::City,
        _ if under(SMALL) => Backdrop::None,
        _ => Backdrop::Lattice,
    };
    Empty { hints, backdrop }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERY: [Surface; 4] = [Surface::Home, Surface::Tab, Surface::Pane, Surface::OutTab];

    /// Every surface's hints are rows of the one list, in its order and words.
    #[test]
    fn every_surface_shows_the_same_hints_in_the_same_order() {
        for surface in EVERY {
            let hints = empty(surface, (1000., 800.), false).hints;
            let at: Vec<usize> = hints.iter().filter_map(|hint| ALL.iter().position(|h| h == hint)).collect();
            assert!(!hints.is_empty() && at.is_sorted() && at.len() == hints.len(), "{surface:?}: {hints:?}");
        }
        let big = (1000., 800.);
        assert_eq!(empty(Surface::Tab, big, true).hints, empty(Surface::Home, big, false).hints);
        assert_eq!(empty(Surface::Pane, big, true).hints, [Hint::OpenApp]);
        assert_eq!(empty(Surface::OutTab, big, false).hints, [Hint::Terminal, Hint::NewTask]);
        let labels: Vec<&str> = ALL.iter().map(|hint| hint.label()).collect();
        assert_eq!(labels, ["Open an app", "Terminal", "New task"]);
    }

    /// The lattice behind every surface, but a pane too small for it.
    #[test]
    fn a_small_pane_shows_its_hints_alone() {
        for surface in EVERY {
            assert_eq!(empty(surface, (1000., 800.), false).backdrop, Backdrop::Lattice, "{surface:?}");
            let small = if surface == Surface::Pane { Backdrop::None } else { Backdrop::Lattice };
            assert_eq!(empty(surface, (200., 120.), false).backdrop, small, "{surface:?}");
        }
        assert_eq!(empty(Surface::Pane, (SMALL.0 - 1., 800.), false).backdrop, Backdrop::None);
        assert_eq!(empty(Surface::Pane, (800., SMALL.1 - 1.), false).backdrop, Backdrop::None);
        assert_eq!(empty(Surface::Pane, SMALL, false).backdrop, Backdrop::Lattice);
    }

    /// Every surface with a folder draws its city, but Home (it has none) and a pane too small
    /// for one, which keeps its lattice, or nothing when smaller still.
    #[test]
    fn every_surface_with_a_folder_draws_its_city_but_a_small_pane() {
        for surface in EVERY {
            let city = if surface == Surface::Home { Backdrop::Lattice } else { Backdrop::City };
            assert_eq!(empty(surface, (1000., 800.), true).backdrop, city, "{surface:?}");
            let small = if surface == Surface::Pane { Backdrop::Lattice } else { city };
            assert_eq!(empty(surface, (CITY_LEAST.0 - 1., 300.), true).backdrop, small, "{surface:?}");
        }
        assert_eq!(empty(Surface::Pane, CITY_LEAST, true).backdrop, Backdrop::City);
        assert_eq!(empty(Surface::Pane, (CITY_LEAST.0, CITY_LEAST.1 - 1.), true).backdrop, Backdrop::Lattice);
        assert_eq!(empty(Surface::Pane, (200., 120.), true).backdrop, Backdrop::None);
    }
}
