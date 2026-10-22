//! The judgement of one answer call (design 0005 `questions answer`,
//! PLN-R24 to PLN-R26).
//!
//! A call is judged in two steps. Its grammar is judged first, over the
//! items as plain JSON and before any set is read. Then the call is judged
//! against the set's facts as recorded before it: the set first, then each
//! item in the call's order through its checks in a fixed order, so one call
//! always gets one answer. The whole call is judged against the round as it
//! stood before the call, so one call is one round. Nothing here records
//! anything.

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::event::{
    QUESTION_ANSWERED, QUESTION_ANSWERED_VERSION, QUESTION_DEFERRED, QUESTION_DEFERRED_VERSION,
    answered_payload, deferred_payload,
};
use super::round::{Answer, Deferral, Round, SetFacts, round_of};
use crate::capture::INLINE_TEXT_LIMIT;

/// The refusal code for a call on a set whose abandonment is recorded.
pub const QUESTION_SET_ABANDONED: &str = "question-set-abandoned";
/// The refusal code for an item naming a question the set lacks.
pub const NO_SUCH_QUESTION: &str = "no-such-question";
/// The refusal code for an item naming a question already answered or
/// deferred.
pub const QUESTION_CLOSED: &str = "question-closed";
/// The refusal code for an answer to a question outside the round as it
/// stood before the call.
pub const QUESTION_NOT_IN_ROUND: &str = "question-not-in-round";
/// The refusal code for an own answer that is missing, empty or only
/// whitespace.
pub const ANSWER_BLANK: &str = "answer-blank";
/// The refusal code for a deferral whose reason is missing, empty or only
/// whitespace.
pub const DEFERRAL_REASON: &str = "deferral-reason";

/// What one item does with its question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// Takes the recommended answer.
    Take,
    /// Gives the owner's own answer, as written; empty when the item gave
    /// none, which the call's judgement refuses.
    Answer(String),
    /// Defers the question for the owner's reason, as written; empty when
    /// the item gave none, which the call's judgement refuses.
    Defer(String),
}

/// One item of a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The question it settles.
    pub question: String,
    /// What it does.
    pub act: Act,
}

/// A call whose grammar is judged and accepted. Only [`judge_call`] builds
/// one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    items: Vec<Item>,
}

impl Call {
    /// The items in the call's order.
    pub fn items(&self) -> &[Item] {
        &self.items
    }
}

/// A call refused `typed-content` for its grammar: the item at `position`,
/// counted from 0, or the call itself when `position` is `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallFault {
    /// The faulty item's position, when the fault is one item's.
    pub position: Option<usize>,
}

/// Why a call is refused against its set. Each item refusal names the
/// question as the item gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerRefusal {
    /// The set's abandonment is recorded.
    SetAbandoned,
    /// The set has no question with this id.
    NoSuchQuestion(String),
    /// The question is already answered or deferred.
    QuestionClosed(String),
    /// The question is not in the round as it stood before the call.
    NotInRound(String),
    /// The own answer is missing, empty or only whitespace.
    AnswerBlank(String),
    /// The deferral's reason is missing, empty or only whitespace.
    DeferralReason(String),
}

impl AnswerRefusal {
    /// The refusal's public code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::SetAbandoned => QUESTION_SET_ABANDONED,
            Self::NoSuchQuestion(_) => NO_SUCH_QUESTION,
            Self::QuestionClosed(_) => QUESTION_CLOSED,
            Self::NotInRound(_) => QUESTION_NOT_IN_ROUND,
            Self::AnswerBlank(_) => ANSWER_BLANK,
            Self::DeferralReason(_) => DEFERRAL_REASON,
        }
    }

    /// The question the refusal names, if it is an item's.
    pub fn question(&self) -> Option<&str> {
        match self {
            Self::SetAbandoned => None,
            Self::NoSuchQuestion(id)
            | Self::QuestionClosed(id)
            | Self::NotInRound(id)
            | Self::AnswerBlank(id)
            | Self::DeferralReason(id) => Some(id),
        }
    }
}

/// How one question is settled by an accepted call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settlement {
    /// Answered with `answer`, `accepted` when the item took the
    /// recommendation.
    Answered {
        question: String,
        answer: String,
        accepted: bool,
    },
    /// Deferred for `reason`.
    Deferred { question: String, reason: String },
}

impl Settlement {
    /// The question settled.
    pub fn question(&self) -> &str {
        match self {
            Self::Answered { question, .. } | Self::Deferred { question, .. } => question,
        }
    }

    /// The event type recording it.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Answered { .. } => QUESTION_ANSWERED,
            Self::Deferred { .. } => QUESTION_DEFERRED,
        }
    }

    /// The payload version of its event type.
    pub fn type_version(&self) -> u32 {
        match self {
            Self::Answered { .. } => QUESTION_ANSWERED_VERSION,
            Self::Deferred { .. } => QUESTION_DEFERRED_VERSION,
        }
    }

    /// Its event's payload, as settled in `round` of set `set` by `owner` at
    /// `at`.
    pub fn payload(&self, set: &str, round: u32, owner: &str, at: &str) -> Value {
        match self {
            Self::Answered {
                question,
                answer,
                accepted,
            } => answered_payload(set, question, round, answer, *accepted, owner, at),
            Self::Deferred { question, reason } => {
                deferred_payload(set, question, round, reason, owner, at)
            }
        }
    }
}

/// A call accepted against its set: what it settles and in which round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    facts: SetFacts,
    round: u32,
    settlements: Vec<Settlement>,
}

impl Settled {
    /// The round every settlement of the call is recorded in.
    pub fn round(&self) -> u32 {
        self.round
    }

    /// The settlements in the call's order.
    pub fn settlements(&self) -> &[Settlement] {
        &self.settlements
    }

    /// The set's round as it stands once the settlements are recorded by
    /// `owner` at `at`.
    pub fn round_after(&self, owner: &str, at: &str) -> Round {
        let mut facts = self.facts.clone();
        for settlement in &self.settlements {
            match settlement {
                Settlement::Answered {
                    question,
                    answer,
                    accepted,
                } => facts.answers.push(Answer {
                    question: question.clone(),
                    answer: answer.clone(),
                    accepted: *accepted,
                    round: self.round,
                    owner: owner.to_owned(),
                    at: at.to_owned(),
                }),
                Settlement::Deferred { question, reason } => facts.deferrals.push(Deferral {
                    question: question.clone(),
                    reason: reason.clone(),
                    round: self.round,
                    owner: owner.to_owned(),
                    at: at.to_owned(),
                }),
            }
        }
        round_of(&facts)
    }
}

/// Judges a call's grammar over its items as plain JSON: a non-empty list
/// of objects, each `{question, act}` with `act` one of `take`, `answer`
/// (with the owner's words in `answer`) or `defer` (with the reason in
/// `reason`), no member its act does not take, every text within 4,096
/// bytes, and no question named twice.
pub fn judge_call(items: &Value) -> Result<Call, CallFault> {
    let Value::Array(entries) = items else {
        return Err(CallFault { position: None });
    };
    if entries.is_empty() {
        return Err(CallFault { position: None });
    }
    let mut seen = HashSet::new();
    let mut judged = Vec::with_capacity(entries.len());
    for (position, entry) in entries.iter().enumerate() {
        let fault = CallFault {
            position: Some(position),
        };
        let item = item(entry).ok_or(fault)?;
        // A question named twice would settle it twice in one call.
        if !seen.insert(item.question.clone()) {
            return Err(fault);
        }
        judged.push(item);
    }
    Ok(Call { items: judged })
}

/// One item read in the grammar, or `None` when it breaks it.
fn item(entry: &Value) -> Option<Item> {
    let Value::Object(object) = entry else {
        return None;
    };
    let question = text(object, "question")??;
    let act = text(object, "act")??;
    let (act, member) = match act.as_str() {
        "take" => (Act::Take, None),
        "answer" => (
            Act::Answer(text(object, "answer")?.unwrap_or_default()),
            Some("answer"),
        ),
        "defer" => (
            Act::Defer(text(object, "reason")?.unwrap_or_default()),
            Some("reason"),
        ),
        _ => return None,
    };
    if object
        .keys()
        .any(|name| name != "question" && name != "act" && Some(name.as_str()) != member)
    {
        return None;
    }
    Some(Item { question, act })
}

/// `Some(None)` when `name` is absent, `Some(Some(text))` for a text within
/// the bound, and `None` for any other value. The bound is on bytes as
/// written, before any trimming.
fn text(object: &Map<String, Value>, name: &str) -> Option<Option<String>> {
    match object.get(name) {
        None => Some(None),
        Some(Value::String(text)) if text.len() <= INLINE_TEXT_LIMIT => Some(Some(text.clone())),
        Some(_) => None,
    }
}

/// Judges the accepted `call` against the facts of its set as recorded
/// before the call: the set first, then each item in the call's order
/// through `no-such-question`, `question-closed`, `question-not-in-round`
/// (answers only), `answer-blank` and `deferral-reason`. Every settlement
/// carries the set's current round number.
pub fn judge_answers(facts: &SetFacts, call: &Call) -> Result<Settled, AnswerRefusal> {
    if facts.abandoned.is_some() {
        return Err(AnswerRefusal::SetAbandoned);
    }
    // The round as it stood before the call, so an answer made askable
    // only by another answer in the same call is refused.
    let before = round_of(facts);
    let in_round: HashSet<&str> = before.current.iter().map(|q| q.id.as_str()).collect();
    let closed: HashSet<&str> = facts
        .answers
        .iter()
        .map(|answer| answer.question.as_str())
        .chain(
            facts
                .deferrals
                .iter()
                .map(|deferral| deferral.question.as_str()),
        )
        .collect();
    let mut settlements = Vec::with_capacity(call.items.len());
    for item in &call.items {
        let id = || item.question.clone();
        let Some(question) = facts.questions.iter().find(|q| q.id == item.question) else {
            return Err(AnswerRefusal::NoSuchQuestion(id()));
        };
        if closed.contains(item.question.as_str()) {
            return Err(AnswerRefusal::QuestionClosed(id()));
        }
        let answers = !matches!(item.act, Act::Defer(_));
        if answers && !in_round.contains(item.question.as_str()) {
            return Err(AnswerRefusal::NotInRound(id()));
        }
        settlements.push(match &item.act {
            // No text is compared with the recommendation: the item says
            // whether it takes it.
            Act::Take => Settlement::Answered {
                question: id(),
                answer: question.recommended.clone(),
                accepted: true,
            },
            Act::Answer(answer) if answer.trim().is_empty() => {
                return Err(AnswerRefusal::AnswerBlank(id()));
            }
            Act::Answer(answer) => Settlement::Answered {
                question: id(),
                answer: answer.clone(),
                accepted: false,
            },
            Act::Defer(reason) if reason.trim().is_empty() => {
                return Err(AnswerRefusal::DeferralReason(id()));
            }
            Act::Defer(reason) => Settlement::Deferred {
                question: id(),
                reason: reason.clone(),
            },
        });
    }
    Ok(Settled {
        facts: facts.clone(),
        round: before.number,
        settlements,
    })
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005 PLN-R24 to
    //! PLN-R26, the phase's decisions and the plan's set R2, never from
    //! running this code.

    use serde_json::json;

    use super::*;
    use crate::questions::{Question, SetState};

    const OWNER: &str = "Ada Lovelace";
    const AT: &str = "2026-10-22T09:00:01Z";
    const SET: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";

    fn question(id: &str, text: &str, recommended: &str, depends_on: &[&str]) -> Question {
        Question {
            id: id.into(),
            question: text.into(),
            recommended: recommended.into(),
            depends_on: depends_on.iter().map(|on| (*on).to_owned()).collect(),
        }
    }

    /// Set R2 as the plan writes it out, with nothing recorded.
    fn r2() -> SetFacts {
        SetFacts {
            questions: vec![
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
                    &["q1"],
                ),
            ],
            answers: Vec::new(),
            deferrals: Vec::new(),
            abandoned: None,
        }
    }

    fn call(items: Value) -> Call {
        judge_call(&items).unwrap_or_else(|fault| panic!("{items}: {fault:?}"))
    }

    fn judged(facts: &SetFacts, items: Value) -> Result<Settled, AnswerRefusal> {
        judge_answers(facts, &call(items))
    }

    #[test]
    fn a_take_or_an_own_answer_recorded_with_the_wrong_text_or_flag_is_caught() {
        let cases = [
            (
                json!([{"question": "q1", "act": "take"}]),
                "q1",
                "SQLite",
                true,
            ),
            (
                json!([{"question": "q2", "act": "take"}]),
                "q2",
                "Yes, one per user",
                true,
            ),
            (
                json!([{"question": "q1", "act": "answer", "answer": "SQLite"}]),
                "q1",
                "SQLite",
                false,
            ),
            (
                json!([{"question": "q1", "act": "answer", "answer": "  Postgres, for now "}]),
                "q1",
                "  Postgres, for now ",
                false,
            ),
        ];
        for (items, id, answer, accepted) in cases {
            let settled = judged(&r2(), items.clone()).unwrap();
            assert_eq!(settled.round(), 1, "{items}");
            assert_eq!(settled.settlements().len(), 1, "{items}");
            let settlement = &settled.settlements()[0];
            assert_eq!(settlement.type_name(), "question.answered", "{items}");
            assert_eq!(
                settlement.payload(SET, settled.round(), OWNER, AT),
                json!({"set": SET, "question": id, "round": 1, "answer": answer,
                    "accepted": accepted, "owner": OWNER, "at": AT}),
                "{items}"
            );
        }
    }

    fn refused(facts: &SetFacts, items: Value) -> (&'static str, Option<String>) {
        let refusal = judged(facts, items.clone()).expect_err(&items.to_string());
        (refusal.code(), refusal.question().map(str::to_owned))
    }

    fn named(code: &'static str, id: &str) -> (&'static str, Option<String>) {
        (code, Some(id.to_owned()))
    }

    fn answer(id: &str, text: &str, accepted: bool, round: u32) -> Answer {
        Answer {
            question: id.into(),
            answer: text.into(),
            accepted,
            round,
            owner: OWNER.into(),
            at: AT.into(),
        }
    }

    fn deferral(id: &str, reason: &str, round: u32) -> Deferral {
        Deferral {
            question: id.into(),
            reason: reason.into(),
            round,
            owner: OWNER.into(),
            at: AT.into(),
        }
    }

    /// R2 with q1 answered and q2 deferred, both at round 1 by one call.
    fn r2_settled() -> SetFacts {
        SetFacts {
            answers: vec![answer("q1", "SQLite", true, 1)],
            deferrals: vec![deferral("q2", "after the spike", 1)],
            ..r2()
        }
    }

    #[test]
    fn a_call_grammar_fault_accepted_or_named_wrongly_is_caught() {
        let long = "x".repeat(4097);
        let take = json!({"question": "q1", "act": "take"});
        let cases = [
            (json!([]), None),
            (json!({}), None),
            (json!(["q1"]), Some(0)),
            (json!([{"question": "q1"}]), Some(0)),
            (json!([{"question": "q1", "act": "skip"}]), Some(0)),
            (
                json!([{"question": "q1", "act": "take", "answer": "x"}]),
                Some(0),
            ),
            (json!([{"question": 1, "act": "take"}]), Some(0)),
            (
                json!([{"question": "q1", "act": "take", "note": "x"}]),
                Some(0),
            ),
            (
                json!([{"question": "q1", "act": "answer", "answer": "x", "reason": "y"}]),
                Some(0),
            ),
            (
                json!([{"question": "q1", "act": "defer", "reason": "y", "answer": "x"}]),
                Some(0),
            ),
            (json!([take, take]), Some(1)),
            (json!([{"question": long, "act": "take"}]), Some(0)),
            (
                json!([take, {"question": "q2", "act": "answer", "answer": long}]),
                Some(1),
            ),
            (
                json!([{"question": "q2", "act": "defer", "reason": long}]),
                Some(0),
            ),
        ];
        for (items, position) in cases {
            assert_eq!(judge_call(&items), Err(CallFault { position }), "{items}");
        }
        assert_eq!(
            judge_call(&json!([take])).unwrap().items(),
            [Item {
                question: "q1".into(),
                act: Act::Take,
            }]
        );
    }

    #[test]
    fn an_answer_made_askable_only_by_the_same_call_accepted_is_caught() {
        assert_eq!(
            refused(
                &r2(),
                json!([{"question": "q1", "act": "take"},
                    {"question": "q3", "act": "answer", "answer": "In the home folder"}])
            ),
            named(QUESTION_NOT_IN_ROUND, "q3")
        );
    }

    #[test]
    fn a_deferral_of_a_waiting_question_refused_is_caught() {
        let settled = judged(
            &r2(),
            json!([{"question": "q3", "act": "defer", "reason": "decide after the spike"}]),
        )
        .unwrap();
        assert_eq!(settled.round(), 1);
        assert_eq!(
            settled.settlements(),
            [Settlement::Deferred {
                question: "q3".into(),
                reason: "decide after the spike".into(),
            }]
        );
    }

    #[test]
    fn a_settled_or_unknown_question_settled_is_caught() {
        let cases = [
            (
                json!([{"question": "q1", "act": "answer", "answer": "Postgres"}]),
                named(QUESTION_CLOSED, "q1"),
            ),
            (
                json!([{"question": "q1", "act": "defer", "reason": "later"}]),
                named(QUESTION_CLOSED, "q1"),
            ),
            (
                json!([{"question": "q2", "act": "take"}]),
                named(QUESTION_CLOSED, "q2"),
            ),
            (
                json!([{"question": "q2", "act": "defer", "reason": "again"}]),
                named(QUESTION_CLOSED, "q2"),
            ),
            (
                json!([{"question": "q9", "act": "take"}]),
                named(NO_SUCH_QUESTION, "q9"),
            ),
        ];
        for (items, expected) in cases {
            assert_eq!(refused(&r2_settled(), items.clone()), expected, "{items}");
        }
    }

    #[test]
    fn a_blank_or_missing_answer_or_reason_accepted_is_caught() {
        let cases = [
            (
                json!([{"question": "q1", "act": "answer", "answer": "   "}]),
                named(ANSWER_BLANK, "q1"),
            ),
            (
                json!([{"question": "q1", "act": "answer"}]),
                named(ANSWER_BLANK, "q1"),
            ),
            (
                json!([{"question": "q2", "act": "defer", "reason": ""}]),
                named(DEFERRAL_REASON, "q2"),
            ),
            (
                json!([{"question": "q2", "act": "defer", "reason": "   "}]),
                named(DEFERRAL_REASON, "q2"),
            ),
            (
                json!([{"question": "q2", "act": "defer"}]),
                named(DEFERRAL_REASON, "q2"),
            ),
        ];
        for (items, expected) in cases {
            assert_eq!(refused(&r2(), items.clone()), expected, "{items}");
        }
        assert!(judged(&r2(), json!([{"question": "q1", "act": "take"}])).is_ok());
    }

    #[test]
    fn an_abandoned_set_answered_or_judged_after_its_items_is_caught() {
        let abandoned = SetFacts {
            abandoned: Some(crate::questions::Abandonment {
                cause: crate::questions::Cause::Superseded,
                at: AT.into(),
            }),
            ..r2()
        };
        for items in [
            json!([{"question": "q9", "act": "take"}]),
            json!([{"question": "q1", "act": "take"}]),
            json!([{"question": "q2", "act": "defer", "reason": "later"}]),
        ] {
            assert_eq!(
                refused(&abandoned, items.clone()),
                (QUESTION_SET_ABANDONED, None),
                "{items}"
            );
        }
    }

    #[test]
    fn refusals_judged_in_another_order_are_caught() {
        let cases = [
            (
                json!([{"question": "q3", "act": "answer", "answer": "x"},
                    {"question": "q9", "act": "take"}]),
                named(QUESTION_NOT_IN_ROUND, "q3"),
            ),
            (
                json!([{"question": "q9", "act": "take"},
                    {"question": "q3", "act": "answer", "answer": "x"}]),
                named(NO_SUCH_QUESTION, "q9"),
            ),
            (
                json!([{"question": "q3", "act": "answer", "answer": "  "}]),
                named(QUESTION_NOT_IN_ROUND, "q3"),
            ),
        ];
        for (items, expected) in cases {
            assert_eq!(refused(&r2(), items.clone()), expected, "{items}");
        }
    }

    #[test]
    fn a_call_settled_under_another_round_number_is_caught() {
        let settled = judged(&r2_settled(), json!([{"question": "q3", "act": "take"}])).unwrap();
        assert_eq!(settled.round(), 2);
        assert_eq!(
            settled.settlements()[0].payload(SET, settled.round(), OWNER, AT)["round"],
            json!(2)
        );
    }

    #[test]
    fn a_receipt_round_left_as_before_the_call_is_caught() {
        let after = judged(&r2(), json!([{"question": "q1", "act": "take"}]))
            .unwrap()
            .round_after(OWNER, AT);
        assert_eq!(after.number, 2);
        assert_eq!(after.state, SetState::Open);
        let ids: Vec<&str> = after.current.iter().map(|q| q.id.as_str()).collect();
        assert_eq!(ids, ["q2", "q3"]);
        assert_eq!(after.answered, [answer("q1", "SQLite", true, 1)]);

        let after = judged(
            &r2(),
            json!([{"question": "q1", "act": "defer", "reason": "later"}]),
        )
        .unwrap()
        .round_after(OWNER, AT);
        assert_eq!(after.number, 2);
        assert_eq!(after.state, SetState::Open);
        let ids: Vec<&str> = after.current.iter().map(|q| q.id.as_str()).collect();
        assert_eq!(ids, ["q2"]);
        assert_eq!(after.waiting.len(), 1);
        assert_eq!(after.waiting[0].id, "q3");
        assert_eq!(
            after.waiting[0].on,
            [("q1".to_owned(), crate::questions::Pending::Deferred)]
        );
    }
}
