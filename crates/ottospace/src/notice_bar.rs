//! The notice bar: the one app-level message, full width between the top row and the rest.
//! Which notice shows, its level and who may dismiss it are the core's (`ottospace_core::notice`).

use std::time::Instant;

use gpui::{AnyView, App, Context, Div, MouseButton, SharedString, Window, div, prelude::*, px, rgb};
use ottospace_core::model::Action;
use ottospace_core::notice::{self, Kind, Notice, Offer};
use ottospace_core::theme;

use crate::{Mark, TOLD_FOR, TOP_BAR, Tooltip, Workspace, mark, recessed};

/// How deep the shadows on the bar's edges are, in points.
const DEPTH: f32 = 7.;

/// The bar as it moves: open or closed, and how far it has got.
pub(crate) struct Bar {
    open: bool,
    /// Where the height was when it last began to move, and when that was.
    from: f32,
    since: Instant,
    /// Its height in this frame: the rest of the window starts this much lower.
    pub(crate) height: f32,
    /// What it says. Kept while it closes: the bar still shows it.
    last: Option<Notice>,
}

impl Bar {
    /// It is opening or closing: not at rest.
    pub(crate) fn moves(&self) -> bool {
        self.height != if self.open { notice::HEIGHT } else { 0. }
    }
}

impl Default for Bar {
    fn default() -> Self {
        Self { open: false, from: 0., since: Instant::now(), height: 0., last: None }
    }
}

impl Workspace {
    /// Every notice there is to show, newest and most pressing first: `line` (the last step
    /// that failed), what is not saved, what a step did, the settings' line, a problem found at
    /// start, a key not registered, the editor's note.
    fn notices(&self, line: Option<Notice>) -> Vec<Notice> {
        let said = |kind: Kind, text: Option<String>| text.map(|text| Notice { kind, text });
        let unsaved = self.unsaved.clone().or(self.kept_marks.unsaved.clone()).or(self.kept_workbooks.unsaved.clone());
        let unsaved = unsaved.or(self.kept_layout.unsaved.clone());
        let told = self.told.as_ref().filter(|(_, at)| at.elapsed() < TOLD_FOR).map(|(text, _)| text.clone());
        // One line of all that was found at start: none hides another.
        let start = [&self.notice, &self.kept_marks.notice, &self.kept_workbooks.notice, &self.kept_layout.notice];
        let start: Vec<String> = start.into_iter().flatten().cloned().collect();
        let start = (!start.is_empty()).then(|| start.join(" · "));
        let all = [
            line,
            said(Kind::Unsaved, unsaved),
            said(Kind::Told, told),
            said(self.model.settings_kind(), self.model.settings_line()),
            said(Kind::Start, start),
            said(Kind::Keys, self.key_notice.clone()),
            said(Kind::Editor, self.model.editor_note().map(str::to_string)),
        ];
        all.into_iter().flatten().collect()
    }

    /// The notice the bar shows, if any. Of the last step's line, what the bar holds: a
    /// client's answer is not on it, also mid-request, before `swap_origin` settles it.
    pub(crate) fn notice_shown(&self) -> Option<Notice> {
        notice::shown(&self.notices(self.held_now().said), &self.model.notice_hidden)
    }

    /// The reply's `notice`: as the bar's, but with the last step's line whoever's it was, so
    /// a client reads why its own request did nothing.
    pub(crate) fn notice_replied(&self) -> Option<Notice> {
        // Why a move did nothing (`Effect::Notice`) is a warning; any other failure an error.
        let refused = self.error.is_some() && self.error == self.refused;
        let kind = if refused { Kind::Refused } else { Kind::Failed };
        let line = self.error.clone().map(|text| Notice { kind, text });
        notice::shown(&self.notices(line), &self.model.notice_hidden)
    }

    /// The user dismissed a notice of `kind`: the slot that held it is empty.
    pub(crate) fn notice_dismissed(&mut self, kind: Kind) {
        match kind {
            Kind::Failed | Kind::Refused => {
                (self.error, self.error_seen, self.bar_line) = (None, None, notice::Held::default());
            }
            Kind::Told => self.told = None,
            // Said again by the next save that fails.
            Kind::Unsaved => {
                self.unsaved = None;
                self.kept_marks.unsaved = None;
                self.kept_workbooks.unsaved = None;
                self.kept_layout.unsaved = None;
            }
            Kind::Keys => self.key_notice = None,
            _ => {}
        }
    }

    /// × on the bar, or Esc: the core takes it from the user alone. False: no notice shows.
    pub(crate) fn dismiss_notice(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        let Some(shown) = self.notice_shown() else { return false };
        self.dispatch(Action::NoticeDismiss { notice: shown.kind, text: shown.text }, self.input, window, cx);
        cx.notify();
        true
    }

    /// Whether the bar must be drawn again: a notice came, went, or says something else.
    pub(crate) fn notice_changed(&self) -> bool {
        let shown = self.notice_shown();
        shown.is_some() != self.bar.open || (shown.is_some() && shown != self.bar.last)
    }

    /// Once a frame, before it is drawn: the bar's height now. It grows from nothing and
    /// shrinks back, and asks for the next frame only while it moves.
    pub(crate) fn move_notice_bar(&mut self, window: &Window) {
        let shown = self.notice_shown();
        let (open, bar) = (shown.is_some(), &mut self.bar);
        if open != bar.open {
            (bar.open, bar.from, bar.since) = (open, bar.height, Instant::now());
        }
        let rest = if open { notice::HEIGHT } else { 0. };
        if bar.height != rest {
            // As the agent dial does: no motion under the system's Reduce Motion.
            let calm = ottospace_mac::meeting::reduce_motion();
            bar.height = notice::height(bar.from, open, bar.since.elapsed().as_secs_f32() * 1000., calm);
        }
        if bar.height != rest {
            window.request_animation_frame();
        }
        bar.last = shown.or(bar.last.take().filter(|_| bar.height > 0.));
    }

    /// The bar, `height` points of it showing: a level glyph, the text (in full on hover), what
    /// it offers, and ×. Laid out at its full height and cut, so it is uncovered, not squeezed.
    pub(crate) fn render_notice_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        let shown = self.bar.last.clone().filter(|_| self.bar.height > 0.)?;
        let (background, text) = theme::notice(shown.level());
        let full: SharedString = shown.text.clone().into();
        let tip = move |_: &mut Window, cx: &mut App| AnyView::from(cx.new(|_| Tooltip(full.clone())));
        let button = |id: &'static str| {
            let button = div().id(id).relative().flex_none().px_1p5().rounded_sm().cursor_pointer();
            // The click is the bar's: it does not also leave a mode, as one on the workspace would.
            button
                .hover(|s| s.bg(gpui::white().opacity(0.12)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let offer = shown.kind.offer().map(|offer| {
            let action = match offer {
                Offer::OpenSettings => Action::EditSettings,
            };
            let click = cx.listener(move |this, _, window, cx| {
                this.dispatch(action.clone(), this.input, window, cx);
                cx.notify();
            });
            button("notice-offer")
                .underline()
                .child(offer.label())
                .child(mark(&self.marks, Mark::NoticeOffer))
                .child(self.controls.mark("notice:offer"))
                .on_click(click)
        });
        let close = button("notice-close")
            .child("×")
            .child(mark(&self.marks, Mark::NoticeClose))
            .child(self.controls.mark("notice:close"))
            .on_click(cx.listener(|this, _, window, cx| {
                this.dismiss_notice(window, cx);
            }));
        let line = div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(px(notice::HEIGHT))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .text_color(rgb(text))
            .child(div().flex_none().child(shown.glyph()))
            .child(div().id("notice-text").min_w_0().truncate().child(shown.text.clone()).tooltip(tip))
            .children(offer)
            .child(div().flex_1())
            .child(close);
        let bar = recessed(line, DEPTH).absolute().top(px(TOP_BAR)).left_0().w_full().h(px(self.bar.height));
        Some(bar.bg(rgb(background)).child(mark(&self.marks, Mark::Notice)))
    }

    /// The bar in the state: null while it takes no room.
    pub(crate) fn notice_snapshot(&self) -> serde_json::Value {
        let Some(shown) = self.bar.last.as_ref().filter(|_| self.bar.height > 0.) else {
            return serde_json::Value::Null;
        };
        let marks = self.marks.borrow();
        let drawn = |mark: Mark| marks.get(&mark).copied().map(crate::rect);
        serde_json::json!({
            "kind": shown.kind,
            "level": shown.level(),
            "text": shown.text,
            // What it offers beside the text (`open_settings`), and where; null: nothing.
            "offer": shown.kind.offer(),
            "offer_rect": shown.kind.offer().and_then(|_| drawn(Mark::NoticeOffer)),
            "close": drawn(Mark::NoticeClose),
            // Where it is drawn, and how much of it shows: less than its full height while it moves.
            "rect": drawn(Mark::Notice),
            "height": self.bar.height,
        })
    }
}
