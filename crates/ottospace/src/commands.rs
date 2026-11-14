//! The command palette, drawn: a box over the workspace, a filter and the commands it matches.
//! Running one dispatches its menu item's action, so it does what that key or menu item does.

use gpui::{
    Context, InteractiveElement, IntoElement, KeyDownEvent, Keymap, Keystroke, ParentElement, Styled, Window, actions,
    div, px, rgb,
};
use ottospace_core::commands::{Command, Palette, TaskChoice};
use ottospace_core::keys::FilterKey;
use ottospace_core::{Action, theme};

use crate::*;

thread_local! {
    /// The query and highlighted row last drawn.
    static SHOWN: std::cell::RefCell<Option<(String, usize)>> = const { std::cell::RefCell::new(None) };
}

// ⌘P, and the Developer menu's own.
actions!(
    ottospace,
    [CommandPalette, CopyState, OpenSettings, OpenFolder, AxInspector, InspectorFilter, InspectorRefresh, BringBack]
);

/// The action a command's menu item dispatches. A new command must name one here.
pub fn action(command: Command) -> Box<dyn gpui::Action> {
    match command {
        Command::SplitRight => Box::new(SplitRight),
        Command::SplitDown => Box::new(SplitDown),
        Command::PickPane => Box::new(PickPane),
        Command::PickPaneDown => Box::new(PickPaneDown),
        Command::ClosePane => Box::new(ClosePane),
        Command::FocusLeft => Box::new(FocusLeft),
        Command::FocusRight => Box::new(FocusRight),
        Command::FocusUp => Box::new(FocusUp),
        Command::FocusDown => Box::new(FocusDown),
        Command::FocusNext => Box::new(FocusNext),
        Command::FocusPrev => Box::new(FocusPrev),
        Command::ResizeLeft => Box::new(ResizeLeft),
        Command::ResizeRight => Box::new(ResizeRight),
        Command::ResizeUp => Box::new(ResizeUp),
        Command::ResizeDown => Box::new(ResizeDown),
        Command::Equalize => Box::new(Equalize),
        Command::FitPane => Box::new(FitPane),
        Command::SwapLeft => Box::new(SwapLeft),
        Command::SwapRight => Box::new(SwapRight),
        Command::SwapUp => Box::new(SwapUp),
        Command::SwapDown => Box::new(SwapDown),
        Command::NewTab => Box::new(NewTab),
        Command::NewTabOutside => Box::new(NewTabOutside),
        Command::RemoveTabFromTask => Box::new(RemoveTabFromTask),
        Command::MoveTabToTask => Box::new(MoveTabToTask),
        Command::NextTab => Box::new(NextTab),
        Command::PrevTab => Box::new(PrevTab),
        Command::CloseTab => Box::new(CloseTab),
        Command::NewTask => Box::new(NewTask),
        Command::NextTask => Box::new(NextTask),
        Command::CapturePane => Box::new(CapturePane),
        Command::TalkFocus => Box::new(TalkFocus),
        Command::TalkToggle => Box::new(TalkToggle),
        Command::TalkGo => Box::new(TalkGo),
        Command::StopAgent => Box::new(StopAgent),
        Command::SaveWorkbook => Box::new(SaveWorkbook),
        Command::AddTabToWorkbook => Box::new(AddTabToWorkbook),
        Command::SaveAllWorkbook => Box::new(SaveAllWorkbook),
        Command::StartFresh => Box::new(StartFresh),
        Command::MarkPane => Box::new(MarkPane),
        Command::MarkTab => Box::new(MarkTab),
        Command::MarkJump => Box::new(MarkJump),
        Command::ListMode => Box::new(ListMode),
        Command::SneakMode => Box::new(SneakMode),
        Command::VisualMode => Box::new(VisualMode),
        Command::Zoom => Box::new(Zoom),
        Command::ShowKeys => Box::new(ShowKeys),
        Command::AxOverlay => Box::new(AxOverlay),
        Command::AxInspector => Box::new(AxInspector),
        Command::InspectorFilter => Box::new(InspectorFilter),
        Command::InspectorRefresh => Box::new(InspectorRefresh),
        Command::CopyState => Box::new(CopyState),
        Command::OpenSettings => Box::new(OpenSettings),
        Command::OpenFolder => Box::new(OpenFolder),
        Command::BringBack => Box::new(BringBack),
        Command::Hide => Box::new(Hide),
        Command::Quit => Box::new(Quit),
    }
}

/// The first key bound to a command, as the docs write it: "⌘⇧D", "⌘⌥←". None: no key.
pub fn key_of(keymap: &Keymap, command: Command) -> Option<String> {
    let action = action(command);
    let binding = keymap.bindings_for_action(action.as_ref()).next()?;
    let strokes: Vec<String> = binding.keystrokes().iter().map(|stroke| key_text(stroke.inner())).collect();
    Some(strokes.join(" "))
}

fn key_text(stroke: &Keystroke) -> String {
    let m = stroke.modifiers;
    let mods = [(m.platform, "⌘"), (m.control, "⌃"), (m.alt, "⌥"), (m.shift, "⇧")];
    let mut text: String = mods.iter().filter(|(on, _)| *on).map(|(_, sign)| *sign).collect();
    text += &match stroke.key.as_str() {
        "left" => "←".to_string(),
        "right" => "→".to_string(),
        "up" => "↑".to_string(),
        "down" => "↓".to_string(),
        "tab" => "Tab".to_string(),
        "space" => "Space".to_string(),
        key => key.to_uppercase(),
    };
    text
}

impl Workspace {
    /// The palette's actions and the Developer menu's, on the workspace's root element.
    pub fn command_actions<E: InteractiveElement>(&self, root: E, cx: &mut Context<Self>) -> E {
        root.on_action(cx.listener(|this, _: &CommandPalette, window, cx| this.toggle_palette(window, cx)))
            .on_action(cx.listener(|this, _: &CopyState, window, cx| {
                let state = this.snapshot(window, cx);
                this.copy_text(state, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, _, cx| {
                this.error = open_in_user_folder(Some("settings.json")).err();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &AxInspector, window, cx| this.toggle_inspector(window, cx)))
            .on_action(
                cx.listener(|this, _: &InspectorFilter, window, cx| this.inspector_command("filter", window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &InspectorRefresh, window, cx| this.inspector_command("refresh", window, cx)),
            )
            .on_action(cx.listener(|this, _: &OpenFolder, _, cx| {
                this.error = open_in_user_folder(None).err();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &BringBack, _, cx| {
                this.error = Some(this.bring_back());
                cx.notify();
            }))
    }

    /// Bring All Windows Back: the user's alone (menu, palette), never a client's. A test run
    /// moves its canary's windows only, and with hosting off none.
    fn bring_back(&self) -> String {
        if self.input != Origin::User {
            return "only the user brings windows back".into();
        }
        if !self.hosting {
            return "window hosting is off: no window moved".into();
        }
        let hosted: Vec<u32> = self.hosted.values().map(|h| h.window_id).collect();
        match crate::give_back::rescue(self.host_only, &hosted) {
            1 => "moved 1 window back on screen".into(),
            n => format!("moved {n} windows back on screen"),
        }
    }

    /// ⌘P: open the palette, or close it.
    fn toggle_palette(&mut self, window: &Window, cx: &mut Context<Self>) {
        let action = if self.model.palette.is_some() { Action::ClosePalette } else { Action::PaletteOpen };
        self.dispatch(action, self.input, window, cx);
    }

    /// While the palette is open it takes every key: text edits the filter, ↑↓ move, ↵ runs.
    /// ⌘ shortcuts the filter does not use still reach their bindings, and wait behind it.
    pub fn palette_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let action = |key| match key {
            FilterKey::Close => Action::ClosePalette,
            FilterKey::Choose => Action::PaletteChoose { index: None },
            FilterKey::Move(delta) => Action::PaletteMove { delta },
        };
        self.filtered_list_key(event, action, window, cx);
    }

    /// The command chosen: its menu item's action, dispatched as a key's binding would be.
    pub fn run_command(&mut self, command: Command, window: &Window, cx: &mut Context<Self>) {
        window.defer(cx, move |window, cx| window.dispatch_action(action(command), cx));
    }

    /// The rows as drawn, for the state snapshot: each command's id, name, group and key. While
    /// it lists tasks: Move Tab to Task's id, the task's title, "Move to" and no key.
    pub fn palette_rows(&self, cx: &App) -> Option<serde_json::Value> {
        let palette = self.model.palette.as_ref()?;
        let keymap = cx.key_bindings();
        let row = |(command, group, name, key)| {
            serde_json::json!({
                "command": command,
                "name": name,
                "group": group,
                "key": key,
            })
        };
        Some(palette_items(palette, &keymap.borrow()).into_iter().map(row).collect())
    }

    /// The palette: a box near the top, over the dimmed workspace. A click outside closes it.
    pub fn render_palette(&self, palette: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        // The highlighted row comes into view when it or the query changes (↑↓, typing, a paste).
        let now = (palette.query.text().to_string(), palette.selected);
        if SHOWN.replace(Some(now.clone())) != Some(now) {
            self.bars.scroll_to_item("palette", palette.selected);
        }
        let items = palette_items(palette, &cx.key_bindings().borrow());
        let rows = items.into_iter().enumerate().map(|(i, (_, group, name, key))| {
            dialog::filter_row(("command", i), i == palette.selected)
                .child(div().w(px(72.)).flex_none().text_xs().text_color(rgb(theme::MUTED)).child(group))
                .child(div().flex_1().min_w_0().truncate().child(name))
                .children(key.map(|key| div().flex_none().text_xs().text_color(rgb(theme::MUTED)).child(key)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::PaletteChoose { index: Some(i) }, this.input, window, cx);
                }))
                .into_any_element()
        });
        let rows = rows.collect();
        let tasks = palette.tasks.is_some();
        let (title, hint) = if tasks { ("Move to", "type a task") } else { ("Run", "type a command") };
        let header = div()
            .h(px(40.))
            .child(div().text_xs().text_color(rgb(theme::COMMAND)).child(title))
            .child(self.render_field(hint, cx));
        let panel = dialog::floating(560.).max_w_full().max_h(px(440.));
        let texts = match tasks {
            true => ("No task matches", "↑↓ move · ↵ move the tab · esc close"),
            false => ("No command matches", "↑↓ move · ↵ run · esc close"),
        };
        let panel = self.list_panel(panel, header, "palette", rows, texts);
        dialog::backdrop("palette-backdrop", 0.35, Action::ClosePalette, cx)
            .pt(px(72.))
            .px_4()
            .flex()
            .flex_col()
            .items_center()
            .child(panel)
    }
}

/// The palette's rows: (command, group, name, key). While it lists tasks, each task is a row of
/// Move Tab to Task, under "Move to".
fn palette_items(palette: &Palette, keymap: &Keymap) -> Vec<(Command, &'static str, String, Option<String>)> {
    if palette.tasks.is_some() {
        let task = |choice: &TaskChoice| (Command::MoveTabToTask, "Move to", choice.title.clone(), None);
        return palette.task_matches().into_iter().map(task).collect();
    }
    let command = |c: Command| (c, c.group().label(), c.name().to_string(), key_of(keymap, c));
    palette.matches().into_iter().map(command).collect()
}

/// Opens the user's folder, or a file in it, as Finder would. A test run that named no folder has none.
fn open_in_user_folder(file: Option<&str>) -> Result<(), String> {
    let Some(home) = config::get().home.as_ref() else { return Err("no ottospace folder in this run".into()) };
    let path = file.map_or_else(|| home.clone(), |file| home.join(file));
    if !path.exists() {
        return Err(format!("{} does not exist yet", path.display()));
    }
    // `-t`: a text file opens in the text editor, not in whatever claims `.json`.
    let mut open = std::process::Command::new("open");
    open.args(file.map(|_| "-t")).arg(&path);
    open.spawn().map(drop).map_err(|err| format!("open: {err}"))
}

#[cfg(test)]
mod tests {
    use gpui::{KeyBinding, MenuItem};
    use ottospace_core::commands::ALL;

    use super::*;

    /// Every menu item that is no text-field edit, by name and the action it dispatches.
    fn menu_items() -> Vec<(String, &'static str)> {
        fn walk(items: &[MenuItem], out: &mut Vec<(String, &'static str)>) {
            for item in items {
                match item {
                    MenuItem::Action { name, action, os_action: None } => out.push((name.to_string(), action.name())),
                    MenuItem::Submenu(menu) => walk(&menu.items, out),
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        for menu in menus::app_menus() {
            walk(&menu.items, &mut out);
        }
        out
    }

    /// The palette lists the menu bar: a menu item with no command, or a command in no menu, fails.
    #[test]
    fn every_menu_item_is_a_command_of_the_same_name() {
        let mut items = menu_items();
        // The palette's own item opens it: it is no row of it.
        items.retain(|(_, action)| *action != gpui::Action::name(&CommandPalette));
        let commands: Vec<(String, &'static str)> =
            ALL.iter().map(|c| (c.name().to_string(), action(*c).name())).collect();
        for item in &items {
            assert!(commands.contains(item), "menu item {item:?} is no command");
        }
        for command in &commands {
            assert!(items.contains(command), "command {command:?} is in no menu");
        }
    }

    #[test]
    fn a_command_shows_its_first_key() {
        let keymap = Keymap::new(vec![
            KeyBinding::new("cmd-alt-left", FocusLeft, None),
            KeyBinding::new("cmd-alt-h", FocusLeft, None),
            KeyBinding::new("cmd-shift-d", SplitDown, None),
            KeyBinding::new("cmd-ctrl-a", AxOverlay, None),
        ]);
        assert_eq!(key_of(&keymap, Command::FocusLeft).as_deref(), Some("⌘⌥←"));
        assert_eq!(key_of(&keymap, Command::SplitDown).as_deref(), Some("⌘⇧D"));
        assert_eq!(key_of(&keymap, Command::AxOverlay).as_deref(), Some("⌘⌃A"));
        assert_eq!(key_of(&keymap, Command::Quit), None);
    }
}
