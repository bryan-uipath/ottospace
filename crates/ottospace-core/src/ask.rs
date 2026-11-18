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
    /// ↑/↓, j/k: a row up or down, through every question's rows.
    Move { down: bool },
    /// Space: the cursor's row picked; in a multi-select question, toggled.
    Toggle,
    /// A number key, 1 to 9: that row of the cursor's question, as space.
    Number { n: usize },
    /// A click on a row: the cursor there, as space.
    Row { question: usize, row: usize },
    /// ↵: the answers go, once each question has one.
    Submit,
    /// Esc: the card is declined; the agent goes on.
    Decline,
}

/// One question's answer: the options picked, by index, and the user's own words.
#[derive(Clone, PartialEq, Eq)]
pub struct Chosen {
    pub options: Vec<usize>,
    pub other: Option<String>,
}

impl fmt::Debug for Chosen {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let other = self.other.as_ref().map(String::len);
        write!(f, "Chosen({:?}, other: {other:?} bytes)", self.options)
    }
}

/// The card's state: its questions, the cursor and what is picked.
#[derive(Clone, PartialEq)]
pub struct Form {
    pub questions: Vec<Question>,
    /// (question, row). A question's last row, `options.len()`, is "Other…".
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

    pub fn rows(&self, question: usize) -> usize {
        self.questions.get(question).map_or(0, |q| q.options.len() + 1)
    }

    pub fn picked(&self, question: usize, row: usize) -> bool {
        self.picked.get(question).and_then(|rows| rows.get(row)).copied().unwrap_or(false)
    }

    /// A question's "Other…" words.
    pub fn other(&self, question: usize) -> Option<&Field> {
        self.other.get(question)
    }

    /// The cursor is on an "Other…" row: typing goes there.
    pub fn on_other(&self) -> bool {
        let (q, r) = self.cursor;
        r + 1 == self.rows(q)
    }

    /// The "Other…" words under the cursor, to type in.
    pub fn typing(&mut self) -> Option<&mut Field> {
        let q = self.cursor.0;
        self.on_other().then(|| self.other.get_mut(q)).flatten()
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

    /// Moves or picks; `Submit` and `Decline` are the card's (`submit`), and do nothing here.
    pub fn act(&mut self, act: Act) {
        let (q, r) = self.cursor;
        match act {
            Act::Move { down: true } if r + 1 < self.rows(q) => self.cursor = (q, r + 1),
            Act::Move { down: true } if q + 1 < self.questions.len() => self.cursor = (q + 1, 0),
            Act::Move { down: false } if r > 0 => self.cursor = (q, r - 1),
            Act::Move { down: false } if q > 0 => self.cursor = (q - 1, self.rows(q - 1) - 1),
            Act::Toggle => self.pick(q, r),
            Act::Number { n } if (1..=self.rows(q)).contains(&n) => {
                self.cursor = (q, n - 1);
                self.pick(q, n - 1);
            }
            Act::Row { question, row } if row < self.rows(question) => {
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

    /// ↵: each question's answer, or `None` with the cursor on the first one unanswered. On a
    /// single-select question with nothing picked, the cursor's row is picked first: ↓ ↵ answers.
    pub fn submit(&mut self) -> Option<Vec<Chosen>> {
        let (q, r) = self.cursor;
        let unpicked = !self.picked[q].contains(&true);
        if !self.questions[q].multi && unpicked && !self.on_other() {
            self.pick(q, r);
        }
        let all: Vec<Chosen> = (0..self.questions.len()).map(|q| self.chosen(q)).collect();
        match all.iter().position(|c| c.options.is_empty() && c.other.is_none()) {
            Some(open) => {
                let other = self.rows(open) - 1;
                self.cursor = (open, if self.picked(open, other) { other } else { 0 });
                None
            }
            None => Some(all),
        }
    }

    /// What the state says of it: counts and indexes, never a word.
    pub fn shape(&self) -> FormShape {
        let questions = self.questions.iter().enumerate().map(|(q, question)| QuestionShape {
            options: question.options.len(),
            multi: question.multi,
            picked: (0..self.rows(q)).filter(|r| self.picked(q, *r)).collect(),
        });
        FormShape { questions: questions.collect(), cursor: [self.cursor.0, self.cursor.1] }
    }
}

/// A question card as the state reply has it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FormShape {
    pub questions: Vec<QuestionShape>,
    /// [question, row]; a question's row `options` is "Other…".
    pub cursor: [usize; 2],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct QuestionShape {
    pub options: usize,
    pub multi: bool,
    /// The rows picked, by index; `options` is "Other…".
    pub picked: Vec<usize>,
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

    /// ↓ goes through every question's rows, "Other…" last in each; it stops at either end.
    #[test]
    fn the_cursor_runs_through_every_row() {
        let mut f = form(&[(2, false), (1, true)]);
        let mut seen = vec![f.cursor];
        for _ in 0..6 {
            f.act(Act::Move { down: true });
            seen.push(f.cursor);
        }
        assert_eq!(seen, [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 1), (1, 1)]);
        assert!(f.on_other());
        for _ in 0..3 {
            f.act(Act::Move { down: false });
        }
        assert_eq!(f.cursor, (0, 1));
    }

    /// One pick in a single-select question, toggles in a multi-select one; a number key picks
    /// in the cursor's question, a click anywhere; a number past the rows does nothing.
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
    }

    /// ↵ on a single-select question with nothing picked picks the cursor's row; with a question
    /// still unanswered it sends nothing and goes there.
    #[test]
    fn submit_waits_for_every_answer() {
        let mut f = form(&[(2, false), (2, true)]);
        f.act(Act::Move { down: true });
        assert_eq!(f.submit(), None);
        assert_eq!((f.cursor, f.shape().questions[0].picked.clone()), ((1, 0), vec![1]));
        assert_eq!(f.submit(), None, "↵ toggles nothing in a multi-select question");
        f.act(Act::Toggle);
        f.act(Act::Move { down: true });
        f.act(Act::Toggle);
        assert_eq!(f.submit(), Some(vec![chosen(&[1], None), chosen(&[0, 1], None)]));
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
        assert_eq!(f.submit(), None, "Other picked, no words");
        assert_eq!(f.cursor, (1, 2));
        f.typing().unwrap().apply(Edit::Insert("huge".into()));
        f.typed();
        f.act(Act::Row { question: 1, row: 1 });
        assert_eq!(f.submit(), Some(vec![chosen(&[], Some("  green ")), chosen(&[1], Some("huge"))]), "as typed");
    }

    /// The state's shape: counts, picks and the cursor, never a word of the card's or the user's.
    #[test]
    fn its_shape_holds_no_words() {
        let mut f = form(&[(2, true)]);
        f.act(Act::Row { question: 0, row: 2 });
        f.typing().unwrap().apply(Edit::Insert("secret words".into()));
        f.typed();
        let shape = serde_json::to_string(&f.shape()).unwrap();
        assert_eq!(shape, r#"{"questions":[{"options":2,"multi":true,"picked":[2]}],"cursor":[0,2]}"#);
        assert!(!format!("{f:?}").contains("secret"));
        assert_eq!(format!("{:?}", chosen(&[1], Some("secret"))), "Chosen([1], other: Some(6) bytes)");
    }
}
