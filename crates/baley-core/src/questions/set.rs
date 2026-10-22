//! The judgement of one question set (design 0005 PLN-R23).
//!
//! A set arrives as plain JSON and is judged whole: its grammar over the
//! whole list first, then each question in submitted order, then cycles
//! over the whole set, so one set always gets one answer. The same judge
//! serves an analyzer's return and a plan's `questions` field. Nothing here
//! records anything.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::capture::INLINE_TEXT_LIMIT;

/// The refusal code for a set that is not a list of questions in the
/// grammar: an entry that is not an object, a missing, null, wrong-type or
/// unknown member, a `depends_on` that is not a list of distinct ids, or a
/// text over 4,096 bytes.
pub const TYPED_CONTENT: &str = "typed-content";
/// The refusal code for a blank id or one an earlier question already has.
pub const QUESTION_ID: &str = "question-id";
/// The refusal code for a blank question or recommended answer.
pub const QUESTION_BLANK: &str = "question-blank";
/// The refusal code for a dependency on an id outside the set or on the
/// question itself.
pub const QUESTION_DEPENDENCY: &str = "question-dependency";
/// The refusal code for questions that depend on each other in a cycle.
pub const QUESTION_CYCLE: &str = "question-cycle";

const MEMBERS: &[&str] = &["id", "question", "recommended", "depends_on"];

/// Why a question set is refused. Positions count from 0 in the list as
/// submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetRefusal {
    /// The question at `position` breaks the grammar, or the set is not a
    /// list when `position` is `None`.
    TypedContent { position: Option<usize> },
    /// The question at `position` has a blank id, or `id` is an earlier
    /// question's.
    QuestionId { position: usize, id: String },
    /// The question at `position` has a blank question or recommended
    /// answer.
    QuestionBlank { position: usize, id: String },
    /// The question at `position` depends on an id the set lacks or on
    /// itself.
    QuestionDependency { position: usize, id: String },
    /// Every question on a dependency cycle, as position and id, in
    /// submitted order.
    QuestionCycle { questions: Vec<(usize, String)> },
}

impl SetRefusal {
    /// The refusal's public code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::TypedContent { .. } => TYPED_CONTENT,
            Self::QuestionId { .. } => QUESTION_ID,
            Self::QuestionBlank { .. } => QUESTION_BLANK,
            Self::QuestionDependency { .. } => QUESTION_DEPENDENCY,
            Self::QuestionCycle { .. } => QUESTION_CYCLE,
        }
    }
}

/// One question, its text kept exactly as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// Its id, unique within its set.
    pub id: String,
    /// The one decision the owner must make.
    pub question: String,
    /// The answer the asker recommends.
    pub recommended: String,
    /// The ids of the questions in the same set whose answers it needs,
    /// as given.
    pub depends_on: Vec<String>,
}

/// A question set judged whole and accepted. Only [`judge_set`] builds
/// one, so nothing unjudged can be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionSet {
    questions: Vec<Question>,
}

impl QuestionSet {
    /// The questions in submitted order.
    pub fn questions(&self) -> &[Question] {
        &self.questions
    }
}

/// Judges one question list given as plain JSON: the grammar over the whole
/// list, then each question in submitted order, then cycles. An empty list
/// is accepted, since an empty set is closed at once.
pub fn judge_set(value: &Value) -> Result<QuestionSet, SetRefusal> {
    let Value::Array(entries) = value else {
        return Err(SetRefusal::TypedContent { position: None });
    };
    let questions = entries
        .iter()
        .enumerate()
        .map(|(position, entry)| {
            grammar(entry).ok_or(SetRefusal::TypedContent {
                position: Some(position),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let ids: HashSet<&str> = questions.iter().map(|q| q.id.as_str()).collect();
    let mut seen = HashSet::new();
    for (position, question) in questions.iter().enumerate() {
        let id = || question.id.clone();
        if question.id.trim().is_empty() || !seen.insert(question.id.as_str()) {
            return Err(SetRefusal::QuestionId { position, id: id() });
        }
        if question.question.trim().is_empty() || question.recommended.trim().is_empty() {
            return Err(SetRefusal::QuestionBlank { position, id: id() });
        }
        if question
            .depends_on
            .iter()
            .any(|on| *on == question.id || !ids.contains(on.as_str()))
        {
            return Err(SetRefusal::QuestionDependency { position, id: id() });
        }
    }
    let on_cycle = on_cycles(&questions);
    if !on_cycle.is_empty() {
        return Err(SetRefusal::QuestionCycle {
            questions: on_cycle
                .into_iter()
                .map(|position| (position, questions[position].id.clone()))
                .collect(),
        });
    }
    Ok(QuestionSet { questions })
}

/// One entry read in the grammar, or `None` when it breaks it.
fn grammar(entry: &Value) -> Option<Question> {
    let Value::Object(object) = entry else {
        return None;
    };
    if object.keys().any(|name| !MEMBERS.contains(&name.as_str())) {
        return None;
    }
    // The bound is on bytes as written, before any trimming.
    let text = |name: &str| match object.get(name) {
        Some(Value::String(text)) if text.len() <= INLINE_TEXT_LIMIT => Some(text.clone()),
        _ => None,
    };
    let id = text("id")?;
    let question = text("question")?;
    let recommended = text("recommended")?;
    let Some(Value::Array(entries)) = object.get("depends_on") else {
        return None;
    };
    let mut seen = HashSet::new();
    let depends_on = entries
        .iter()
        .map(|on| on.as_str().filter(|on| seen.insert(*on)).map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    Some(Question {
        id,
        question,
        recommended,
        depends_on,
    })
}

/// The positions of every question on a dependency cycle, ascending.
///
/// A question is on a cycle when its strongly connected component holds
/// more than one question; self-dependencies are refused before this runs.
/// A question that only depends on a cycle is not on it. Tarjan's
/// algorithm, kept iterative so a long chain in an untrusted list cannot
/// exhaust the stack.
fn on_cycles(questions: &[Question]) -> Vec<usize> {
    let by_id: HashMap<&str, usize> = questions
        .iter()
        .enumerate()
        .map(|(position, question)| (question.id.as_str(), position))
        .collect();
    let edges: Vec<Vec<usize>> = questions
        .iter()
        .map(|question| {
            question
                .depends_on
                .iter()
                .map(|on| by_id[on.as_str()])
                .collect()
        })
        .collect();
    let count = edges.len();
    let mut index = vec![usize::MAX; count];
    let mut low = vec![0; count];
    let mut on_stack = vec![false; count];
    let mut stack = Vec::new();
    let mut next = 0;
    let mut on_cycle = Vec::new();
    for root in 0..count {
        if index[root] != usize::MAX {
            continue;
        }
        let mut calls = vec![(root, 0)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&mut (node, ref mut edge)) = calls.last_mut() {
            if let Some(&to) = edges[node].get(*edge) {
                *edge += 1;
                if index[to] == usize::MAX {
                    index[to] = next;
                    low[to] = next;
                    next += 1;
                    stack.push(to);
                    on_stack[to] = true;
                    calls.push((to, 0));
                } else if on_stack[to] {
                    low[node] = low[node].min(index[to]);
                }
                continue;
            }
            calls.pop();
            if let Some(&(parent, _)) = calls.last() {
                low[parent] = low[parent].min(low[node]);
            }
            if low[node] == index[node] {
                let mut component = Vec::new();
                while let Some(member) = stack.pop() {
                    on_stack[member] = false;
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                if component.len() > 1 {
                    on_cycle.extend(component);
                }
            }
        }
    }
    on_cycle.sort_unstable();
    on_cycle
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from PLN-R23, the phase's decisions
    //! and the plan's test data, never from running this code.

    use serde_json::json;

    use super::*;

    /// Set Q1 as the plan writes it out.
    fn q1() -> Value {
        json!([
            {"id": "q1", "question": "Which store backs the ledger?", "recommended": "SQLite",
                "depends_on": []},
            {"id": "q2", "question": "Where does the ledger live?",
                "recommended": "Outside the checkout", "depends_on": ["q1"]}
        ])
    }

    /// Q1 with the member at `pointer` set to `to`.
    fn q1_with(pointer: &str, to: Value) -> Value {
        let mut value = q1();
        *value.pointer_mut(pointer).expect("pointer into Q1") = to;
        value
    }

    /// Q1 with the member `member` of question `at` removed.
    fn q1_without(at: usize, member: &str) -> Value {
        let mut value = q1();
        value[at].as_object_mut().unwrap().remove(member);
        value
    }

    /// A question with a made-up text and recommendation.
    fn question(id: &str, depends_on: &[&str]) -> Value {
        json!({"id": id, "question": format!("Decide {id}?"), "recommended": "Yes",
            "depends_on": depends_on})
    }

    fn refusal(value: &Value) -> SetRefusal {
        judge_set(value).expect_err("refused")
    }

    fn typed(position: usize) -> SetRefusal {
        SetRefusal::TypedContent {
            position: Some(position),
        }
    }

    fn cycle(questions: &[(usize, &str)]) -> SetRefusal {
        SetRefusal::QuestionCycle {
            questions: questions
                .iter()
                .map(|(position, id)| (*position, (*id).to_owned()))
                .collect(),
        }
    }

    fn written(id: &str, question: &str, recommended: &str, depends_on: &[&str]) -> Question {
        Question {
            id: id.into(),
            question: question.into(),
            recommended: recommended.into(),
            depends_on: depends_on.iter().map(|on| (*on).to_owned()).collect(),
        }
    }

    #[test]
    fn a_bad_question_set_accepted_or_named_wrongly_is_caught() {
        let accepted = judge_set(&q1()).expect("Q1 is accepted");
        assert_eq!(
            accepted.questions(),
            [
                written("q1", "Which store backs the ledger?", "SQLite", &[]),
                written(
                    "q2",
                    "Where does the ledger live?",
                    "Outside the checkout",
                    &["q1"]
                ),
            ]
        );
        let id = |position: usize, id: &str| SetRefusal::QuestionId {
            position,
            id: id.into(),
        };
        let blank = |position: usize, id: &str| SetRefusal::QuestionBlank {
            position,
            id: id.into(),
        };
        let dependency = |position: usize, id: &str| SetRefusal::QuestionDependency {
            position,
            id: id.into(),
        };
        let mut owner = q1();
        owner[0]
            .as_object_mut()
            .unwrap()
            .insert("owner".into(), json!("Ada"));
        let long = "a".repeat(4_097);
        let cases = [
            (q1_with("/1/id", json!(" ")), id(1, " ")),
            (q1_with("/1/id", json!("q1")), id(1, "q1")),
            (q1_with("/0/question", json!("  ")), blank(0, "q1")),
            (q1_with("/1/recommended", json!("")), blank(1, "q2")),
            (q1_with("/1/depends_on", json!(["q9"])), dependency(1, "q2")),
            (q1_with("/0/depends_on", json!(["q1"])), dependency(0, "q1")),
            (
                q1_with("/0/depends_on", json!(["q2"])),
                cycle(&[(0, "q1"), (1, "q2")]),
            ),
            (owner, typed(0)),
            (q1_without(1, "depends_on"), typed(1)),
            (q1_without(0, "question"), typed(0)),
            (q1_with("/1/depends_on", json!("q1")), typed(1)),
            (
                json!({"q1": q1()[0].clone()}),
                SetRefusal::TypedContent { position: None },
            ),
            (q1_with("/0/question", json!(long)), typed(0)),
            (q1_with("/0/recommended", json!(long)), typed(0)),
            (q1_with("/0/id", json!(long)), typed(0)),
        ];
        for (value, expected) in cases {
            assert_eq!(refusal(&value), expected, "{value}");
        }
    }

    #[test]
    fn an_accepted_set_trimmed_reordered_or_emptied_is_caught() {
        let value = json!([
            {"id": "q2", "question": " Where does the ledger live? ",
                "recommended": "Outside the checkout ", "depends_on": [" q1"]},
            {"id": " q1", "question": "Which store backs the ledger?", "recommended": "SQLite",
                "depends_on": []}
        ]);
        let accepted = judge_set(&value).expect("a forward dependency is accepted");
        assert_eq!(
            accepted.questions(),
            [
                written(
                    "q2",
                    " Where does the ledger live? ",
                    "Outside the checkout ",
                    &[" q1"]
                ),
                written(" q1", "Which store backs the ledger?", "SQLite", &[]),
            ]
        );
        let empty = judge_set(&json!([])).expect("an empty set is accepted");
        assert!(empty.questions().is_empty());
    }

    #[test]
    fn a_text_bound_counted_in_characters_or_off_by_one_is_caught() {
        let at_bound = format!("{}a", "€".repeat(1_365));
        assert_eq!(at_bound.len(), 4_096);
        assert!(judge_set(&q1_with("/0/question", json!(at_bound))).is_ok());
        let over = format!("{at_bound}a");
        assert_eq!(refusal(&q1_with("/0/question", json!(over))), typed(0));
        let padded = format!(" {}", "a".repeat(4_096));
        assert_eq!(refusal(&q1_with("/0/question", json!(padded))), typed(0));
    }

    #[test]
    fn a_question_only_waiting_on_a_cycle_named_on_it_is_caught() {
        let three = json!([
            question("q1", &["q2"]),
            question("q2", &["q3"]),
            question("q3", &["q1"]),
            question("q4", &["q1"]),
        ]);
        assert_eq!(refusal(&three), cycle(&[(0, "q1"), (1, "q2"), (2, "q3")]));
        let two_cycles = json!([
            question("q1", &["q2"]),
            question("q2", &["q1"]),
            question("q3", &["q4"]),
            question("q4", &["q3"]),
        ]);
        assert_eq!(
            refusal(&two_cycles),
            cycle(&[(0, "q1"), (1, "q2"), (2, "q3"), (3, "q4")])
        );
    }

    #[test]
    fn a_repeated_dependency_accepted_is_caught() {
        assert_eq!(
            refusal(&q1_with("/1/depends_on", json!(["q1", "q1"]))),
            typed(1)
        );
        let mut value = q1();
        value.as_array_mut().unwrap().push(json!(
            {"id": "q3", "question": "Who reads the ledger?", "recommended": "The owner",
                "depends_on": ["q1", "q2", "q1"]}
        ));
        assert_eq!(refusal(&value), typed(2));
    }

    #[test]
    fn faults_judged_in_another_order_are_caught() {
        let mut unknown = json!([question(" ", &[]), question("b", &[]), question("c", &[])]);
        unknown[2]
            .as_object_mut()
            .unwrap()
            .insert("owner".into(), json!("Ada"));
        assert_eq!(refusal(&unknown), typed(2));

        let repeated = json!([question("a", &["q9"]), question("a", &[])]);
        assert_eq!(
            refusal(&repeated),
            SetRefusal::QuestionDependency {
                position: 0,
                id: "a".into()
            }
        );

        let mut both = q1_with("/0/id", json!(" "));
        both[0]["question"] = json!("");
        assert_eq!(
            refusal(&both),
            SetRefusal::QuestionId {
                position: 0,
                id: " ".into()
            }
        );
    }

    #[test]
    fn a_long_chain_exhausting_the_stack_is_caught() {
        let count = 50_000;
        let chain: Vec<Value> = (0..count)
            .map(|at| {
                let next = format!("c{}", at + 1);
                let depends_on: Vec<&str> = if at + 1 < count {
                    vec![next.as_str()]
                } else {
                    Vec::new()
                };
                question(&format!("c{at}"), &depends_on)
            })
            .collect();
        let accepted = judge_set(&Value::Array(chain)).expect("a long chain is accepted");
        assert_eq!(accepted.questions().len(), count);
    }
}
