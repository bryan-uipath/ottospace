//! The macOS menu bar: the same actions the keys run, findable by name.

use gpui::{Action, Menu, MenuItem, OsAction, SystemMenuType};

use crate::*;

fn item(name: &'static str, action: impl Action) -> MenuItem {
    MenuItem::action(name, action)
}

/// The menu bar. A feature that is off has no item: Marks' go with `marks`.
pub fn app_menus(features: &ottospace_core::features::Features) -> Vec<Menu> {
    let marks = features.on(ottospace_core::features::Feature::Marks);
    let transcripts = features.on(ottospace_core::features::Feature::Transcripts);
    let transcribe = [
        MenuItem::separator(),
        item("Start or Stop Transcribing", listen::Transcribe),
        item("Pause or Resume Transcribing", listen::PauseTranscribing),
    ];
    let mark_tab = [MenuItem::separator(), item("Mark Tab", MarkTab), item("Go to Mark…", MarkJump)];
    vec![
        Menu {
            name: "ottospace".into(),
            items: vec![
                item("About ottospace", About),
                MenuItem::separator(),
                item("Open Settings", commands::EditSettings),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                item("Features…", features::ShowFeatures),
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
                item("Move Tab to…", MoveTabToTask),
                item("Save Tab as Workspace", SaveWorkbook),
                item("Add Tab to Workspace…", AddTabToWorkbook),
                item("Save All Tabs as Workspace", SaveAllWorkbook),
                item("Keep Workspace", KeepWorkbook),
                MenuItem::separator(),
                item("Add Folder to Repos", AddToRepos),
                MenuItem::separator(),
                item("Start Fresh", StartFresh),
                MenuItem::separator(),
                item("Close Pane", ClosePane),
                item("Close Tab", CloseTab),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Undo", Undo, OsAction::Undo),
                MenuItem::os_action("Redo", Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", Cut, OsAction::Cut),
                MenuItem::os_action("Copy", CopySelection, OsAction::Copy),
                MenuItem::os_action("Paste", PasteText, OsAction::Paste),
                MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                item("Select with the Keyboard", VisualMode),
            ],
        },
        Menu {
            name: "View".into(),
            items: [
                item("Drive the Bar with the Keyboard", ListMode),
                item("Label the Controls", SneakMode),
                item("Fill the Whole Display", FillDisplay),
                item("Command Palette…", commands::CommandPalette),
                MenuItem::separator(),
                item("Previous Day in Calendar", commands::CalendarBack),
                item("Next Day in Calendar", commands::CalendarForward),
                item("Today in Calendar", commands::CalendarToday),
            ]
            .into_iter()
            .chain(mark_tab.into_iter().filter(|_| marks))
            .chain(transcribe.into_iter().filter(|_| transcripts))
            .chain([MenuItem::separator(), item("Next Tab", NextTab), item("Previous Tab", PrevTab)])
            .chain([
                MenuItem::separator(),
                item("Go to Home", GoHome),
                item("Next Tab Group", NextSpace),
                item("Previous Tab Group", PrevSpace),
                item("Next Conversation in This Workspace", NextChat),
                item("Previous Conversation in This Workspace", PrevChat),
                item("Hide", HideSpace),
            ])
            .collect(),
        },
        Menu {
            name: "Pane".into(),
            items: [
                item("Terminal Right", SplitRight),
                item("Terminal Below", SplitDown),
                item("New Pane…", PickPane),
                item("New Pane Below…", PickPaneDown),
            ]
            .into_iter()
            .chain(marks.then(|| item("Mark Pane", MarkPane)))
            .chain([
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
            ])
            .collect(),
        },
        Menu {
            name: "Tasks".into(),
            items: vec![
                item("New Task", NewTask),
                item("Next Task", NextTask),
                item("Capture the Pane", CapturePane),
                MenuItem::separator(),
                // The conversation in the drawer.
                item("Type in the Conversation", TalkFocus),
                item("Edit Message in Editor", EditMessage),
                item("Open or Close the Conversation", TalkToggle),
                item("Stop the Agent", StopAgent),
                item("Toggle Agent Dial", DialToggle),
            ],
        },
        Menu {
            name: "Developer".into(),
            items: vec![
                item("Accessibility Overlay", AxOverlay),
                item("Accessibility Inspector", commands::AxInspector),
                item("Filter the Accessibility Inspector", commands::InspectorFilter),
                item("Read the Inspected App Again", commands::InspectorRefresh),
                item("Copy State as JSON", commands::CopyState),
                item("Open the Settings File", commands::OpenSettings),
                item("Open the ottospace Folder", commands::OpenFolder),
                item("Bring All Windows Back", commands::BringBack),
            ],
        },
        // Named "Window": AppKit lists the open windows in it.
        Menu { name: "Window".into(), items: vec![item("Zoom", ZoomWindow)] },
        Menu {
            name: "Help".into(),
            items: vec![item("Keys", ShowKeys), item("Reset Key Counts", commands::ResetKeyCounts)],
        },
    ]
}
