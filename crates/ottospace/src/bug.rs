//! The bug reporter, the shell half: what `developer.ottospace_repo` names on disk, the frozen
//! picture of ottospace's own window, and the overlay the user marks it in. Whether the button
//! shows, the marks, what is blanked and the brief are the core's (`ottospace_core::bug`). See
//! `docs/ai/bug-reporter.md`.
//!
//! The picture is of this window alone, by its window id (ScreenCaptureKit, as `record.rs`):
//! never the screen, never another app's window. Only the user starts a report.

use std::sync::Arc;

use gpui::{
    AnyElement, Context, CursorStyle, Div, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, ObjectFit,
    SharedString, Stateful, Window, div, img, prelude::*, px, relative, rgb, rgba,
};
use ottospace_core::bug::{self as core_bug, Brief, Mark, RepoFacts, Report};
use ottospace_core::capture::Shape;
use ottospace_core::settings::{Settings, expand_home};
use ottospace_core::{Action, Kind, Origin, PaneId, theme};

use crate::{Workspace, collect, config, dialog, ns_view, rect, sneak, ui};

/// A comment's card, in points.
const CARD: f32 = 260.;

/// What the setting names: its folder (`~` expanded), whether it is there, and whether it is a
/// git checkout (`.git`, a folder or a worktree's file). `None`: not set.
pub(crate) fn repo_facts(settings: &Settings) -> Option<RepoFacts> {
    let named = settings.developer.ottospace_repo.as_deref()?;
    let path = expand_home(named, &config::user_home());
    let folder = std::path::Path::new(&path);
    let (exists, git) = (folder.is_dir(), folder.join(".git").exists());
    Some(RepoFacts { path, exists, git })
}

/// The window as frozen for report `word`: its pixels (RGBA), kept to write at Send, and the PNG drawn.
pub(crate) struct Frozen {
    word: String,
    size: (usize, usize),
    rgba: Vec<u8>,
    image: Arc<gpui::Image>,
}

/// The OS version, `macOS 26.1`, from `kern.osproductversion`. Empty: it could not be read.
fn os_version() -> String {
    let mut buffer = [0u8; 64];
    let mut len = buffer.len();
    let name = c"kern.osproductversion";
    let read =
        unsafe { libc::sysctlbyname(name.as_ptr(), buffer.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) };
    let bytes = &buffer[..len.min(buffer.len())];
    let text = String::from_utf8_lossy(bytes.split(|b| *b == 0).next().unwrap_or_default()).into_owned();
    if read == 0 { format!("macOS {text}") } else { String::new() }
}

/// `rgba`, `w` pixels wide, with each of `blank` (window points, `scale` pixels a point) filled.
fn blanked(mut rgba: Vec<u8>, w: usize, scale: f64, blank: &[[f64; 4]]) -> Vec<u8> {
    let h = rgba.len() / (w * 4).max(1);
    let fill = theme::WELL.to_be_bytes();
    for [x, y, bw, bh] in blank {
        let px = |v: f64, most: usize| ((v * scale).round().max(0.0) as usize).min(most);
        let (x0, x1, y0, y1) = (px(*x, w), px(x + bw, w), px(*y, h), px(y + bh, h));
        for row in y0..y1 {
            for at in (row * w + x0)..(row * w + x1) {
                rgba[at * 4..at * 4 + 4].copy_from_slice(&[fill[1], fill[2], fill[3], 255]);
            }
        }
    }
    rgba
}

/// The marks as the annotated copy draws them: each box with its number; the whole app as the
/// picture's edge.
fn shapes(brief: &Brief) -> Vec<Shape> {
    let mark = |kept: &core_bug::Kept| {
        let rect = kept.rect.unwrap_or([0.004, 0.004, 0.992, 0.992]);
        let note = Shape::Note { at: [rect[0], (rect[1] - 0.03).max(0.0)], text: kept.n.to_string() };
        [Shape::Box { rect }, note]
    };
    brief.marks.iter().flat_map(mark).collect()
}

impl Workspace {
    /// The user's bug button or key (or a test's `bug_start`, as the user): the window's own
    /// picture is taken off the main thread, then the report starts over it. `Err`: why nothing
    /// starts. Only the user's: `origin` is theirs, or the core refuses.
    pub(crate) fn bug_start(&mut self, origin: Origin, window: &Window, cx: &mut Context<Self>) -> Result<(), String> {
        // One at a time: a second press while one is open takes no second picture.
        if self.model.bug.is_some() || self.bug_pending {
            return Ok(());
        }
        let size = window.viewport_size();
        let points = [f64::from(f32::from(size.width)), f64::from(f32::from(size.height))];
        let controls: Vec<(String, [f64; 4])> =
            self.controls.drawn_with("").into_iter().map(|(id, b)| (id, rect(b).map(f64::from))).collect();
        let panes = self.bug_panes();
        let drawn = (panes.clone(), self.bug_apps(), size);
        let word = collect::new_word().ok_or("bug report: the system's random source could not be read")?;
        let word_kept = word.clone();
        let start = move |picture| Action::BugStart {
            word: word.clone(),
            size: points,
            controls: controls.clone(),
            panes: panes.clone(),
            picture,
        };
        if let Some(why) = self.model.refusal(&start(false), origin) {
            return Err(why.into());
        }
        // This window alone, by its id, at its pixels: what it shows now, before the report is drawn.
        let id = ns_view(window).map(|view| unsafe { ottospace_mac::window::number(view) }).filter(|id| *id > 0);
        let scale = window.scale_factor();
        let pixels = |v: gpui::Pixels| (f32::from(v) * scale).round().max(1.0) as usize;
        let (w, h) = (pixels(size.width), pixels(size.height));
        self.bug_pending = true;
        cx.spawn_in(window, async move |this, cx| {
            let shot = cx.background_executor().spawn(async move {
                let rgba = ottospace_mac::capture::window_rgba(u32::try_from(id?).ok()?, w, h).ok()?;
                let png = ottospace_mac::image::png_from_rgba(w, h, &rgba)?;
                Some((rgba, png))
            });
            let shot = shot.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.bug_pending = false;
                // The panes moved, an app pane came or went, or the window changed while it was
                // taken: what it blanks would miss them, so no picture is kept.
                let now = (this.bug_panes(), this.bug_apps(), window.viewport_size());
                let shot = shot.filter(|_| now == drawn);
                this.dispatch(start(shot.is_some()), origin, window, cx);
                // Kept only for the report that started: one was open already, or it was refused.
                let started = this.model.bug.as_ref().is_some_and(|report| report.word == word_kept);
                if let Some((rgba, png)) = shot.filter(|_| started) {
                    let image = Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png));
                    this.bug_shot = Some(Frozen { word: word_kept.clone(), size: (w, h), rgba, image });
                }
            });
        })
        .detach();
        Ok(())
    }

    /// The main window's panes and where, in window points: the picture is of that window.
    fn bug_panes(&self) -> Vec<(PaneId, [f64; 4])> {
        let shown = self.model.main_panes();
        shown.into_iter().filter_map(|id| Some((id, rect(self.cells.get(&id)?.get()).map(f64::from)))).collect()
    }

    /// Which of them are app panes: the ones blanked.
    fn bug_apps(&self) -> Vec<PaneId> {
        let app = |id: &PaneId| matches!(self.model.pane(*id).map(|pane| &pane.kind), Some(Kind::App { .. }));
        self.model.main_panes().into_iter().filter(app).collect()
    }

    /// `Effect::BugDropped`: the picture goes.
    pub(crate) fn bug_dropped(&mut self, word: &str) {
        if self.bug_shot.as_ref().is_some_and(|shot| shot.word == word) {
            self.bug_shot = None;
        }
    }

    /// Send, with what the shell says of itself: the build, the OS, the event log's last lines.
    pub(crate) fn bug_send(&mut self, window: &Window, cx: &mut Context<Self>) {
        let (commit, date) = (option_env!("OTTOSPACE_COMMIT"), option_env!("OTTOSPACE_BUILD_DATE"));
        let build = crate::version_line(env!("CARGO_PKG_VERSION"), commit, date);
        let log = crate::event_log::last_lines(core_bug::MAX_EVENTS);
        self.dispatch(Action::BugSend { build, os: os_version(), log }, self.input, window, cx);
        cx.notify();
    }

    /// `Effect::BugFiled`: the task's folder (new, inside the materials folder) gets the brief at
    /// once, for the agent that starts once its worktree is made; then, off the main thread, the
    /// picture with `blank` filled and a copy with the marks, and the brief again without them if
    /// they could not be written. A folder that is there already is never written into.
    pub(crate) fn bug_file(
        &mut self,
        word: &str,
        name: &str,
        blank: &[[f64; 4]],
        brief: &Brief,
        cx: &mut Context<Self>,
    ) {
        let shot = self.bug_shot.take().filter(|shot| shot.word == word);
        let Some(dir) = collect::task_dir(brief.task, name) else { return };
        if std::fs::symlink_metadata(&dir).is_ok() {
            self.error = Some(format!("bug report: {} is there already: nothing was kept", dir.display()));
            return;
        }
        let at = dir.to_string_lossy().into_owned();
        let pictured = brief.clone();
        let written = |brief: &Brief| {
            let text = core_bug::markdown(brief, Some(&at));
            let marks = ottospace_core::pretty_json(&brief.marks);
            [("brief.md", text), ("brief.json", ottospace_core::pretty_json(brief)), ("1.json", marks)]
                .iter()
                .try_for_each(|(name, text)| config::write_private(&dir, name, text.as_bytes()))
        };
        let shown = shot.is_some();
        let first = if shown { pictured.clone() } else { pictured.clone().without_picture() };
        if let Err(err) = written(&first) {
            self.error = Some(format!("bug report: could not keep the brief: {err}"));
            return;
        }
        let Some(shot) = shot.filter(|_| brief.picture.is_some()) else { return };
        let (dir, blank) = (dir.clone(), blank.to_vec());
        let work = cx.background_executor().spawn(async move {
            let (w, h) = shot.size;
            // Pixels a point: the picture's width over the window's.
            let scale = w as f64 / pictured.captured.content.window[0].max(1.0);
            let rgba = blanked(shot.rgba, w, scale, &blank);
            let png = ottospace_mac::image::png_from_rgba(w, h, &rgba)?;
            let drawn = ottospace_mac::image::annotate(&png, &shapes(&pictured), theme::SNEAK, scale)?;
            let taken = config::write_private(&dir, "report.png", &png);
            taken.and_then(|()| config::write_private(&dir, "report.annotated.png", &drawn)).ok()
        });
        let at = at.clone();
        let brief = brief.clone();
        cx.spawn(async move |this, cx| {
            if work.await.is_none() {
                // Named in the brief only if both are there.
                let text = core_bug::markdown(&brief.clone().without_picture(), Some(&at));
                let dir = std::path::Path::new(&at);
                let _ = config::write_private(dir, "brief.md", text.as_bytes());
                let _ = config::write_private(
                    dir,
                    "brief.json",
                    ottospace_core::pretty_json(&brief.without_picture()).as_bytes(),
                );
                let _ = this.update(cx, |this, cx| {
                    this.error = Some("bug report: the picture could not be kept".into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// The overlay takes every key but ⌘ shortcuts. While a comment is typed its field has them:
    /// ↵ or Esc ends it. Else Esc cancels the report. ⌘↵ sends it.
    pub(crate) fn bug_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let (key, m) = (event.keystroke.key.as_str(), event.keystroke.modifiers);
        let typing = self.model.field().is_some();
        let action = match key {
            _ if event.is_held && matches!(key, "enter" | "escape") => return cx.stop_propagation(),
            "enter" if m.platform => return self.bug_send(window, cx),
            "enter" | "escape" if typing => Action::BugEdit { mark: None },
            "escape" => Action::BugCancel,
            _ if typing => {
                if self.field_key(event, window, cx) {
                    cx.stop_propagation();
                }
                return;
            }
            _ if m.platform => return,
            _ => return cx.stop_propagation(),
        };
        self.dispatch(action, self.input, window, cx);
        cx.stop_propagation();
    }

    /// Where `position` is on the window, 0..1 of it.
    fn on_window(window: &Window, position: gpui::Point<gpui::Pixels>) -> [f64; 2] {
        let size = window.viewport_size();
        let (w, h) = (f32::from(size.width).max(1.0), f32::from(size.height).max(1.0));
        [f64::from(f32::from(position.x) / w), f64::from(f32::from(position.y) / h)]
    }

    /// The pointer moved, anywhere in the window: a box begun on the picture goes on.
    pub(crate) fn bug_move(&mut self, event: &MouseMoveEvent, window: &Window, cx: &mut Context<Self>) {
        if !self.model.bug.as_ref().is_some_and(Report::pressed) {
            return;
        }
        let at = Self::on_window(window, event.position);
        let action = match event.pressed_button {
            Some(MouseButton::Left) => Action::BugDrag { to: at },
            // The button came up outside the window.
            _ => Action::BugRelease { at },
        };
        self.dispatch(action, self.input, window, cx);
    }

    pub(crate) fn bug_release(&mut self, position: gpui::Point<gpui::Pixels>, window: &Window, cx: &mut Context<Self>) {
        if self.model.bug.as_ref().is_some_and(Report::pressed) {
            let at = Self::on_window(window, position);
            self.dispatch(Action::BugRelease { at }, self.input, window, cx);
        }
    }

    /// What a control of the overlay does, by its name: its click.
    pub(crate) fn click_bug(&mut self, control: &str, window: &Window, cx: &mut Context<Self>) {
        let number = control.rsplit_once(':').and_then(|(_, n)| n.parse::<u32>().ok());
        let action = match (control.split(':').nth(1).unwrap_or_default(), number) {
            ("whole", _) => Action::BugWhole,
            ("apps", _) => Action::BugApps { on: None },
            ("cancel", _) => Action::BugCancel,
            ("send", _) => return self.bug_send(window, cx),
            ("comment", Some(n)) => Action::BugEdit { mark: Some(n) },
            ("remove", Some(n)) => Action::BugRemove { mark: n },
            _ => return,
        };
        self.dispatch(action, self.input, window, cx);
        cx.notify();
    }

    /// A button of the overlay's bar or a card: `bug:<name>`.
    fn bug_button(&self, name: String, kind: ui::Button, state: ui::State, cx: &mut Context<Self>) -> Stateful<Div> {
        let control = name.clone();
        ui::button(SharedString::from(name.clone()), kind, state)
            .child(self.controls.mark(name))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| this.click_bug(&control, window, cx)))
    }

    /// The bar over the picture: what to do, and its buttons.
    fn bug_bar(&self, report: &Report, cx: &mut Context<Self>) -> Div {
        let apps = if report.apps { "☑ Include app panes" } else { "☐ Include app panes" };
        let secondary = |label: &'static str| ui::Button::Secondary(label.into());
        // Primary once there is a mark to send.
        let send = match report.marks.is_empty() {
            true => secondary("Send to an agent  ⌘↵"),
            false => ui::Button::Primary("Send to an agent  ⌘↵".into()),
        };
        let bar = dialog::panel(theme::BORDER_STRONG)
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_sm().text_color(rgb(theme::TEXT)).child("Report a bug in ottospace"))
                    .child(div().text_xs().text_color(rgb(theme::MUTED)).child("drag a box round what is wrong"))
                    .child(self.bug_button("bug:whole".into(), secondary("Whole app"), ui::State::Rest, cx))
                    .child(self.bug_button("bug:apps".into(), secondary(apps), ui::State::Rest, cx))
                    .child(self.bug_button("bug:cancel".into(), secondary("Cancel  esc"), ui::State::Rest, cx))
                    .child(self.bug_button("bug:send".into(), send, ui::State::Rest, cx)),
            )
            .child(div().text_xs().text_color(rgb(theme::MUTED)).child(
                "Only ottospace's window is pictured, and it stays on this Mac for the agent you start. \
                 It may show what is on screen now.",
            ));
        div().absolute().top(px(10.)).left_0().right_0().flex().justify_center().child(bar)
    }

    /// A mark's comment card, below its box (above it near the foot), within the window as it is
    /// now (`ww`×`wh` points): the picture stretches with a resize.
    fn bug_card(&self, [ww, wh]: [f32; 2], report: &Report, mark: &Mark, cx: &mut Context<Self>) -> AnyElement {
        let (x, y) = match mark.rect {
            Some([x, y, _, h]) => {
                let (left, top, bottom) = (x as f32 * ww, y as f32 * wh, (y + h) as f32 * wh);
                let y = if bottom + 96. < wh { bottom + 6. } else { (top - 90.).max(70.) };
                (left.min(ww - CARD - 8.).max(8.), y)
            }
            None => (16., 76.),
        };
        let n = mark.n;
        let typing = report.typing == Some(n);
        let body = match typing {
            true => div().flex().child(self.render_field_of("bug", "what is wrong here?", cx)).into_any_element(),
            false => {
                let text = mark.comment.text();
                let (text, colour) =
                    if text.is_empty() { ("Add a comment", theme::MUTED) } else { (text, theme::TEXT) };
                let control = format!("bug:comment:{n}");
                div()
                    .id(SharedString::from(control.clone()))
                    .relative()
                    .cursor(CursorStyle::IBeam)
                    .text_color(rgb(colour))
                    .child(text.to_string())
                    .child(self.controls.mark(control.clone()))
                    .on_click(cx.listener(move |this, _, window, cx| this.click_bug(&control, window, cx)))
                    .into_any_element()
            }
        };
        let label = if mark.rect.is_some() { format!("{n}") } else { format!("{n} · whole app") };
        dialog::panel(if typing { theme::SNEAK } else { theme::BORDER_STRONG })
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(CARD))
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .text_xs()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().flex().items_center().justify_between().child(sneak::chip().text_xs().child(label)).child(
                self.bug_button(
                    format!("bug:remove:{n}"),
                    ui::Button::Ghost(ui::Face::Label("×".into())),
                    ui::State::Rest,
                    cx,
                ),
            ))
            .child(body)
            .into_any_element()
    }

    /// Over everything while a report is made: the frozen window, app panes blanked, the marks,
    /// their cards and the bar.
    pub(crate) fn render_bug(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let report = self.model.bug.as_ref()?;
        let layer = div().id("bug-report").absolute().top_0().left_0().size_full().occlude();
        let shot = self.bug_shot.as_ref().filter(|shot| shot.word == report.word);
        let picture = match shot {
            Some(shot) => img(shot.image.clone())
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .object_fit(ObjectFit::Fill)
                .into_any_element(),
            // No picture (no Screen Recording): the window as it is, dimmed.
            None => div().absolute().top_0().left_0().size_full().bg(rgba(0x09090bb0)).into_any_element(),
        };
        // In fractions of the frozen window, as the picture is stretched: a resize keeps them on the panes.
        let [ww, wh] = report.size;
        let blanked = report.blanked().into_iter().map(|[x, y, w, h]| {
            div()
                .absolute()
                .left(relative((x / ww) as f32))
                .top(relative((y / wh) as f32))
                .w(relative((w / ww) as f32))
                .h(relative((h / wh) as f32))
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(theme::WELL))
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .child("App pane hidden")
        });
        let outline = |[x, y, w, h]: [f64; 4], lit: bool| {
            let (x, y, w, h) = (x as f32, y as f32, w as f32, h as f32);
            div()
                .absolute()
                .left(relative(x))
                .top(relative(y))
                .w(relative(w))
                .h(relative(h))
                .border_2()
                .border_color(rgb(theme::SNEAK))
                .when(lit, |d| d.bg(rgba(0xffb9001a)))
        };
        let boxes = report.marks.iter().map(|mark| {
            let lit = report.typing == Some(mark.n);
            match mark.rect {
                Some(r) => outline(r, lit)
                    .child(sneak::chip().absolute().top(px(-11.)).left(px(-11.)).text_xs().child(mark.n.to_string())),
                None => div().absolute().top_0().left_0().size_full().border_4().border_color(rgb(theme::SNEAK)),
            }
        });
        let drag = report.drag.map(|r| outline(r, true));
        let size = window.viewport_size();
        let now = [f32::from(size.width), f32::from(size.height)];
        let cards: Vec<AnyElement> = report.marks.iter().map(|mark| self.bug_card(now, report, mark, cx)).collect();
        let layer = layer
            .cursor(CursorStyle::Crosshair)
            .child(picture)
            .children(blanked)
            .children(boxes)
            .children(drag)
            .children(cards)
            .child(self.bug_bar(report, cx))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    let at = Self::on_window(window, event.position);
                    this.dispatch(Action::BugPress { at }, this.input, window, cx);
                }),
            )
            // The layer occludes the window's own handlers: the drag goes on here.
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| this.bug_move(event, window, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, window, cx| {
                    this.bug_release(event.position, window, cx)
                }),
            );
        Some(layer.into_any_element())
    }

    /// The state's `bug`: whether the button shows, the report (its marks and comments, what has
    /// the keyboard), what is blanked, the picture's size and where the overlay's controls are.
    /// Never the picture itself.
    pub(crate) fn bug_snapshot(&self) -> serde_json::Value {
        let shot = self.model.bug.as_ref().and(self.bug_shot.as_ref()).map(|shot| shot.size);
        serde_json::json!({
            "icon": self.model.bug_icon(),
            "report": self.model.bug,
            "blanked": self.model.bug.as_ref().map(Report::blanked),
            "pixels": shot,
            // A test run only, as `overflow`.
            "controls": self.model.bug.as_ref().filter(|_| std::env::var_os("OTTOSPACE_E2E").is_some()).map(|_| {
                let drawn = self.controls.drawn_with("bug:").into_iter().map(|(id, b)| (id, rect(b)));
                drawn.collect::<std::collections::BTreeMap<String, [f32; 4]>>()
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An app pane's rect in points is filled at the picture's scale, and only what is in the picture.
    #[test]
    fn a_blanked_rect_is_filled_at_the_pictures_scale_and_cut_at_its_edge() {
        let filled = |rgba: &[u8]| rgba.chunks_exact(4).map(|p| p[3] == 255).collect::<Vec<bool>>();
        // 4×4 pixels of a 2×2 point window: the point at (1, 0) is the top right four pixels.
        let top_right = blanked(vec![0; 64], 4, 2.0, &[[1.0, 0.0, 1.0, 1.0]]);
        let want = [0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0].map(|v| v == 1);
        assert_eq!(filled(&top_right), want);
        // Past the edges, or before them: no panic, the part inside.
        let past = blanked(vec![0; 64], 4, 2.0, &[[-1.0, 1.0, 9.0, 9.0]]);
        assert_eq!(filled(&past), [[false; 8], [true; 8]].concat());
        assert_eq!(blanked(vec![0; 64], 4, 2.0, &[]), vec![0; 64]);
    }
}
