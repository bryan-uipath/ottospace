//! Sneak mode in the shell: where ottospace's own controls were drawn, the label pills over them,
//! and what a typed label does. Labels, keys and leaving are the core's (`ottospace_core::sneak`).
//!
//! Only ottospace's own controls. Nothing here reads or presses a hosted app's accessibility
//! nodes: see `docs/ai/1-initial-brainstorm/accessibility.md`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::time::Duration;

use gpui::{
    AnyElement, Bounds, BoxShadow, Context, FontWeight, Hsla, IntoElement, KeyDownEvent, Pixels, Window, black, canvas,
    div, point, prelude::*, px, rgb,
};
use ottospace_core::ax::{Frame, Source, Target};
use ottospace_core::sneak::{Sneak, SneakKey, sneak_key};
use ottospace_core::{Action, Axis, theme};

use crate::{SECTIONS, SPLITS, Workspace, control};

/// The top-bar buttons' names in a control's id, in the order of `SPLITS`. e.g. `top:new_tab`.
pub const TOP: [&str; 3] = ["new_tab", "split_right", "split_down"];

#[derive(Default)]
struct Painted {
    /// Counts frames, so a caller can wait for the next one.
    frame: u64,
    /// Each control's id and the part of it that shows, in window points.
    controls: BTreeMap<String, Bounds<Pixels>>,
}

/// ottospace's own controls as the last frame drew them: what sneak mode puts labels on.
#[derive(Clone, Default)]
pub struct Controls(Rc<RefCell<Painted>>);

impl Controls {
    /// A new frame: a control that is not drawn again is gone.
    pub fn begin(&self) {
        let mut painted = self.0.borrow_mut();
        painted.frame += 1;
        painted.controls.clear();
    }

    /// An invisible layer over its parent that notes where the parent is. Goes beside a control's
    /// click handler; `Workspace::activate` must know the id. e.g. `rail:apps`, `tab:2`, `pane:1`.
    pub fn mark(&self, id: impl Into<String>) -> AnyElement {
        let (painted, id) = (self.0.clone(), id.into());
        let note = move |bounds: Bounds<Pixels>, window: &mut Window, _: &mut gpui::App| {
            // Only the part that shows: a row scrolled out of its list gets no label.
            let shown = bounds.intersect(&window.content_mask().bounds);
            if shown.size.width > px(1.) && shown.size.height > px(1.) {
                painted.borrow_mut().controls.insert(id, shown);
            }
        };
        canvas(note, |_, _, _, _| {}).absolute().top_0().left_0().size_full().into_any_element()
    }

    fn frame(&self) -> u64 {
        self.0.borrow().frame
    }

    fn targets(&self) -> Vec<Target> {
        let frame = |b: &Bounds<Pixels>| Frame {
            x: f32::from(b.origin.x).into(),
            y: f32::from(b.origin.y).into(),
            w: f32::from(b.size.width).into(),
            h: f32::from(b.size.height).into(),
        };
        let target = |(id, bounds): (&String, &Bounds<Pixels>)| Target {
            frame: frame(bounds),
            source: Source::Own { id: id.clone() },
        };
        self.0.borrow().controls.iter().map(target).collect()
    }
}

/// The label pills: typed letters dimmed, labels the letters rule out not drawn.
pub fn labels(sneak: &Sneak) -> impl IntoElement + use<> {
    let dim = Hsla::from(rgb(theme::BACKGROUND)).opacity(0.35);
    let pills = sneak.visible().map(|t| {
        let f = t.target.frame;
        let (typed, rest) = t.label.split_at(sneak.typed.len().min(t.label.len()));
        let pill = div()
            .flex()
            .flex_none()
            .px(px(3.))
            .rounded_sm()
            .bg(rgb(theme::SNEAK))
            .text_color(rgb(theme::BACKGROUND))
            .text_xs()
            .line_height(px(15.))
            .font_weight(FontWeight::BOLD)
            // The raised-panel shadow, scaled down for a chip.
            .shadow(vec![BoxShadow {
                color: black().opacity(0.7),
                offset: point(px(0.), px(2.)),
                blur_radius: px(6.),
                spread_radius: px(0.),
            }])
            .child(div().text_color(dim).child(typed.to_uppercase()))
            .child(rest.to_uppercase());
        // A pane's label sits in its middle, a small control's on it, a row's or bar's at its left.
        let body = matches!(&t.target.source, Source::Own { id } if id.starts_with("pane:"));
        div()
            .absolute()
            .left(px(f.x as f32))
            .top(px(f.y as f32))
            .w(px(f.w as f32))
            .h(px(f.h as f32))
            .flex()
            .items_center()
            .when(body || f.w <= 40., |d| d.justify_center())
            .when(!body && f.w > 40., |d| d.pl(px(2.)))
            .child(pill)
    });
    div().absolute().top_0().left_0().size_full().children(pills)
}

impl Workspace {
    /// ⌘⌃G: labels on the controls of the last painted frame, or out of the mode if it is on.
    pub(crate) fn toggle_sneak(&mut self, window: &Window, cx: &mut Context<Self>) {
        let targets = self.controls.targets();
        self.dispatch(Action::Sneak { targets }, window, cx);
    }

    /// The `sneak` request: enters the mode (afresh, if it is on) and replies with the state.
    pub(crate) fn serve_sneak(&mut self, reply: Sender<String>, window: &Window, cx: &mut Context<Self>) {
        // Labels come from a painted frame, and the request before this one (a split, a section)
        // may not be painted yet: draw, and wait for that frame.
        self.dispatch(Action::LeaveSneak, window, cx);
        let seen = self.controls.frame();
        cx.spawn_in(window, async move |this, cx| {
            // Half a second at most: a window nobody can see is not painted.
            for _ in 0..60 {
                if this.update(cx, |this, _| this.controls.frame()).is_ok_and(|frame| frame != seen) {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(8)).await;
            }
            let answer = this.update_in(cx, |this, window, cx| {
                this.toggle_sneak(window, cx);
                match this.model.sneak {
                    Some(_) => this.snapshot(window),
                    None => {
                        control::error("sneak mode starts from normal or list mode: leave the app or close the box")
                    }
                }
            });
            let _ = reply.send(answer.unwrap_or_else(|_| control::error("ottospace is closing")));
        })
        .detach();
    }

    /// Sneak mode takes every key but ⌘ shortcuts: letters type a label, ⌫ takes one back, Esc leaves.
    pub(crate) fn sneak_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        if key.modifiers.platform {
            return; // ⌘ shortcuts still reach their bindings: each ends the mode and acts
        }
        // Nothing reaches the terminal under the labels.
        cx.stop_propagation();
        // ⌃A is not `a`. A held key types once.
        let plain = !key.modifiers.control && !key.modifiers.alt && !event.is_held;
        let action = match sneak_key(&key.key).filter(|_| plain) {
            Some(SneakKey::Type(letter)) => Action::SneakType { text: letter.to_string() },
            Some(SneakKey::Back) => Action::SneakBackspace,
            Some(SneakKey::Leave) => Action::LeaveSneak,
            None => return,
        };
        self.dispatch(action, window, cx);
    }

    /// Does what a click on the control `id` does: the same `Action` or method its click handler
    /// runs. No mouse event is made, and no other app's control is pressed.
    pub(crate) fn activate(&mut self, id: &str, window: &Window, cx: &mut Context<Self>) {
        let (kind, name) = id.split_once(':').unwrap_or((id, ""));
        let number = name.parse::<u32>().ok();
        // A tab is named by its id; the actions take its place among the tabs.
        let tab = number.and_then(|id| self.model.tabs.iter().position(|tab| tab.id == id));
        let action = match (kind, name) {
            ("rail", _) => {
                let section = SECTIONS.iter().find(|(_, _, label)| label.eq_ignore_ascii_case(name));
                section.map(|(section, ..)| Action::ToggleSection { section: *section })
            }
            ("bar", "close") => Some(Action::CloseSidebar),
            ("bar", "refresh") => {
                self.refresh_lists();
                cx.notify();
                None
            }
            ("app", _) => {
                let _ = self.open_app(name, Axis::Right, false, window, cx);
                None
            }
            ("repo", _) => Some(Action::OpenRepo { path: name.into(), axis: Axis::Right }),
            ("task" | "home-row" | "home-node", _) => number.map(|id| Action::TaskOpen { id: Some(id) }),
            ("home", "new_task") => {
                self.new_task(window, cx);
                None
            }
            ("tab", _) => tab.map(|index| Action::SelectTab { index }),
            ("tab-close", _) => tab.map(|index| Action::CloseTab { index: Some(index) }),
            ("top", "task_leave") => Some(Action::TaskLeave),
            ("top", _) => {
                if let Some((.., axis, new_tab)) = TOP.iter().position(|top| *top == name).and_then(|i| SPLITS.get(i)) {
                    self.open_picker(*axis, *new_tab, window, cx);
                }
                None
            }
            ("header", _) => number.map(|pane| Action::Focus { pane }),
            ("close", _) => number.map(|pane| Action::Close { pane: Some(pane) }),
            // A click on a pane focuses it. On an app pane it hands the app the keyboard; the
            // click it also replays into the app has no place here.
            ("pane", _) => number.map(|pane| match self.hosted.contains_key(&pane) {
                true => Action::Enter { pane },
                false => Action::Focus { pane },
            }),
            _ => None,
        };
        if let Some(action) = action {
            self.dispatch(action, window, cx);
        }
    }
}
