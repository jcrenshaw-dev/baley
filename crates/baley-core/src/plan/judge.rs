//! The judgement of one submitted plan whole (design 0005 sections 3 and 6,
//! PLN-R11 to PLN-R13).
//!
//! A plan arrives as plain JSON and the phase's facts as plain values the
//! binary gathered from its views. The plan is judged in one fixed order,
//! the first fault answering, so one plan always gets one answer: (1) the
//! plan's own members. The judge trusts the facts and judges the plan.
//! Nothing here records anything.

use std::num::NonZeroU32;

use serde_json::Value;

use super::content;

/// The refusal code for a plan out of its grammar: not an object, or a
/// member unknown, missing, of another type or out of range. It names the
/// path of the first fault.
pub const TYPED_CONTENT: &str = "typed-content";

/// Why a plan is refused. Positions count from 0 in the list as submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanRefusal {
    /// The plan breaks its grammar at `path`: member names joined by `.`
    /// and a list position written `[<n>]`, such as `tasks[0].verify[1]`.
    /// `None` when the plan is not an object.
    TypedContent { path: Option<String> },
}

impl PlanRefusal {
    /// The refusal's public code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::TypedContent { .. } => TYPED_CONTENT,
        }
    }

    /// A `typed-content` refusal naming `path`.
    pub(super) fn typed(path: impl Into<String>) -> Self {
        Self::TypedContent {
            path: Some(path.into()),
        }
    }
}

/// The kind of an evidence item (design 0005 section 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceKind {
    /// A file or record that must exist.
    Artifact,
    /// A value that must pass from a caller to a callee.
    Link,
    /// A command whose result proves the truth.
    Check,
    /// Something a named person must see.
    Observation,
}

/// What a plan is judged on besides itself. The binary gathers every value
/// from its views (D-09) and the judge trusts them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanFacts {
    /// The phase's committed stories, in commitment order.
    pub stories: Vec<CommittedStory>,
    /// The evidence items of the phase's approved plans.
    pub approved: Vec<ApprovedItem>,
    /// The phase's current size: the tasks of its approved plans, each
    /// plan's counted once.
    pub size: u32,
    /// The phase's capacity in tasks, or `None` when none is set.
    pub capacity: Option<NonZeroU32>,
}

/// One story committed to the phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedStory {
    /// The story's id.
    pub id: u32,
    /// Its current truths, in the story's order. Empty when the story holds
    /// no approved truth.
    pub truths: Vec<CurrentTruth>,
}

/// One current truth of a committed story, its slot texts as approved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentTruth {
    /// Its id, unique within its story.
    pub id: String,
    /// Its current version.
    pub version: u32,
    /// The one trigger, or a `for-any` truth's input.
    pub trigger: String,
    /// The one observer.
    pub observer: String,
    /// The one outcome.
    pub outcome: String,
    /// The tool or live service a person needs to prove it, if any.
    pub human_verify: Option<String>,
}

/// One evidence item of an approved plan of the phase, as recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedItem {
    /// The number of the approved plan that holds it.
    pub plan: u32,
    /// Its id.
    pub id: String,
    /// The story of the truth it serves.
    pub story: u32,
    /// The id of the truth it serves.
    pub truth: String,
    /// The version of the truth it serves.
    pub truth_version: u32,
    /// Its kind.
    pub kind: EvidenceKind,
    /// Its whole definition as recorded.
    pub definition: Value,
}

/// A plan judged whole and accepted, every member kept exactly as
/// submitted. Only [`judge_plan`] builds one, so nothing unjudged can be
/// recorded.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub(super) phase: u32,
    pub(super) stories: Vec<u32>,
    pub(super) files: Vec<String>,
    pub(super) directories: Vec<String>,
    pub(super) goal: String,
    pub(super) context: String,
    pub(super) notes: String,
    pub(super) tasks: Vec<Value>,
    pub(super) evidence_map: Vec<Value>,
    pub(super) questions: Value,
}

impl Plan {
    /// The phase the plan is for.
    pub fn phase(&self) -> u32 {
        self.phase
    }

    /// The stories the plan serves, in submitted order.
    pub fn stories(&self) -> &[u32] {
        &self.stories
    }

    /// The lease's files, in submitted order.
    pub fn files(&self) -> &[String] {
        &self.files
    }

    /// The lease's directories, in submitted order.
    pub fn directories(&self) -> &[String] {
        &self.directories
    }

    /// The plan's goal.
    pub fn goal(&self) -> &str {
        &self.goal
    }

    /// The plan's context.
    pub fn context(&self) -> &str {
        &self.context
    }

    /// The plan's notes.
    pub fn notes(&self) -> &str {
        &self.notes
    }

    /// The tasks, in submitted order.
    pub fn tasks(&self) -> &[Value] {
        &self.tasks
    }

    /// The evidence items, in submitted order.
    pub fn evidence_map(&self) -> &[Value] {
        &self.evidence_map
    }

    /// The `questions` member as submitted.
    pub fn questions(&self) -> &Value {
        &self.questions
    }

    /// The plan's size: the number of its tasks.
    pub fn size(&self) -> usize {
        self.tasks.len()
    }
}

/// Judges the submitted plan `plan` against the phase's `facts` in the
/// fixed order, the first fault answering: (1) the plan's own members.
pub fn judge_plan(plan: &Value, _facts: &PlanFacts) -> Result<Plan, PlanRefusal> {
    content::read(plan)
}
