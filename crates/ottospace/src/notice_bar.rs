//! The notice bar: the one app-level message, full width at the window's foot, above the
//! timeline and the status bar. Nothing above it moves when it opens: the content's foot rises.
//! Which notice shows, its level and who may dismiss it are the core's (`ottospace_core::notice`).

use std::time::Instant;

use gpui::{AnyView, App, Context, Div, MouseButton, SharedString, Window, div, prelude::*, px, rgb};
use ottospace_core::model::Action;
use ottospace_core::notice::{self, Kind, Notice, Offer};
use ottospace_core::theme;

use crate::{Mark, TOLD_FOR, Tooltip, Workspace, mark, recessed};

/// How deep the shadows on the bar's edges are, in points.
const DEPTH: f32 = 7.;

/// The bar as it moves: open or closed, and how far it has got.
pub(crate) struct Bar {
    open: bool,
    /// Where the height was when it last began to move, and when that was.
    from: f32,
    since: Instant,
    /// Its height in this frame: the content above it ends this much higher.
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
    /// Every notice there is to show, newest and most pressing first: a meeting that started,
    /// `line` (the last step that failed), what is not saved, what a step did, the settings'
    /// line, a problem found at start, a key not registered, the editor's note.
    fn notices(&self, line: Option<Notice>) -> Vec<Notice> {
        let said = |kind: Kind, text: Option<String>| text.map(|text| Notice { kind, text });
        let unsaved = self.unsaved.clone().or(self.kept_marks.unsaved.clone()).or(self.kept_workbooks.unsaved.clone());
        let unsaved = unsaved.or(self.kept_layout.unsaved.clone());
        let told = self.told.as_ref().filter(|(_, at)| at.elapsed() < TOLD_FOR).map(|(text, _)| text.clone());
        // One line of all that was found at start: none hides another.
        let start = [&self.notice, &self.kept_marks.notice, &self.kept_workbooks.notice, &self.kept_layout.notice];
        let start: Vec<String> = start.into_iter().flatten().cloned().collect();
        let start = (!start.is_empty()).then(|| start.join(" · "));
        let meeting = self.model.calendar.banner_line(crate::now_ms()).map(|(_, line, _)| format!("Meeting: {line}"));
        let all = [
            said(Kind::Meeting, meeting),
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
        // A meeting does not cover a line the bar does not hold (why a client's step did nothing);
        // over the bar's own line it stands, as on the bar.
        let theirs = line.is_some() && line != self.held_now().said;
        let mut all = self.notices(line);
        all.retain(|n| !theirs || n.kind != Kind::Meeting);
        notice::shown(&all, &self.model.notice_hidden)
    }

    /// The user dismissed a notice of `kind`: the slot that held it is empty.
    pub(crate) fn notice_dismissed(&mut self, kind: Kind) {
        match kind {
            Kind::Failed | Kind::Refused => {
                (self.error, self.error_seen, self.bar_line) = (None, (None, None), notice::Held::default());
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
    /// A meeting's is the calendar's Dismiss.
    pub(crate) fn dismiss_notice(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        let Some(shown) = self.notice_shown() else { return false };
        let action = match self.meeting_shown(&shown) {
            Some((id, _)) => Action::CalendarDismiss { id },
            None => Action::NoticeDismiss { notice: shown.kind, text: shown.text },
        };
        self.dispatch(action, self.input, window, cx);
        cx.notify();
        true
    }

    /// The meeting `shown` is about, if it is one: its id, and whether it has a link to join.
    fn meeting_shown(&self, shown: &Notice) -> Option<(u64, bool)> {
        let (id, _, link) = self.model.calendar.banner_line(crate::now_ms()).filter(|_| shown.kind == Kind::Meeting)?;
        Some((id, link))
    }

    /// What `shown` offers beside its text, and the action it takes: Join only for a link.
    pub(crate) fn notice_offer(&self, shown: &Notice) -> Option<(Offer, Action)> {
        match shown.kind.offer()? {
            Offer::OpenSettings => Some((Offer::OpenSettings, Action::EditSettings)),
            Offer::Join => {
                let (id, link) = self.meeting_shown(shown)?;
                link.then_some((Offer::Join, Action::CalendarJoin { id }))
            }
        }
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
    /// it offers, and ×. Laid out at its full height on its foot and cut, so its top edge rises
    /// over it: uncovered, not squeezed.
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
        let offer = self.notice_offer(&shown).map(|(offer, action)| {
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
            .bottom_0()
            .left_0()
            .w_full()
            .h(px(notice::HEIGHT))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .text_color(rgb(text))
            .child(glyph(&shown, background, text))
            .child(crate::cut_text(shown.text.clone()).flex_1().id("notice-text").tooltip(tip))
            .children(offer)
            .child(close);
        let bar = recessed(line, DEPTH).flex_none().w_full().h(px(self.bar.height));
        Some(bar.bg(rgb(background)).child(mark(&self.marks, Mark::Notice)))
    }

    /// The bar in the state: null while it takes no room.
    pub(crate) fn notice_snapshot(&self) -> serde_json::Value {
        let Some(shown) = self.bar.last.as_ref().filter(|_| self.bar.height > 0.) else {
            return serde_json::Value::Null;
        };
        let marks = self.marks.borrow();
        let drawn = |mark: Mark| marks.get(&mark).copied().map(crate::rect);
        let offer = self.notice_offer(shown).map(|(offer, _)| offer);
        serde_json::json!({
            "kind": shown.kind,
            "level": shown.level(),
            // A meeting's title is never here: "Meeting started".
            "text": shown.public_text(),
            // What it offers beside the text (`open_settings`, `join`), and where; null: nothing.
            "offer": offer,
            "offer_rect": offer.and_then(|_| drawn(Mark::NoticeOffer)),
            "close": drawn(Mark::NoticeClose),
            // Where it is drawn, and how much of it shows: less than its full height while it moves.
            "rect": drawn(Mark::Notice),
            "height": self.bar.height,
        })
    }
}

/// The level's glyph. An error's `!` sits in a filled circle of the text's colour: no cross
/// beside the text that could read as the × on the right.
fn glyph(shown: &Notice, background: u32, text: u32) -> Div {
    let glyph = div().flex_none().child(shown.glyph());
    if shown.level() != notice::Level::Error {
        return glyph;
    }
    let round = glyph.size(px(13.)).rounded_full().flex().items_center().justify_center();
    round.bg(rgb(text)).text_color(rgb(background)).font_weight(gpui::FontWeight::BOLD)
}

impl Workspace {
    /// The calendar's banner in the state: where its Join and Dismiss are on the bar, while the
    /// bar shows the meeting (null rects under an error that covers it).
    pub(crate) fn meeting_controls(&self) -> (Option<[f32; 4]>, Option<[f32; 4]>) {
        let shown = self.bar.last.as_ref().filter(|n| n.kind == Kind::Meeting && self.bar.height > 0.);
        let Some(shown) = shown else { return (None, None) };
        let marks = self.marks.borrow();
        let drawn = |mark: Mark| marks.get(&mark).copied().map(crate::rect);
        let join = self.notice_offer(shown).and_then(|_| drawn(Mark::NoticeOffer));
        (join, drawn(Mark::NoticeClose))
    }
}
