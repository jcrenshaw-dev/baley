//! The scope events: their types, their registration, their streams and
//! their payloads (design 0004 section 6, design 0001 Streams and
//! Payloads).
//!
//! No payload holds a caller or a time of its own, except an approval's
//! owner and time: the envelope carries both, hashed with the event.

use baley_store::{Hash, PayloadRef};
use serde_json::{Value, json};

use super::submission::{Description, StoryStatus};
use crate::registry::{Registry, RegistryError, UpcastError, Upcaster};

/// The stream a project's submissions, description and approvals go on.
pub const PROJECT_STREAM: &str = "project";
/// The stream a project's stories and phases go on.
pub const ROADMAP_STREAM: &str = "roadmap";

/// A scope submission held as a draft: `{digest, kind, target, body}`, the
/// target what the draft is filed under among its kind's drafts, and the
/// body a reference to a `record` payload holding the submission's
/// canonical JSON. Accepted scope never changes on it. Version 1 had no
/// target, and only starts were recorded at it.
pub const SCOPE_SUBMITTED: &str = "scope.submitted";
/// The current `scope.submitted` payload version.
pub const SCOPE_SUBMITTED_VERSION: u32 = 2;
/// The project description an approval accepted:
/// `{what, core_value, constraints, submission}`.
pub const PROJECT_DESCRIBED: &str = "project.described";
/// The current `project.described` payload version.
pub const PROJECT_DESCRIBED_VERSION: u32 = 1;
/// The owner's approval of a submission by digest:
/// `{submission, kind, owner, at}`.
pub const SCOPE_APPROVED: &str = "scope.approved";
/// The current `scope.approved` payload version.
pub const SCOPE_APPROVED_VERSION: u32 = 1;
/// One story with its Baley-assigned id:
/// `{id, sentence, status, phase, submission}`.
pub const STORY_DECLARED: &str = "story.declared";
/// The current `story.declared` payload version.
pub const STORY_DECLARED_VERSION: u32 = 1;
/// One phase with its Baley-assigned number:
/// `{number, name, goal, detail, depends_on, stories, submission}`.
pub const PHASE_DECLARED: &str = "phase.declared";
/// The current `phase.declared` payload version.
pub const PHASE_DECLARED_VERSION: u32 = 1;
/// The full order of phase numbers after a change: `{order, submission}`.
pub const PHASE_REORDERED: &str = "phase.reordered";
/// The current `phase.reordered` payload version.
pub const PHASE_REORDERED_VERSION: u32 = 1;
/// One phase withdrawn, kept with status `withdrawn`:
/// `{number, submission}`.
pub const PHASE_WITHDRAWN: &str = "phase.withdrawn";
/// The current `phase.withdrawn` payload version.
pub const PHASE_WITHDRAWN_VERSION: u32 = 1;
/// One story's corrected wording and status:
/// `{id, sentence, status, submission}`.
pub const STORY_CORRECTED: &str = "story.corrected";
/// The current `story.corrected` payload version.
pub const STORY_CORRECTED_VERSION: u32 = 1;
/// One story moved to a phase, or to the backlog when `phase` is null:
/// `{id, status, phase, submission}`.
pub const STORY_REASSIGNED: &str = "story.reassigned";
/// The current `story.reassigned` payload version.
pub const STORY_REASSIGNED_VERSION: u32 = 1;
/// One story dropped, with status `dropped`: `{id, status, submission}`.
pub const STORY_DROPPED: &str = "story.dropped";
/// The current `story.dropped` payload version.
pub const STORY_DROPPED_VERSION: u32 = 1;
/// The full priority order of story ids after a change:
/// `{order, submission}`.
pub const STORY_REPRIORITIZED: &str = "story.reprioritized";
/// The current `story.reprioritized` payload version.
pub const STORY_REPRIORITIZED_VERSION: u32 = 1;

/// The status a dropped story has. Only events and views hold it, so no
/// submission can state it.
const DROPPED: &str = "dropped";

/// The change kind of a submission that starts a project.
pub const START_KIND: &str = "start";
/// The target every start draft is filed under, since a project has one
/// start.
pub const START_TARGET: &str = "project";

/// Registers the eleven scope types: `scope.submitted` at version 2, read
/// from version 1 by adding the start's target, and the others at version
/// 1 with no upcasters.
pub fn register_scope_events(registry: &mut Registry) -> Result<(), RegistryError> {
    registry.register(
        SCOPE_SUBMITTED,
        SCOPE_SUBMITTED_VERSION,
        [(1, submitted_v1_to_v2 as Upcaster)],
    )?;
    for (name, version) in [
        (PROJECT_DESCRIBED, PROJECT_DESCRIBED_VERSION),
        (SCOPE_APPROVED, SCOPE_APPROVED_VERSION),
        (STORY_DECLARED, STORY_DECLARED_VERSION),
        (PHASE_DECLARED, PHASE_DECLARED_VERSION),
        (PHASE_REORDERED, PHASE_REORDERED_VERSION),
        (PHASE_WITHDRAWN, PHASE_WITHDRAWN_VERSION),
        (STORY_CORRECTED, STORY_CORRECTED_VERSION),
        (STORY_REASSIGNED, STORY_REASSIGNED_VERSION),
        (STORY_DROPPED, STORY_DROPPED_VERSION),
        (STORY_REPRIORITIZED, STORY_REPRIORITIZED_VERSION),
    ] {
        registry.register(name, version, [])?;
    }
    Ok(())
}

/// Reads a version-1 `scope.submitted` as version 2. Only start drafts
/// were recorded at version 1, so each gets the start's target, and any
/// other kind there is refused rather than filed under a guessed target.
fn submitted_v1_to_v2(payload: Value) -> Result<Value, UpcastError> {
    let Value::Object(mut payload) = payload else {
        return Err(UpcastError("the payload is not an object".into()));
    };
    if payload.get("kind").and_then(Value::as_str) != Some(START_KIND) {
        return Err(UpcastError(
            "only starts were recorded at version 1, and this submission is not one".into(),
        ));
    }
    payload.insert("target".into(), json!(START_TARGET));
    Ok(Value::Object(payload))
}

/// The payload of `scope.submitted`: the submission's digest, its change
/// kind, the target its draft is filed under and the reference to its
/// stored body, never the body's text.
pub fn submitted_payload(digest: Hash, kind: &str, target: &str, body: &PayloadRef) -> Value {
    json!({
        "digest": digest.to_hex(),
        "kind": kind,
        "target": target,
        "body": body.to_value(),
    })
}

/// The payload of `project.described` for `description`, accepted from the
/// submission with digest `submission`.
pub fn described_payload(description: &Description, submission: Hash) -> Value {
    json!({
        "what": description.what,
        "core_value": description.core_value,
        "constraints": description.constraints,
        "submission": submission.to_hex(),
    })
}

/// The payload of `scope.approved`: the approved digest, its change kind,
/// the owner's name and Baley's time of the approval.
pub fn approved_payload(submission: Hash, kind: &str, owner: &str, at: &str) -> Value {
    json!({
        "submission": submission.to_hex(),
        "kind": kind,
        "owner": owner,
        "at": at,
    })
}

/// The payload of `story.declared`: story `id` in `phase`, or in the
/// backlog when `phase` is `None`.
pub fn story_declared_payload(
    id: u32,
    sentence: &str,
    status: StoryStatus,
    phase: Option<u32>,
    submission: Hash,
) -> Value {
    json!({
        "id": id,
        "sentence": sentence,
        "status": status.as_str(),
        "phase": phase,
        "submission": submission.to_hex(),
    })
}

/// The payload of `phase.declared`: `detail` is the reference to a `record`
/// payload holding the detail text, and dependencies and stories are phase
/// numbers and story ids, never labels.
pub fn phase_declared_payload(
    number: u32,
    name: &str,
    goal: &str,
    detail: &PayloadRef,
    depends_on: &[u32],
    stories: &[u32],
    submission: Hash,
) -> Value {
    json!({
        "number": number,
        "name": name,
        "goal": goal,
        "detail": detail.to_value(),
        "depends_on": depends_on,
        "stories": stories,
        "submission": submission.to_hex(),
    })
}

/// The payload of `phase.reordered`: the full order of phase numbers.
pub fn phase_reordered_payload(order: &[u32], submission: Hash) -> Value {
    json!({
        "order": order,
        "submission": submission.to_hex(),
    })
}

/// The payload of `phase.withdrawn` for phase `number`.
pub fn phase_withdrawn_payload(number: u32, submission: Hash) -> Value {
    json!({
        "number": number,
        "submission": submission.to_hex(),
    })
}

/// The payload of `story.corrected`: story `id`'s new wording and its
/// status after the correction.
pub fn story_corrected_payload(
    id: u32,
    sentence: &str,
    status: StoryStatus,
    submission: Hash,
) -> Value {
    json!({
        "id": id,
        "sentence": sentence,
        "status": status.as_str(),
        "submission": submission.to_hex(),
    })
}

/// The payload of `story.reassigned`: story `id` in `phase`, or in the
/// backlog when `phase` is `None`, with its status after the move.
pub fn story_reassigned_payload(
    id: u32,
    status: StoryStatus,
    phase: Option<u32>,
    submission: Hash,
) -> Value {
    json!({
        "id": id,
        "status": status.as_str(),
        "phase": phase,
        "submission": submission.to_hex(),
    })
}

/// The payload of `story.dropped` for story `id`.
pub fn story_dropped_payload(id: u32, submission: Hash) -> Value {
    json!({
        "id": id,
        "status": DROPPED,
        "submission": submission.to_hex(),
    })
}

/// The payload of `story.reprioritized`: the full priority order of story
/// ids.
pub fn story_reprioritized_payload(order: &[u32], submission: Hash) -> Value {
    json!({
        "order": order,
        "submission": submission.to_hex(),
    })
}

#[cfg(test)]
mod tests {
    //! Expected payloads are written out from design 0004 section 6's field
    //! names, never from running this code.

    use baley_store::{EventSchema, RetentionClass};

    use super::*;

    /// The scope types still at version 1.
    const NAMES: [&str; 5] = [
        "project.described",
        "scope.approved",
        "story.declared",
        "phase.declared",
        "phase.reordered",
    ];

    fn digest() -> Hash {
        Hash([0x5c; 32])
    }

    fn reference(bytes: u64) -> PayloadRef {
        PayloadRef {
            hash: Hash([0xab; 32]),
            bytes,
            class: RetentionClass::Record,
        }
    }

    #[test]
    fn a_scope_event_left_unregistered_or_at_another_version_is_caught() {
        let mut registry = Registry::new();
        register_scope_events(&mut registry).unwrap();
        for name in NAMES {
            assert_eq!(registry.current_version(name), Some(1), "{name}");
            assert!(!registry.reads(name, 2), "{name}");
        }
        assert_eq!(registry.current_version("scope.submitted"), Some(2));
        assert!(!registry.reads("scope.submitted", 3));
        assert!(register_scope_events(&mut registry).is_err());
    }

    #[test]
    fn a_submission_event_carrying_its_body_or_missing_its_digest_is_caught() {
        assert_eq!(
            submitted_payload(digest(), START_KIND, START_TARGET, &reference(300)),
            json!({"digest": "5c".repeat(32), "kind": "start", "target": "project",
                "body": {"payload": "ab".repeat(32), "bytes": 300, "class": "record"}})
        );
    }

    #[test]
    fn a_submission_event_without_its_target_is_caught() {
        assert_eq!(
            submitted_payload(digest(), "phase-edit", "2", &reference(300)),
            json!({"digest": "5c".repeat(32), "kind": "phase-edit", "target": "2",
                "body": {"payload": "ab".repeat(32), "bytes": 300, "class": "record"}})
        );
        let mut registry = Registry::new();
        register_scope_events(&mut registry).unwrap();
        assert_eq!(registry.current_version("scope.submitted"), Some(2));
        assert!(registry.reads("scope.submitted", 1));
    }

    #[test]
    fn a_version_1_start_draft_upcast_without_the_start_target_is_caught() {
        let body = json!({"payload": "ab".repeat(32), "bytes": 300, "class": "record"});
        let start = json!({"digest": "5c".repeat(32), "kind": "start", "body": body});
        assert_eq!(
            submitted_v1_to_v2(start),
            Ok(
                json!({"digest": "5c".repeat(32), "kind": "start", "target": "project",
                "body": body})
            )
        );
        let edit = json!({"digest": "5c".repeat(32), "kind": "phase-edit", "body": body});
        assert!(submitted_v1_to_v2(edit).is_err());
    }

    #[test]
    fn an_approval_payload_missing_its_owner_time_or_kind_is_caught() {
        assert_eq!(
            approved_payload(digest(), START_KIND, "Ada Owner", "2026-10-13T09:00:02Z"),
            json!({"submission": "5c".repeat(32), "kind": "start", "owner": "Ada Owner",
                "at": "2026-10-13T09:00:02Z"})
        );
    }

    #[test]
    fn a_declaration_payload_with_inline_detail_or_story_labels_is_caught() {
        let detail = reference(18);
        assert_eq!(
            phase_declared_payload(
                2,
                "Approval",
                "The owner approves",
                &detail,
                &[1],
                &[2],
                digest()
            ),
            json!({"number": 2, "name": "Approval", "goal": "The owner approves",
                "detail": {"payload": "ab".repeat(32), "bytes": 18, "class": "record"},
                "depends_on": [1], "stories": [2], "submission": "5c".repeat(32)})
        );
        assert_eq!(
            story_declared_payload(
                3,
                "Teammates see the backlog",
                StoryStatus::Deferred,
                None,
                digest()
            ),
            json!({"id": 3, "sentence": "Teammates see the backlog", "status": "deferred",
                "phase": null, "submission": "5c".repeat(32)})
        );
        let description = Description {
            what: "A ledger for agent work".into(),
            core_value: "Nothing moves on an unproven claim".into(),
            constraints: vec!["One binary".into()],
        };
        assert_eq!(
            described_payload(&description, digest()),
            json!({"what": "A ledger for agent work",
                "core_value": "Nothing moves on an unproven claim",
                "constraints": ["One binary"], "submission": "5c".repeat(32)})
        );
        assert_eq!(
            phase_reordered_payload(&[1, 2], digest()),
            json!({"order": [1, 2], "submission": "5c".repeat(32)})
        );
    }

    #[test]
    fn a_change_event_left_unregistered_or_at_another_version_is_caught() {
        let mut registry = Registry::new();
        register_scope_events(&mut registry).unwrap();
        for name in [
            "phase.withdrawn",
            "story.corrected",
            "story.reassigned",
            "story.dropped",
            "story.reprioritized",
        ] {
            assert_eq!(registry.current_version(name), Some(1), "{name}");
            assert!(!registry.reads(name, 2), "{name}");
        }
        assert!(register_scope_events(&mut registry).is_err());
    }

    #[test]
    fn a_change_event_payload_missing_a_member_or_carrying_another_is_caught() {
        let submission = "5c".repeat(32);
        assert_eq!(
            phase_withdrawn_payload(2, digest()),
            json!({"number": 2, "submission": submission})
        );
        assert_eq!(
            story_corrected_payload(
                3,
                "Teammates read the backlog",
                StoryStatus::Deferred,
                digest()
            ),
            json!({"id": 3, "sentence": "Teammates read the backlog", "status": "deferred",
                "submission": submission})
        );
        assert_eq!(
            story_reassigned_payload(3, StoryStatus::Active, Some(2), digest()),
            json!({"id": 3, "status": "active", "phase": 2, "submission": submission})
        );
        assert_eq!(
            story_reassigned_payload(1, StoryStatus::Active, None, digest()),
            json!({"id": 1, "status": "active", "phase": null, "submission": submission})
        );
        assert_eq!(
            story_dropped_payload(3, digest()),
            json!({"id": 3, "status": "dropped", "submission": submission})
        );
        assert_eq!(
            story_reprioritized_payload(&[2, 1, 3], digest()),
            json!({"order": [2, 1, 3], "submission": submission})
        );
    }
}
