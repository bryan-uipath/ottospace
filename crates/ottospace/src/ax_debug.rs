//! Accessibility debugging: an app pane's tree as JSON for the socket, and an overlay that
//! outlines each interactive node with the label sneak mode would give it. An experiment; see
//! `docs/ai/1-initial-brainstorm/accessibility.md`.

use std::sync::mpsc::Sender;
use std::time::Instant;

use gpui::{BackgroundExecutor, Bounds, BoxShadow, IntoElement, Pixels, black, div, point, prelude::*, px, rgb};
use ottospace_core::ax::{self, Frame, Labelled, Limits, Source, Tree, Truncated};
use ottospace_core::{PaneId, theme};
use ottospace_mac::{ax as mac_ax, ax_tree};
use serde::Serialize;

use crate::control;

/// An app pane's window, as plain data so it can cross threads.
#[derive(Clone, Copy)]
pub struct Hosted {
    pane: PaneId,
    pid: i32,
    /// Window-server id.
    window: u32,
    /// Title-bar height cropped off the parked stream (0: none).
    crop: f64,
    /// The pane content area, in ottospace window coordinates.
    content: Frame,
}

impl Hosted {
    pub fn new(pane: PaneId, pid: i32, window: u32, crop: f64, content: Bounds<Pixels>) -> Self {
        let (o, s) = (content.origin, content.size);
        let content = Frame {
            x: f32::from(o.x).into(),
            y: f32::from(o.y).into(),
            w: f32::from(s.width).into(),
            h: f32::from(s.height).into(),
        };
        Self { pane, pid, window, crop, content }
    }
}

/// A window's tree, and the window's own frame (global) read just before it.
struct Read {
    window: Frame,
    tree: Tree,
    millis: f64,
}

/// Blocking AX IPC to the app: run it off the UI thread.
fn read(pid: i32, id: u32, limits: Limits) -> Result<Read, String> {
    let start = Instant::now();
    let window = mac_ax::Window::find(pid, id).ok_or("the window is gone")?;
    let f = window.frame().ok_or("could not read the window frame")?;
    let tree = ax_tree::read(&window, limits, ax_tree::Fetch::Batched);
    let window = Frame { x: f.origin.x, y: f.origin.y, w: f.size.width, h: f.size.height };
    Ok(Read { window, tree, millis: start.elapsed().as_secs_f64() * 1000.0 })
}

/// Answers `ax_tree` from a background thread, so a slow app does not stall the UI.
pub fn serve_tree(
    hosted: Option<Hosted>,
    depth: Option<usize>,
    nodes: Option<usize>,
    reply: Sender<String>,
    executor: &BackgroundExecutor,
) {
    let Some(Hosted { pane, pid, window, .. }) = hosted else {
        let _ = reply.send(control::error("not an app pane"));
        return;
    };
    let defaults = Limits::default();
    let limits = Limits { depth: depth.unwrap_or(defaults.depth), nodes: nodes.unwrap_or(defaults.nodes) };
    executor
        .spawn(async move {
            let answer = match read(pid, window, limits) {
                Ok(r) => serde_json::json!({ "ok": true, "pane": pane, "window": r.window, "millis": r.millis, "tree": r.tree })
                    .to_string(),
                Err(err) => control::error(&err),
            };
            let _ = reply.send(answer);
        })
        .detach();
}

/// What the overlay draws. In the state snapshot while the overlay is on.
#[derive(Serialize)]
pub struct Overlay {
    pub pane: PaneId,
    /// In ottospace window coordinates, as sneak mode would label them.
    pub targets: Vec<Labelled>,
    pub count: usize,
    pub truncated: Truncated,
    pub millis: f64,
    pub error: Option<String>,
}

impl Overlay {
    /// Reads the window's tree and labels its interactive nodes. Blocking, like `read`.
    pub fn load(hosted: Hosted) -> Self {
        let Hosted { pane, pid, window, crop, content } = hosted;
        match read(pid, window, Limits::default()) {
            Ok(r) => Self {
                pane,
                // Only app nodes for now; ottospace's own controls would join the same list.
                targets: ax::assign(ax::app_targets(pane, &r.tree.root, r.window, crop, content), ax::ALPHABET),
                count: r.tree.count,
                truncated: r.tree.truncated,
                millis: r.millis,
                error: None,
            },
            Err(err) => Self {
                pane,
                targets: Vec::new(),
                count: 0,
                truncated: Truncated::default(),
                millis: 0.0,
                error: Some(err),
            },
        }
    }

    /// Outlines and label chips over the whole window, plus a line saying what was read. A parked
    /// window is sized to its pane and fills it, so its frames map 1:1 into the pane.
    pub fn render(&self) -> impl IntoElement {
        let boxes = self.targets.iter().map(|t| {
            let f = t.target.frame;
            // The role only where it fits: small neighbours (traffic lights) would overlap.
            let text = match &t.target.source {
                Source::App { role, .. } if f.w >= 64.0 => format!("{} {}", t.label, role.trim_start_matches("AX")),
                _ => t.label.clone(),
            };
            div()
                .absolute()
                .left(px(f.x as f32))
                .top(px(f.y as f32))
                .w(px(f.w as f32))
                .h(px(f.h as f32))
                .border_1()
                .border_color(rgb(theme::SNEAK))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .px_1()
                        .rounded_sm()
                        .whitespace_nowrap()
                        .bg(rgb(theme::SNEAK))
                        .text_color(rgb(theme::BACKGROUND))
                        .text_xs()
                        // The raised-panel shadow, scaled down for a chip.
                        .shadow(vec![BoxShadow {
                            color: black().opacity(0.7),
                            offset: point(px(0.), px(2.)),
                            blur_radius: px(6.),
                            spread_radius: px(0.),
                        }])
                        .child(text),
                )
        });
        let t = self.truncated;
        let cut = if t.depth || t.nodes || t.timeout { " (truncated)" } else { "" };
        let summary = match &self.error {
            Some(err) => format!("AX: {err}"),
            None => format!("AX: {} nodes, {} targets, {:.0} ms{cut}", self.count, self.targets.len(), self.millis),
        };
        div().absolute().top_0().left_0().size_full().children(boxes).child(
            div()
                .absolute()
                .bottom(px(28.))
                .right(px(4.))
                .px_2()
                .bg(rgb(theme::RAISED))
                .text_xs()
                .text_color(rgb(if self.error.is_some() { theme::ERROR } else { theme::MUTED }))
                .child(summary),
        )
    }
}
