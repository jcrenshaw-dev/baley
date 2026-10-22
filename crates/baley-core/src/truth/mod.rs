//! Truths' pure half (design 0005 sections 5 and 6, PLN-R2 to PLN-R5): the
//! judgement of one truth submission and the refusal codes it answers with,
//! the versions a revision's truths get at approval, and the
//! `story.refined` event that records an approved revision.
//!
//! Nothing here reads a store, a file or a clock: the binary supplies every
//! value it judges.

mod event;
mod submission;
mod version;

pub use event::{STORY_REFINED, STORY_REFINED_VERSION, refined_payload, register_truth_events};
pub use submission::{
    STORY_TRUTHS, TRUTH_ATTESTATION, TRUTH_FORM, TRUTH_ID, TRUTH_INTERNAL, TRUTH_PROSE_ORACLE,
    Truth, TruthRefusal, TruthSubmission, judge_truths,
};
pub use version::{Mark, Versioned, Versions, VersionsExhausted, versions};
