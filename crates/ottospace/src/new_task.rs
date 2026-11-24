//! The new-task box. `new_task_content` is what it holds: the line, the dropdown that finishes
//! the tag at the cursor, the hints. It knows nothing of where it is: no size, no position.
//! The drawer hosts it, in its new-task state (`render_new_task`), or in its collecting state
//! as the draft's title (`collect.rs`). Which one is the core's `compose_host`.

use std::path::PathBuf;

use gpui::{
    App, Bounds, Context, Div, ExternalPaths, FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, PathPromptOptions,
    Pixels, Point, Window, div, prelude::*, px, rgb,
};
use ottospace_core::complete::{self, Dropdown, Row};
use ottospace_core::model::{Pointed, TAG_KEYS, TerminalInfo, fit_status};
use ottospace_core::{Action, Origin, theme};

use crate::{Workspace, dialog, ui};

/// The dropdown's width, and a row's height: the shell knows its room from them.
const MENU_WIDTH: f32 = 340.;
const ROW: f32 = 26.;
/// A picker's list: wide enough for a model's id and its word of detail.
const PICKER_WIDTH: f32 = 280.;

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
    /// its agent; ⇧↵ or ⌥↵ starts a new line; ⌘⇧↵ adds it for later; ⌘↵ adds and starts it; ⌥⌘N switches between this checkout
    /// and a new worktree, where offered; Tab takes a row; ↑ ↓ (⌃P ⌃N) move over a list's rows while one shows, else a
    /// row of the text; ⌃G edits the line in the user's editor; Esc closes the dropdown,
    /// then the box; ⌘O picks files. The rest edit the line. ⌘ shortcuts the line does not use
    /// still reach their bindings; pane ones wait.
    pub fn compose_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        let m = key.modifiers;
        let plain = !m.shift && !m.alt && !m.platform;
        // A list shows rows (the tag's dropdown, a picker's): ↑ ↓ are its rows; else the text's.
        let rows = self.model.dropdown().is_some_and(|menu| !menu.rows.is_empty());
        let listed = rows || self.model.picker_menu().is_some();
        let action = match key.key.as_str() {
            "enter" if event.is_held => return,
            "enter" if m.platform && m.shift => Action::ComposeSubmit { open: false },
            "enter" if m.platform => Action::ComposeSubmit { open: true },
            // In the agent's list: the highlighted one becomes the default.
            "enter" if m.alt && self.model.picker_default().is_some() => Action::ComposeDefault,
            // In the bar's one-line title ⇧↵ files for later, as ⌘⇧↵ does.
            "enter" if m.shift && self.model.bar_draft().is_some() => Action::ComposeSubmit { open: false },
            // A line break, in the box.
            "enter" if (m.shift || m.alt) && !m.control && self.model.bar_draft().is_none() => {
                let edit = ottospace_core::edit::Edit::Replace { range: None, text: "\n".into() };
                Action::FieldEdit { edit }
            }
            "enter" => Action::ComposeEnter,
            "escape" => Action::ComposeEscape,
            "tab" if m.shift => Action::ComposePickerStep { delta: -1 },
            "tab" => Action::ComposeComplete,
            "up" if plain && !m.control && listed => Action::ComposeMove { delta: -1 },
            "down" if plain && !m.control && listed => Action::ComposeMove { delta: 1 },
            "p" if plain && m.control => Action::ComposeMove { delta: -1 },
            "n" if plain && m.control => Action::ComposeMove { delta: 1 },
            // The line in the user's editor.
            "g" if plain && m.control => Action::EditorOpen,
            "n" if m.platform && m.alt && !m.shift && !m.control => Action::ComposeWorktree { new: None },
            // ✎: mark up a pane for the task, and come back.
            "m" if m.platform && m.shift && !m.alt && !m.control => Action::ComposePen,
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
        self.tell_terminals();
        self.offer_installed(cx);
    }

    /// Each terminal's directory, its shell's and its program, now.
    pub fn tell_terminals(&mut self) {
        let info = |(pane, terminal): (&ottospace_core::PaneId, &crate::Terminal)| TerminalInfo {
            pane: *pane,
            cwd: terminal.cwd(),
            program: terminal.running().and_then(|work| work.program),
            shell: terminal.shell_cwd(),
        };
        let terminals = self.terminals.iter().map(info).collect();
        let home = std::env::var("HOME").ok().filter(|home| !home.is_empty());
        self.model.update(Action::SetTerminals { terminals, home }.sent_by(Origin::System));
    }

    /// Files or folders for the line: dropped on the box, picked, or pasted. Each goes in at
    /// the cursor as a path word.
    pub fn compose_attach(&mut self, paths: &[PathBuf], origin: Origin, window: &Window, cx: &mut Context<Self>) {
        // A name that is not text cannot be in the line: the core is told how many were left out.
        let (named, unreadable) = crate::dropped_names(paths);
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
        // Nor under its dropdown, which may hang past it.
        let listed = self.model.dropdown().is_some() || self.model.picker_menu().is_some();
        let menu = listed && self.field.borrow().menu.contains(&position);
        if menu || self.field.borrow().over_panel(position) {
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
            // A tag's row says what it is; a picker's needs not (its picker says it).
            .when(self.model.picker_menu().is_none(), |d| {
                d.child(div().w(px(46.)).flex_none().text_color(rgb(tag_color(menu.mark))).child(row.kind))
            })
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
                .child(ui::pill(key, ui::Pill::Key { lit: false, small: true }))
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

    /// The dropdown, hung under the line at its tag (over it, if the window has no room below),
    /// over what is under it: nothing in the box moves when it opens.
    fn render_dropdown(&self, menu: &Dropdown, cx: &mut Context<Self>) -> gpui::Deferred {
        // Above or below by the tallest it can be, not by its rows now: it never jumps as they change.
        let height = px(ROW * (complete::MAX_ROWS + 1) as f32 + 10.);
        let place = self.field_place(menu.start);
        // The line's left edge is the content's: the dropdown is pushed right to sit under its tag.
        let indent = place.map_or(px(0.), |(x, field, _)| (x - field.left()).min(field.size.width - px(MENU_WIDTH)));
        let above = place.is_some_and(|(_, field, viewport)| field.bottom() + height + px(90.) > viewport);
        let list = self.menu_list(menu, cx).left(indent.max(px(0.)));
        let list = if above { list.bottom_full().mb_1() } else { list.top_full().mt_1() };
        gpui::deferred(list)
    }

    /// A list of rows that floats over the box, `MENU_WIDTH` wide at most: the tag's dropdown, or
    /// a picker's. The caller places it. Where it is drawn is kept, so a click on it is the box's.
    fn menu_list(&self, menu: &Dropdown, cx: &mut Context<Self>) -> Div {
        // At most `MAX_ROWS`, as many as the dropdown's, around the highlighted one: a long list of
        // projects stays in the window, and ↑ ↓ bring the rest into view.
        let most = complete::MAX_ROWS;
        let from = menu.selected.unwrap_or(0).saturating_sub(most - 1).min(menu.rows.len().saturating_sub(most));
        let shown = menu.rows.iter().enumerate().skip(from).take(most);
        let rows = shown.map(|(i, row)| self.dropdown_row(i, row, menu, cx).into_any_element());
        let more = menu.rows.len().saturating_sub(most);
        let note = menu.note.clone().or_else(|| (more > 0).then(|| format!("{more} more: ↑ ↓"))).map(|note| {
            div().h(px(ROW)).px_2().flex().items_center().text_xs().text_color(rgb(theme::MUTED)).child(note)
        });
        let drawn = self.field.clone();
        div()
            .absolute()
            .w(px(MENU_WIDTH))
            .max_w_full()
            .py_1()
            .flex()
            .flex_col()
            .rounded(px(dialog::INNER_RADIUS))
            .bg(rgb(theme::BACKGROUND))
            .border_1()
            .border_color(rgb(theme::BORDER_STRONG))
            .shadow(dialog::shadow())
            .overflow_hidden()
            .occlude()
            .child(
                gpui::canvas(move |bounds, _, _| drawn.borrow_mut().menu = bounds, |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .children(rows)
            .children(note)
    }

    /// What the box will point at, inside its field: the quick note's pills (a meeting, a
    /// transcript, what is in front) and the folder its agent runs in, each with × to leave it out.
    /// A meeting's title is drawn from the calendar here, never kept in the state.
    fn attachments(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        use ottospace_core::model::{Owner, quick::Pill};
        let off = |id: String, mark: Option<String>, action: Action| self.pill_off(id, mark, action, cx);
        let label = |pill: &Pill, at: u64| match pill {
            Pill::Meeting(meeting) => format!("Meeting · {}", meeting.title),
            Pill::Transcript(_) => {
                format!("Transcript · {}", ottospace_core::calendar::clock(at, crate::calendar::zone_offset(at)))
            }
            Pill::Front { of: of @ Owner::Task { .. } } => format!("Task · {}", self.model.space_title(of)),
            Pill::Front { of } => format!("Workspace · {}", self.model.space_title(of)),
        };
        let pills = self.model.quick_pills().zip(self.model.quick_at()).into_iter().flat_map(|(pills, at)| {
            pills.iter().map(move |pill| {
                let kind = pill.kind();
                let name = serde_json::json!(kind).as_str().unwrap_or_default().to_string();
                // A meeting's title is another's words: in its own colour, as data.
                let accent = matches!(pill, Pill::Meeting(_)).then_some(theme::VISUAL);
                let x = off(format!("pill-off:{name}"), Some(format!("pill:{name}")), Action::QuickPill { kind });
                ui::chip(label(pill, at), accent).child(x).into_any_element()
            })
        });
        let pills: Vec<_> = pills.collect();
        // The folder taken from the dropdown: the agent runs there. × leaves the path in the line.
        let folder = self.model.compose_folder().map(|folder| {
            let text = format!("runs in {}", ottospace_core::transcript::clean(folder));
            let x = off("compose-folder-clear".into(), None, Action::ComposeFolderClear);
            ui::chip(text, None).id("compose-folder").child(x).into_any_element()
        });
        pills.into_iter().chain(folder).chain(self.snapshots(cx)).collect()
    }

    /// The box's ✎ captures, a pill each: the part marked, cropped from its picture, where it is
    /// from and how many marks; × drops it.
    fn snapshots(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let Some(draft) = self.model.capture.as_ref().filter(|draft| draft.boxed) else { return Vec::new() };
        let pill = |material: &ottospace_core::capture::Material| {
            let n = material.n;
            let marks = material.marks.shapes.len();
            let label =
                format!("Snapshot · {} · {marks} mark{}", material.source.label(), if marks == 1 { "" } else { "s" });
            let off = self.pill_off(
                format!("snapshot-off:{n}"),
                Some(format!("snapshot:{n}")),
                Action::CaptureRemove { material: n },
                cx,
            );
            let crop = ottospace_core::capture::crop(&material.marks.shapes).unwrap_or([0., 0., 1., 1.]);
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(self.crop_thumb(n, crop))
                .child(ui::chip(label, None).child(off))
                .into_any_element()
        };
        draft.materials.iter().map(pill).collect()
    }

    /// The part `crop` (`[x, y, w, h]`, 0..1) of capture `n`'s picture, 44 by 28 points: the
    /// picture scaled so the part fills it.
    fn crop_thumb(&self, n: u32, crop: [f64; 4]) -> Div {
        use crate::collect::{Frozen, Shot};
        let (w, h) = (44., 28.);
        let picture = match self.shots.get(&n) {
            Some(Shot { frozen: Frozen::App { buffer, .. }, .. }) => {
                Some(gpui::surface(buffer.clone()).object_fit(gpui::ObjectFit::Fill).size_full().into_any_element())
            }
            Some(Shot { png: Some(png), .. }) => {
                Some(gpui::img(png.clone()).object_fit(gpui::ObjectFit::Fill).size_full().into_any_element())
            }
            _ => None,
        };
        let [x, y, cw, ch] = crop.map(|v| v as f32);
        let (sw, sh) = (w / cw.max(0.01), h / ch.max(0.01));
        let scaled = div().absolute().left(px(-x * sw)).top(px(-y * sh)).w(px(sw)).h(px(sh)).children(picture);
        div()
            .relative()
            .flex_none()
            .w(px(w))
            .h(px(h))
            .overflow_hidden()
            .rounded(px(theme::RADIUS_SM))
            .border_1()
            .border_color(rgb(theme::BORDER_STRONG))
            .bg(rgb(theme::SELECTED))
            .child(scaled)
    }

    /// A picker in the box's edge: its sigil and what ↵ will use, ▾; a click opens its list under
    /// it. Its list, while open.
    fn picker(&self, mark: char, label: String, cx: &mut Context<Self>) -> gpui::AnyElement {
        let menu = self.model.picker_menu().filter(|menu| menu.mark == mark);
        let open = menu.is_some();
        let id = format!("picker:{mark}");
        // Its own width, not the picker's; the agent's, at the edge's right, opens leftward.
        let list = menu.map(|menu| {
            let list = self.menu_list(&menu, cx).w(px(PICKER_WIDTH)).max_w(px(PICKER_WIDTH)).top_full().mt_1();
            gpui::deferred(if mark == '@' { list.right_0() } else { list.left_0() })
        });
        ui::picker(gpui::SharedString::from(id.clone()), label, tag_color(mark), open)
            .child(self.controls.mark(id))
            // On the press, and no further: the line keeps the keyboard.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::ComposePicker { mark: Some(mark) }, this.input, window, cx);
                    cx.stop_propagation();
                }),
            )
            .children(list)
            .into_any_element()
    }

    /// A pill's ×: `action` on a click; `mark`, where it is drawn (`compose_rects`, sneak).
    fn pill_off(&self, id: String, mark: Option<String>, action: Action, cx: &Context<Self>) -> gpui::Stateful<Div> {
        div()
            .id(gpui::SharedString::from(id))
            .relative()
            .px_1()
            .rounded_sm()
            .cursor_pointer()
            .text_color(rgb(theme::MUTED))
            .hover(|s| s.bg(rgb(theme::BORDER_STRONG)).text_color(rgb(theme::TEXT)))
            .child("×")
            .children(mark.map(|mark| self.controls.mark(mark)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(
                cx.listener(move |this, _, window, cx| drop(this.dispatch(action.clone(), this.input, window, cx))),
            )
    }

    /// The project and repository pickers, and where there is a choice, this checkout or a new
    /// worktree (⌥⌘N).
    fn pickers(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        // The quick note: its project's alone, × leaves it untagged.
        if self.model.quick_pills().is_some() {
            let project = self.model.quick_project();
            let off = project.as_ref().map(|_| {
                let untag = Action::ComposePick { mark: '#', name: None };
                self.pill_off("quick-project-off".into(), Some("quick-project-off".into()), untag, cx)
            });
            let label = project.map_or("#no project".into(), |name| format!("#{name}"));
            let pill =
                div().flex().flex_shrink().min_w_0().items_center().child(self.picker('#', label, cx)).children(off);
            return vec![pill.into_any_element()];
        }
        let Some(choices) = self.model.compose_choices() else { return Vec::new() };
        let project = choices.project.map_or("#no project".into(), |name| format!("#{name}"));
        let place = choices.place.map_or(">no repository".into(), |place| format!(">{place}"));
        let worktree = self.model.compose_worktree().map(|choice| {
            let label = if choice.new { "new worktree" } else { "this checkout" };
            ui::picker("pick:worktree", label.to_string(), theme::COMPOSE, false)
                .child(self.controls.mark("pick:worktree"))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.dispatch(Action::ComposeWorktree { new: None }, this.input, window, cx);
                        cx.stop_propagation();
                    }),
                )
                .into_any_element()
        });
        [self.picker('#', project, cx), self.picker('>', place, cx)].into_iter().chain(worktree).collect()
    }

    /// The agent's picker, at the edge's right: the default shows from the start.
    fn agent_picker(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let choices = self.model.compose_choices()?;
        Some(self.picker('@', format!("@{}", choices.agent), cx))
    }

    /// The box's one primary button: ↵ as a click. Start for a task; File for the quick note.
    fn compose_button(&self, quick: bool, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let label = if quick { "File ↵" } else { "Start ↵" };
        ui::button("compose-start", ui::Button::Primary(label.into()), ui::State::Rest)
            .flex_none()
            .whitespace_nowrap()
            .child(self.controls.mark("compose-start"))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    // Start starts: an open picker's list closes first, its row not taken.
                    if this.model.picker_menu().is_some() {
                        this.dispatch(Action::ComposePicker { mark: None }, this.input, window, cx);
                    }
                    this.dispatch(Action::ComposeEnter, this.input, window, cx);
                    cx.stop_propagation();
                }),
            )
    }

    /// The new-task box's content, for any host, top to bottom: the field (the line, with the tags
    /// it understands in their colours; what it will point at; + to attach files), the one line
    /// that says what ↵ does, and the tags' sheet when asked for. The dropdown for the tag at the
    /// cursor hangs over what is below. As wide as the host makes it. Files dropped on it go in.
    pub fn new_task_content(&self, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let menu = self.model.dropdown().map(|menu| self.render_dropdown(&menu, cx));
        // What the box could not do: a name with no room, files left out.
        let note =
            self.model.compose_note().map(|note| div().text_xs().text_color(rgb(theme::ERROR)).child(note.to_string()));
        let panel = self.field.clone();
        let tip = |text: &'static str| {
            move |_: &mut Window, cx: &mut gpui::App| gpui::AnyView::from(cx.new(|_| crate::Tooltip(text.into())))
        };
        let attach = ui::button("compose-attach", ui::Button::Ghost(ui::Face::Icon("icons/plus.svg")), ui::State::Rest)
            .child(self.controls.mark("compose-attach"))
            .tooltip(tip("Attach files · ⌘O"))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.pick_files(window, cx);
                    cx.stop_propagation();
                }),
            );
        // Not over a draft in the bar: that one is the capture key's.
        let pen = self.model.bar_draft().is_none().then(|| {
            ui::button("compose-pen", ui::Button::Ghost(ui::Face::Icon("icons/pen.svg")), ui::State::Rest)
                .child(self.controls.mark("compose-pen"))
                .tooltip(tip("Mark up a pane for it · ⌘⇧M"))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.dispatch(Action::ComposePen, this.input, window, cx);
                        cx.stop_propagation();
                    }),
                )
        });
        let quick = self.model.quick_pills().is_some();
        let line = div()
            .relative()
            .min_h(px(40.))
            .px_3()
            .flex()
            .items_center()
            .text_base()
            .child(self.render_field_of("compose", if quick { "Jot it down" } else { "What needs doing?" }, cx))
            .children(menu);
        let attached = self.attachments(cx);
        let attached =
            (!attached.is_empty()).then(|| div().px_2().pb_1().flex().flex_wrap().gap_1().children(attached));
        // One row, never two: +, the pickers (cut short with … when there is no room), the button.
        let edge = div()
            .id("compose-edge")
            .relative()
            .h(px(36.))
            .px_1()
            .flex()
            .items_center()
            .gap_1()
            .overflow_hidden()
            .border_t_1()
            .border_color(rgb(theme::BORDER))
            .child(attach)
            .children(pen)
            .children(self.pickers(cx))
            .child(div().flex_1())
            .children(self.agent_picker(cx))
            .child(self.compose_button(quick, cx))
            .child(self.controls.mark("compose-edge"));
        let field = ui::well(div().flex().flex_col()).child(line).children(attached).child(edge);
        let sheet_open = self.model.compose_hint().is_some();
        let tags = ui::button(
            "compose-sheet",
            ui::Button::Ghost(ui::Face::Label("? tags".into())),
            if sheet_open { ui::State::Pressed } else { ui::State::Rest },
        )
        .child(self.controls.mark("compose-sheet"))
        .tooltip(tip("Tags a line may hold · ⌘/"))
        // On the press, and no further: the line keeps the keyboard and its cursor.
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, window, cx| {
                this.dispatch(Action::ComposeSheet, this.input, window, cx);
                cx.stop_propagation();
            }),
        );
        // As it fit its room when last drawn; measured again as drawn, and drawn again if it changed.
        let full = self.model.compose_status().unwrap_or_default();
        let status_text =
            Some(self.field.borrow().status.clone()).filter(|(of, _)| *of == full).map_or(full.clone(), |(_, fit)| fit);
        let room = self.field.clone();
        let measure = move |bounds: Bounds<Pixels>, window: &mut Window, _: &mut App| {
            let width = f32::from(bounds.size.width);
            let fit = fit_status(&full, |text| crate::status_bar::width(window, text) <= width);
            let mut room = room.borrow_mut();
            if room.status != (full.clone(), fit.clone()) {
                room.status = (full, fit);
                window.refresh();
            }
        };
        // One line: what ↵ does with the line as it is, then the other keys.
        let status = div()
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .text_color(rgb(theme::MUTED))
            // One line, whatever it says: what is under it never moves. Too long, its trailing
            // keys go, then words of what ↵ does (the core says which), never part of a word.
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(gpui::canvas(measure, |_, _, _, _| {}).absolute().size_full())
                    .child(status_text),
            )
            .child(tags);
        let sheet = sheet_open.then(|| self.tag_caps(cx));
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
            // The draft's title says what it is; the drawer's header does that in its new-task state.
            .children(
                (self.model.compose_host() == Some(ottospace_core::model::Host::Bar))
                    .then(|| div().text_xs().text_color(rgb(theme::COMPOSE)).child("NEW TASK")),
            )
            // Where the content is drawn: an ⌥-click over it references nothing.
            .child(
                gpui::canvas(move |bounds, _, _| panel.borrow_mut().set_panel(bounds), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(field)
            .children(note)
            .child(status)
            .children(sheet)
    }

    /// The drawer in its new-task state, `width` points wide: one-click picks for where and who,
    /// what ↵ will do when not the default, and the box at the bottom. With the box closed (nothing
    /// open yet, `Model::welcome`) a click on its line opens it.
    pub fn render_new_task(&self, width: f32, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let open = self.model.compose.is_some();
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
            .child(self.new_task_body(cx).flex_1().min_h_0())
    }

    /// The box centred over the dimmed panes (`Model::compose_centred`), `width` points wide: the
    /// drawer's picks and lines, then the line. Its bounds are where ↵'s grow starts (`fly.rs`).
    pub(crate) fn render_new_task_centred(&self, width: f32, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        ui::lifted(div())
            .id("new-task-centred")
            .relative()
            .w(px(width))
            .occlude()
            .child(self.new_task_body(cx))
            .child(self.controls.mark("new-task-centred"))
            .child(crate::mark(&self.marks, crate::Mark::CentredBox))
    }

    /// The new-task state's box (or, closed, the line to open it), under the line that says where it goes.
    fn new_task_body(&self, cx: &mut Context<Self>) -> Div {
        let open = self.model.compose.is_some();
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
        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .children(pinned)
            .children(open.then(|| self.new_task_content(cx)))
            .children(idle)
    }
}
