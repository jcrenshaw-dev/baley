//! The question events: their types, their registration, their streams and
//! their payloads (design 0005 section 6, design 0001 Streams).
//!
//! Every text is inline, never a payload reference: a set's texts are each
//! bounded at 4,096 bytes by its judge. Unlike the work-order events, these
//! payloads carry a time of their own, `at`, the time of the command that
//! recorded them, and an answer or deferral carries the owner's name, as
//! `scope.approved` does, so the record says who settled a question and
//! when in its own words.

use baley_store::Hash;
use serde_json::{Value, json};

use super::set::QuestionSet;
use crate::registry::{Registry, RegistryError};
use crate::scope::ROADMAP_STREAM;

/// A question set opened, on its subject's stream:
/// `{set, subject, story, work_order, questions, at}` for a story's set and
/// `{set, subject, phase, plan_digest, questions, at}` for a plan draft's.
pub const QUESTIONS_OPENED: &str = "questions.opened";
/// The current `questions.opened` payload version.
pub const QUESTIONS_OPENED_VERSION: u32 = 1;
/// One question answered, on its set's stream:
/// `{set, question, round, answer, accepted, owner, at}`.
pub const QUESTION_ANSWERED: &str = "question.answered";
/// The current `question.answered` payload version.
pub const QUESTION_ANSWERED_VERSION: u32 = 1;
/// One question deferred, on its set's stream:
/// `{set, question, round, reason, owner, at}`.
pub const QUESTION_DEFERRED: &str = "question.deferred";
/// The current `question.deferred` payload version.
pub const QUESTION_DEFERRED_VERSION: u32 = 1;
/// A question set abandoned, on its set's stream: `{set, cause, at}`.
pub const QUESTIONS_ABANDONED: &str = "questions.abandoned";
/// The current `questions.abandoned` payload version.
pub const QUESTIONS_ABANDONED_VERSION: u32 = 1;

/// Registers the four question types at version 1 with no upcasters.
pub fn register_question_events(registry: &mut Registry) -> Result<(), RegistryError> {
    registry.register(QUESTIONS_OPENED, QUESTIONS_OPENED_VERSION, [])?;
    registry.register(QUESTION_ANSWERED, QUESTION_ANSWERED_VERSION, [])?;
    registry.register(QUESTION_DEFERRED, QUESTION_DEFERRED_VERSION, [])?;
    registry.register(QUESTIONS_ABANDONED, QUESTIONS_ABANDONED_VERSION, [])
}

/// Whose decisions a set holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    /// A story's refinement: the story and the analyzer's questions work
    /// order that returned the set.
    Story { story: u32, work_order: String },
    /// A plan draft: its phase and the draft's digest.
    Plan { phase: u32, plan_digest: Hash },
}

/// The stream a set of `subject` and every record on it go on: `roadmap`
/// for a story's set, `phase/<n>` for a plan draft's.
pub fn set_stream(subject: &Subject) -> String {
    match subject {
        Subject::Story { .. } => ROADMAP_STREAM.to_owned(),
        Subject::Plan { phase, .. } => format!("phase/{phase}"),
    }
}

/// Why a set was abandoned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// A later set was opened for the same story.
    Superseded,
    /// The plan draft the set belongs to was replaced.
    DraftReplaced,
}

impl Cause {
    /// The cause as the record spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Superseded => "superseded",
            Self::DraftReplaced => "draft-replaced",
        }
    }

    /// The cause a record spells, if it is one.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "superseded" => Some(Self::Superseded),
            "draft-replaced" => Some(Self::DraftReplaced),
            _ => None,
        }
    }
}

/// The payload of `questions.opened` for the accepted `questions`, opened
/// as set `set` of `subject` at `at`. The questions are written in
/// submitted order, each as `{id, question, recommended, depends_on}`.
pub fn opened_payload(set: &str, subject: &Subject, questions: &QuestionSet, at: &str) -> Value {
    let questions: Vec<Value> = questions
        .questions()
        .iter()
        .map(|question| {
            json!({
                "id": question.id,
                "question": question.question,
                "recommended": question.recommended,
                "depends_on": question.depends_on,
            })
        })
        .collect();
    match subject {
        Subject::Story { story, work_order } => json!({
            "set": set,
            "subject": "story",
            "story": story,
            "work_order": work_order,
            "questions": questions,
            "at": at,
        }),
        Subject::Plan { phase, plan_digest } => json!({
            "set": set,
            "subject": "plan",
            "phase": phase,
            "plan_digest": plan_digest.to_hex(),
            "questions": questions,
            "at": at,
        }),
    }
}

/// The payload of `question.answered`: `answer` given to `question` of set
/// `set` in `round`, `accepted` when it takes the recommended answer, by
/// `owner` at `at`.
pub fn answered_payload(
    set: &str,
    question: &str,
    round: u32,
    answer: &str,
    accepted: bool,
    owner: &str,
    at: &str,
) -> Value {
    json!({
        "set": set,
        "question": question,
        "round": round,
        "answer": answer,
        "accepted": accepted,
        "owner": owner,
        "at": at,
    })
}

/// The payload of `question.deferred`: `question` of set `set` deferred in
/// `round` for `reason`, by `owner` at `at`.
pub fn deferred_payload(
    set: &str,
    question: &str,
    round: u32,
    reason: &str,
    owner: &str,
    at: &str,
) -> Value {
    json!({
        "set": set,
        "question": question,
        "round": round,
        "reason": reason,
        "owner": owner,
        "at": at,
    })
}

/// The payload of `questions.abandoned`: set `set` abandoned for `cause`
/// at `at`.
pub fn abandoned_payload(set: &str, cause: Cause, at: &str) -> Value {
    json!({"set": set, "cause": cause.as_str(), "at": at})
}

#[cfg(test)]
mod tests {
    //! Expected payloads are written out from design 0005 section 6, the
    //! phase's decisions and the plan's test data, never from running this
    //! code.

    use baley_store::EventSchema;

    use super::*;
    use crate::questions::judge_set;

    const SET: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const ORDER: &str = "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e";
    const AT: &str = "2026-10-22T09:00:00Z";
    const AT1: &str = "2026-10-22T09:00:01Z";

    #[test]
    fn a_question_event_left_unregistered_or_at_another_version_is_caught() {
        let mut registry = Registry::new();
        register_question_events(&mut registry).unwrap();
        for name in [
            "questions.opened",
            "question.answered",
            "question.deferred",
            "questions.abandoned",
        ] {
            assert_eq!(registry.current_version(name), Some(1), "{name}");
            assert!(!registry.reads(name, 2), "{name}");
        }
        assert!(register_question_events(&mut registry).is_err());
    }

    #[test]
    fn an_opened_payload_missing_a_member_or_carrying_another_is_caught() {
        let set = judge_set(&json!([
            {"id": "q1", "question": "Which store backs the ledger?", "recommended": "SQLite",
                "depends_on": []},
            {"id": "q2", "question": " Where does the ledger live? ",
                "recommended": "Outside the checkout", "depends_on": ["q1"]}
        ]))
        .unwrap();
        let written = json!([
            {"id": "q1", "question": "Which store backs the ledger?", "recommended": "SQLite",
                "depends_on": []},
            {"id": "q2", "question": " Where does the ledger live? ",
                "recommended": "Outside the checkout", "depends_on": ["q1"]}
        ]);
        let story = Subject::Story {
            story: 3,
            work_order: ORDER.into(),
        };
        assert_eq!(
            opened_payload(SET, &story, &set, AT),
            json!({"set": SET, "subject": "story", "story": 3, "work_order": ORDER,
                "questions": written, "at": AT})
        );
        let plan = Subject::Plan {
            phase: 2,
            plan_digest: Hash([0x5c; 32]),
        };
        assert_eq!(
            opened_payload(SET, &plan, &set, AT),
            json!({"set": SET, "subject": "plan", "phase": 2, "plan_digest": "5c".repeat(32),
                "questions": written, "at": AT})
        );
    }

    #[test]
    fn an_answered_or_deferred_payload_missing_its_round_owner_or_time_is_caught() {
        assert_eq!(
            answered_payload(SET, "q1", 2, "SQLite", true, "Ada Lovelace", AT1),
            json!({"set": SET, "question": "q1", "round": 2, "answer": "SQLite",
                "accepted": true, "owner": "Ada Lovelace", "at": AT1})
        );
        assert_eq!(
            deferred_payload(SET, "q2", 2, "  after the spike ", "Ada Lovelace", AT1),
            json!({"set": SET, "question": "q2", "round": 2, "reason": "  after the spike ",
                "owner": "Ada Lovelace", "at": AT1})
        );
    }

    #[test]
    fn an_abandonment_cause_spelled_otherwise_is_caught() {
        assert_eq!(
            abandoned_payload(SET, Cause::Superseded, AT),
            json!({"set": SET, "cause": "superseded", "at": AT})
        );
        assert_eq!(
            abandoned_payload(SET, Cause::DraftReplaced, AT),
            json!({"set": SET, "cause": "draft-replaced", "at": AT})
        );
    }

    #[test]
    fn a_story_set_off_the_roadmap_stream_or_a_plan_set_off_its_phase_stream_is_caught() {
        let story = Subject::Story {
            story: 1,
            work_order: ORDER.into(),
        };
        assert_eq!(set_stream(&story), "roadmap");
        let plan = Subject::Plan {
            phase: 3,
            plan_digest: Hash([0x5c; 32]),
        };
        assert_eq!(set_stream(&plan), "phase/3");
    }
}
