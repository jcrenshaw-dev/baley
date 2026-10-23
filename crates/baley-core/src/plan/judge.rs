//! The judgement of one submitted plan whole (design 0005 sections 3 and 6,
//! PLN-R11 to PLN-R13).
//!
//! A plan arrives as plain JSON and the phase's facts as plain values the
//! binary gathered from its views. The plan is judged in one fixed order,
//! the first fault answering, so one plan always gets one answer: (1) the
//! plan's own members, task by task within `tasks` and item by item within
//! `evidence_map`; (2) the `questions` member through the question set
//! judge; (3) each task file inside the lease; (4) no check field with a
//! code blank; (5) a committed story holding a truth; (6) every story the
//! plan serves committed to the phase; (7) each evidence item bound to its
//! own story's current truth; (8) each item id holding one definition
//! across the phase; (9) each link's value named by its truth; (10) every
//! truth the plan serves named by an item of the phase; (11) each such
//! truth holding a check. The judge trusts the facts and judges the plan.
//! Nothing here records anything.

use std::num::NonZeroU32;

use serde_json::Value;

use super::evidence::{self, EvidenceItem};
use super::task::{self, Task};
use super::{binding, content, coverage, link, stories};
use crate::questions::{QuestionSet, SetRefusal};

/// The refusal code for a plan out of its grammar: not an object, or a
/// member unknown, missing, of another type or out of range. It names the
/// path of the first fault.
pub const TYPED_CONTENT: &str = "typed-content";
/// The refusal code for a task naming a file outside the plan's lease.
pub const LEASE: &str = "lease";
/// The refusal code for a check whose command is blank.
pub const CHECK_COMMAND: &str = "check-command";
/// The refusal code for a check whose expected value is blank.
pub const CHECK_EXPECTED: &str = "check-expected";
/// The refusal code for a check whose test file is blank.
pub const CHECK_TEST_FILE: &str = "check-test-file";
/// The refusal code for a check whose boundary is blank, at either level
/// (D-07).
pub const CHECK_BOUNDARY: &str = "check-boundary";
/// The refusal code for a plan for a phase none of whose committed stories
/// holds a current truth.
pub const NO_TRUTHS: &str = "no-truths";
/// The refusal code for a plan serving a story not committed to the phase.
pub const STORY_NOT_IN_PHASE: &str = "story-not-in-phase";
/// The refusal code for an evidence item naming no current truth of a story
/// the plan serves.
pub const EVIDENCE_ITEM_TRUTH: &str = "evidence-item-truth";
/// The refusal code for an evidence item naming its truth at a version
/// other than the current one.
pub const TRUTH_VERSION_MISMATCH: &str = "truth-version-mismatch";
/// The refusal code for an evidence item whose id an approved plan of the
/// phase holds with another definition.
pub const EVIDENCE_ITEM_CONFLICT: &str = "evidence-item-conflict";
/// The refusal code for a link whose value its truth does not name.
pub const LINK_VALUE_NOT_NAMED: &str = "link-value-not-named";
/// The refusal code for a truth the plan serves that no evidence item of
/// the phase names.
pub const UNCOVERED_TRUTH: &str = "uncovered-truth";
/// The refusal code for a truth the plan serves whose evidence items across
/// the phase hold no check.
pub const TRUTH_WITHOUT_CHECK: &str = "truth-without-check";

/// Why a plan is refused. Positions count from 0 in the list as submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanRefusal {
    /// The plan breaks its grammar at `path`: member names joined by `.`
    /// and a list position written `[<n>]`, such as `tasks[0].verify[1]`.
    /// `None` when the plan is not an object.
    TypedContent { path: Option<String> },
    /// The task at `position`, whose id is `id`, names `file`, which lies
    /// outside the plan's lease.
    Lease {
        position: usize,
        id: String,
        file: String,
    },
    /// The question set judge refused the `questions` member: its refusal
    /// unchanged, positions counting within `questions` (D-10).
    Questions(SetRefusal),
    /// The check at `position`, whose id is `id`, has a blank command.
    CheckCommand { position: usize, id: String },
    /// The check at `position`, whose id is `id`, has a blank expected
    /// value.
    CheckExpected { position: usize, id: String },
    /// The check at `position`, whose id is `id`, has a blank test file.
    CheckTestFile { position: usize, id: String },
    /// The check at `position`, whose id is `id`, has a blank boundary.
    CheckBoundary { position: usize, id: String },
    /// No committed story of the phase holds a current truth.
    NoTruths,
    /// The plan serves `story`, which is not committed to the phase.
    StoryNotInPhase { story: u32 },
    /// The item at `position`, whose id is `id`, names no current truth of
    /// a story the plan serves.
    EvidenceItemTruth { position: usize, id: String },
    /// The item at `position`, whose id is `id`, names its truth at a
    /// version other than `current`, the truth's current version.
    TruthVersionMismatch {
        position: usize,
        id: String,
        current: u32,
    },
    /// The item at `position`, whose id is `id`, differs from the item of
    /// approved plan `plan` that holds the same id.
    EvidenceItemConflict {
        position: usize,
        id: String,
        plan: u32,
    },
    /// The link at `position`, whose id is `id`, carries a value its truth
    /// does not name.
    LinkValueNotNamed { position: usize, id: String },
    /// No item of the phase names `truth` of `story` at `version`, its
    /// current version.
    UncoveredTruth {
        story: u32,
        truth: String,
        version: u32,
    },
    /// The items of the phase naming `truth` of `story` at `version`, its
    /// current version, hold no check.
    TruthWithoutCheck {
        story: u32,
        truth: String,
        version: u32,
    },
}

impl PlanRefusal {
    /// The refusal's public code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::TypedContent { .. } => TYPED_CONTENT,
            Self::Lease { .. } => LEASE,
            Self::Questions(refusal) => refusal.code(),
            Self::CheckCommand { .. } => CHECK_COMMAND,
            Self::CheckExpected { .. } => CHECK_EXPECTED,
            Self::CheckTestFile { .. } => CHECK_TEST_FILE,
            Self::CheckBoundary { .. } => CHECK_BOUNDARY,
            Self::NoTruths => NO_TRUTHS,
            Self::StoryNotInPhase { .. } => STORY_NOT_IN_PHASE,
            Self::EvidenceItemTruth { .. } => EVIDENCE_ITEM_TRUTH,
            Self::TruthVersionMismatch { .. } => TRUTH_VERSION_MISMATCH,
            Self::EvidenceItemConflict { .. } => EVIDENCE_ITEM_CONFLICT,
            Self::LinkValueNotNamed { .. } => LINK_VALUE_NOT_NAMED,
            Self::UncoveredTruth { .. } => UNCOVERED_TRUTH,
            Self::TruthWithoutCheck { .. } => TRUTH_WITHOUT_CHECK,
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
    pub(super) tasks: Vec<Task>,
    pub(super) evidence_map: Vec<EvidenceItem>,
    pub(super) questions: QuestionSet,
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
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// The evidence items, in submitted order.
    pub fn evidence_map(&self) -> &[EvidenceItem] {
        &self.evidence_map
    }

    /// The plan's accepted question set, its questions as written.
    pub fn questions(&self) -> &QuestionSet {
        &self.questions
    }

    /// The plan's size: the number of its tasks.
    pub fn size(&self) -> usize {
        self.tasks.len()
    }
}

/// Judges the submitted plan `submitted` against the phase's `facts` in the
/// fixed order, the first fault answering: (1) the plan's own members;
/// (2) the `questions` member through `judge_set`; (3) `lease`, task by
/// task and file by file; (4) the blank check fields, item by item;
/// (5) `no-truths`; (6) `story-not-in-phase`, in the plan's story order;
/// (7) `evidence-item-truth` and `truth-version-mismatch`, item by item;
/// (8) `evidence-item-conflict`, item by item;
/// (9) `link-value-not-named`, link by link; then over the served truths
/// in order, with the approved plans' items, (10) `uncovered-truth` and
/// (11) `truth-without-check`.
pub fn judge_plan(submitted: &Value, facts: &PlanFacts) -> Result<Plan, PlanRefusal> {
    let plan = content::read(submitted)?;
    task::within_lease(&plan.tasks, &plan.files, &plan.directories)?;
    evidence::blank_fields(&plan.evidence_map)?;
    stories::holds_truths(facts)?;
    stories::committed(&plan.stories, facts)?;
    let truths = binding::bind(&plan.stories, &plan.evidence_map, facts)?;
    // The member walk has read `evidence_map` as a list by now.
    let definitions = submitted["evidence_map"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    coverage::consistent(definitions, &plan.evidence_map, &facts.approved)?;
    link::named(&plan.evidence_map, &truths)?;
    let phase = coverage::Phase::new(&plan.stories, &plan.evidence_map, facts);
    phase.covered()?;
    phase.checked()?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, the phase's
    //! decisions and the plan's test data, never from running this code.

    use serde_json::json;

    use super::*;
    use crate::plan::fixtures::{accepted, f0, f1, p1, qa, qb, refusal, typed, with, without};
    use crate::questions::Question;

    #[test]
    fn a_question_refusal_renamed_or_a_plan_question_rule_added_is_caught() {
        let facts = f1();
        let plan = accepted(&with(p1(), "", "questions", json!([qa(), qb()])), &facts);
        assert_eq!(
            plan.questions().questions(),
            [
                Question {
                    id: "q1".into(),
                    question: "Which store backs the ledger?".into(),
                    recommended: "SQLite".into(),
                    depends_on: Vec::new(),
                },
                Question {
                    id: "q2".into(),
                    question: "Where does the ledger live?".into(),
                    recommended: "Outside the checkout".into(),
                    depends_on: vec!["q1".into()],
                },
            ]
        );
        let twice = refusal(&with(p1(), "", "questions", json!([qa(), qa()])), &facts);
        assert_eq!(
            twice,
            PlanRefusal::Questions(SetRefusal::QuestionId {
                position: 1,
                id: "q1".into()
            })
        );
        assert_eq!(twice.code(), "question-id");
        let waiting = refusal(&with(p1(), "", "questions", json!([qb()])), &facts);
        assert_eq!(
            waiting,
            PlanRefusal::Questions(SetRefusal::QuestionDependency {
                position: 0,
                id: "q2".into()
            })
        );
        assert_eq!(waiting.code(), "question-dependency");
    }

    fn lease(position: usize, id: &str, file: &str) -> PlanRefusal {
        PlanRefusal::Lease {
            position,
            id: id.into(),
            file: file.into(),
        }
    }

    #[test]
    fn a_plan_with_bad_content_a_bad_lease_or_an_uncommitted_story_accepted_or_named_wrongly_is_caught()
     {
        let facts = f1();
        assert_eq!(accepted(&p1(), &facts).size(), 1);
        let mut over = vec![json!("docs")];
        over.extend((0..=254).map(|n| json!(format!("d/{n}"))));
        let cases = [
            (with(p1(), "", "plan", json!(1)), typed("plan")),
            (
                with(
                    p1(),
                    "",
                    "question_set",
                    json!("5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d"),
                ),
                typed("question_set"),
            ),
            (with(p1(), "", "size", json!(1)), typed("size")),
            (with(p1(), "", "suite", json!("cargo test")), typed("suite")),
            (with(p1(), "", "owner", json!("Ada")), typed("owner")),
            (without(p1(), "", "goal"), typed("goal")),
            (with(p1(), "", "phase", json!("8")), typed("phase")),
            (
                with(p1(), "", "stories", json!([1, 1])),
                typed("stories[1]"),
            ),
            (
                with(p1(), "", "directories", json!(["docs/"])),
                typed("directories[0]"),
            ),
            (
                with(p1(), "", "files", json!(["/src/judge.rs"])),
                typed("files[0]"),
            ),
            (
                with(p1(), "", "files", json!(["src/../judge.rs"])),
                typed("files[0]"),
            ),
            (
                with(p1(), "", "files", json!(["src/judge.rs", "src/judge.rs"])),
                typed("files[1]"),
            ),
            (
                with(
                    with(p1(), "", "files", json!([])),
                    "",
                    "directories",
                    json!([]),
                ),
                typed("files"),
            ),
            (
                with(p1(), "", "directories", Value::Array(over)),
                typed("directories[255]"),
            ),
            (
                with(
                    p1(),
                    "/tasks/0",
                    "files",
                    json!(["src/judge.rs", "src/other.rs"]),
                ),
                lease(0, "k1", "src/other.rs"),
            ),
            (
                with(p1(), "", "stories", json!([1, 4])),
                PlanRefusal::StoryNotInPhase { story: 4 },
            ),
            (
                with(
                    p1(),
                    "",
                    "questions",
                    json!([{"id": "q1", "question": "Which store backs the ledger?",
                        "recommended": "SQLite", "depends_on": ["q9"]}]),
                ),
                PlanRefusal::Questions(SetRefusal::QuestionDependency {
                    position: 0,
                    id: "q1".into(),
                }),
            ),
            (
                with(p1(), "", "questions", json!("none")),
                PlanRefusal::Questions(SetRefusal::TypedContent { position: None }),
            ),
        ];
        for (plan, refused) in cases {
            assert_eq!(refusal(&plan, &facts), refused, "{plan}");
        }
        assert_eq!(refusal(&p1(), &f0()), PlanRefusal::NoTruths);
    }

    #[test]
    fn faults_judged_in_another_order_are_caught() {
        let facts = f1();
        let other = |plan| with(plan, "/tasks/0", "files", json!(["src/other.rs"]));
        let blank = |plan| with(plan, "/evidence_map/0/spec", "command", json!(""));
        let waiting = |plan| with(plan, "", "questions", json!([qb()]));
        let owner = with(p1(), "", "owner", json!("Ada"));
        let cases = [
            (
                with(owner, "", "directories", json!(["docs/"])),
                &facts,
                typed("owner"),
            ),
            (waiting(without(p1(), "", "goal")), &facts, typed("goal")),
            (
                waiting(other(p1())),
                &facts,
                PlanRefusal::Questions(SetRefusal::QuestionDependency {
                    position: 0,
                    id: "q2".into(),
                }),
            ),
            (blank(other(p1())), &facts, lease(0, "k1", "src/other.rs")),
        ];
        for (plan, facts, refused) in cases {
            assert_eq!(refusal(&plan, facts), refused, "{plan}");
        }
        assert_eq!(
            refusal(&blank(p1()), &f0()),
            PlanRefusal::CheckCommand {
                position: 0,
                id: "c1".into()
            }
        );
        assert_eq!(
            refusal(&with(p1(), "", "stories", json!([1, 4])), &f0()),
            PlanRefusal::NoTruths
        );
    }
}
