//! Plans' pure half (design 0005 sections 3 and 6, PLN-R5, PLN-R9's judge,
//! PLN-R11 to PLN-R13): the judgement of one submitted plan whole against
//! the phase's facts, the refusal codes it answers with, and the accepted
//! plan only that judgement builds.
//!
//! Nothing here reads a store, a file or a clock: the binary supplies every
//! value it judges.

mod content;
mod evidence;
#[cfg(test)]
mod fixtures;
mod judge;
mod lease;
mod task;

pub use evidence::{
    Artifact, Check, CheckTest, EvidenceItem, Expected, Level, Link, Observation, Spec,
};
pub use judge::{
    ApprovedItem, CommittedStory, CurrentTruth, EvidenceKind, LEASE, Plan, PlanFacts, PlanRefusal,
    TYPED_CONTENT, judge_plan,
};
pub use lease::{inside_lease, plain_relative_path};
pub use task::Task;
