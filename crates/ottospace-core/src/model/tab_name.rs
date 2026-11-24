//! A tab's own name: typed in the tab (Rename in its right click's menu, a double-click on its
//! title, Rename Tab), it wins over the automatic title from then on; cleared, the automatic one
//! shows again.

use super::{HOME_TAB, Model, Owner, Tab};
use crate::edit::{Edit, Field};
use crate::marks::{self, MAX_NAME, MarkInput};

/// Why a client's rename of a tab is refused.
pub(crate) const USERS_ALONE: &str = "only the user names a tab";

impl Tab {
    /// The title ottospace gives it, whether or not the user's name hides it.
    pub(crate) fn auto_title(&self) -> &str {
        self.auto.as_deref().unwrap_or(&self.title)
    }

    /// Sets the automatic title: shown unless the user named the tab, else kept under their name.
    pub(crate) fn set_auto(&mut self, title: String) {
        match &mut self.auto {
            Some(auto) => *auto = title,
            None => self.title = title,
        }
    }

    /// The user's name, cleaned (one line, trimmed, at most `MAX_NAME`): empty gives the
    /// automatic title back.
    pub(crate) fn rename(&mut self, name: &str) {
        let name = marks::clean(name);
        match (name.is_empty(), self.auto.take()) {
            (true, Some(auto)) => self.title = auto,
            (true, None) => {}
            (false, auto) => {
                self.auto = Some(auto.unwrap_or_else(|| std::mem::take(&mut self.title)));
                self.title = name;
            }
        }
    }
}

impl Model {
    /// Starts typing tab `tab`'s name (default: the tab on show) in the tab, all selected. Not
    /// Home's, nor Decisions', whose title says how far through its cards.
    pub(crate) fn tab_edit(&mut self, tab: Option<u32>) {
        let id = tab.unwrap_or(self.tabs[self.main_at()].id);
        let Some(found) = self.tabs.iter().find(|open| open.id == id) else { return };
        if id == HOME_TAB || found.space == Owner::Decisions || !self.mark_input_may_start() {
            return;
        }
        let mut text = Field::new(&found.title, MAX_NAME);
        text.apply(Edit::SelectAll);
        self.mark_input = Some(MarkInput::Tab { tab: id, text });
    }

    /// A name typed for a tab that closed meanwhile: the typing ends.
    pub(crate) fn settle_tab_name(&mut self) {
        if let Some(MarkInput::Tab { tab, .. }) = &self.mark_input
            && !self.tabs.iter().any(|open| open.id == *tab)
        {
            self.mark_input = None;
        }
    }

    /// Keeps a typed name for tab `id` (`Tab::rename`).
    pub(crate) fn tab_rename(&mut self, id: u32, name: &str) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
            tab.rename(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::edit::Edit;
    use crate::layout::Axis;
    use crate::model::{Action, Kind, Mode, Model, Origin};

    fn typed(m: &mut Model, text: &str) {
        m.update(Action::FieldEdit { edit: Edit::SelectAll });
        m.update(Action::FieldEdit { edit: Edit::Insert(text.into()) });
    }

    fn title(m: &Model) -> &str {
        &m.tabs[m.active].title
    }

    #[test]
    fn a_typed_name_wins_and_an_empty_one_gives_the_automatic_title_back() {
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Axis::Right, beside: None });
        let auto = title(&m).to_string();
        m.update(Action::TabEdit { tab: None });
        assert_eq!(
            (m.mode(), m.field().map(|(of, f)| (of, f.text().to_string()))),
            (Mode::Mark, Some(("tab", auto.clone())))
        );
        typed(&mut m, "  the plan\n ");
        m.update(Action::MarkSubmit);
        assert_eq!((title(&m), m.mode(), m.tabs[m.active].auto.is_some()), ("the plan", Mode::Normal, true));
        // Esc keeps it as it was.
        m.update(Action::TabEdit { tab: None });
        typed(&mut m, "other");
        m.update(Action::MarkCancel);
        assert_eq!(title(&m), "the plan");
        // Empty: the automatic title again.
        m.update(Action::TabEdit { tab: None });
        typed(&mut m, "   ");
        m.update(Action::MarkSubmit);
        assert_eq!((title(&m), m.tabs[m.active].auto.is_some()), (auto.as_str(), false));
    }

    #[test]
    fn a_name_is_one_line_and_capped() {
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Axis::Right, beside: None });
        let id = m.tabs[m.active].id;
        m.tab_rename(id, &format!("a\tb{}", "x".repeat(100)));
        assert_eq!(title(&m).chars().count(), crate::marks::MAX_NAME);
        assert!(title(&m).starts_with("ab") && !title(&m).contains('\t'));
    }

    #[test]
    fn an_automatic_title_goes_under_the_users_name() {
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Axis::Right, beside: None });
        let id = m.tabs[m.active].id;
        m.tab_rename(id, "mine");
        m.tabs[m.active].set_auto("app".into());
        assert_eq!((title(&m), m.tabs[m.active].auto_title()), ("mine", "app"));
        m.tab_rename(id, "");
        assert_eq!(title(&m), "app");
    }

    #[test]
    fn only_the_user_names_a_tab() {
        let mut m = Model::default();
        m.update(Action::Open { kind: Kind::Terminal, axis: Axis::Right, beside: None });
        let before = title(&m).to_string();
        m.update(Action::TabEdit { tab: None }.sent_by(Origin::Socket));
        assert_eq!((m.mode(), m.mark_input.is_none()), (Mode::Normal, true));
        m.update(Action::TabEdit { tab: None });
        typed(&mut m, "x");
        m.update(Action::MarkSubmit.sent_by(Origin::Socket));
        m.update(Action::MarkCancel);
        assert_eq!(title(&m), before);
        // Home's tab has no name to type.
        let mut home = Model::default();
        home.update(Action::TabEdit { tab: Some(crate::model::HOME_TAB) });
        assert!(home.mark_input.is_none());
    }
}
