//! The app's notices, drawn as cards of the toast (`toast`): the window's top right, over the
//! panes, taking no room. Which notices show, in what order, their level and who may dismiss one
//! are the core's (`ottospace_core::notice`). While a live app's window is in front, over the
//! panes, the cards are drawn in a floating panel above it (`float_panel`), at the same place.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gpui::{AnyElement, App, Bounds, ClickEvent, Context, Div, SharedString, Window, div, point, prelude::*, px, size};
use ottospace_core::ax::Frame;
use ottospace_core::model::{Action, Origin};
use ottospace_core::notice::{self, Kind, Notice, Offer};

use ottospace_mac::recovery::to_frame;

use crate::{Mark, SPACE_TABS, TOLD_FOR, TOP_BAR, Workspace, ax, mark, on_screen, toast};

/// A card's click handler.
type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// The toast as it moves: in or gone, and how far it has got.
pub(crate) struct Toast {
    open: bool,
    /// Where it was when it last began to move, and when that was.
    from: f32,
    since: Instant,
    /// How far in it is this frame: 0 gone, 1 at rest.
    shown: f32,
    /// Its cards and how many more. Kept while it goes: it still shows them.
    last: (Vec<Notice>, usize),
    /// Frames asked for because it moves (the state's `counts.toast_frames`, test runs).
    pub(crate) frames: u64,
    /// Where the window last placed it: `[top, right, room]` (`toast::place`).
    at: Cell<Option<[f32; 3]>>,
    /// What the floating panel last drew, and where it is in the window (`[x, y]`).
    in_panel: (Vec<Notice>, usize),
    panel_at: Cell<[f32; 2]>,
}

impl Toast {
    /// It is coming in or going: not at rest.
    pub(crate) fn moves(&self) -> bool {
        self.shown != if self.open { 1. } else { 0. }
    }
}

impl Default for Toast {
    fn default() -> Self {
        let (at, in_panel, panel_at) = (Cell::new(None), (Vec::new(), 0), Cell::new([0.; 2]));
        let last = (Vec::new(), 0);
        Self { open: false, from: 0., since: Instant::now(), shown: 0., last, frames: 0, at, in_panel, panel_at }
    }
}

impl Workspace {
    /// Every notice there is to show, newest and most pressing first: a meeting that started,
    /// `line` (the last step that failed), what is not saved, a workspace's change offered
    /// back, what a step did, the settings' line, a problem found at start, a key not registered, the editor's note.
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
            self.model.workbooks.notice(),
            self.model.done_notice(),
            said(Kind::Told, told),
            said(self.model.settings_kind(), self.model.settings_line()),
            said(Kind::Start, start),
            said(Kind::Keys, self.key_notice.clone()),
            said(Kind::Editor, self.model.editor_note().map(str::to_string)),
        ];
        all.into_iter().flatten().collect()
    }

    /// The toast's top card, if any. Of the last step's line, what the toast holds: a
    /// client's answer is not on it, also mid-request, before `swap_origin` settles it.
    pub(crate) fn notice_shown(&self) -> Option<Notice> {
        notice::shown(&self.notices(self.held_now().said), &self.model.notice_hidden)
    }

    /// The reply's `notice`: as the top card's, but with the last step's line whoever's it was, so
    /// a client reads why its own request did nothing.
    pub(crate) fn notice_replied(&self) -> Option<Notice> {
        // Why a move did nothing (`Effect::Notice`) is a warning; any other failure an error.
        let refused = self.error.is_some() && self.error == self.refused;
        let kind = if refused { Kind::Refused } else { Kind::Failed };
        let line = self.error.clone().map(|text| Notice { kind, text });
        // A meeting does not cover a line the toast does not hold (why a client's step did nothing);
        // over the toast's own line it stands, as on the toast.
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

    /// The toast's cards now, and how many more (`notice::stack`).
    pub(crate) fn notice_stack(&self) -> (Vec<Notice>, usize) {
        notice::stack(&self.notices(self.held_now().said), &self.model.notice_hidden)
    }

    /// Esc: the top card goes. The core takes it from the user alone. False: no notice shows.
    pub(crate) fn dismiss_notice(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        let Some(shown) = self.notice_shown() else { return false };
        self.dismiss_card(&shown, window, cx);
        true
    }

    /// × on the card that said `drawn`, if it is still there. A meeting's is the calendar's Dismiss.
    pub(crate) fn dismiss_card(&mut self, drawn: &Notice, window: &Window, cx: &mut Context<Self>) {
        if !self.card_there(drawn) {
            return;
        }
        let action = match self.meeting_shown(drawn) {
            Some((id, _)) => Action::CalendarDismiss { id },
            None => Action::NoticeDismiss { notice: drawn.kind, text: drawn.text.clone() },
        };
        self.dispatch(action, self.input, window, cx);
        // A closed temporary workspace's offer ended: it goes now, not at the next tick.
        if drawn.kind == Kind::Undo {
            self.dispatch(Action::Tick { now: crate::now_ms() }, Origin::System, window, cx);
        }
        cx.notify();
    }

    /// Whether the card that said `drawn` is still in the toast. A meeting's line ages ("started
    /// 1m ago") between frames: its card is still the meeting's.
    fn card_there(&self, drawn: &Notice) -> bool {
        let same = |card: &Notice| card == drawn || (card.kind == Kind::Meeting && drawn.kind == Kind::Meeting);
        self.notice_stack().0.iter().any(same)
    }

    /// The meeting `shown` is about, if it is one: its id, and whether it has a link to join.
    fn meeting_shown(&self, shown: &Notice) -> Option<(u64, bool)> {
        let (id, _, link) = self.model.calendar.banner_line(crate::now_ms()).filter(|_| shown.kind == Kind::Meeting)?;
        Some((id, link))
    }

    /// Whether `card` is "Done · <task>" with its New task here: the task's workspace stays.
    pub(crate) fn offers_new_task(&self, card: &Notice) -> bool {
        self.model.done_notice().as_ref() == Some(card) && self.model.done_here()
    }

    /// What `shown` offers beside its text, and the action it takes: Join only for a link.
    pub(crate) fn notice_offer(&self, shown: &Notice) -> Option<(Offer, Action)> {
        match shown.kind.offer()? {
            Offer::OpenSettings => Some((Offer::OpenSettings, Action::EditSettings)),
            // "Done · <task>"'s, else a workspace's.
            Offer::Undo if self.model.done_notice().as_ref() == Some(shown) => Some((Offer::Undo, Action::DoneUndo)),
            Offer::Undo => Some((Offer::Undo, Action::WorkbookUndo)),
            Offer::Join => {
                let (id, link) = self.meeting_shown(shown)?;
                link.then_some((Offer::Join, Action::CalendarJoin { id }))
            }
        }
    }

    /// A click or label on `drawn`'s offer: taken only while that card is still there and offers
    /// it. An undo whose time is up takes nothing back, nor one that came since in its place.
    pub(crate) fn take_notice_offer(&mut self, drawn: &Notice, offer: Offer, window: &Window, cx: &mut Context<Self>) {
        self.tick_now();
        let there = self.card_there(drawn);
        let still = self.notice_offer(drawn).filter(|(now, _)| there && *now == offer);
        if let Some((_, action)) = still {
            self.dispatch(action, self.input, window, cx);
        }
        cx.notify();
    }

    /// Whether the toast must be drawn again: a notice came, went, or says something else.
    pub(crate) fn notice_changed(&self) -> bool {
        let stack = self.notice_stack();
        !stack.0.is_empty() != self.toast.open || (!stack.0.is_empty() && stack != self.toast.last)
    }

    /// Once a frame, before it is drawn: how far in the toast is. It slides in and out, and asks
    /// for the next frame only while it moves; under Reduce Motion it is there at once.
    pub(crate) fn move_toast(&mut self, window: &Window) {
        let stack = self.notice_stack();
        let (open, toast) = (!stack.0.is_empty(), &mut self.toast);
        if open != toast.open {
            (toast.open, toast.from, toast.since) = (open, toast.shown, Instant::now());
        }
        let rest = if open { 1. } else { 0. };
        if toast.shown != rest {
            let calm = crate::dial::calm();
            toast.shown = notice::shown_in(toast.from, open, toast.since.elapsed().as_secs_f32() * 1000., calm);
        }
        if toast.shown != rest {
            toast.frames += 1;
            window.request_animation_frame();
        }
        if open {
            toast.last = stack;
        } else if toast.shown == 0. {
            toast.last = (Vec::new(), 0);
        }
    }

    /// The toast's cards: each a level glyph, the text (in full on hover), what it offers, and ×.
    /// `panel`: for the floating panel, whose clicks act in the main window, noted in its marks.
    fn toast_cards(&self, panel: bool, cx: &mut Context<Self>) -> Option<(Vec<toast::Card>, usize)> {
        let (shown, more) = self.toast.last.clone();
        // The panel shows them at rest; the window as far in as they are.
        if shown.is_empty() || (!panel && self.toast.shown == 0.) {
            return None;
        }
        let marks = if panel { &self.toast_panel.marks } else { &self.marks };
        let label = |id: String| (!panel).then(|| self.controls.mark(id));
        let cards = shown.into_iter().enumerate().map(|(at, notice)| {
            let offer = self.notice_offer(&notice).map(|(offer, _)| {
                let drawn = notice.clone();
                let take = move |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
                    this.take_notice_offer(&drawn, offer, window, cx)
                };
                toast::button(("toast-offer", at))
                    .child(offer.label())
                    .child(mark(marks, Mark::ToastOffer(at)))
                    .children(label(format!("toast:{at}:offer")))
                    .on_click(self.card_click(panel, take, cx))
            });
            // "Done · <task>": a new task in its workspace, which stays.
            let new_task = self.offers_new_task(&notice).then(|| {
                let open = |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| this.new_task_here(window, cx);
                toast::button(("toast-new-task", at))
                    .child("New task here ⌘N")
                    .child(mark(marks, Mark::ToastNewTask(at)))
                    .children(label(format!("toast:{at}:new-task")))
                    .on_click(self.card_click(panel, open, cx))
            });
            let drawn = notice.clone();
            let dismiss = move |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
                this.dismiss_card(&drawn, window, cx)
            };
            let close = toast::button(("toast-close", at))
                .child("×")
                .child(mark(marks, Mark::ToastClose(at)))
                .children(label(format!("toast:{at}:close")))
                .on_click(self.card_click(panel, dismiss, cx));
            toast::Card {
                level: notice.level(),
                glyph: notice.glyph(),
                title: None,
                text: SharedString::from(notice.text.clone()),
                actions: offer.into_iter().chain(new_task).collect(),
                close,
                marks: vec![mark(marks, Mark::ToastCard(at)).into_any_element()],
            }
        });
        Some((cards.collect(), more))
    }

    /// A card's click: `act`, in the main window. From the panel it is carried there, after the
    /// event: what it asks of hosted windows is measured against the main window.
    fn card_click(
        &self,
        panel: bool,
        act: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> Click {
        if !panel {
            return Box::new(cx.listener(move |this, _, window, cx| act(this, window, cx)));
        }
        let (ws, main, act) = (cx.entity().downgrade(), self.main_window, Rc::new(act));
        Box::new(move |_, _, cx: &mut App| {
            let (ws, act) = (ws.clone(), act.clone());
            cx.defer(move |cx| drop(main.update(cx, |_, window, cx| ws.update(cx, |ws, cx| act(ws, window, cx)))));
        })
    }

    /// The toast in the window, over the panes' top right: `right` points from the window's edge,
    /// clear of the drawer and its rail; `room`, the panes' width. None while the panel shows it.
    pub(crate) fn render_toast(&self, right: f32, room: f32, cx: &mut Context<Self>) -> Option<Div> {
        let at = [TOP_BAR + SPACE_TABS + 8., right, room];
        self.toast.at.set(Some(at));
        if self.toast_panel.drawn().is_some() {
            return None;
        }
        let (cards, more) = self.toast_cards(false, cx)?;
        Some(toast::stack(at, cards, more, self.toast.shown, mark(&self.marks, Mark::Toast)))
    }

    /// The floating panel's content: the same cards, at rest.
    fn draw_toast_panel(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.toast_panel.marks.borrow_mut().clear();
        match self.toast_cards(true, cx) {
            Some((cards, more)) => {
                toast::panel(cards, more, mark(&self.toast_panel.marks, Mark::Toast)).into_any_element()
            }
            None => div().into_any_element(),
        }
    }

    /// Each tick: the toast floats in its panel while a live app's window lies over its place and
    /// that app is in front, ottospace's window on the Space shown; else the window draws it. Its
    /// cards come from here then: the window under the app may draw no frame.
    pub(crate) fn sync_toast_panel(&mut self, window: &Window, cx: &mut Context<Self>) {
        let stack = self.notice_stack();
        let place = self.toast.at.get().filter(|_| !stack.0.is_empty()).map(|at| {
            // A notice's card has no title.
            let height = toast::height(stack.0.iter().map(|_| false), stack.1);
            toast::place(at, f32::from(window.viewport_size().width), height)
        });
        let screen = place.map(|[x, y, w, h]| {
            to_frame(on_screen(Bounds { origin: point(px(x), px(y)), size: size(px(w), px(h)) }, window, cx))
        });
        let live = self.model.live.and_then(|pane| self.hosted.get(&pane));
        let over = |f: &Frame| {
            let a = live.map(|h| h.placed).unwrap_or_default();
            a.origin.x < f.x + f.w
                && f.x < a.origin.x + a.size.width
                && a.origin.y < f.y + f.h
                && f.y < a.origin.y + a.size.height
        };
        let front = live.is_some_and(|h| ax::is_active(h.pid))
            && crate::ns_view(window).is_some_and(|view| {
                // SAFETY: GPUI's own view, on the main thread.
                unsafe { ottospace_mac::window::on_active_space(view) }
            });
        let want = screen.filter(|f| front && over(f));
        if let (Some(_), Some([x, y, ..])) = (want, place) {
            self.toast.panel_at.set([x, y]);
            self.toast.last.clone_from(&stack);
        }
        let redraw = self.toast.in_panel != self.toast.last;
        self.toast.in_panel.clone_from(&self.toast.last);
        let was = self.toast_panel.frame().is_some();
        self.toast_panel.sync(want, redraw, Self::draw_toast_panel, |ws| &mut ws.toast_panel, cx);
        // The window draws it again, or stops.
        if was != want.is_some() {
            cx.notify();
        }
    }

    /// Where the toast's part `mark` is, in the window's points: the panel's, while it draws them,
    /// moved by where it lies in the window.
    fn toast_rect(&self, mark: Mark) -> Option<[f32; 4]> {
        if self.toast_panel.drawn().is_none() {
            return self.marks.borrow().get(&mark).copied().map(crate::rect);
        }
        let [x, y] = self.toast.panel_at.get();
        let [mx, my, w, h] = self.toast_panel.marks.borrow().get(&mark).copied().map(crate::rect)?;
        Some([x + mx, y + my, w, h])
    }

    /// The toast in the state: null while it shows nothing.
    pub(crate) fn toast_snapshot(&self) -> serde_json::Value {
        let (shown, more) = &self.toast.last;
        let panel = self.toast_panel.drawn();
        if shown.is_empty() || (self.toast.shown == 0. && panel.is_none()) {
            return serde_json::Value::Null;
        }
        let drawn = |mark: Mark| self.toast_rect(mark);
        let cards: Vec<serde_json::Value> = shown
            .iter()
            .enumerate()
            .map(|(at, notice)| {
                let offer = self.notice_offer(notice).map(|(offer, _)| offer);
                serde_json::json!({
                    "kind": notice.kind,
                    "level": notice.level(),
                    // A meeting's title is never here: "Meeting started".
                    "text": notice.public_text(),
                    // What it offers beside the text (`open_settings`, `join`, `undo`), and where; null: nothing.
                    "offer": offer,
                    "offer_rect": offer.and_then(|_| drawn(Mark::ToastOffer(at))),
                    "close": drawn(Mark::ToastClose(at)),
                    // "Done · <task>"'s New task here (⌘N): where, or null.
                    "new_task": drawn(Mark::ToastNewTask(at)).filter(|_| self.offers_new_task(notice)),
                    "rect": drawn(Mark::ToastCard(at)),
                })
            })
            .collect();
        serde_json::json!({
            // The cards, the top one first: what Esc dismisses.
            "cards": cards,
            // How many more notices there are than cards.
            "more": more,
            // Where the stack is drawn, and how far in it is: less than 1 while it moves.
            "rect": drawn(Mark::Toast),
            // The floating panel it is drawn in over a live app's window, `[x, y, w, h]` on the
            // screens (Accessibility's coordinates), or null: in ottospace's window. Its top left
            // is `rect`'s.
            "panel": panel.map(|f| [f.x, f.y, f.w, f.h]),
            "shown": self.toast.shown,
        })
    }
}

impl Workspace {
    /// The calendar's banner in the state: where its Join and Dismiss are, while a card shows
    /// the meeting.
    pub(crate) fn meeting_controls(&self) -> (Option<[f32; 4]>, Option<[f32; 4]>) {
        let shown = self.toast.shown > 0. || self.toast_panel.drawn().is_some();
        let at = self.toast.last.0.iter().position(|n| n.kind == Kind::Meeting).filter(|_| shown);
        let Some(at) = at else { return (None, None) };
        let drawn = |mark: Mark| self.toast_rect(mark);
        let join = self.notice_offer(&self.toast.last.0[at]).and_then(|_| drawn(Mark::ToastOffer(at)));
        (join, drawn(Mark::ToastClose(at)))
    }
}
