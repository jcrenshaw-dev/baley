//! Question sets' pure half (design 0005 sections 5 to 7, ADR 0030): the
//! set judge and the refusal codes it answers with.
//!
//! Nothing here reads a store, a file or a clock: the binary supplies every
//! value it judges.

mod set;

pub use set::{
    QUESTION_BLANK, QUESTION_CYCLE, QUESTION_DEPENDENCY, QUESTION_ID, Question, QuestionSet,
    SetRefusal, TYPED_CONTENT, judge_set,
};
