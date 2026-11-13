//! The accessibility inspector, drawn: a panel on the right with an app pane's tree, and an
//! outline of the selected node over the pane. Read-only: it reads the tree (off the UI thread,
//! within `inspect::LIMITS`) and never presses, sets or switches anything in the app.

use std::cell::Cell;
use std::time::{Duration, Instant};

use gpui::{Context, Div, IntoElement, KeyDownEvent, MouseButton, Window, div, px, rgb};
use ottospace_core::ax::Frame;
use ottospace_core::inspect::{self, Row};
use ottospace_core::{Action, Kind, Origin, PaneId, theme};

use crate::*;

thread_local! {
    /// A read is on its way; when the last one ended, and how long it took.
    static READING: Cell<bool> = const { Cell::new(false) };
    static LAST: Cell<Option<(Instant, f64)>> = const { Cell::new(None) };
}

/// Read again while the panel shows, this often at least; never more than a quarter of the time.
const REFRESH: Duration = Duration::from_secs(2);

/// What the panel says of an app that exposes almost nothing.
pub fn sparse_note(nodes: usize) -> String {
    format!(
        "This app exposes {nodes} elements. Electron/Chromium apps build their full tree only when an assistive app asks; ottospace does not switch that on."
    )
}

impl Workspace {
    /// Opens the inspector on the focused app pane, or closes it.
    pub fn toggle_inspector(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.model.inspector.is_some() {
            self.dispatch(Action::InspectorClose, self.input, window, cx);
            return;
        }
        let pane = self.model.focused.and_then(|id| self.model.pane(id));
        let Some((pane, Kind::App { name, .. })) = pane.map(|p| (p.id, &p.kind)) else {
            self.error = Some("the accessibility inspector needs a focused app pane".into());
            cx.notify();
            return;
        };
        let app = name.clone();
        self.dispatch(Action::InspectorOpen { pane, app }, self.input, window, cx);
        self.read_inspected(window, cx);
    }

    /// Reads the inspected pane's window again, off the UI thread, unless a read is on its way.
    /// A pane with no window behind it (a test's fixture) is not read.
    pub fn read_inspected(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(pane) = self.model.inspector.as_ref().map(|i| i.pane) else { return };
        let Some((pid, id)) = self.hosted.get(&pane).map(|h| (h.pid, h.window_id)) else { return };
        if READING.get() {
            return;
        }
        READING.set(true);
        cx.spawn_in(window, async move |this, cx| {
            let read =
                cx.background_executor().spawn(async move { ax_debug::read_with(pid, id, inspect::LIMITS) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                READING.set(false);
                let action = match read {
                    Ok((frame, tree, millis)) => {
                        LAST.set(Some((Instant::now(), millis)));
                        Action::InspectorRead { pane, tree, window: frame, millis }
                    }
                    Err(why) => {
                        LAST.set(Some((Instant::now(), 0.)));
                        Action::InspectorFailed { pane, why }
                    }
                };
                this.dispatch(action, Origin::System, window, cx);
            });
        })
        .detach();
    }

    /// From the tick: read again once `REFRESH` has passed, or four times the last read's time.
    pub fn refresh_inspector(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.model.inspector.is_none() {
            return;
        }
        let due = LAST
            .get()
            .is_none_or(|(at, millis)| at.elapsed() >= REFRESH.max(Duration::from_secs_f64(millis * 4. / 1000.)));
        if due {
            self.read_inspected(window, cx);
        }
    }

    /// While the filter has the keyboard: it types, ↑↓ select, ↵ or Esc gives the keyboard back. ⌘ shortcuts the field does not use reach their bindings.
    pub fn inspector_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let action = match event.keystroke.key.as_str() {
            "escape" => Action::InspectorFilter { on: false },
            "down" => Action::InspectorMove { delta: 1 },
            "up" => Action::InspectorMove { delta: -1 },
            "enter" => Action::InspectorFilter { on: false },
            _ => {
                if self.field_key(event, window, cx) {
                    cx.stop_propagation();
                }
                return;
            }
        };
        self.dispatch(action, self.input, window, cx);
        self.scroll_to_selected();
        cx.stop_propagation();
    }

    fn scroll_to_selected(&self) {
        let Some(inspector) = &self.model.inspector else { return };
        let at = inspector.selected.as_ref().and_then(|path| inspector.rows().iter().position(|r| r.path == *path));
        if let Some(at) = at {
            self.bars.scroll_to_item("inspector", at);
        }
    }

    /// The pane's content area and the title bar cropped off its stream, as the overlay uses them.
    fn inspected_area(&self, pane: PaneId) -> Option<(f64, Frame)> {
        match self.hosted.get(&pane) {
            Some(h) => Some((h.crop, sneak::frame(h.bounds.get()))),
            None => Some((0., sneak::frame(self.cells.get(&pane)?.get()))),
        }
    }

    /// Where the selected node is outlined, in window points: only over a drawn, parked pane (a
    /// live window covers ottospace).
    fn inspector_outline(&self) -> Option<Frame> {
        let inspector = self.model.inspector.as_ref()?;
        let drawn = self.model.layout.as_ref().is_some_and(|l| l.leaves().contains(&inspector.pane));
        if !drawn || self.model.live == Some(inspector.pane) {
            return None;
        }
        let (crop, content) = self.inspected_area(inspector.pane)?;
        inspector.outline(crop, content)
    }

    pub fn render_inspector_outline(&self) -> Option<Div> {
        let f = self.inspector_outline()?;
        let outline = div()
            .absolute()
            .left(px(f.x as f32))
            .top(px(f.y as f32))
            .w(px(f.w as f32))
            .h(px(f.h as f32))
            .border_2()
            .border_color(rgb(theme::SNEAK));
        Some(outline)
    }

    /// For the state snapshot: counts and roles, never the app's text. While a filter is typed
    /// the rows are left out: which rows match would tell what text the app holds.
    pub fn inspector_snapshot(&self) -> Option<serde_json::Value> {
        let inspector = self.model.inspector.as_ref()?;
        let rows = Some(inspector.rows()).filter(|_| inspector.filter.text().is_empty());
        let selected = |rows: &Vec<Row>| {
            let path = inspector.selected.as_ref()?;
            rows.iter().position(|r| r.path == *path)
        };
        Some(serde_json::json!({
            "pane": inspector.pane,
            "counts": inspector.counts(),
            "millis": inspector.read.as_ref().map(|read| read.millis),
            "rows": rows.as_ref().map(|rows| rows.iter().take(inspect::MAX_ROWS).collect::<Vec<_>>()),
            "row_count": rows.as_ref().map(Vec::len),
            "selected": rows.as_ref().and_then(selected),
            "outline": rows.as_ref().and_then(|_| self.inspector_outline()),
        }))
    }

    /// The panel on the right: the header and counts, the filter, the rows, the selected node.
    pub fn render_inspector(&self, width: f32, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let inspector = self.model.inspector.as_ref()?;
        let counts = inspector.counts();
        let rows = inspector.rows();
        let selected = inspector.selected.clone();
        let small = |text: String| div().text_xs().text_color(rgb(theme::MUTED)).child(text);
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .relative()
                .px_1p5()
                .rounded_sm()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .child(label)
        };
        let header =
            div()
                .px_3()
                .pt_2()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .child(format!("{}: {} nodes, {} pressable", inspector.app, counts.nodes, counts.pressable)),
                )
                .child(
                    button("inspector-refresh", "Refresh")
                        .text_xs()
                        .child(self.controls.mark("inspector:refresh"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.click_inspector("inspector", "refresh", window, cx)),
                        ),
                )
                .child(button("inspector-close", "×").child(self.controls.mark("inspector:close")).on_click(
                    cx.listener(|this, _, window, cx| this.click_inspector("inspector", "close", window, cx)),
                ));
        let t = counts.truncated;
        let cut: Vec<&str> = [(t.depth, "depth"), (t.nodes, "node count"), (t.timeout, "time")]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, why)| *why)
            .collect();
        let read = match (&inspector.read, &inspector.error) {
            (_, Some(err)) => format!("could not read: {err}"),
            (None, None) => "reading…".into(),
            (Some(read), None) if cut.is_empty() => {
                format!("read in {:.0} ms · {} settable", read.millis, counts.settable)
            }
            (Some(read), None) => format!("read in {:.0} ms · stopped at the {} limit", read.millis, cut.join(", ")),
        };
        let roles = counts.roles.iter().take(8).map(|(role, n)| {
            div()
                .px_1p5()
                .rounded_sm()
                .bg(rgb(theme::SELECTED))
                .text_xs()
                .child(format!("{} {n}", inspect::short_role(role)))
        });
        let filter = div()
            .id("inspector-filter")
            .relative()
            .mx_3()
            .px_2()
            .h(px(26.))
            .flex()
            .items_center()
            .rounded_md()
            .border_1()
            .border_color(rgb(if inspector.filtering { theme::INSPECT } else { theme::BORDER }))
            .text_xs()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.click_inspector("inspector", "filter", window, cx)),
            )
            .child(self.controls.mark("inspector:filter"))
            .child(self.render_field_of("inspector", "filter by role or text", cx));
        let more = rows.len().saturating_sub(inspect::MAX_ROWS);
        let list = rows.into_iter().take(inspect::MAX_ROWS).enumerate().map(|(i, row)| {
            let on = selected.as_ref() == Some(&row.path);
            self.inspector_row(i, row, on, cx)
        });
        let list =
            div().id("inspector-rows").flex_1().min_h_0().overflow_y_scroll().children(list).when(more > 0, |d| {
                d.child(small(format!("{more} more rows: type in the filter to narrow")).px_3().py_1())
            });
        let detail = inspector.detail().map(|d| {
            let frame = d.frame.map_or("no frame".into(), |f| format!("{:.0},{:.0} {:.0}×{:.0}", f.x, f.y, f.w, f.h));
            let actions = if d.actions.is_empty() { "none".into() } else { d.actions.join(", ") };
            div()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(rgb(theme::BORDER))
                .flex()
                .flex_col()
                .gap_0p5()
                .text_xs()
                .child(format!("{}{}", d.role, d.subrole.map(|s| format!(" · {s}")).unwrap_or_default()))
                .child(small(format!("id {}", d.identifier.unwrap_or_else(|| "none".into()))))
                .child(small(format!("frame {frame}")))
                .child(small(format!("actions {actions}")))
        });
        let panel = div()
            .id("inspector")
            .w(px(width))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .gap_1()
            .bg(rgb(theme::RAISED))
            .border_l_1()
            .border_color(rgb(theme::BORDER))
            .child(header)
            .child(small(read).px_3())
            .when(counts.sparse, |d| d.child(small(sparse_note(counts.nodes)).px_3().text_color(rgb(theme::SNEAK))))
            .child(div().px_3().flex().flex_wrap().gap_1().children(roles))
            .child(filter)
            .child(self.bars.area("inspector", list))
            .children(detail);
        Some(panel.into_any_element())
    }

    fn inspector_row(&self, i: usize, row: Row, on: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let flag = |text: &'static str| div().px_1().rounded_sm().bg(rgb(theme::BORDER_STRONG)).child(text);
        let fold = if row.children == 0 {
            " "
        } else if row.folded {
            "▸"
        } else {
            "▾"
        };
        div()
            .id(("inspector-row", i))
            .relative()
            .h(px(22.))
            .pl(px(12. + 12. * row.depth.min(20) as f32))
            .pr_3()
            .flex()
            .items_center()
            .gap_1p5()
            .text_xs()
            .cursor_pointer()
            .when(on, |d| d.bg(rgb(theme::SELECTED)))
            .hover(|s| s.bg(rgb(theme::SELECTED)))
            .child(self.controls.mark(format!("inspector-row:{i}")))
            .child(
                div()
                    .id(("inspector-fold", i))
                    .relative()
                    .w(px(10.))
                    .child(fold)
                    .when(row.children > 0, |d| d.child(self.controls.mark(format!("inspector-fold:{i}"))))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.click_inspector("inspector-fold", &i.to_string(), window, cx);
                    })),
            )
            .child(div().flex_none().text_color(rgb(theme::PRIMARY)).child(row.role))
            .child(div().flex_1().min_w_0().truncate().text_color(rgb(theme::MUTED)).child(row.text))
            .when(row.pressable, |d| d.child(flag("press")))
            .when(row.settable, |d| d.child(flag("set")))
            .when(row.focused, |d| d.child(flag("focus")))
            .on_click(
                cx.listener(move |this, _, window, cx| {
                    this.click_inspector("inspector-row", &i.to_string(), window, cx)
                }),
            )
    }

    /// A click on one of the panel's controls, or its sneak label: `inspector:refresh`,
    /// `inspector:close`, `inspector:filter`, `inspector-row:3` (select) or `inspector-fold:3`.
    pub fn click_inspector(&mut self, kind: &str, name: &str, window: &Window, cx: &mut Context<Self>) {
        let Some(inspector) = &self.model.inspector else { return };
        let row = |at: &str| inspector.rows().into_iter().nth(at.parse().ok()?).map(|row| row.path);
        let action = match (kind, name) {
            // A read on demand is the user's: a client's label does not hurry the app.
            ("inspector", "refresh") if self.input != Origin::User => return,
            ("inspector", "refresh") => {
                LAST.set(None);
                return self.read_inspected(window, cx);
            }
            ("inspector", "close") => Action::InspectorClose,
            ("inspector", "filter") if !inspector.filtering => Action::InspectorFilter { on: true },
            ("inspector-row", at) => match row(at) {
                Some(path) => Action::InspectorSelect { path: Some(path) },
                None => return,
            },
            ("inspector-fold", at) => match row(at) {
                Some(path) => Action::InspectorFold { path },
                None => return,
            },
            _ => return,
        };
        self.dispatch(action, self.input, window, cx);
    }
}
