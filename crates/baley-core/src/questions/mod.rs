//! Question sets' pure half (design 0005 sections 5 to 7, ADR 0030): the
//! set judge and the refusal codes it answers with, and the four question
//! events with their streams and payloads, the `questions` view, and the
//! round worked out from a set's recorded facts.
//!
//! Nothing here reads a store, a file or a clock: the binary supplies every
//! value it judges.

mod event;
mod round;
mod set;
mod view;

pub use event::{
    Cause, QUESTION_ANSWERED, QUESTION_ANSWERED_VERSION, QUESTION_DEFERRED,
    QUESTION_DEFERRED_VERSION, QUESTIONS_ABANDONED, QUESTIONS_ABANDONED_VERSION, QUESTIONS_OPENED,
    QUESTIONS_OPENED_VERSION, Subject, abandoned_payload, answered_payload, deferred_payload,
    opened_payload, register_question_events, set_stream,
};
pub use round::{
    Abandonment, Answer, Asked, Deferral, Pending, Round, SetFacts, SetState, UnreadableSet,
    Waiting, round_of, set_facts,
};
pub use set::{
    QUESTION_BLANK, QUESTION_CYCLE, QUESTION_DEPENDENCY, QUESTION_ID, Question, QuestionSet,
    SetRefusal, TYPED_CONTENT, judge_set,
};
pub use view::{QUESTIONS_VIEW, QuestionsProjector, questions_key, questions_spec};
