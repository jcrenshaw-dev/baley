//! The `story.refined` event: its type, its registration and its payload
//! (design 0005 section 6, design 0004 section 6, design 0001 Streams).
//!
//! A truth revision is approved as `story.refined`, never as
//! `scope.approved`, and the event is the approval's whole record, recorded
//! on the `roadmap` stream with actor `owner`.

use baley_store::Hash;
use serde_json::{Value, json};

use super::submission::Truth;
use crate::registry::{Registry, RegistryError};

/// A story's truths approved, on the `roadmap` stream:
/// `{story, set, submission, owner, at, truths}`.
///
/// The truths are inline, each with its version, and never a reference to
/// the draft's body: a purge of that body cannot remove truths in force
/// (D-04).
pub const STORY_REFINED: &str = "story.refined";
/// The current `story.refined` payload version.
pub const STORY_REFINED_VERSION: u32 = 1;

/// Registers `story.refined` at version 1 with no upcasters.
pub fn register_truth_events(registry: &mut Registry) -> Result<(), RegistryError> {
    registry.register(STORY_REFINED, STORY_REFINED_VERSION, [])
}

/// The payload of `story.refined`: story `story`'s whole truth list, each
/// truth with its version, drafted from question set `set` and approved as
/// the submission with digest `submission` by `owner` at `at`. Each truth
/// is written with every member it was submitted with: both attestations,
/// true in any accepted truth, and `human_verify` when it was given.
pub fn refined_payload(
    story: u32,
    set: &str,
    submission: Hash,
    owner: &str,
    at: &str,
    truths: &[(Truth, u32)],
) -> Value {
    let truths: Vec<Value> = truths
        .iter()
        .map(|(truth, version)| {
            let mut written = json!({
                "id": truth.id,
                "form": truth.form,
                "trigger": truth.trigger,
                "observer": truth.observer,
                "verb": truth.verb,
                "outcome": truth.outcome,
                "kind": truth.kind,
                "observable": true,
                "fixed_oracle": true,
                "version": version,
            });
            if let Some(human_verify) = &truth.human_verify {
                written["human_verify"] = json!(human_verify);
            }
            written
        })
        .collect();
    json!({
        "story": story,
        "set": set,
        "submission": submission.to_hex(),
        "owner": owner,
        "at": at,
        "truths": truths,
    })
}

#[cfg(test)]
mod tests {
    //! Expected payloads are written out from design 0005 section 6, D-04,
    //! D-06 and D-13 and the plan's test data, never from running this
    //! code.

    use baley_store::EventSchema;
    use serde_json::json;

    use super::*;

    const SET: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const AT: &str = "2026-10-22T09:00:00Z";

    #[test]
    fn a_refined_event_left_unregistered_or_at_another_version_is_caught() {
        let mut registry = Registry::new();
        register_truth_events(&mut registry).unwrap();
        assert_eq!(registry.current_version("story.refined"), Some(1));
        assert!(!registry.reads("story.refined", 2));
        assert!(register_truth_events(&mut registry).is_err());
    }

    #[test]
    fn a_refined_payload_missing_its_set_digest_owner_or_a_truth_is_caught() {
        let t1 = Truth {
            id: "t1".into(),
            form: "when".into(),
            trigger: "the owner approves a draft by a stale digest".into(),
            observer: "the owner".into(),
            verb: "is refused".into(),
            outcome: "stale-draft naming the first differing part".into(),
            kind: "literal".into(),
            human_verify: None,
        };
        let t2 = Truth {
            id: "t2".into(),
            form: "for-any".into(),
            trigger: "order the answers of a set are recorded in".into(),
            observer: "a ledger reader".into(),
            verb: "gets".into(),
            outcome: "the same round".into(),
            kind: "property".into(),
            human_verify: Some("a live endpoint".into()),
        };
        assert_eq!(
            refined_payload(
                1,
                SET,
                Hash([0x5c; 32]),
                "Ada Owner",
                AT,
                &[(t1, 1), (t2, 2)]
            ),
            json!({
                "story": 1,
                "set": SET,
                "submission": "5c".repeat(32),
                "owner": "Ada Owner",
                "at": AT,
                "truths": [
                    {"id": "t1", "form": "when",
                        "trigger": "the owner approves a draft by a stale digest",
                        "observer": "the owner", "verb": "is refused",
                        "outcome": "stale-draft naming the first differing part",
                        "kind": "literal", "observable": true, "fixed_oracle": true,
                        "version": 1},
                    {"id": "t2", "form": "for-any",
                        "trigger": "order the answers of a set are recorded in",
                        "observer": "a ledger reader", "verb": "gets",
                        "outcome": "the same round", "kind": "property",
                        "observable": true, "fixed_oracle": true,
                        "human_verify": "a live endpoint", "version": 2}
                ]
            })
        );
    }
}
