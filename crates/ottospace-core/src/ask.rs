//! A question card: the agent asks the user (`RunnerSpec::structured.asks`, Claude Code's
//! `AskUserQuestion`). Each question has its options and an "Other…" row that takes typed words;
//! one is picked, or several in a multi-select question. The cursor, the picks and what ↵ sends.
//! Pure. The words are `Text` and the typed ones a `Field`: drawn, never in the state.

use std::fmt;

use serde::Serialize;

use crate::edit::Field;
use crate::transcript::Text;

/// The most questions a card takes; a request with more is an ordinary permission card.
pub const MAX_QUESTIONS: usize = 8;
/// The most options a question takes: with "Other…" each row has a number key, 1 to 9.
pub const MAX_OPTIONS: usize = 8;
/// The most characters of an "Other…" answer.
pub const MAX_OTHER: usize = 2000;

/// What a declined question card tells the agent. Fixed words.
pub const DECLINED: &str = "The user declined to answer.";

#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    /// A short tag over it, `Colour`.
    pub header: Text,
    pub question: Text,
    pub options: Vec<Choice>,
    /// Several options may be picked.
    pub multi: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub label: Text,
    pub description: Text,
}

/// The user's act on the card shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// ↑/↓, j/k: a row up or down in the tab on show.
    Move { down: bool },
    /// ⇥ / ⇧⇥: the next or the previous tab, Review last.
    Tab { forward: bool },
    /// A click on a tab: that question, or Review (`questions.len()`).
    Show { tab: usize },
    /// Space: the cursor's row picked; in a multi-select question, toggled. On Review: its question.
    Toggle,
    /// A number key, 1 to 9: that row of the tab on show, as space.
    Number { n: usize },
    /// A click on a row: the cursor there, as space. `question` is the tab: Review's rows are its
    /// questions.
    Row { question: usize, row: usize },
    /// ↵: on a question of several, it is answered and the next tab shows; on Review, or on a
    /// card of one question, the answers go.
    Submit,
    /// Esc: the card is declined; the agent goes on.
    Decline,
}

/// One question's answer: the options picked, by index, and the user's own words. Neither:
/// no answer (left out of what goes back).
#[derive(Clone, PartialEq, Eq)]
pub struct Chosen {
    pub options: Vec<usize>,
    pub other: Option<String>,
}

impl Chosen {
    pub fn is_empty(&self) -> bool {
        self.options.is_empty() && self.other.is_none()
    }
}

impl fmt::Debug for Chosen {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let other = self.other.as_ref().map(String::len);
        write!(f, "Chosen({:?}, other: {other:?} bytes)", self.options)
    }
}

/// The card's state: its questions, the tab on show, the cursor and what is picked.
#[derive(Clone, PartialEq)]
pub struct Form {
    pub questions: Vec<Question>,
    /// (tab, row). A tab is a question, its last row, `options.len()`, "Other…"; with several
    /// questions the tab `questions.len()` is Review, a row per question.
    pub cursor: (usize, usize),
    /// Each question's rows picked, "Other…" last.
    picked: Vec<Vec<bool>>,
    other: Vec<Field>,
}

impl fmt::Debug for Form {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.shape())
    }
}

impl Form {
    /// `None` for no question: there is nothing to answer.
    pub fn new(questions: Vec<Question>) -> Option<Self> {
        if questions.is_empty() {
            return None;
        }
        let picked = questions.iter().map(|q| vec![false; q.options.len() + 1]).collect();
        let other = questions.iter().map(|_| Field::new("", MAX_OTHER)).collect();
        Some(Self { questions, cursor: (0, 0), picked, other })
    }

    /// Its tabs: each question, then Review; none with one question.
    pub fn tabs(&self) -> usize {
        match self.questions.len() {
            1 => 0,
            n => n + 1,
        }
    }

    /// Review is on show.
    pub fn review(&self) -> bool {
        self.cursor.0 == self.questions.len()
    }

    /// The rows of a tab: a question's options and "Other…", or Review's questions.
    pub fn rows(&self, tab: usize) -> usize {
        match self.questions.get(tab) {
            Some(q) => q.options.len() + 1,
            None if tab == self.questions.len() && self.tabs() > 0 => self.questions.len(),
            None => 0,
        }
    }

    pub fn picked(&self, question: usize, row: usize) -> bool {
        self.picked.get(question).and_then(|rows| rows.get(row)).copied().unwrap_or(false)
    }

    /// A question has an answer: an option, or words under "Other…".
    pub fn answered(&self, question: usize) -> bool {
        question < self.questions.len() && !self.chosen(question).is_empty()
    }

    /// How many questions have an answer.
    pub fn answers(&self) -> usize {
        (0..self.questions.len()).filter(|q| self.answered(*q)).count()
    }

    /// A question's "Other…" words.
    pub fn other(&self, question: usize) -> Option<&Field> {
        self.other.get(question)
    }

    /// The cursor is on an "Other…" row: typing goes there.
    pub fn on_other(&self) -> bool {
        let (q, r) = self.cursor;
        !self.review() && r + 1 == self.rows(q)
    }

    /// The "Other…" words under the cursor, to type in.
    pub fn typing(&mut self) -> Option<&mut Field> {
        let q = self.cursor.0;
        self.on_other().then(|| self.other.get_mut(q)).flatten()
    }

    /// `text` as the "Other…" words of the question on show (the first, on Review), picked, the
    /// cursor on it: what the user would type there. Nothing is sent.
    pub fn fill_other(&mut self, text: &str) {
        let q = if self.review() { 0 } else { self.cursor.0 };
        self.cursor = (q, self.rows(q) - 1);
        self.other[q].replace_all(text);
        self.typed();
    }

    /// Words typed under the cursor pick its "Other…" row: in a single-select question, alone.
    pub fn typed(&mut self) {
        let (q, r) = self.cursor;
        if self.on_other() && self.other[q].text().trim().is_empty() {
            return;
        }
        if self.on_other() && !self.picked(q, r) {
            self.pick(q, r);
        }
    }

    /// A tab on show, its cursor on the first row picked, else the first.
    fn show(&mut self, tab: usize) {
        let row = (0..self.rows(tab)).find(|r| tab < self.questions.len() && self.picked(tab, *r));
        self.cursor = (tab, row.unwrap_or(0));
    }

    /// Moves or picks; `Submit` and `Decline` are the card's (`submit`), and do nothing here.
    pub fn act(&mut self, act: Act) {
        let (q, r) = self.cursor;
        let review = self.review();
        match act {
            Act::Move { down: true } if r + 1 < self.rows(q) => self.cursor = (q, r + 1),
            Act::Move { down: false } if r > 0 => self.cursor = (q, r - 1),
            Act::Tab { forward: true } if q + 1 < self.tabs() => self.show(q + 1),
            Act::Tab { forward: false } if q > 0 && self.tabs() > 0 => self.show(q - 1),
            Act::Show { tab } if tab < self.tabs() => self.show(tab),
            // On Review a row is its question: picking it goes back there.
            Act::Toggle if review => self.show(r),
            Act::Toggle => self.pick(q, r),
            Act::Number { n } if review && (1..=self.rows(q)).contains(&n) => self.show(n - 1),
            Act::Number { n } if (1..=self.rows(q)).contains(&n) => {
                self.cursor = (q, n - 1);
                self.pick(q, n - 1);
            }
            Act::Row { question, row } if question == self.questions.len() && row < self.rows(question) => {
                self.show(row)
            }
            Act::Row { question, row } if question < self.questions.len() && row < self.rows(question) => {
                self.cursor = (question, row);
                self.pick(question, row);
            }
            _ => {}
        }
    }

    /// Picks a row; in a multi-select question it toggles it. An "Other…" row with no words is
    /// picked but answers nothing until they are typed.
    fn pick(&mut self, q: usize, r: usize) {
        let multi = self.questions[q].multi;
        let rows = &mut self.picked[q];
        match multi {
            true => rows[r] = !rows[r],
            false => {
                rows.iter_mut().for_each(|row| *row = false);
                rows[r] = true;
            }
        }
    }

    fn chosen(&self, q: usize) -> Chosen {
        let n = self.questions[q].options.len();
        let options = (0..n).filter(|r| self.picked(q, *r)).collect();
        // Blank words answer nothing; others go as typed: ` | ` may be the answer.
        let words = self.other[q].text();
        let other = (self.picked(q, n) && !words.trim().is_empty()).then(|| words.to_string());
        Chosen { options, other }
    }

    /// ↵. On a single-select question with nothing picked, the cursor's row is picked first:
    /// ↓ ↵ answers. One question: its answer, or `None` until it has one. Several: on a
    /// question, the next tab shows and `None`; on Review, every answer, an unanswered one
    /// empty (no answer).
    pub fn submit(&mut self) -> Option<Vec<Chosen>> {
        let (q, r) = self.cursor;
        if !self.review() && !self.questions[q].multi && !self.picked[q].contains(&true) && !self.on_other() {
            self.pick(q, r);
        }
        let all: Vec<Chosen> = (0..self.questions.len()).map(|q| self.chosen(q)).collect();
        match self.tabs() {
            0 if all[0].is_empty() => None,
            0 => Some(all),
            _ if self.review() => Some(all),
            _ => {
                self.show(q + 1);
                None
            }
        }
    }

    /// The answers in a line, as the chat folds the card: `Scope: Untracked files only · Base:
    /// develop`. The words are the agent's and the user's: drawn, never in the state.
    pub fn summary(&self) -> String {
        let answer = |q: usize| format!("{}: {}", self.name(q), self.parts(q).join(", "));
        let answered = (0..self.questions.len()).filter(|q| self.answered(*q));
        answered.map(answer).collect::<Vec<String>>().join(" · ")
    }

    /// What a tab or a Review row calls a question: its header, else `Question 2`.
    pub fn name(&self, question: usize) -> String {
        match self.questions.get(question) {
            Some(q) if !q.header.is_empty() => q.header.text().to_string(),
            _ => format!("Question {}", question + 1),
        }
    }

    /// A question's answer as Send sends it: the labels picked, then the user's words.
    pub fn parts(&self, question: usize) -> Vec<String> {
        if question >= self.questions.len() {
            return Vec::new();
        }
        let chosen = self.chosen(question);
        let labels = chosen.options.iter().map(|o| self.questions[question].options[*o].label.text().to_string());
        labels.chain(chosen.other).collect()
    }

    /// What the state says of it: counts and indexes, never a word.
    pub fn shape(&self) -> FormShape {
        let questions = self.questions.iter().enumerate().map(|(q, question)| QuestionShape {
            options: question.options.len(),
            multi: question.multi,
            picked: (0..self.rows(q)).filter(|r| self.picked(q, *r)).collect(),
            answered: self.answered(q),
        });
        FormShape { questions: questions.collect(), cursor: [self.cursor.0, self.cursor.1], tabs: self.tabs() }
    }
}

/// A question card as the state reply has it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FormShape {
    pub questions: Vec<QuestionShape>,
    /// [tab, row]: a question's row `options` is "Other…"; the tab `questions.len()` is Review.
    pub cursor: [usize; 2],
    /// Each question and Review; 0 with one question.
    pub tabs: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct QuestionShape {
    pub options: usize,
    pub multi: bool,
    /// The rows picked, by index; `options` is "Other…".
    pub picked: Vec<usize>,
    /// It has an answer: an option, or words under "Other…".
    pub answered: bool,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::edit::Edit;

    pub(crate) fn question(options: usize, multi: bool) -> Question {
        let choice = |i: usize| Choice { label: Text::new(&format!("o{i}")), description: Text::default() };
        Question {
            header: Text::new("H"),
            question: Text::new("Q?"),
            options: (0..options).map(choice).collect(),
            multi,
        }
    }

    fn form(questions: &[(usize, bool)]) -> Form {
        Form::new(questions.iter().map(|(n, multi)| question(*n, *multi)).collect()).unwrap()
    }

    fn chosen(options: &[usize], other: Option<&str>) -> Chosen {
        Chosen { options: options.to_vec(), other: other.map(str::to_string) }
    }

    #[test]
    fn no_question_is_no_card() {
        assert_eq!(Form::new(Vec::new()), None);
    }

    /// ↑/↓ stay in the tab on show, "Other…" last; ⇥ and ⇧⇥ go between tabs, Review last; one
    /// question has no tabs.
    #[test]
    fn the_cursor_stays_in_its_tab_and_tabs_go_on_to_review() {
        let mut f = form(&[(2, false), (1, true)]);
        assert_eq!((f.tabs(), f.rows(2)), (3, 2));
        let mut seen = vec![f.cursor];
        for _ in 0..3 {
            f.act(Act::Move { down: true });
            seen.push(f.cursor);
        }
        assert_eq!(seen, [(0, 0), (0, 1), (0, 2), (0, 2)]);
        assert!(f.on_other());
        for (act, at) in [(true, 1), (true, 2), (true, 2), (false, 1), (false, 0), (false, 0)] {
            f.act(Act::Tab { forward: act });
            assert_eq!(f.cursor.0, at);
        }
        f.act(Act::Show { tab: 2 });
        assert!(f.review() && !f.on_other());
        f.act(Act::Show { tab: 3 });
        assert_eq!(f.cursor, (2, 0), "no such tab");
        let mut one = form(&[(2, false)]);
        one.act(Act::Tab { forward: true });
        one.act(Act::Show { tab: 1 });
        assert_eq!((one.tabs(), one.cursor), (0, (0, 0)));
    }

    /// One pick in a single-select question, toggles in a multi-select one; a number key picks
    /// in the tab on show, a click anywhere; a number past the rows does nothing. A tab shows
    /// with its cursor on its first pick.
    #[test]
    fn picks_are_single_or_toggled() {
        let mut f = form(&[(3, false), (3, true)]);
        f.act(Act::Number { n: 2 });
        f.act(Act::Number { n: 3 });
        assert_eq!(f.shape().questions[0].picked, [2]);
        f.act(Act::Number { n: 9 });
        assert_eq!((f.cursor, f.shape().questions[0].picked.clone()), ((0, 2), vec![2]));
        f.act(Act::Row { question: 1, row: 0 });
        f.act(Act::Row { question: 1, row: 2 });
        f.act(Act::Toggle);
        f.act(Act::Row { question: 1, row: 1 });
        assert_eq!(f.shape().questions[1].picked, [0, 1]);
        f.act(Act::Row { question: 5, row: 0 });
        assert_eq!(f.cursor, (1, 1), "no such row");
        f.act(Act::Tab { forward: false });
        assert_eq!(f.cursor, (0, 2));
    }

    /// One question: ↵ sends once it has an answer; on a single-select question with nothing
    /// picked it picks the cursor's row first.
    #[test]
    fn one_question_sends_on_enter() {
        let mut f = form(&[(2, true)]);
        assert_eq!(f.submit(), None, "↵ toggles nothing in a multi-select question");
        f.act(Act::Toggle);
        assert_eq!(f.submit(), Some(vec![chosen(&[0], None)]));
        let mut f = form(&[(2, false)]);
        f.act(Act::Move { down: true });
        assert_eq!(f.submit(), Some(vec![chosen(&[1], None)]));
    }

    /// Several: ↵ answers and the next tab shows, Review after the last; on Review ↵ sends every
    /// answer, one left unanswered empty. A Review row goes back to its question.
    #[test]
    fn several_questions_go_on_to_review_then_send() {
        let mut f = form(&[(2, false), (2, true), (2, false)]);
        f.act(Act::Move { down: true });
        assert_eq!(f.submit(), None);
        assert_eq!((f.cursor, f.answered(0)), ((1, 0), true));
        // Nothing picked in the multi-select one: ↵ goes on, it stays unanswered.
        assert_eq!(f.submit(), None);
        assert_eq!(f.submit(), None);
        assert!(f.review() && !f.answered(1) && f.answered(2));
        assert_eq!(f.answers(), 2);
        f.act(Act::Row { question: 3, row: 1 });
        assert_eq!(f.cursor, (1, 0));
        f.act(Act::Toggle);
        f.act(Act::Show { tab: 3 });
        f.act(Act::Number { n: 3 });
        assert_eq!(f.cursor, (2, 0), "a number on Review goes to that question");
        f.act(Act::Show { tab: 3 });
        let all = vec![chosen(&[1], None), chosen(&[0], None), chosen(&[0], None)];
        assert_eq!(f.submit(), Some(all));
        let mut skipped = form(&[(2, false), (2, true)]);
        skipped.act(Act::Show { tab: 2 });
        assert_eq!(skipped.submit(), Some(vec![chosen(&[], None), chosen(&[], None)]), "no answer is sent as none");
    }

    /// Words typed on "Other…" pick it (alone, in a single-select question); picked with no
    /// words it answers nothing; in a multi-select question it joins the options.
    #[test]
    fn other_takes_typed_words() {
        let mut f = form(&[(2, false), (2, true)]);
        f.act(Act::Number { n: 1 });
        f.act(Act::Move { down: true });
        f.act(Act::Move { down: true });
        assert!(f.on_other());
        f.typing().unwrap().apply(Edit::Insert("  ".into()));
        f.typed();
        assert_eq!(f.shape().questions[0].picked, [0], "blank words pick nothing");
        f.typing().unwrap().apply(Edit::Insert("green ".into()));
        f.typed();
        assert_eq!(f.shape().questions[0].picked, [2]);
        f.act(Act::Row { question: 1, row: 2 });
        assert!(!f.answered(1), "Other picked, no words");
        f.typing().unwrap().apply(Edit::Insert("huge".into()));
        f.typed();
        f.act(Act::Row { question: 1, row: 1 });
        f.act(Act::Show { tab: 2 });
        let all = vec![chosen(&[], Some("  green ")), chosen(&[1], Some("huge"))];
        assert_eq!(f.submit(), Some(all), "as typed");
        assert_eq!(f.summary(), "H:   green  · H: o1, huge");
    }

    /// The state's shape: counts, picks and the cursor, never a word of the card's or the user's.
    #[test]
    fn its_shape_holds_no_words() {
        let mut f = form(&[(2, true)]);
        f.act(Act::Row { question: 0, row: 2 });
        f.typing().unwrap().apply(Edit::Insert("secret words".into()));
        f.typed();
        let shape = serde_json::to_string(&f.shape()).unwrap();
        assert_eq!(
            shape,
            r#"{"questions":[{"options":2,"multi":true,"picked":[2],"answered":true}],"cursor":[0,2],"tabs":0}"#
        );
        assert!(!format!("{f:?}").contains("secret"));
        assert_eq!(format!("{:?}", chosen(&[1], Some("secret"))), "Chosen([1], other: Some(6) bytes)");
    }
}
