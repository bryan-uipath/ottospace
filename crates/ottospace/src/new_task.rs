//! The new-task box. `new_task_content` is what it holds: the line, the dropdown that finishes
//! the tag at the cursor, the hints. It knows nothing of where it is: no size, no position.
//! The drawer hosts it, in its new-task state (`render_new_task`), or in its collecting state
//! as the draft's title (`collect.rs`). Which one is the core's `compose_host`.

use std::path::PathBuf;

use gpui::{
    Bounds, Context, Div, ExternalPaths, FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, PathPromptOptions,
    Pixels, Point, Window, div, prelude::*, px, rgb,
};
use ottospace_core::complete::{self, Dropdown, Enter, Row};
use ottospace_core::model::{Pointed, TAG_KEYS, TerminalInfo};
use ottospace_core::{Action, Origin, theme};

use crate::{Workspace, dialog, keycaps};

/// The dropdown's width, and a row's height: the shell knows its room from them.
const MENU_WIDTH: f32 = 340.;
const ROW: f32 = 26.;

/// A tag's colour, by its mark: in the line, and on the dropdown's kind marks.
pub fn tag_color(mark: char) -> u32 {
    match mark {
        '#' => theme::COMPOSE,
        '>' => theme::PRIMARY,
        '@' => theme::VISUAL,
        '$' => theme::SNEAK,
        '^' => theme::PICK,
        _ => theme::TEXT,
    }
}

impl Workspace {
    /// The box's keys. ↵ takes the dropdown's highlighted row, else adds the task and starts
    /// its agent; ⇧↵ adds it for later; ⌘↵ adds and starts it; ⌥⌘N switches between this checkout
    /// and a new worktree, where offered; Tab takes a row; ↑ ↓ (⌃P ⌃N) move over the rows; ⌃G edits the line in the user's editor; Esc closes the dropdown,
    /// then the box; ⌘O picks files. The rest edit the line. ⌘ shortcuts the line does not use
    /// still reach their bindings; pane ones wait.
    pub fn compose_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        let m = key.modifiers;
        let plain = !m.shift && !m.alt && !m.platform;
        let action = match key.key.as_str() {
            "enter" if event.is_held => return,
            "enter" if m.platform => Action::ComposeSubmit { open: true },
            "enter" if m.shift => Action::ComposeSubmit { open: false },
            "enter" => Action::ComposeEnter,
            "escape" => Action::ComposeEscape,
            "tab" => Action::ComposeComplete,
            "up" if plain && !m.control => Action::ComposeMove { delta: -1 },
            "down" if plain && !m.control => Action::ComposeMove { delta: 1 },
            "p" if plain && m.control => Action::ComposeMove { delta: -1 },
            "n" if plain && m.control => Action::ComposeMove { delta: 1 },
            // The line in the user's editor.
            "g" if plain && m.control => Action::EditorOpen,
            "n" if m.platform && m.alt && !m.shift && !m.control => Action::ComposeWorktree { new: None },
            "o" if m.platform && !m.shift && !m.alt && !m.control => {
                self.pick_files(window, cx);
                return cx.stop_propagation();
            }
            // What types text is not stopped: the input method inserts it.
            _ => {
                if self.field_key(event, window, cx) {
                    cx.stop_propagation();
                }
                return;
            }
        };
        self.dispatch(action, self.input, window, cx);
        cx.stop_propagation();
    }

    /// The box is about to open: tells the core what a `^reference` may name that only the
    /// shell knows. Each terminal's directory and program, and the apps installed.
    pub fn tell_compose_context(&mut self, cx: &mut Context<Self>) {
        let info = |(pane, terminal): (&ottospace_core::PaneId, &crate::Terminal)| TerminalInfo {
            pane: *pane,
            cwd: terminal.cwd(),
            program: terminal.running().and_then(|work| work.program),
        };
        let terminals = self.terminals.iter().map(info).collect();
        let home = std::env::var("HOME").ok().filter(|home| !home.is_empty());
        self.model.update(Action::SetTerminals { terminals, home }.sent_by(Origin::System));
        self.offer_installed(cx);
    }

    /// Files or folders for the line: dropped on the box, picked, or pasted. Each goes in at
    /// the cursor as a path word.
    pub fn compose_attach(&mut self, paths: &[PathBuf], origin: Origin, window: &Window, cx: &mut Context<Self>) {
        // A name that is not text cannot be in the line: the core is told how many were left out.
        let named: Vec<String> = paths.iter().filter_map(|path| path.to_str().map(str::to_string)).collect();
        let unreadable = paths.len() - named.len();
        let home = std::env::var("HOME").ok().filter(|home| !home.is_empty());
        self.dispatch(Action::ComposeAttach { paths: named, home, unreadable }, origin, window, cx);
    }

    /// ⌘O, or the box's + button: the system's file picker. What is chosen goes into the line.
    /// Not in a test run: nothing could answer the panel.
    pub fn pick_files(&mut self, window: &Window, cx: &mut Context<Self>) {
        if std::env::var_os("OTTOSPACE_E2E").is_some() {
            return;
        }
        let options = PathPromptOptions { files: true, directories: true, multiple: true, prompt: Some("Add".into()) };
        let chosen = cx.prompt_for_paths(options);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else { return };
            let _ = this.update_in(cx, |this, window, cx| this.compose_attach(&paths, Origin::User, window, cx));
        })
        .detach();
    }

    /// ⌘V with files on the clipboard (copied in Finder): they go into the new-task line as
    /// paths. False: there are none, or the line is not the new-task box's.
    pub fn paste_files(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        let composing = self.model.field().is_some_and(|(of, _)| of == "compose");
        if !composing {
            return false;
        }
        // A test run's stand-in holds no files: the system's is never read.
        let files = self.clipboard.files();
        if files.is_empty() {
            return false;
        }
        self.compose_attach(&files, self.input, window, cx);
        true
    }

    /// The tab or pane drawn at a window position: what an ⌥-click there would reference.
    /// Nothing under the box's own panel: that click is the box's.
    fn pointed_at(&self, position: Point<Pixels>) -> Option<(Pointed, Bounds<Pixels>)> {
        if self.field.borrow().over_panel(position) {
            return None;
        }
        let tab = self.model.tabs.iter().find_map(|tab| {
            let bounds = self.controls.bounds(&format!("tab:{}", tab.id))?;
            bounds.contains(&position).then_some((Pointed::Tab(tab.id), bounds))
        });
        let shown = self.model.layout.as_ref().map(|layout| layout.leaves()).unwrap_or_default();
        let pane = shown.into_iter().find_map(|id| {
            let bounds = self.frames.get(&id)?.get();
            bounds.contains(&position).then_some((Pointed::Pane(id), bounds))
        });
        tab.or(pane)
    }

    /// The pointer moved: with ⌥ held while the box is open, what is under it is outlined.
    pub fn compose_hover(&mut self, position: Point<Pixels>, alt: bool, cx: &mut Context<Self>) {
        self.hover_at = position;
        let pointing = self.model.compose.as_ref().filter(|_| alt).and_then(|_| self.pointed_at(position));
        let outline = pointing.map(|(_, bounds)| bounds);
        if self.pointing != outline {
            self.pointing = outline;
            cx.notify();
        }
    }

    /// An ⌥-click while the box is open: the tab or pane under it goes into the line as a
    /// reference. True: the click was the box's, whatever was under it. Nothing else changes.
    pub fn compose_point(&mut self, event: &MouseDownEvent, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.model.compose.is_none() || !event.modifiers.alt {
            return false;
        }
        if let Some((target, _)) = self.pointed_at(event.position) {
            self.dispatch(Action::ComposeRef { target }, self.input, window, cx);
        }
        true
    }

    /// The outline of what an ⌥-click would reference.
    pub fn render_pointing(&self) -> Option<Div> {
        let bounds = self.pointing.filter(|_| self.model.compose.is_some())?;
        let outline = div()
            .absolute()
            .left(bounds.origin.x)
            .top(bounds.origin.y)
            .w(bounds.size.width)
            .h(bounds.size.height)
            .rounded(px(4.))
            .border_2()
            .border_color(rgb(theme::PICK));
        Some(outline)
    }

    /// One row of the dropdown: what it is, its name with the matched letters stronger, a word
    /// of detail.
    fn dropdown_row(
        &self,
        index: usize,
        row: &Row,
        menu: &Dropdown,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let letters = row.name.chars().enumerate().fold(Vec::<(bool, String)>::new(), |mut runs, (at, c)| {
            let hit = row.matched.contains(&at);
            match runs.last_mut() {
                Some((was, text)) if *was == hit => text.push(c),
                _ => runs.push((hit, c.to_string())),
            }
            runs
        });
        let name = letters.into_iter().map(|(hit, text)| {
            let part = div().flex_none().child(text);
            if hit { part.text_color(rgb(theme::TEXT)).font_weight(FontWeight::BOLD) } else { part }
        });
        div()
            .id(("compose-row", index))
            .h(px(ROW))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .cursor_pointer()
            .when(menu.selected == Some(index), |d| d.bg(rgb(theme::SELECTED)))
            .hover(|s| s.bg(rgb(theme::SELECTED)))
            .child(div().w(px(46.)).flex_none().text_color(rgb(tag_color(menu.mark))).child(row.kind))
            .child(
                div().flex().flex_none().max_w(px(180.)).overflow_hidden().text_color(rgb(theme::MUTED)).children(name),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(rgb(theme::MUTED))
                    .opacity(0.7)
                    .child(row.detail.clone()),
            )
            // On the press, and no further: a click must not move the line's cursor or close the box.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::ComposeAccept { index }, this.input, window, cx);
                    cx.stop_propagation();
                }),
            )
    }

    /// The tags a line may hold, under it: each mark in a small cap, then its word, e.g. [>] repo.
    /// A click types the mark at the cursor, as the user's key would, and its dropdown opens.
    fn tag_caps(&self, cx: &mut Context<Self>) -> Div {
        let tags = TAG_KEYS.iter().map(|&(key, word)| {
            let mark = key.chars().next().filter(|_| key.chars().count() == 1);
            let id = format!("compose:tag:{key}");
            div()
                .id(gpui::SharedString::from(id.clone()))
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .rounded_sm()
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .child(keycaps::cap(key, false, true))
                .child(word)
                .child(self.controls.mark(id))
                .when_some(mark, |d, mark| {
                    // On the press, and no further: the line keeps the keyboard and its cursor.
                    d.cursor_pointer().hover(|s| s.text_color(rgb(theme::TEXT))).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.dispatch(Action::ComposeTag { mark }, this.input, window, cx);
                            cx.stop_propagation();
                        }),
                    )
                })
        });
        div().flex_1().min_w_0().flex().flex_wrap().items_center().gap_x_3().gap_y_1().children(tags)
    }

    /// The dropdown, and whether it goes above the line: under the tag it finishes, unless
    /// the window has no room below.
    fn render_dropdown(&self, menu: &Dropdown, cx: &mut Context<Self>) -> (Div, bool) {
        let rows = menu.rows.iter().enumerate().map(|(i, row)| self.dropdown_row(i, row, menu, cx).into_any_element());
        let note = menu.note.clone().map(|note| {
            div().h(px(ROW)).px_2().flex().items_center().text_xs().text_color(rgb(theme::MUTED)).child(note)
        });
        // Above or below by the tallest it can be, not by its rows now: it never jumps as they change.
        let height = px(ROW * (complete::MAX_ROWS + 1) as f32 + 10.);
        let place = self.field_place(menu.start);
        // The line's left edge is the content's: the dropdown is pushed right to sit under its tag.
        let indent = place.map_or(px(0.), |(x, field, _)| (x - field.left()).min(field.size.width - px(MENU_WIDTH)));
        let above = place.is_some_and(|(_, field, viewport)| field.bottom() + height + px(90.) > viewport);
        let drawn = self.field.clone();
        let list = div()
            .relative()
            .w(px(MENU_WIDTH))
            .max_w_full()
            .ml(indent.max(px(0.)))
            .py_1()
            .flex()
            .flex_col()
            .rounded(px(dialog::INNER_RADIUS))
            .bg(rgb(theme::BACKGROUND))
            .border_1()
            .border_color(rgb(theme::BORDER_STRONG))
            .overflow_hidden()
            .child(
                gpui::canvas(move |bounds, _, _| drawn.borrow_mut().menu = bounds, |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .children(rows)
            .children(note);
        (list, above)
    }

    /// The new-task box's content, for any host: the line, with the tags it understands in
    /// their colours, and the dropdown for the tag at the cursor. As wide as the host makes it.
    /// Files dropped on it go into the line.
    pub fn new_task_content(&self, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let menu = self.model.dropdown();
        let dropdown = menu.as_ref().map(|menu| self.render_dropdown(menu, cx));
        let (over, under) = match dropdown {
            Some((list, true)) => (Some(list), None),
            Some((list, false)) => (None, Some(list)),
            None => (None, None),
        };
        let muted = |text: &'static str| div().text_xs().text_color(rgb(theme::MUTED)).child(text);
        // What ↵ does when it is not "add": it finishes the tag. Else the one key a text box does not have.
        let keys: &[&'static str] = match self.model.enter() {
            Enter::Wait => &[],
            _ if self.model.enter_completes() => &["↵", "completes"],
            _ => &["⇧", "↵", "later"],
        };
        // As wide whatever it says: the tags beside it wrap the same, and the line above stays put.
        let caps = keys.split_last().map(|(word, caps)| {
            let caps = caps.iter().map(|text| keycaps::cap(*text, false, true));
            div().flex().items_center().gap_1().children(caps).child(muted(*word))
        });
        let keys = div().w(px(96.)).flex_none().flex().justify_end().children(caps);
        // The tags, a cap each: dimmer once something is typed.
        let typed = self.model.compose_line().is_some_and(|line| !line.is_empty());
        let hint = self.model.compose_hint().map(|_| self.tag_caps(cx).when(typed, |d| d.opacity(0.6)));
        // What the box could not do: a name with no room, files left out.
        let note =
            self.model.compose_note().map(|note| div().text_xs().text_color(rgb(theme::ERROR)).child(note.to_string()));
        let panel = self.field.clone();
        let attach = div()
            .id("compose-attach")
            .px_1p5()
            .rounded_sm()
            .text_xs()
            .text_color(rgb(theme::MUTED))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
            .child("+ file  ⌘O")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.pick_files(window, cx);
                    cx.stop_propagation();
                }),
            );
        // The folder taken from the dropdown: the agent runs there. × leaves the path in the line.
        let folder = self.model.compose_folder().map(|folder| {
            let clear = div()
                .id("compose-folder-clear")
                .px_1()
                .rounded_sm()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
                .child("×")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.dispatch(Action::ComposeFolderClear, this.input, window, cx);
                        cx.stop_propagation();
                    }),
                );
            div()
                .id("compose-folder")
                .h(px(20.))
                .max_w(px(220.))
                .overflow_hidden()
                .flex()
                .items_center()
                .gap_1()
                .pl_2()
                .rounded_md()
                .border_1()
                .border_color(rgb(theme::BORDER_STRONG))
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .child("runs in")
                .child(crate::cut_text(ottospace_core::transcript::clean(folder)).text_color(rgb(theme::TEXT)))
                .child(div().flex_none().child(clear))
        });
        let line = div()
            .min_h(px(34.))
            .px_2()
            .flex()
            .items_center()
            .rounded(px(dialog::INNER_RADIUS))
            .bg(rgb(theme::BACKGROUND))
            .border_1()
            .border_color(rgb(theme::BACKGROUND))
            .text_base()
            .child(self.render_field_of("compose", "What needs doing?", cx));
        div()
            .id("compose-content")
            .relative()
            .flex()
            .flex_col()
            .gap_2()
            // While files are dragged over it, the box says it will take them.
            .drag_over::<ExternalPaths>(|style, _, _, _| style.bg(rgb(theme::SELECTED)))
            .on_drop(cx.listener(|this, dropped: &ExternalPaths, window, cx| {
                this.compose_attach(dropped.paths(), this.input, window, cx);
            }))
            .child(
                div()
                    // As high with the folder as without: the line under it stays put.
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .justify_between()
                    // The draft's title says what it is; the drawer's header does that in its new-task state.
                    .child(div().text_xs().text_color(rgb(theme::COMPOSE)).children(
                        (self.model.compose_host() == Some(ottospace_core::model::Host::Bar)).then_some("NEW TASK"),
                    ))
                    .child(div().flex().items_center().gap_2().children(folder).child(attach)),
            )
            // Where the content is drawn: an ⌥-click over it references nothing.
            .child(
                gpui::canvas(move |bounds, _, _| panel.borrow_mut().set_panel(bounds), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .children(over)
            .child(line)
            .children(under)
            .children(note)
            // One row while the box is open: the tags, then the key. A narrow box wraps it, never clips.
            .child(div().min_h(px(16.)).flex().items_start().gap_3().children(hint).child(keys))
    }

    /// The drawer in its new-task state, `width` points wide: one-click picks for where and who,
    /// what ↵ will do when not the default, and the box at the bottom. With the box closed (nothing
    /// open yet, `Model::welcome`) a click on its line opens it.
    pub fn render_new_task(&self, width: f32, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let open = self.model.compose.is_some();
        let picks = self.model.compose_picks();
        let draft = self.model.compose_line().map(|line| self.model.draft(line));
        let repo = draft.as_ref().and_then(|draft| draft.repo.clone());
        let agent = draft.as_ref().and_then(|draft| draft.agent.clone());
        let pick = |mark: char, name: Option<String>, label: String, on: bool| {
            let id = format!("pick:{mark}{}", name.as_deref().unwrap_or(""));
            div()
                .id(gpui::SharedString::from(id.clone()))
                .relative()
                .px_2()
                .py_0p5()
                .rounded_md()
                .border_1()
                .border_color(rgb(if on { tag_color(mark) } else { theme::BORDER_STRONG }))
                .text_xs()
                .text_color(rgb(if on { theme::TEXT } else { theme::MUTED }))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .child(label)
                .child(self.controls.mark(id))
                // A pick fills the line: with the box closed it opens first.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| {
                    if this.model.compose.is_none() {
                        this.new_task(window, cx);
                    }
                    this.dispatch(Action::ComposePick { mark, name: name.clone() }, this.input, window, cx);
                }))
        };
        let wheres = picks
            .wheres
            .iter()
            .map(|name| pick('>', Some(name.clone()), format!(">{name}"), repo.as_ref() == Some(name)));
        let none = pick('>', None, "no repository".into(), open && repo.is_none());
        let whos = picks.whos.iter().map(|name| {
            let on = agent.as_ref().is_some_and(|agent| agent.eq_ignore_ascii_case(name));
            pick('@', Some(name.clone()), format!("@{name}"), on)
        });
        // Where it works, where there is a choice: this checkout, or a new worktree (⌥⌘N switches).
        let choice = self.model.compose_worktree().map(|choice| {
            let side = |new: bool, label: &'static str| {
                let id = format!("pick:worktree:{}", if new { "new" } else { "checkout" });
                let on = choice.new == new;
                div()
                    .id(gpui::SharedString::from(id.clone()))
                    .relative()
                    .px_2()
                    .py_0p5()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(if on { theme::COMPOSE } else { theme::BORDER_STRONG }))
                    .text_xs()
                    .text_color(rgb(if on { theme::TEXT } else { theme::MUTED }))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(theme::SELECTED)))
                    .child(label)
                    .child(self.controls.mark(id))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        drop(this.dispatch(Action::ComposeWorktree { new: Some(new) }, this.input, window, cx))
                    }))
            };
            let key = div().text_xs().text_color(rgb(theme::MUTED)).child("⌥⌘N");
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_1()
                .child(side(false, "this checkout"))
                .child(side(true, "new worktree"))
                .child(key)
        });
        let plan =
            self.model.compose_plan_line().map(|text| div().text_xs().text_color(rgb(theme::COMPOSE)).child(text));
        // The view moved on while the box was open: it still goes where it was opened; Switch brings it here.
        let pinned = self.model.compose_space().filter(|_| self.model.compose_elsewhere()).map(|of| {
            let switch = div()
                .id("compose-switch")
                .relative()
                .cursor_pointer()
                .text_color(rgb(theme::TEXT))
                .hover(|s| s.underline())
                .child(format!("Switch to {}", self.model.space_title(self.model.space())))
                .child(self.controls.mark("compose-switch"))
                .child(crate::mark(&self.marks, crate::Mark::ComposeSwitch))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(
                    cx.listener(|this, _, window, cx| drop(this.dispatch(Action::ComposeHere, this.input, window, cx))),
                );
            // From Home or a session's space ↵ makes a workspace: no name to give it yet.
            let name =
                if self.model.joins().is_some() { self.model.space_title(&of) } else { "a new workspace".into() };
            let goes = format!("This task goes to {name} · ");
            div().flex().flex_wrap().text_xs().text_color(rgb(theme::COMPOSE)).child(goes).child(switch)
        });
        let idle = (!open).then(|| {
            dialog::panel(theme::BORDER_STRONG)
                .id("drawer-new-task")
                .p_3()
                .cursor_pointer()
                .hover(|s| s.border_color(rgb(theme::COMPOSE)))
                .text_color(rgb(theme::MUTED))
                .child("What needs doing?")
                .child(self.controls.mark("drawer:new_task"))
                .on_click(cx.listener(|this, _, window, cx| this.new_task(window, cx)))
        });
        let close = div()
            .id("new-task-close")
            .relative()
            .px_1p5()
            .rounded_sm()
            .cursor_pointer()
            .text_color(rgb(theme::MUTED))
            .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
            .child("×")
            .child(self.controls.mark("drawer:close"))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(
                cx.listener(|this, _, window, cx| drop(this.dispatch(Action::CloseCompose, this.input, window, cx))),
            );
        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .child(div().flex().flex_wrap().gap_1().children(wheres).child(none))
            .child(div().flex().flex_wrap().gap_1().children(whos))
            .children(choice)
            .child(div().flex_1())
            .children(pinned)
            .children(plan)
            .children(open.then(|| self.new_task_content(cx)))
            .children(idle);
        // Its header is as high as the top row above it.
        let header = div()
            .h(px(crate::TOP_BAR))
            .flex_none()
            .px_3()
            .flex()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .child(div().text_xs().text_color(rgb(theme::COMPOSE)).child("NEW TASK"))
            .children(open.then_some(close));
        div()
            .id("new-task-drawer")
            .flex_none()
            .w(px(width))
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::RAISED))
            .border_l_1()
            .border_color(rgb(theme::BORDER))
            .child(header)
            .child(body)
    }
}
