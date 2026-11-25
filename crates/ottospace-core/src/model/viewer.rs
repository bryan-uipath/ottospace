//! The file viewer pane (`Kind::File`): which pane a file opens in, the view's line, keys with a
//! count, ⌃G's editor beside it. The shell reads and watches the file, draws it, and says what it
//! read (`ViewerRead`) and drew (`ViewerScrolled`); never its text. See `docs/ai/file-viewer.md`.

use super::{Effect, Kind, Mode, Model, PaneId};
use crate::layout::Axis;
use crate::viewer::{self, Lines, Read, Shows, Viewer};
use crate::vim::{self, Press};

/// Why a client's report of what was read or drawn of a file is refused.
pub(crate) const SHELLS_ALONE: &str = "only ottospace says what it read and drew of a file";
/// Why an `open_file` does nothing.
pub(crate) const NOT_SOUND: &str = "open_file: the path must be absolute and one line";

impl Model {
    /// The view of file viewer `pane`, if it is one.
    pub fn viewer(&self, pane: PaneId) -> Option<&Viewer> {
        self.viewers.get(&pane)
    }

    /// The focused viewer, while ottospace has the keyboard: its keys move it.
    pub fn keyboard_viewer(&self) -> Option<PaneId> {
        let id = self.focused.filter(|_| self.mode() == Mode::Normal && !self.keys)?;
        matches!(self.pane(id)?.kind, Kind::File { .. }).then_some(id)
    }

    /// True if `key` is the focused viewer's: a count, a move, or ⌃G. Else it is a shortcut's.
    pub fn viewer_takes(&self, key: &str) -> bool {
        let view = self.keyboard_viewer().and_then(|id| self.viewer(id));
        view.is_some_and(|view| {
            key == "ctrl-g" || view.pending.takes(key) || (key == "escape" && view.marked.is_some())
        })
    }

    /// What is typed of a move in the focused viewer (`10`, `g`), for the status bar.
    pub fn viewer_pending(&self) -> Option<String> {
        self.viewer(self.keyboard_viewer()?)?.pending.pending()
    }

    /// The viewer that `from` opened to be reused, in `from`'s tab.
    fn reusable(&self, from: PaneId) -> Option<PaneId> {
        let tab = self.tab_of(from)?;
        let mine = |(id, view): (&PaneId, &Viewer)| {
            (view.opener == Some(from) && self.tab_of(*id) == Some(tab)).then_some(*id)
        };
        self.viewers.iter().find_map(mine)
    }

    /// A viewer of `path` in `from`'s tab: `from` itself first.
    fn viewer_of(&self, path: &str, from: PaneId) -> Option<PaneId> {
        let tab = self.tab_of(from)?;
        let shows = |id: &PaneId| {
            self.pane(*id).is_some_and(|pane| matches!(&pane.kind, Kind::File { path: p } if p == path))
                && self.tab_of(*id) == Some(tab)
        };
        Some(from).filter(shows).or_else(|| self.viewers.keys().copied().find(shows))
    }

    /// Viewer `id` of `path` marks `at` and goes to it. Drawn markdown shows its source instead,
    /// read again.
    fn mark_lines(&mut self, id: PaneId, path: &str, at: Lines, effects: &mut Vec<Effect>) {
        let Some(view) = self.viewers.get_mut(&id) else { return };
        if view.shows == Shows::Markdown {
            *view = Viewer::at(path, Some(at), view.opener);
            effects.push(Effect::ViewFile { pane: id });
        } else {
            view.mark(Some(at));
            effects.push(Effect::ViewerScroll { pane: id, top: view.top });
        }
    }

    /// `OpenFile`: the file in the viewer `from` opened before (`reuse`), else in a new one beside
    /// `from` (an empty `from` shows it). A reused viewer, or one opened to be, leaves the
    /// keyboard where it was: the next file `from` names goes there too. At lines (`at`), a
    /// viewer of the file in `from`'s tab moves to them and marks them, focused unless `reuse`.
    pub(super) fn open_file(
        &mut self,
        path: String,
        axis: Axis,
        beside: Option<PaneId>,
        reuse: bool,
        at: Option<Lines>,
        effects: &mut Vec<Effect>,
    ) {
        // The user's is said on the bar; a client's request is answered by the shell (`control`).
        if !viewer::sound(&path) {
            if self.by_user {
                effects.push(Effect::Notice { text: NOT_SOUND.into() });
            }
            return;
        }
        if beside.is_some_and(|id| self.pane(id).is_none()) {
            return;
        }
        // Go to ranks it first next time.
        self.recent_files.retain(|seen| *seen != path);
        self.recent_files.insert(0, path.clone());
        self.recent_files.truncate(crate::goto::MOST_RECENT);
        let from = beside.or(self.focused);
        // A split goes into the tab of the pane it is beside.
        if let Some(from) = from {
            self.show_tab_of(from, effects);
        }
        // Line 0 is none; a range written backwards, the same lines.
        let at = at.and_then(|at| Lines::new(at.first, at.last));
        // At lines, a viewer of the file first: the opener's viewer is not made a second one.
        if let Some((id, at)) = at.and_then(|at| Some((self.viewer_of(&path, from?)?, at))) {
            self.mark_lines(id, &path, at, effects);
            if !reuse {
                self.focus(id, effects);
            }
            return;
        }
        if let Some(id) = from.filter(|_| reuse).and_then(|from| self.reusable(from)) {
            let same = self.pane(id).is_some_and(|pane| pane.kind == Kind::File { path: path.clone() });
            if !same {
                if let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == id) {
                    pane.kind = Kind::File { path: path.clone() };
                }
                // A mark of the file it showed stands for it no more.
                self.marks.forget_pane(id);
                self.viewers.insert(id, Viewer::at(&path, at, from));
                effects.push(Effect::ViewFile { pane: id });
            }
            return;
        }
        let id = self.open(Kind::File { path: path.clone() }, axis, from, effects);
        let opener = from.filter(|from| reuse && *from != id);
        self.viewers.insert(id, Viewer::at(&path, at, opener));
        effects.push(Effect::ViewFile { pane: id });
        if let Some(from) = opener {
            self.focus(from, effects);
        }
    }

    /// A key in the focused viewer: a count, a move (`10j`, `3G`), or ⌃G.
    pub(super) fn viewer_key(&mut self, key: &str, effects: &mut Vec<Effect>) {
        let Some(id) = self.keyboard_viewer() else { return };
        let Some(view) = self.viewers.get_mut(&id) else { return };
        match view.pending.press(key) {
            Press::Move { motion, count } => {
                let lines = match view.read {
                    Read::Shown { lines } => lines.saturating_sub(1),
                    _ => 0,
                };
                let top = vim::moved(motion, count, view.top, view.most.unwrap_or(lines), view.page);
                if top != view.top {
                    view.top = top;
                    effects.push(Effect::ViewerScroll { pane: id, top });
                }
            }
            Press::Unbound if key == "ctrl-g" => self.viewer_edit(Some(id), effects),
            // Esc with nothing typed: the marked lines are marked no more.
            Press::Unbound if key == "escape" => view.marked = None,
            Press::Pending | Press::Cleared | Press::Unbound => {}
        }
    }

    /// The shell drew viewer `pane`: where it is, and how far it goes.
    pub(super) fn viewer_scrolled(&mut self, pane: PaneId, top: u32, most: u32, page: u32) {
        if let Some(view) = self.viewers.get_mut(&pane) {
            (view.top, view.most, view.page) = (top.min(most), Some(most), page);
        }
    }

    /// What the shell read of `path`, if viewer `pane` still shows it.
    pub(super) fn viewer_read(&mut self, pane: PaneId, path: &str, read: Read) {
        let shows = self.pane(pane).is_some_and(|open| open.kind == Kind::File { path: path.to_string() });
        if let Some(view) = self.viewers.get_mut(&pane).filter(|_| shows) {
            if let Read::Shown { lines } = read {
                view.marked = view.marked.and_then(|at| at.within(lines));
            }
            // Read again as anything else (another size, no picture): fitted again.
            if view.read != read {
                (view.zoom, view.pan) = (None, [0., 0.]);
            }
            view.read = read;
        }
    }

    /// The picture in viewer `pane` (default: the focused pane) zoomed by `step` from its fit;
    /// fitted again, it is centred.
    pub(super) fn viewer_zoom(&mut self, pane: Option<PaneId>, step: viewer::ZoomStep, fit: f32) {
        let Some(view) = pane.or(self.focused).and_then(|id| self.viewers.get_mut(&id)) else { return };
        if !matches!(view.read, Read::Picture { .. }) {
            return;
        }
        let (was, zoom) = (view.zoom.unwrap_or(fit), viewer::zoomed(view.zoom, fit, step));
        // About the pane's centre: the point there stays there.
        let by = zoom.map_or(0., |zoom| zoom / was);
        let by = if by.is_finite() { by } else { 0. };
        view.pan = view.pan.map(|at| (at * by).clamp(-viewer::MAX_PAN, viewer::MAX_PAN));
        view.zoom = zoom;
    }

    /// A zoomed picture moved by `by` points. Fitted, it stays centred; never further than
    /// `viewer::MAX_PAN` either way (the shell keeps it on screen as it draws).
    pub(super) fn viewer_pan(&mut self, pane: PaneId, by: [f32; 2]) {
        let Some(view) = self.viewers.get_mut(&pane).filter(|view| view.zoom.is_some()) else { return };
        if by.iter().all(|d| d.is_finite()) {
            let moved = |at: f32, d: f32| (at + d).clamp(-viewer::MAX_PAN, viewer::MAX_PAN);
            view.pan = [moved(view.pan[0], by[0]), moved(view.pan[1], by[1])];
        }
    }

    /// An animation's play or pause (default: the focused pane's).
    pub(super) fn viewer_play(&mut self, pane: Option<PaneId>, play: bool) {
        let Some(view) = pane.or(self.focused).and_then(|id| self.viewers.get_mut(&id)) else { return };
        if matches!(view.read, Read::Picture { frames, .. } if frames > 1) {
            view.playing = Some(play);
        }
    }

    /// ⌃G or Edit: a terminal beside the viewer runs the user's editor on its file.
    pub(super) fn viewer_edit(&mut self, pane: Option<PaneId>, effects: &mut Vec<Effect>) {
        let Some(id) = pane.or(self.focused) else { return };
        let Some(Kind::File { path }) = self.pane(id).map(|pane| pane.kind.clone()) else { return };
        self.show_tab_of(id, effects);
        let terminal = self.open(Kind::Terminal, Axis::Right, Some(id), effects);
        effects.push(Effect::EditFile { pane: terminal, path });
    }

    /// After every step: one view for each viewer pane, read when it is new (a restored or
    /// split one); none for another pane. An opener that closed opens none; a count typed in a
    /// viewer that lost the keyboard is dropped.
    pub(super) fn settle_viewers(&mut self, effects: &mut Vec<Effect>) {
        let files: Vec<(PaneId, String)> = self
            .panes
            .iter()
            .filter_map(|pane| match &pane.kind {
                Kind::File { path } => Some((pane.id, path.clone())),
                _ => None,
            })
            .collect();
        self.viewers.retain(|id, _| files.iter().any(|(file, _)| file == id));
        for (id, path) in &files {
            if !self.viewers.contains_key(id) {
                self.viewers.insert(*id, Viewer::new(path, 0, None));
                effects.push(Effect::ViewFile { pane: *id });
            }
        }
        let (keyboard, panes) = (self.keyboard_viewer(), self.panes.iter().map(|pane| pane.id).collect::<Vec<_>>());
        for (id, view) in &mut self.viewers {
            view.opener = view.opener.filter(|opener| panes.contains(opener));
            if keyboard != Some(*id) {
                view.pending = vim::Keys::default();
            }
        }
    }

    /// A restored viewer: its file, at its line, read again.
    pub(crate) fn restore_viewer(&mut self, pane: PaneId, path: &str, top: u32, effects: &mut Vec<Effect>) {
        if let Some(open) = self.panes.iter_mut().find(|open| open.id == pane) {
            open.kind = Kind::File { path: path.to_string() };
            self.viewers.insert(pane, Viewer::new(path, top, None));
            effects.push(Effect::Opened(pane));
            effects.push(Effect::ViewFile { pane });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Action, Origin};
    use crate::viewer::Shows;
    use crate::viewer::ZoomStep;

    const DOC: &str = "/w/app/README.md";

    fn open(m: &mut Model, path: &str, beside: Option<PaneId>, reuse: bool) -> Vec<Effect> {
        m.update(Action::OpenFile { path: path.into(), axis: Axis::Right, beside, reuse, at: None })
    }

    fn file(m: &Model, pane: PaneId) -> Option<String> {
        match &m.pane(pane)?.kind {
            Kind::File { path } => Some(path.clone()),
            _ => None,
        }
    }

    fn read(m: &mut Model, pane: PaneId, path: &str, lines: u32) {
        m.update(Action::ViewerRead { pane, path: path.into(), read: Read::Shown { lines } }.sent_by(Origin::System));
    }

    fn keys(m: &mut Model, keys: &[&str]) -> Vec<Effect> {
        keys.iter().flat_map(|key| m.update(Action::ViewerKey { key: key.to_string() })).collect()
    }

    #[test]
    fn a_file_opens_in_a_viewer_as_its_name_says() {
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Axis::Right, beside: None });
        let effects = open(&mut m, DOC, None, false);
        let id = m.focused.unwrap_or_default();
        assert_eq!((file(&m, id).as_deref(), m.panes.len()), (Some(DOC), 2));
        assert!(effects.contains(&Effect::Opened(id)) && effects.contains(&Effect::ViewFile { pane: id }));
        assert_eq!(m.viewer(id).map(|v| (v.shows, v.top, v.read.clone())), Some((Shows::Markdown, 0, Read::Reading)));
        open(&mut m, "/w/app/shot.png", None, false);
        assert_eq!(m.focused.and_then(|id| m.viewer(id)).map(|v| v.shows), Some(Shows::Image));
        // Only an absolute path, and only beside a pane that is there.
        let refused = open(&mut m, "README.md", None, false);
        assert_eq!((refused, m.panes.len()), (vec![Effect::Notice { text: NOT_SOUND.into() }], 3));
        assert_eq!((open(&mut m, DOC, Some(99), false), m.panes.len()), (vec![], 3));
    }

    /// The explorer's rule: a pane's next file goes to the viewer it opened, in its tab, and the
    /// keyboard stays on it. Another pane's viewer, or one moved to another tab, is not reused.
    #[test]
    fn the_viewer_beside_is_reused_by_the_pane_that_opened_it() {
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Axis::Right, beside: None });
        let opener = m.focused.unwrap_or_default();
        open(&mut m, DOC, Some(opener), true);
        let viewer = m.panes.last().map(|pane| pane.id).unwrap_or_default();
        assert_eq!(
            (m.focused, m.panes.len(), m.viewer(viewer).and_then(|v| v.opener)),
            (Some(opener), 2, Some(opener))
        );
        m.update(Action::ViewerScrolled { pane: viewer, top: 30, most: 90, page: 20 }.sent_by(Origin::System));
        let effects = open(&mut m, "/w/app/src/main.rs", Some(opener), true);
        assert_eq!((m.panes.len(), file(&m, viewer).as_deref()), (2, Some("/w/app/src/main.rs")));
        assert_eq!(
            (effects, m.viewer(viewer).map(|v| (v.top, v.shows))),
            (vec![Effect::ViewFile { pane: viewer }], Some((0, Shows::Code)))
        );
        // The same file again changes nothing.
        assert_eq!(open(&mut m, "/w/app/src/main.rs", Some(opener), true), vec![]);
        // Without `reuse`, or from another pane: a viewer of its own.
        open(&mut m, DOC, Some(opener), false);
        assert_eq!(m.panes.len(), 3);
        m.update(Action::MovePane { pane: Some(viewer), tab: 9 });
        m.update(Action::NewTab);
        m.update(Action::MovePane { pane: Some(viewer), tab: 1 });
        open(&mut m, DOC, Some(opener), true);
        assert_eq!(m.panes.len(), 4, "the viewer it opened is in another tab now");
        // Its opener closed: the viewer is no one's to reuse.
        m.update(Action::Close { pane: Some(opener) });
        assert!(m.viewers.values().all(|v| v.opener.is_none()));
    }

    #[test]
    fn keys_move_the_view_by_a_count_and_the_status_bar_shows_it_typed() {
        let mut m = Model::default();
        open(&mut m, "/w/app/src/main.rs", None, false);
        let id = m.focused.unwrap_or_default();
        read(&mut m, id, "/w/app/src/main.rs", 200);
        assert_eq!(keys(&mut m, &["1", "0"]), vec![]);
        assert_eq!(m.viewer_pending().as_deref(), Some("10"));
        assert_eq!(keys(&mut m, &["j"]), vec![Effect::ViewerScroll { pane: id, top: 10 }]);
        assert_eq!(m.viewer_pending(), None);
        // Until the shell has laid it out, the last line is as far as it goes.
        keys(&mut m, &["G"]);
        assert_eq!(m.viewer(id).map(|v| v.top), Some(199));
        m.update(Action::ViewerScrolled { pane: id, top: 199, most: 160, page: 40 }.sent_by(Origin::System));
        assert_eq!(m.viewer(id).map(|v| v.top), Some(160), "laid out, it is no further than it goes");
        assert_eq!(keys(&mut m, &["3", "G"]), vec![Effect::ViewerScroll { pane: id, top: 2 }]);
        assert_eq!(keys(&mut m, &["ctrl-d"]), vec![Effect::ViewerScroll { pane: id, top: 22 }]);
        assert_eq!(keys(&mut m, &["g", "g"]), vec![Effect::ViewerScroll { pane: id, top: 0 }]);
        // Esc takes a count back; a count left typed goes when the viewer loses the keyboard.
        keys(&mut m, &["5", "escape", "j"]);
        assert_eq!(m.viewer(id).map(|v| v.top), Some(1));
        keys(&mut m, &["4"]);
        m.update(Action::Split { axis: Axis::Down, kind: Kind::Terminal });
        assert_eq!(m.viewer(id).and_then(|v| v.pending.pending()), None);
    }

    fn open_at(m: &mut Model, path: &str, beside: Option<PaneId>, reuse: bool, at: (u32, u32)) -> Vec<Effect> {
        let at = Some(Lines { first: at.0, last: at.1 });
        m.update(Action::OpenFile { path: path.into(), axis: Axis::Right, beside, reuse, at })
    }

    fn marked(m: &Model, pane: PaneId) -> Option<(u32, u32)> {
        m.viewer(pane)?.marked.map(|at| (at.first, at.last))
    }

    /// Evidence's rule: a file opened at lines shows them marked, a few lines below the top. Asked
    /// again, the viewer of the file moves and marks the new ones; past the end, the last line.
    #[test]
    fn a_file_opened_at_lines_marks_them_until_asked_for_others_or_esc() {
        const CODE: &str = "/w/app/src/main.rs";
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Axis::Right, beside: None });
        let terminal = m.focused.unwrap_or_default();
        open_at(&mut m, CODE, None, false, (40, 42));
        let id = m.focused.unwrap_or_default();
        assert_eq!((marked(&m, id), m.viewer(id).map(|v| v.top)), (Some((40, 42)), Some(36)));
        read(&mut m, id, CODE, 100);
        m.update(Action::ViewerScrolled { pane: id, top: 36, most: 60, page: 40 }.sent_by(Origin::System));
        // From the terminal again: the same viewer, no new pane, focused.
        m.update(Action::Focus { pane: terminal });
        let effects = open_at(&mut m, CODE, Some(terminal), false, (90, 80));
        assert_eq!((m.panes.len(), m.focused, marked(&m, id)), (2, Some(id), Some((80, 90))));
        assert!(effects.contains(&Effect::ViewerScroll { pane: id, top: 60 }), "no further than it goes: {effects:?}");
        // Past the end: the last line. Line 0 is none: a new viewer, as with no lines.
        open_at(&mut m, CODE, Some(id), false, (500, 600));
        assert_eq!(marked(&m, id), Some((100, 100)));
        assert_eq!(
            open_at(&mut m, CODE, Some(id), false, (0, 0)),
            vec![Effect::Opened(3), Effect::ViewFile { pane: 3 }]
        );
        assert_eq!(marked(&m, 3), None);
        m.update(Action::Close { pane: Some(3) });
        // The file shorter now: the mark is cut to it.
        read(&mut m, id, CODE, 50);
        assert_eq!(marked(&m, id), Some((50, 50)));
        // Esc with nothing typed takes the mark off, and is the viewer's only while there is one.
        m.update(Action::Focus { pane: id });
        assert!(m.viewer_takes("escape"));
        keys(&mut m, &["escape"]);
        assert_eq!((marked(&m, id), m.viewer_takes("escape")), (None, false));
        // Drawn markdown at a line: its source, read again.
        open(&mut m, DOC, Some(terminal), true);
        let doc = m.panes.last().map(|pane| pane.id).unwrap_or_default();
        read(&mut m, doc, DOC, 30);
        let effects = open_at(&mut m, DOC, Some(terminal), true, (12, 12));
        assert_eq!(effects, vec![Effect::ViewFile { pane: doc }]);
        let view = m.viewer(doc).map(|v| (v.shows, v.read.clone(), v.marked, v.opener));
        assert_eq!(view, Some((Shows::Code, Read::Reading, Lines::new(12, 12), Some(terminal))));
        assert_eq!(m.focused, Some(terminal), "a reused viewer leaves the keyboard where it was");
        // Another file in the reused viewer: no mark of the last one.
        open(&mut m, CODE, Some(terminal), true);
        assert_eq!(marked(&m, doc), None);
        // At lines of a file another viewer shows: that one, not the terminal's own made a second.
        let panes = m.panes.len();
        open_at(&mut m, CODE, Some(terminal), true, (5, 5));
        assert_eq!((m.panes.len(), file(&m, doc).as_deref(), marked(&m, id)), (panes, Some(CODE), Some((5, 5))));
    }

    #[test]
    fn ctrl_g_opens_the_users_editor_on_the_file_in_a_terminal_beside() {
        let mut m = Model::default();
        open(&mut m, DOC, None, false);
        let id = m.focused.unwrap_or_default();
        let effects = keys(&mut m, &["ctrl-g"]);
        let terminal = m.focused.unwrap_or_default();
        assert_ne!(terminal, id);
        assert_eq!(m.pane(terminal).map(|p| p.kind.clone()), Some(Kind::Terminal));
        assert!(effects.contains(&Effect::EditFile { pane: terminal, path: DOC.into() }), "{effects:?}");
        assert_eq!(m.tab_of(terminal), m.tab_of(id));
        // On a pane that is no viewer: nothing.
        assert_eq!(m.update(Action::ViewerEdit { pane: Some(terminal) }), vec![]);
    }

    #[test]
    fn only_the_shell_says_what_it_read_and_drew_and_only_of_the_file_shown() {
        let mut m = Model::default();
        open(&mut m, DOC, None, false);
        let id = m.focused.unwrap_or_default();
        let shown = Read::Shown { lines: 9 };
        for action in [
            Action::ViewerRead { pane: id, path: DOC.into(), read: shown.clone() },
            Action::ViewerScrolled { pane: id, top: 3, most: 3, page: 3 },
        ] {
            assert_eq!(m.refusal(&action, Origin::Socket), Some(SHELLS_ALONE));
        }
        m.update(
            Action::ViewerRead { pane: id, path: "/w/app/old.md".into(), read: shown.clone() }.sent_by(Origin::System),
        );
        assert_eq!(m.viewer(id).map(|v| v.read.clone()), Some(Read::Reading), "a read of the file it showed before");
        read(&mut m, id, DOC, 9);
        assert_eq!(m.viewer(id).map(|v| v.read.clone()), Some(shown));
    }

    #[test]
    fn a_viewer_is_kept_with_its_line_and_comes_back_reading_its_file() {
        let mut m = Model::default();
        open(&mut m, DOC, None, false);
        let id = m.focused.unwrap_or_default();
        m.update(Action::ViewerScrolled { pane: id, top: 42, most: 100, page: 20 }.sent_by(Origin::System));
        let layout = m.layout_file(&[], &[], 0);
        let kept = &layout.tabs[0].tab.panes[0].kind;
        assert_eq!(kept, &crate::marks::RecordKind::File { path: DOC.into(), top: 42 });
        let text = crate::layout_file::write(&layout);
        let mut r = Model::default();
        let parsed = crate::layout_file::parse(&text).expect("it reads back");
        let effects = r.update(
            Action::RestoreLayout { layout: Box::new(parsed), gone: vec![], home: None, launch: false }
                .sent_by(Origin::System),
        );
        let pane = r.panes.first().map(|pane| pane.id).unwrap_or_default();
        assert_eq!((file(&r, pane).as_deref(), r.viewer(pane).map(|v| v.top)), (Some(DOC), Some(42)));
        assert!(effects.contains(&Effect::ViewFile { pane }), "{effects:?}");
    }

    #[test]
    fn a_picture_zooms_pans_and_plays_and_text_does_none_of_it() {
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Axis::Right, beside: None });
        open(&mut m, "/w/app/spin.gif", None, false);
        let id = m.focused.unwrap_or_default();
        let zoom = |m: &mut Model, step| m.update(Action::ViewerZoom { pane: None, step, fit: 0.5 });
        // Not read yet: nothing.
        zoom(&mut m, ZoomStep::In);
        assert_eq!(m.viewer(id).map(|v| v.zoom), Some(None));
        let picture = Read::Picture { width: 800, height: 600, frames: 12, bytes: 40_000 };
        m.update(
            Action::ViewerRead { pane: id, path: "/w/app/spin.gif".into(), read: picture }.sent_by(Origin::System),
        );
        // Fitted: a pan does nothing. Nearer: it moves. Fitted again: centred.
        m.update(Action::ViewerPan { pane: id, by: [10., 0.] });
        assert_eq!(m.viewer(id).map(|v| v.pan), Some([0., 0.]));
        zoom(&mut m, ZoomStep::In);
        m.update(Action::ViewerPan { pane: id, by: [10., -4.] });
        assert_eq!(m.viewer(id).map(|v| (v.zoom, v.pan)), Some((Some(0.625), [10., -4.])));
        // Farther: the pan comes in with it, about the centre.
        zoom(&mut m, ZoomStep::Out);
        assert_eq!(m.viewer(id).map(|v| (v.zoom, v.pan)), Some((Some(0.5), [8., -3.2])));
        zoom(&mut m, ZoomStep::Fit);
        assert_eq!(m.viewer(id).map(|v| (v.zoom, v.pan)), Some((None, [0., 0.])));
        // Read again at another size: fitted again.
        zoom(&mut m, ZoomStep::In);
        let other = Read::Picture { width: 80, height: 60, frames: 12, bytes: 900 };
        m.update(Action::ViewerRead { pane: id, path: "/w/app/spin.gif".into(), read: other }.sent_by(Origin::System));
        assert_eq!(m.viewer(id).map(|v| v.zoom), Some(None));
        // Its frames: paused, played.
        m.update(Action::ViewerPlay { pane: None, play: false });
        assert_eq!(m.viewer(id).and_then(|v| v.playing), Some(false));
        // A text file: none of it.
        open(&mut m, DOC, None, false);
        let doc = m.focused.unwrap_or_default();
        read(&mut m, doc, DOC, 10);
        zoom(&mut m, ZoomStep::In);
        m.update(Action::ViewerPlay { pane: None, play: true });
        assert_eq!(m.viewer(doc).map(|v| (v.zoom, v.playing)), Some((None, None)));
    }
}
