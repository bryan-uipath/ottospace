//! The command palette, drawn: a box over the workspace, a filter and the commands it matches.
//! Running one dispatches its menu item's action, so it does what that key or menu item does.

use gpui::{
    Context, InteractiveElement, IntoElement, KeyDownEvent, Keymap, Keystroke, ParentElement, Styled, Window, actions,
    div, px, rgb,
};
use ottospace_core::calendar::DayStep;
use ottospace_core::commands::{Command, FeatureChoice, Palette, TaskChoice, View};
use ottospace_core::features::Stage;
use ottospace_core::goto::{Files, GoTo, Place, PlaceRow};
use ottospace_core::keys::FilterKey;
use ottospace_core::{Action, theme};

use crate::*;

thread_local! {
    /// The list (commands, tasks or flags), query and highlighted row last drawn.
    static SHOWN: std::cell::RefCell<Option<(u8, String, usize)>> = const { std::cell::RefCell::new(None) };
}

// ⌘⇧P, ⌘P (go to a place or file), and the Developer menu's own.
actions!(
    ottospace,
    [
        CommandPalette,
        GoToPlace,
        RenameTab,
        CopyState,
        CitySpin,
        TypingTime,
        OpenSettings,
        EditSettings,
        OpenFolder,
        AxInspector,
        InspectorFilter,
        InspectorRefresh,
        BringBack,
        ResetKeyCounts,
        CalendarBack,
        CalendarForward,
        CalendarToday
    ]
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
        Command::MoveTabToTask => Box::new(MoveTabToTask),
        Command::NextTab => Box::new(NextTab),
        Command::NextSpace => Box::new(NextSpace),
        Command::PrevSpace => Box::new(PrevSpace),
        Command::NextChat => Box::new(NextChat),
        Command::PrevChat => Box::new(PrevChat),
        Command::HideSpace => Box::new(HideSpace),
        Command::GoHome => Box::new(GoHome),
        Command::PrevTab => Box::new(PrevTab),
        Command::CloseTab => Box::new(CloseTab),
        Command::RenameTab => Box::new(RenameTab),
        Command::FoldTab => Box::new(FoldTab),
        Command::ZoomOut => Box::new(crate::zoom::ZoomToggle),
        Command::NewTask => Box::new(NewTask),
        Command::QuickNote => Box::new(QuickNote),
        Command::NextNeedsYou => Box::new(NextTask),
        Command::Decisions => Box::new(OpenDecisions),
        Command::WeekFocus => Box::new(crate::week_focus::WeekFocus),
        Command::CapturePane => Box::new(CapturePane),
        Command::TalkFocus => Box::new(TalkFocus),
        Command::EditMessage => Box::new(EditMessage),
        Command::DrawerToggle => Box::new(TalkToggle),
        Command::DrawerWide => Box::new(TalkWide),
        Command::SidebarToggle => Box::new(SidebarToggle),
        Command::StopAgent => Box::new(StopAgent),
        Command::DialToggle => Box::new(DialToggle),
        Command::SaveWorkbook => Box::new(SaveWorkbook),
        Command::AddTabToWorkbook => Box::new(AddTabToWorkbook),
        Command::SaveAllWorkbook => Box::new(SaveAllWorkbook),
        Command::KeepWorkbook => Box::new(KeepWorkbook),
        Command::AddToRepos => Box::new(AddToRepos),
        Command::StartFresh => Box::new(StartFresh),
        Command::Features => Box::new(features::ShowFeatures),
        Command::MarkPane => Box::new(MarkPane),
        Command::MarkTab => Box::new(MarkTab),
        Command::MarkJump => Box::new(MarkJump),
        Command::ListMode => Box::new(ListMode),
        Command::SneakMode => Box::new(SneakMode),
        Command::VisualMode => Box::new(VisualMode),
        Command::FillDisplay => Box::new(FillDisplay),
        Command::ShowKeys => Box::new(ShowKeys),
        Command::ResetKeyCounts => Box::new(ResetKeyCounts),
        Command::AxOverlay => Box::new(AxOverlay),
        Command::AxInspector => Box::new(AxInspector),
        Command::InspectorFilter => Box::new(InspectorFilter),
        Command::InspectorRefresh => Box::new(InspectorRefresh),
        Command::CopyState => Box::new(CopyState),
        Command::CitySpin => Box::new(CitySpin),
        Command::TypingTime => Box::new(TypingTime),
        Command::CalendarBack => Box::new(CalendarBack),
        Command::CalendarForward => Box::new(CalendarForward),
        Command::CalendarToday => Box::new(CalendarToday),
        Command::Transcribe => Box::new(crate::listen::Transcribe),
        Command::AgentMenu => Box::new(crate::agent_mode::AgentMenuKey),
        Command::AgentOff => Box::new(crate::agent_mode::AgentOff),
        Command::AgentSee => Box::new(crate::agent_mode::AgentSee),
        Command::AgentAct => Box::new(crate::agent_mode::AgentAct),
        Command::PauseTranscript => Box::new(crate::listen::PauseTranscribing),
        Command::OpenSettings => Box::new(OpenSettings),
        Command::OpenFolder => Box::new(OpenFolder),
        Command::BringBack => Box::new(BringBack),
        Command::EditSettings => Box::new(EditSettings),
        Command::About => Box::new(About),
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
    let (mods, key) = key_parts(stroke);
    mods + &key
}

/// A keystroke's modifiers and key, as the docs write them: ("⌘⌥", "←").
pub fn key_parts(stroke: &Keystroke) -> (String, String) {
    let m = stroke.modifiers;
    let mods = [(m.platform, "⌘"), (m.control, "⌃"), (m.alt, "⌥"), (m.shift, "⇧")];
    let text: String = mods.iter().filter(|(on, _)| *on).map(|(_, sign)| *sign).collect();
    let key = match stroke.key.as_str() {
        "left" => "←".to_string(),
        "right" => "→".to_string(),
        "up" => "↑".to_string(),
        "down" => "↓".to_string(),
        "tab" => "Tab".to_string(),
        "enter" => "↵".to_string(),
        "space" => "Space".to_string(),
        key => key.to_uppercase(),
    };
    (text, key)
}

impl Workspace {
    /// The Calendar's day, from the palette or the View menu.
    fn calendar_step(&mut self, step: DayStep, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::CalendarDay { step }, self.input, window, cx);
    }

    /// The palette's actions and the Developer menu's, on the workspace's root element.
    pub fn command_actions<E: InteractiveElement>(&self, root: E, cx: &mut Context<Self>) -> E {
        root.on_action(cx.listener(|this, _: &CommandPalette, window, cx| this.toggle_palette(window, cx)))
            .on_action(cx.listener(|this, _: &GoToPlace, window, cx| {
                let action = if this.model.palette.is_some() { Action::ClosePalette } else { Action::PaletteGoTo };
                this.dispatch(action, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &CopyState, window, cx| {
                let state = this.snapshot(window, cx);
                this.copy_text(state, None, ottospace_core::clip::Verb::Copied, cx);
            }))
            .on_action(cx.listener(|this, _: &CitySpin, window, cx| this.city_spin_start(this.input, window, cx)))
            .on_action(
                cx.listener(|this, _: &TypingTime, window, cx| {
                    this.typing_start(this.input, (false, false), window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &OpenSettings, _, cx| {
                this.error = open_in_user_folder(Some("settings.json")).err();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &features::ShowFeatures, window, cx| {
                this.dispatch(Action::PaletteFeatures, this.input, window, cx);
            }))
            .on_action(cx.listener(|this, _: &EditSettings, window, cx| this.edit_settings(window, cx)))
            .on_action(cx.listener(|this, _: &ResetKeyCounts, window, cx| this.reset_key_use(window, cx)))
            .on_action(cx.listener(|this, _: &CalendarBack, window, cx| this.calendar_step(DayStep::Back, window, cx)))
            .on_action(
                cx.listener(|this, _: &CalendarForward, window, cx| this.calendar_step(DayStep::Forward, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &CalendarToday, window, cx| this.calendar_step(DayStep::Today, window, cx)),
            )
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

    /// ⌘⇧P: open the palette, or close it.
    fn toggle_palette(&mut self, window: &Window, cx: &mut Context<Self>) {
        let action = if self.model.palette.is_some() { Action::ClosePalette } else { Action::PaletteOpen };
        self.dispatch(action, self.input, window, cx);
    }

    /// While the palette is open it takes every key: text edits the filter, ↑↓ move, ↵ runs.
    /// ⌘ shortcuts the filter does not use still reach their bindings, and wait behind it.
    pub fn palette_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        // Go to: Tab goes into a place's files; Esc, or ⌫ with nothing typed, comes back out.
        let view = self.model.palette.as_ref().map(|palette| (palette.view(), palette.query.text().is_empty()));
        let plain = !event.keystroke.modifiers.modified();
        // Opened on the checkout on show: Esc closes, as the palette's does.
        let here = self.model.palette.as_ref().and_then(|p| p.goto.as_ref()?.drill.as_ref()).is_some_and(|d| d.here);
        let goto = match (view, event.keystroke.key.as_str()) {
            (Some((View::Places, _)), "tab") if plain => Some(Action::GoToDrill),
            (Some((View::Files, _)), "escape") if !here => Some(Action::GoToBack),
            (Some((View::Files, true)), "backspace") if plain => Some(Action::GoToBack),
            _ => None,
        };
        if let Some(action) = goto {
            self.dispatch(action, self.input, window, cx);
            return cx.stop_propagation();
        }
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
    /// it lists spaces: Move Tab to…'s id, the space's title, "Move to" and no key. A flag's row
    /// adds its description, stage and whether it is on. `rect`, `group_rect` (null: no group
    /// column), `stage_rect` (null: no stage icon) and `text_rect` (a flag's title over its
    /// description; null otherwise): where they were drawn.
    pub fn palette_rows(&self, cx: &App) -> Option<serde_json::Value> {
        let palette = self.model.palette.as_ref()?;
        if matches!(palette.view(), View::Places | View::Files) {
            return None;
        }
        let keymap = cx.key_bindings();
        let drawn =
            |part: &str, at: usize| DRAWN.with_borrow(|drawn| drawn.get(&format!("{part}:{at}")).map(|b| rect(*b)));
        let row = |(at, item): (usize, Item)| {
            let flag = item.flag.as_ref();
            serde_json::json!({
                "command": item.command,
                "name": item.name,
                "group": item.group,
                "key": item.key,
                "description": flag.map(|choice| choice.feature.description()),
                "stage": flag.map(|choice| choice.feature.stage().label()),
                "on": flag.map(|choice| choice.on),
                "rect": drawn("row", at),
                "group_rect": drawn("group", at),
                "stage_rect": drawn("stage", at),
                "text_rect": drawn("text", at),
            })
        };
        Some(palette_items(palette, &keymap.borrow()).into_iter().enumerate().map(row).collect())
    }

    /// Go to's rows as listed, for a test run's state: each place's kind, name and detail, or each
    /// file. Null outside a test run (a file's path is in no other state) and while it lists commands.
    pub fn goto_rows(&self) -> Option<serde_json::Value> {
        std::env::var_os("OTTOSPACE_E2E")?;
        let palette = self.model.palette.as_ref()?;
        let goto = palette.goto.as_ref().filter(|_| matches!(palette.view(), View::Places | View::Files))?;
        let rows: Vec<serde_json::Value> = match &goto.drill {
            Some(_) => goto
                .rows
                .iter()
                .map(|file| serde_json::json!({ "place": null, "name": null, "detail": null, "file": file }))
                .collect(),
            None => {
                let row = |row: &PlaceRow| serde_json::json!({ "place": kind(&row.place).0, "name": row.name, "detail": row.detail, "file": null });
                goto.place_matches(palette.query.text()).into_iter().map(|(_, r)| row(r)).collect()
            }
        };
        Some(rows.into())
    }

    /// The palette: a box near the top, over the dimmed workspace. A click outside closes it.
    pub fn render_palette(&self, palette: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        // The highlighted row comes into view when it or the query changes (↑↓, typing, a paste).
        let list = u8::from(palette.tasks.is_some()) + 2 * u8::from(palette.features.is_some());
        let now = (list, palette.query.text().to_string(), palette.selected);
        if SHOWN.replace(Some(now.clone())) != Some(now) {
            // The first row: the top. Scrolling it into view measures against the list's last
            // height (one row, or none), which cut a two-line first row (Features…).
            match palette.selected {
                0 => self.bars.step("palette", ottospace_core::scrollbar::Step::Top),
                at => self.bars.scroll_to_item("palette", at),
            }
        }
        DRAWN.with_borrow_mut(|drawn| drawn.clear());
        if let (View::Places | View::Files, Some(goto)) = (palette.view(), &palette.goto) {
            return self.render_goto(palette, goto, cx).into_any_element();
        }
        // The group column only where rows may differ in it: the commands, not tasks or flags.
        let groups = palette.tasks.is_none() && palette.features.is_none();
        let items = palette_items(palette, &cx.key_bindings().borrow());
        let rows = items.into_iter().enumerate().map(|(i, item)| {
            let row = dialog::filter_row(("command", i), i == palette.selected).relative().child(note("row", i));
            let row = row.when(groups, |row| {
                let group = div().relative().w(px(72.)).flex_none().text_xs().text_color(rgb(theme::MUTED));
                row.child(group.child(item.group).child(note("group", i)))
            });
            let row = match &item.flag {
                Some(choice) => flag_row(row, i, choice),
                None => row
                    .child(crate::cut_text(item.name).flex_1())
                    .children(item.key.map(|key| div().flex_none().text_xs().text_color(rgb(theme::MUTED)).child(key))),
            };
            row.on_click(cx.listener(move |this, _, window, cx| {
                this.dispatch(Action::PaletteChoose { index: Some(i) }, this.input, window, cx);
            }))
            .into_any_element()
        });
        let rows = rows.collect();
        let tasks = palette.tasks.is_some();
        let (title, hint) = match (tasks, palette.features.is_some()) {
            (true, _) => ("Move to", "type a task"),
            (_, true) => ("Turn", "type a feature"),
            _ if palette.goto.is_some() => ("Run", "type a command, or delete > for places"),
            _ => ("Run", "type a command"),
        };
        let header = div()
            .h(px(40.))
            .child(div().text_xs().text_color(rgb(theme::COMMAND)).child(title))
            .child(self.render_field(hint, cx));
        let panel = dialog::floating(560.).max_w_full().max_h(px(440.));
        let empty = match (tasks, palette.features.is_some()) {
            (true, _) => "No task matches",
            (_, true) => "No feature matches",
            _ => "No command matches",
        };
        let panel = self.list_panel(panel, header, "palette", rows, empty);
        palette_box(panel, cx).into_any_element()
    }

    /// Go to: the places (each with its kind's icon, its name and a muted detail), or the files
    /// of the place gone into, under its name.
    fn render_goto(&self, palette: &Palette, goto: &GoTo, cx: &mut Context<Self>) -> impl IntoElement {
        let listen: &Context<Self> = cx;
        let choose = move |i: usize| {
            listen.listener(move |this: &mut Self, _: &gpui::ClickEvent, window, cx| {
                this.dispatch(Action::PaletteChoose { index: Some(i) }, this.input, window, cx);
            })
        };
        let selected = palette.selected;
        let (title, hint, empty, rows): (String, _, _, Vec<gpui::AnyElement>) = match &goto.drill {
            None => {
                let places = goto.place_matches(palette.query.text()).into_iter().enumerate();
                let rows = places.map(|(i, (_, row))| {
                    let icon = gpui::svg().path(kind(&row.place).1).size(px(14.)).flex_none();
                    dialog::filter_row(("place", i), i == selected)
                        .child(icon.text_color(rgb(theme::MUTED)))
                        .child(crate::cut_text(row.name.clone()).flex_1())
                        .child(
                            crate::cut_text(row.detail.clone()).max_w(px(240.)).text_xs().text_color(rgb(theme::MUTED)),
                        )
                        .on_click(choose(i))
                        .into_any_element()
                });
                (
                    "Go to".into(),
                    "type a place, Tab or / for its files, > for commands",
                    "No place matches",
                    rows.collect(),
                )
            }
            Some(drill) => {
                let files = drill.row.folder.as_deref().and_then(|folder| goto.files_of(folder));
                let (empty, title) = match files {
                    Some(Files::Read { paths, cut: true }) => {
                        ("No file matches", format!("{} · first {} files", drill.row.name, paths.len()))
                    }
                    Some(Files::Read { .. }) => ("No file matches", drill.row.name.clone()),
                    Some(Files::Unread(why)) => (*why, drill.row.name.clone()),
                    _ => ("reading files…", drill.row.name.clone()),
                };
                let rows = goto.rows.iter().enumerate().map(|(i, file)| {
                    dialog::filter_row(("file", i), i == selected)
                        .child(crate::cut_text(file.clone()).flex_1())
                        .on_click(choose(i))
                        .into_any_element()
                });
                (title, "type a file · ↵ opens · ⌘↵ below · ⌫ places", empty, rows.collect())
            }
        };
        let header = div()
            .h(px(40.))
            .child(div().text_xs().text_color(rgb(theme::COMMAND)).child(title))
            .child(self.render_field(hint, cx));
        let panel = dialog::floating(560.).max_w_full().max_h(px(440.));
        palette_box(self.list_panel(panel, header, "palette", rows, empty), cx)
    }
}

/// The palette's box near the top, over the dimmed workspace. A click outside closes it.
fn palette_box(panel: gpui::Div, cx: &mut Context<Workspace>) -> gpui::Stateful<gpui::Div> {
    dialog::backdrop("palette-backdrop", crate::ui::dialog::DIM, Action::ClosePalette, cx)
        .pt(px(72.))
        .px_4()
        .flex()
        .flex_col()
        .items_center()
        .child(panel)
}

/// A place's kind, as the state names it, and its section's icon.
fn kind(place: &Place) -> (&'static str, &'static str) {
    match place {
        Place::Workspace(_) => ("workspace", "icons/workbook.svg"),
        Place::Checkout(_) => ("checkout", "icons/repos.svg"),
        Place::Task(_) => ("task", "icons/tasks.svg"),
    }
}

thread_local! {
    /// Where each part of a palette row was last drawn, by `<part>:<row>`: for the state.
    static DRAWN: std::cell::RefCell<std::collections::BTreeMap<String, gpui::Bounds<gpui::Pixels>>> =
        std::cell::RefCell::default();
}

/// An invisible layer over its parent that notes where it shows, as `<part>:<row>`: none for a
/// part scrolled out of the list.
fn note(part: &str, row: usize) -> gpui::AnyElement {
    let key = format!("{part}:{row}");
    let note = move |bounds: gpui::Bounds<gpui::Pixels>, window: &mut Window, _: &mut App| {
        let shown = bounds.intersect(&window.content_mask().bounds);
        if shown.size.width > px(1.) && shown.size.height > px(1.) {
            DRAWN.with_borrow_mut(|drawn| drawn.insert(key, shown));
        }
    };
    gpui::canvas(note, |_, _, _, _| {}).absolute().top_0().left_0().size_full().into_any_element()
}

/// A flag's row: the stage's icon, its title, id and key over its description, then On or Off.
pub(crate) fn flag_row(row: gpui::Stateful<gpui::Div>, i: usize, choice: &FeatureChoice) -> gpui::Stateful<gpui::Div> {
    let feature = choice.feature;
    let title = div()
        .flex()
        .items_center()
        .gap(px(theme::SPACE_1_5))
        .child(div().flex_none().child(feature.title()))
        .child(div().flex_none().text_xs().text_color(rgb(theme::MUTED)).child(feature.name()))
        .children(feature.key().map(|key| ui::pill(key, ui::Pill::Key { lit: false, small: true }).flex_none()));
    let (state, colour) = if choice.on { ("On", theme::PRIMARY) } else { ("Off", theme::MUTED) };
    // The title's line: the icon and On or Off sit level with it.
    let line = || div().h(px(ui::TITLE_LINE)).flex_none().flex().items_center();
    // As tall as its lines, padded alike above and below: a one-line description, a shorter row.
    let text = ui::two_lines(title, feature.description()).flex_1().relative().child(note("text", i));
    row.h_auto()
        .py(px(theme::SPACE_1_5))
        .items_start()
        .child(line().w(px(theme::ICON_SM)).children(stage_icon(i, feature.stage())))
        .child(text)
        .child(line().w(px(24.)).justify_end().text_xs().text_color(rgb(colour)).child(state))
}

/// A flask for an Experimental flag, its stage on hover; a Preview one has none.
fn stage_icon(i: usize, stage: Stage) -> Option<gpui::Stateful<gpui::Div>> {
    let icon = ui::stage_icon(stage)?;
    let tip = move |_: &mut Window, cx: &mut App| gpui::AnyView::from(cx.new(|_| Tooltip(stage.label().into())));
    Some(div().id(("stage", i)).relative().child(icon).child(note("stage", i)).tooltip(tip))
}

/// A palette row: its command, group and name, its key, and the flag it turns while it lists flags.
struct Item {
    command: Command,
    group: &'static str,
    name: String,
    key: Option<String>,
    flag: Option<FeatureChoice>,
}

/// The palette's rows. While it lists tasks, each task is a row of Move Tab to…, under "Move to";
/// while it lists flags, each a row of Features….
fn palette_items(palette: &Palette, keymap: &Keymap) -> Vec<Item> {
    let item = |command, group, name, key, flag| Item { command, group, name, key, flag };
    if palette.tasks.is_some() {
        let task = |choice: &TaskChoice| item(Command::MoveTabToTask, "Move to", choice.title.clone(), None, None);
        return palette.task_matches().into_iter().map(task).collect();
    }
    if palette.features.is_some() {
        let flag = |choice: &FeatureChoice| {
            let (title, key) = (choice.feature.title().to_string(), choice.feature.key().map(String::from));
            item(Command::Features, "Features", title, key, Some(choice.clone()))
        };
        return palette.feature_matches().into_iter().map(flag).collect();
    }
    let command = |c: Command| item(c, c.group().label(), c.name().to_string(), key_of(keymap, c), None);
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
        for menu in menus::app_menus(&ottospace_core::features::Features::all()) {
            walk(&menu.items, &mut out);
        }
        out
    }

    /// The palette lists the menu bar: a menu item with no command, or a command in no menu, fails.
    #[test]
    fn every_menu_item_is_a_command_of_the_same_name() {
        let mut items = menu_items();
        // The palette's own items open it: they are no rows of it.
        items.retain(|(_, action)| {
            *action != gpui::Action::name(&CommandPalette) && *action != gpui::Action::name(&GoToPlace)
        });
        // Window ▸ Zoom is the green button's: the palette has Fill the Whole Display.
        items.retain(|(_, action)| *action != gpui::Action::name(&ZoomWindow));
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
