//! Plans' pure half (design 0005 sections 3 and 6, PLN-R5, PLN-R9's judge,
//! PLN-R11 to PLN-R13): the judgement of one submitted plan whole against
//! the phase's facts, the refusal codes it answers with, and the accepted
//! plan only that judgement builds.
//!
//! Nothing here reads a store, a file or a clock: the binary supplies every
//! value it judges.

mod content;
#[cfg(test)]
mod fixtures;
mod judge;

pub use judge::{
    ApprovedItem, CommittedStory, CurrentTruth, EvidenceKind, Plan, PlanFacts, PlanRefusal,
    TYPED_CONTENT, judge_plan,
};
