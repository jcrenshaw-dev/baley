//! Plans' pure half (design 0005 sections 3 and 6, PLN-R5, PLN-R9's judge,
//! PLN-R11 to PLN-R13): the judgement of one submitted plan whole against
//! the phase's facts, the refusal codes it answers with, and the accepted
//! plan only that judgement builds.
//!
//! Nothing here reads a store, a file or a clock: the binary supplies every
//! value it judges.

mod binding;
mod content;
mod evidence;
#[cfg(test)]
mod fixtures;
mod judge;
mod lease;
mod link;
mod stories;
mod task;

pub use evidence::{
    Artifact, Check, CheckTest, EvidenceItem, Expected, Level, Link, Observation, Spec,
};
pub use judge::{
    ApprovedItem, CHECK_BOUNDARY, CHECK_COMMAND, CHECK_EXPECTED, CHECK_TEST_FILE, CommittedStory,
    CurrentTruth, EVIDENCE_ITEM_TRUTH, EvidenceKind, LEASE, LINK_VALUE_NOT_NAMED, NO_TRUTHS, Plan,
    PlanFacts, PlanRefusal, STORY_NOT_IN_PHASE, TRUTH_VERSION_MISMATCH, TYPED_CONTENT, judge_plan,
};
pub use lease::{inside_lease, plain_relative_path};
pub use task::Task;
