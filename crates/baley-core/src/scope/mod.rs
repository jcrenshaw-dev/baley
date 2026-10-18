//! Scope records' pure half (design 0004 sections 5 and 6): the judgement
//! of a start submission and its digest, the judgement of a change
//! submission with its digest and target, the judgement of an approval, the
//! eleven scope events, and the `draft`, `roadmap`, `phase`,
//! `phase_detail` and `backlog` views.
//!
//! Nothing here reads a store, a file or a clock: the binary supplies every
//! value it judges, and stores any body before an event names it.

use baley_store::{Change, DocKey, PurgedEvent};
use serde_json::{Value, json};

mod approval;
mod backlog;
mod change;
mod draft;
mod event;
mod phase;
mod roadmap;
mod rules;
mod submission;

pub use approval::{
    ApprovalFacts, ApprovalRefusal, NumbersExhausted, judge_approval, start_events,
};
pub use backlog::{BACKLOG_VIEW, BacklogProjector, backlog_key, backlog_spec};
pub use change::{
    ChangeKind, ChangeSubmission, Decision, Destination, PhaseDeclaration, PhaseFields,
    ScopeChange, judge_change,
};
pub use draft::{DRAFT_NEWEST_INDEX, DRAFT_VIEW, DraftProjector, draft_key, draft_spec};
pub use event::{
    PHASE_DECLARED, PHASE_DECLARED_VERSION, PHASE_REORDERED, PHASE_REORDERED_VERSION,
    PHASE_WITHDRAWN, PHASE_WITHDRAWN_VERSION, PROJECT_DESCRIBED, PROJECT_DESCRIBED_VERSION,
    PROJECT_STREAM, ROADMAP_STREAM, SCOPE_APPROVED, SCOPE_APPROVED_VERSION, SCOPE_SUBMITTED,
    SCOPE_SUBMITTED_VERSION, START_KIND, START_TARGET, STORY_CORRECTED, STORY_CORRECTED_VERSION,
    STORY_DECLARED, STORY_DECLARED_VERSION, STORY_DROPPED, STORY_DROPPED_VERSION, STORY_REASSIGNED,
    STORY_REASSIGNED_VERSION, STORY_REPRIORITIZED, STORY_REPRIORITIZED_VERSION, approved_payload,
    described_payload, phase_declared_payload, phase_reordered_payload, phase_withdrawn_payload,
    register_scope_events, story_corrected_payload, story_declared_payload, story_dropped_payload,
    story_reassigned_payload, story_reprioritized_payload, submitted_payload,
};
pub use phase::{
    PHASE_DETAIL_VIEW, PHASE_STATUS_INDEX, PHASE_VIEW, PhaseDetailProjector, PhaseProjector,
    phase_detail_key, phase_detail_spec, phase_key, phase_spec,
};
pub use roadmap::{ROADMAP_VIEW, RoadmapProjector, roadmap_key, roadmap_spec};
pub use rules::{
    AcceptedScope, DEPENDENCY_ORDER, EXECUTION_PRESENT, NO_SUCH_STORY, ORDER_INCOMPLETE,
    PHASE_COMPLETE, PHASE_HAS_DEPENDENTS, PROJECT_NOT_STARTED, PhaseProgress, STORY_NOT_SERVED,
    STORY_SHIPPED, STORY_UNDECIDED, ScopeRefusal, judge_progress, judge_scope_change,
};
pub use submission::{
    DEPENDENCY_CYCLE, Description, MALFORMED_SUBMISSION, PROJECT_ALREADY_STARTED, Phase,
    STALE_DRAFT, STORY_DOUBLE_ASSIGNED, Story, StoryStatus, Submission, SubmissionRefusal,
    UNKNOWN_DRAFT, judge_submission,
};

/// The documents `purge` marks purged with its reason: each whose `seq`
/// and `hash` name a reference this project released. A body is stored by
/// hash for every project, so only this mark tells this project's reads
/// not to open it.
fn mark_purged(purge: &PurgedEvent, documents: &[(DocKey, Value)]) -> Vec<Change> {
    documents
        .iter()
        .filter_map(|(key, document)| {
            let Value::Object(fields) = document else {
                return None;
            };
            let seq = fields.get("seq").and_then(Value::as_u64)?;
            let hash = fields.get("hash").and_then(Value::as_str)?;
            let released = purge
                .released
                .iter()
                .any(|(at, released)| *at == seq && released.to_hex() == hash);
            if !released {
                return None;
            }
            let mut body = fields.clone();
            body.insert("state".into(), json!("purged"));
            body.insert("reason".into(), json!(purge.reason));
            Some(Change::Put {
                key: key.clone(),
                body: Value::Object(body),
            })
        })
        .collect()
}
