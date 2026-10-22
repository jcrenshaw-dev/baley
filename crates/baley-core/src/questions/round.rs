//! A set's round, worked out from plain values (design 0005 PLN-R24 to
//! PLN-R26, Figure 7).
//!
//! The round is a function of the set's questions in submitted order and
//! the answers, deferrals and abandonment recorded on it, never of the
//! order those records are given in: they are taken keyed by question. A
//! question is open until it is answered or deferred, and a deferred
//! dependency never counts as answered.

use std::collections::HashMap;

use serde_json::{Map, Value};

use super::event::Cause;
use super::set::Question;

/// One recorded answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The question answered.
    pub question: String,
    /// The owner's answer, the recommended text when the owner took it.
    pub answer: String,
    /// Whether the owner took the recommended answer.
    pub accepted: bool,
    /// The round it was settled in.
    pub round: u32,
    /// Who settled it.
    pub owner: String,
    /// When it was settled.
    pub at: String,
}

/// One recorded deferral.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deferral {
    /// The question deferred.
    pub question: String,
    /// Why the owner leaves it undecided.
    pub reason: String,
    /// The round it was settled in.
    pub round: u32,
    /// Who settled it.
    pub owner: String,
    /// When it was settled.
    pub at: String,
}

/// A recorded abandonment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Abandonment {
    /// Why the set was abandoned.
    pub cause: Cause,
    /// When it was recorded.
    pub at: String,
}

/// Everything recorded about one set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetFacts {
    /// The questions in submitted order.
    pub questions: Vec<Question>,
    /// The answers recorded, in any order.
    pub answers: Vec<Answer>,
    /// The deferrals recorded, in any order.
    pub deferrals: Vec<Deferral>,
    /// The abandonment, when one is recorded.
    pub abandoned: Option<Abandonment>,
}

/// A set's state, derived when read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetState {
    /// A question is open.
    Open,
    /// No question is open.
    Closed,
    /// A later set or the plan draft's replacement abandoned it.
    Abandoned,
}

impl SetState {
    /// The state as answers spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Abandoned => "abandoned",
        }
    }
}

/// Where a dependency a question waits on stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    /// Neither answered nor deferred.
    Open,
    /// Deferred, so the question waiting on it never enters a round.
    Deferred,
}

/// A question of the current round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    /// Its id.
    pub id: String,
    /// The decision to put to the owner.
    pub question: String,
    /// The answer the asker recommends.
    pub recommended: String,
}

/// An open question that is not in the current round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    /// Its id.
    pub id: String,
    /// The decision it holds.
    pub question: String,
    /// Its direct dependencies that are not answered, in its `depends_on`
    /// order.
    pub on: Vec<(String, Pending)>,
}

/// A set's round as worked out from its facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    /// One more than the highest round any answer or deferral carries, 1
    /// when none does.
    pub number: u32,
    /// The set's state.
    pub state: SetState,
    /// Every open question whose dependencies are all answered, in
    /// submitted order.
    pub current: Vec<Asked>,
    /// Every other open question, in submitted order.
    pub waiting: Vec<Waiting>,
    /// The answers, in submitted order of their questions.
    pub answered: Vec<Answer>,
    /// The deferrals, in submitted order of their questions.
    pub deferred: Vec<Deferral>,
    /// The abandonment, when one is recorded.
    pub abandoned: Option<Abandonment>,
}

/// Works out the round of the set `facts` describes.
pub fn round_of(facts: &SetFacts) -> Round {
    let answers: HashMap<&str, &Answer> = facts
        .answers
        .iter()
        .map(|answer| (answer.question.as_str(), answer))
        .collect();
    let deferrals: HashMap<&str, &Deferral> = facts
        .deferrals
        .iter()
        .map(|deferral| (deferral.question.as_str(), deferral))
        .collect();
    let highest = facts
        .answers
        .iter()
        .map(|answer| answer.round)
        .chain(facts.deferrals.iter().map(|deferral| deferral.round))
        .max()
        .unwrap_or(0);
    let mut round = Round {
        number: highest.saturating_add(1),
        state: SetState::Closed,
        current: Vec::new(),
        waiting: Vec::new(),
        answered: Vec::new(),
        deferred: Vec::new(),
        abandoned: facts.abandoned.clone(),
    };
    for question in &facts.questions {
        let id = question.id.as_str();
        if let Some(answer) = answers.get(id) {
            round.answered.push((*answer).clone());
            continue;
        }
        if let Some(deferral) = deferrals.get(id) {
            round.deferred.push((*deferral).clone());
            continue;
        }
        let on: Vec<(String, Pending)> = question
            .depends_on
            .iter()
            .filter(|on| !answers.contains_key(on.as_str()))
            .map(|on| {
                let pending = if deferrals.contains_key(on.as_str()) {
                    Pending::Deferred
                } else {
                    Pending::Open
                };
                (on.clone(), pending)
            })
            .collect();
        if on.is_empty() {
            round.current.push(Asked {
                id: question.id.clone(),
                question: question.question.clone(),
                recommended: question.recommended.clone(),
            });
        } else {
            round.waiting.push(Waiting {
                id: question.id.clone(),
                question: question.question.clone(),
                on,
            });
        }
    }
    round.state = if round.abandoned.is_some() {
        SetState::Abandoned
    } else if round.current.is_empty() && round.waiting.is_empty() {
        SetState::Closed
    } else {
        SetState::Open
    };
    round
}

/// Why a `questions` view document could not be read into a set's facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableSet(pub String);

/// Reads a `questions` view document, as `QuestionsProjector` builds it,
/// into the facts its round is worked out from.
pub fn set_facts(document: &Value) -> Result<SetFacts, UnreadableSet> {
    let unreadable = |what: &str| UnreadableSet(format!("the set's {what} cannot be read"));
    let document = document.as_object().ok_or_else(|| unreadable("document"))?;
    let entries = |name: &str| {
        document
            .get(name)
            .and_then(Value::as_array)
            .ok_or_else(|| unreadable(name))
    };
    let questions = entries("questions")?
        .iter()
        .map(|entry| question(entry).ok_or_else(|| unreadable("questions")))
        .collect::<Result<_, _>>()?;
    let answers = entries("answers")?
        .iter()
        .map(|entry| answer(entry).ok_or_else(|| unreadable("answers")))
        .collect::<Result<_, _>>()?;
    let deferrals = entries("deferrals")?
        .iter()
        .map(|entry| deferral(entry).ok_or_else(|| unreadable("deferrals")))
        .collect::<Result<_, _>>()?;
    let abandoned = match document.get("abandoned") {
        Some(Value::Null) => None,
        Some(Value::Object(entry)) => Some(
            Cause::parse(text(entry, "cause").unwrap_or_default())
                .zip(text(entry, "at"))
                .map(|(cause, at)| Abandonment {
                    cause,
                    at: at.to_owned(),
                })
                .ok_or_else(|| unreadable("abandonment"))?,
        ),
        _ => return Err(unreadable("abandonment")),
    };
    Ok(SetFacts {
        questions,
        answers,
        deferrals,
        abandoned,
    })
}

fn text<'v>(entry: &'v Map<String, Value>, name: &str) -> Option<&'v str> {
    entry.get(name).and_then(Value::as_str)
}

fn round_number(entry: &Map<String, Value>) -> Option<u32> {
    entry
        .get("round")
        .and_then(Value::as_u64)
        .and_then(|round| u32::try_from(round).ok())
}

fn question(entry: &Value) -> Option<Question> {
    let entry = entry.as_object()?;
    let depends_on = entry
        .get("depends_on")?
        .as_array()?
        .iter()
        .map(|on| on.as_str().map(str::to_owned))
        .collect::<Option<_>>()?;
    Some(Question {
        id: text(entry, "id")?.to_owned(),
        question: text(entry, "question")?.to_owned(),
        recommended: text(entry, "recommended")?.to_owned(),
        depends_on,
    })
}

fn answer(entry: &Value) -> Option<Answer> {
    let entry = entry.as_object()?;
    Some(Answer {
        question: text(entry, "question")?.to_owned(),
        answer: text(entry, "answer")?.to_owned(),
        accepted: entry.get("accepted")?.as_bool()?,
        round: round_number(entry)?,
        owner: text(entry, "owner")?.to_owned(),
        at: text(entry, "at")?.to_owned(),
    })
}

fn deferral(entry: &Value) -> Option<Deferral> {
    let entry = entry.as_object()?;
    Some(Deferral {
        question: text(entry, "question")?.to_owned(),
        reason: text(entry, "reason")?.to_owned(),
        round: round_number(entry)?,
        owner: text(entry, "owner")?.to_owned(),
        at: text(entry, "at")?.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    //! Expected rounds are written out from PLN-R24 to PLN-R26, the phase's
    //! decisions and the plan's set R, never from running this code.

    use serde_json::json;

    use super::*;

    const OWNER: &str = "Ada Lovelace";
    const T1: &str = "2026-10-22T09:00:01Z";
    const T2: &str = "2026-10-22T09:00:02Z";

    fn question(id: &str, text: &str, recommended: &str, depends_on: &[&str]) -> Question {
        Question {
            id: id.into(),
            question: text.into(),
            recommended: recommended.into(),
            depends_on: depends_on.iter().map(|on| (*on).to_owned()).collect(),
        }
    }

    /// Set R as the plan writes it out.
    fn r() -> Vec<Question> {
        vec![
            question("q1", "Which store backs the ledger?", "SQLite", &[]),
            question(
                "q2",
                "Do sessions share one ledger?",
                "Yes, one per user",
                &[],
            ),
            question(
                "q3",
                "Where does the ledger live?",
                "Outside the checkout",
                &["q1", "q2"],
            ),
            question("q4", "Is the ledger encrypted at rest?", "No", &[]),
            question(
                "q5",
                "Who may read the ledger?",
                "Only the owner's account",
                &["q4"],
            ),
        ]
    }

    fn answer(id: &str, text: &str, accepted: bool, round: u32) -> Answer {
        Answer {
            question: id.into(),
            answer: text.into(),
            accepted,
            round,
            owner: OWNER.into(),
            at: T1.into(),
        }
    }

    fn take(id: &str, round: u32) -> Answer {
        let recommended = r()
            .into_iter()
            .find(|question| question.id == id)
            .unwrap()
            .recommended;
        answer(id, &recommended, true, round)
    }

    fn deferral(id: &str, reason: &str, round: u32) -> Deferral {
        Deferral {
            question: id.into(),
            reason: reason.into(),
            round,
            owner: OWNER.into(),
            at: T1.into(),
        }
    }

    fn facts(answers: Vec<Answer>, deferrals: Vec<Deferral>) -> SetFacts {
        SetFacts {
            questions: r(),
            answers,
            deferrals,
            abandoned: None,
        }
    }

    fn ids(asked: &[Asked]) -> Vec<&str> {
        asked.iter().map(|asked| asked.id.as_str()).collect()
    }

    fn waits(round: &Round) -> Vec<(&str, Vec<(&str, Pending)>)> {
        round
            .waiting
            .iter()
            .map(|waiting| {
                (
                    waiting.id.as_str(),
                    waiting
                        .on
                        .iter()
                        .map(|(on, pending)| (on.as_str(), *pending))
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn round_one_holding_a_question_with_a_dependency_is_caught() {
        let round = round_of(&facts(Vec::new(), Vec::new()));
        assert_eq!(round.number, 1);
        assert_eq!(round.state, SetState::Open);
        assert_eq!(
            round.current,
            [
                Asked {
                    id: "q1".into(),
                    question: "Which store backs the ledger?".into(),
                    recommended: "SQLite".into(),
                },
                Asked {
                    id: "q2".into(),
                    question: "Do sessions share one ledger?".into(),
                    recommended: "Yes, one per user".into(),
                },
                Asked {
                    id: "q4".into(),
                    question: "Is the ledger encrypted at rest?".into(),
                    recommended: "No".into(),
                },
            ]
        );
        assert_eq!(
            waits(&round),
            [
                ("q3", vec![("q1", Pending::Open), ("q2", Pending::Open)]),
                ("q5", vec![("q4", Pending::Open)]),
            ]
        );
    }

    #[test]
    fn a_question_entering_before_every_dependency_is_answered_is_caught() {
        let round = round_of(&facts(vec![take("q1", 1)], Vec::new()));
        assert_eq!(round.number, 2);
        assert_eq!(ids(&round.current), ["q2", "q4"]);
        assert_eq!(
            waits(&round),
            [
                ("q3", vec![("q2", Pending::Open)]),
                ("q5", vec![("q4", Pending::Open)]),
            ]
        );
    }

    #[test]
    fn a_question_waiting_on_a_deferred_one_entering_a_round_or_closing_unsettled_is_caught() {
        let answers = vec![take("q1", 1), take("q2", 1), take("q3", 2)];
        let deferred = vec![deferral("q4", "after the security review", 3)];
        let round = round_of(&facts(answers.clone(), deferred.clone()));
        assert_eq!(waits(&round), [("q5", vec![("q4", Pending::Deferred)])]);
        assert!(round.current.is_empty());
        assert_eq!(round.state, SetState::Open);

        let mut all = deferred;
        all.push(deferral("q5", "with q4", 4));
        let round = round_of(&facts(answers, all));
        assert_eq!(round.state, SetState::Closed);
    }

    #[test]
    fn a_round_depending_on_the_order_records_are_given_in_is_caught() {
        let a_answers = vec![take("q1", 1), answer("q2", "No, one per project", false, 2)];
        let a_deferrals = vec![deferral("q4", "after the security review", 3)];
        let reversed = |mut answers: Vec<Answer>, mut deferrals: Vec<Deferral>| {
            answers.reverse();
            deferrals.reverse();
            facts(answers, deferrals)
        };
        assert_eq!(
            round_of(&facts(a_answers.clone(), a_deferrals.clone())),
            round_of(&reversed(a_answers, a_deferrals))
        );

        let tied = vec![take("q1", 1), take("q2", 1)];
        let later = vec![deferral("q4", "after the security review", 2)];
        assert_eq!(
            round_of(&facts(tied.clone(), later.clone())),
            round_of(&reversed(tied, later))
        );
    }

    #[test]
    fn a_waiting_list_naming_a_transitive_dependency_is_caught() {
        let chain = SetFacts {
            questions: vec![
                question("c1", "First?", "Yes", &[]),
                question("c2", "Second?", "Yes", &["c1"]),
                question("c3", "Third?", "Yes", &["c2"]),
            ],
            answers: Vec::new(),
            deferrals: Vec::new(),
            abandoned: None,
        };
        let round = round_of(&chain);
        assert_eq!(
            waits(&round),
            [
                ("c2", vec![("c1", Pending::Open)]),
                ("c3", vec![("c2", Pending::Open)]),
            ]
        );
    }

    #[test]
    fn a_round_number_counted_from_records_instead_of_rounds_is_caught() {
        let two = round_of(&facts(vec![take("q1", 1), take("q2", 1)], Vec::new()));
        assert_eq!(two.number, 2);
        let four = round_of(&facts(
            vec![take("q1", 1), take("q2", 1)],
            vec![deferral("q4", "later", 3)],
        ));
        assert_eq!(four.number, 4);
        assert_eq!(round_of(&facts(Vec::new(), Vec::new())).number, 1);
    }

    #[test]
    fn an_abandoned_set_reported_open_or_without_its_records_is_caught() {
        let superseded = Abandonment {
            cause: Cause::Superseded,
            at: T2.into(),
        };
        let one = SetFacts {
            abandoned: Some(superseded.clone()),
            ..facts(vec![take("q1", 1)], Vec::new())
        };
        let round = round_of(&one);
        assert_eq!(round.state, SetState::Abandoned);
        assert_eq!(round.answered, [take("q1", 1)]);
        assert_eq!(round.abandoned, Some(superseded.clone()));

        let every = SetFacts {
            abandoned: Some(superseded),
            ..facts(
                ["q1", "q2", "q3", "q4", "q5"]
                    .iter()
                    .map(|id| take(id, 1))
                    .collect(),
                Vec::new(),
            )
        };
        assert_eq!(round_of(&every).state, SetState::Abandoned);
    }

    #[test]
    fn an_empty_set_reported_open_is_caught() {
        let round = round_of(&SetFacts {
            questions: Vec::new(),
            answers: Vec::new(),
            deferrals: Vec::new(),
            abandoned: None,
        });
        assert_eq!(round.state, SetState::Closed);
        assert_eq!(round.number, 1);
        assert!(round.current.is_empty());
    }

    #[test]
    fn a_view_document_misread_into_the_round_is_caught() {
        let document = json!({
            "set": "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d",
            "subject": {"kind": "story", "story": 1,
                "work_order": "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e"},
            "stream": "roadmap", "seq": 5, "recorded_at": "2026-10-22T09:00:00Z",
            "questions": [
                {"id": "q1", "question": "Which store backs the ledger?",
                    "recommended": "SQLite", "depends_on": []},
                {"id": "q2", "question": "Do sessions share one ledger?",
                    "recommended": "Yes, one per user", "depends_on": []},
                {"id": "q3", "question": "Where does the ledger live?",
                    "recommended": "Outside the checkout", "depends_on": ["q1", "q2"]},
                {"id": "q4", "question": "Is the ledger encrypted at rest?",
                    "recommended": "No", "depends_on": []},
                {"id": "q5", "question": "Who may read the ledger?",
                    "recommended": "Only the owner's account", "depends_on": ["q4"]}
            ],
            "answers": [{"question": "q1", "answer": "SQLite", "accepted": true, "round": 1,
                "owner": OWNER, "at": T1, "seq": 6}],
            "deferrals": [{"question": "q4", "reason": "after the security review",
                "round": 2, "owner": OWNER, "at": T1, "seq": 7}],
            "abandoned": null
        });
        let plain = facts(
            vec![take("q1", 1)],
            vec![deferral("q4", "after the security review", 2)],
        );
        assert_eq!(set_facts(&document), Ok(plain.clone()));
        assert_eq!(round_of(&set_facts(&document).unwrap()), round_of(&plain));
        for member in ["questions", "answers", "deferrals"] {
            let mut without = document.clone();
            without.as_object_mut().unwrap().remove(member);
            assert!(set_facts(&without).is_err(), "{member}");
        }
    }
}
