//! Truths' pure half (design 0005 sections 5 and 6, PLN-R2 to PLN-R5): the
//! judgement of one truth submission and the refusal codes it answers with,
//! and the versions a revision's truths get at approval.
//!
//! Nothing here reads a store, a file or a clock: the binary supplies every
//! value it judges.

mod submission;
mod version;

pub use submission::{
    STORY_TRUTHS, TRUTH_ATTESTATION, TRUTH_FORM, TRUTH_ID, TRUTH_INTERNAL, TRUTH_PROSE_ORACLE,
    Truth, TruthRefusal, TruthSubmission, judge_truths,
};
pub use version::{Mark, Versioned, Versions, VersionsExhausted, versions};
