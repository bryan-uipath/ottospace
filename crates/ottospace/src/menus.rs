//! The macOS menu bar: the same actions the keys run, findable by name.

use gpui::{Action, Menu, MenuItem, OsAction, SystemMenuType};

use crate::*;

fn item(name: &'static str, action: impl Action) -> MenuItem {
    MenuItem::action(name, action)
}

pub fn app_menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "ottospace".into(),
            items: vec![
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                item("Hide ottospace", Hide),
                MenuItem::separator(),
                item("Quit ottospace", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                item("New Tab", NewTab),
                MenuItem::separator(),
                item("Close Pane", ClosePane),
                item("Close Tab", CloseTab),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Copy", CopySelection, OsAction::Copy),
                MenuItem::os_action("Paste", PasteText, OsAction::Paste),
                MenuItem::separator(),
                item("Select with the Keyboard", VisualMode),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                item("Drive the Bar with the Keyboard", ListMode),
                item("Label the Controls", SneakMode),
                item("Fill the Screen", Zoom),
                MenuItem::separator(),
                item("Next Tab", NextTab),
                item("Previous Tab", PrevTab),
            ],
        },
        Menu {
            name: "Pane".into(),
            items: vec![
                item("Terminal Right", SplitRight),
                item("Terminal Below", SplitDown),
                item("New Pane…", PickPane),
                MenuItem::separator(),
                item("Focus Left", FocusLeft),
                item("Focus Right", FocusRight),
                item("Focus Up", FocusUp),
                item("Focus Down", FocusDown),
                item("Next Pane", FocusNext),
                item("Previous Pane", FocusPrev),
                MenuItem::separator(),
                item("Resize Left", ResizeLeft),
                item("Resize Right", ResizeRight),
                item("Resize Up", ResizeUp),
                item("Resize Down", ResizeDown),
                item("Even Out the Splits", Equalize),
                item("Fit to the App's Window", FitPane),
                MenuItem::separator(),
                item("Swap Left", SwapLeft),
                item("Swap Right", SwapRight),
                item("Swap Up", SwapUp),
                item("Swap Down", SwapDown),
            ],
        },
        Menu { name: "Tasks".into(), items: vec![item("New Task", NewTask), item("Next Task", NextTask)] },
        Menu { name: "Help".into(), items: vec![item("Keys", ShowKeys)] },
    ]
}
