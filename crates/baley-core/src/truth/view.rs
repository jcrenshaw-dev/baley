//! The `truths` view: one document per story, keyed by its id, holding the
//! story's current truths and every version ever approved (design 0005
//! sections 6 and 7, PLN-R4, D-11).
//!
//! The store keys every view by project, so a document is one project's
//! story. No refinement state is stored here: the backlog's states are
//! worked out when read.

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, KeyValue, Projector, ProjectorError, ViewSpec,
};
use serde_json::{Map, Value, json};

use super::event::STORY_REFINED;

/// The view's name.
pub const TRUTHS_VIEW: &str = "truths";

/// The key field: the story's id.
const STORY_FIELD: &str = "story";

/// The view's declaration: the story id as an integer key and no index.
pub fn truths_spec() -> ViewSpec {
    ViewSpec {
        name: TRUTHS_VIEW.into(),
        version: 1,
        key: vec![FieldSpec {
            name: STORY_FIELD.into(),
            kind: FieldKind::Integer,
        }],
        indexes: Vec::new(),
        page_bound: 100,
    }
}

/// The key of story `story`'s document.
pub fn truths_key(story: u32) -> DocKey {
    DocKey(vec![KeyValue::Integer(i64::from(story))])
}

/// Keeps the `truths` view current from `story.refined`.
///
/// Each refinement replaces the story's document with these members:
/// - `story`: the story's id.
/// - `set` and `submission`: the question set and approved digest the
///   latest `story.refined` names.
/// - `seq` and `recorded_at`: that event's sequence and time.
/// - `truths`: the current truths in submitted order, each as the event
///   carries it, with its `version`.
/// - `kept`: by truth id, every version ever approved, oldest first, each
///   `{version, seq, truth}` with the sequence of the `story.refined` that
///   first carried it. A truth carried again at the same version adds no
///   entry, so each version is kept once.
///
/// A payload whose story, set, digest or truth list cannot be read is
/// refused.
pub struct TruthsProjector {
    spec: ViewSpec,
}

impl TruthsProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self {
            spec: truths_spec(),
        }
    }
}

impl Default for TruthsProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for TruthsProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[STORY_REFINED]
    }

    fn keys(&self, event: &Event) -> Vec<DocKey> {
        story_of(&event.payload)
            .map(truths_key)
            .into_iter()
            .collect()
    }

    fn apply(
        &self,
        event: &Event,
        documents: &[(DocKey, Value)],
    ) -> Result<Vec<Change>, ProjectorError> {
        let refuse = |message: &str| {
            ProjectorError(format!(
                "{} at seq {}: {message}",
                event.type_name, event.seq
            ))
        };
        let payload = &event.payload;
        let story = story_of(payload).ok_or_else(|| refuse("story is not a story id"))?;
        let text = |name: &str| {
            payload
                .get(name)
                .and_then(Value::as_str)
                .ok_or_else(|| refuse(&format!("{name} is not text")))
        };
        let set = text("set")?;
        let submission = text("submission")?;
        let Some(Value::Array(truths)) = payload.get("truths") else {
            return Err(refuse("truths is not a list"));
        };
        let key = truths_key(story);
        let mut kept = documents
            .iter()
            .find(|(at, _)| *at == key)
            .and_then(|(_, document)| document.get("kept"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for truth in truths {
            let id = truth.get("id").and_then(Value::as_str);
            let version = truth.get("version").and_then(Value::as_u64);
            let (Some(id), Some(version)) = (id, version) else {
                return Err(refuse("a truth has no id or version"));
            };
            if !keep(&mut kept, id, version, event.seq, truth) {
                return Err(refuse("a kept version list is not a list"));
            }
        }
        Ok(vec![Change::Put {
            key,
            body: json!({
                "story": story,
                "set": set,
                "submission": submission,
                "seq": event.seq,
                "recorded_at": event.recorded_at,
                "truths": truths,
                "kept": kept,
            }),
        }])
    }
}

/// Adds `truth` at `version` to `id`'s kept versions unless that version is
/// already kept, so a truth carried again unchanged is kept once. False when
/// the id's kept versions are not a list.
fn keep(kept: &mut Map<String, Value>, id: &str, version: u64, seq: u64, truth: &Value) -> bool {
    let Value::Array(versions) = kept.entry(id).or_insert_with(|| json!([])) else {
        return false;
    };
    if !versions.iter().any(|entry| entry["version"] == version) {
        versions.push(json!({"version": version, "seq": seq, "truth": truth}));
    }
    true
}

/// The payload's story id, when it is a whole number within `u32`.
fn story_of(payload: &Value) -> Option<u32> {
    payload
        .get(STORY_FIELD)
        .and_then(Value::as_u64)
        .and_then(|story| u32::try_from(story).ok())
}

#[cfg(test)]
mod tests {
    //! Expected documents are written out from the projector's documented
    //! members, PLN-R4 and the plan's test data, never from running this
    //! code.

    use std::collections::BTreeMap;

    use baley_store::{Actor, Hash, ProjectId, RequestId};

    use super::*;

    const S5: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const S6: &str = "7f3e4d5c-6071-4c8d-8e9f-1a2b3c4d5e6f";
    const S7: &str = "8a4f5e6d-7182-4d9e-9fa0-2b3c4d5e6f70";
    const AT: &str = "2026-10-22T09:00:00Z";

    type View = BTreeMap<DocKey, Value>;

    fn event(seq: u64, payload: Value) -> Event {
        Event {
            project_id: ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into()),
            seq,
            stream: "roadmap".into(),
            stream_version: seq,
            type_name: "story.refined".into(),
            type_version: 1,
            actor: Actor::Owner,
            caller: None,
            recorded_at: format!("2026-10-22T09:00:{seq:02}Z"),
            request_id: RequestId("9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f".into()),
            git: None,
            policy_version: 0,
            payload,
            prev_hash: None,
            hash: Hash([0; 32]),
        }
    }

    /// A truth as `story.refined` carries it, its outcome naming its
    /// version so two versions differ in text.
    fn truth(id: &str, version: u32) -> Value {
        json!({"id": id, "form": "when", "trigger": format!("{id} is triggered"),
            "observer": "the owner", "verb": "sees", "outcome": format!("{id} at {version}"),
            "kind": "literal", "observable": true, "fixed_oracle": true, "version": version})
    }

    fn refined(seq: u64, story: u32, set: &str, digest: &str, truths: &[(&str, u32)]) -> Event {
        let truths: Vec<Value> = truths.iter().map(|(id, v)| truth(id, *v)).collect();
        event(
            seq,
            json!({"story": story, "set": set, "submission": digest, "owner": "Ada Owner",
                "at": AT, "truths": truths}),
        )
    }

    fn fold(events: &[Event]) -> View {
        let projector = TruthsProjector::new();
        let mut view = View::new();
        for event in events {
            let read: Vec<(DocKey, Value)> = projector
                .keys(event)
                .into_iter()
                .filter_map(|key| view.get(&key).cloned().map(|body| (key, body)))
                .collect();
            for change in projector.apply(event, &read).unwrap() {
                match change {
                    Change::Put { key, body } => {
                        view.insert(key, body);
                    }
                    Change::Delete { key } => {
                        view.remove(&key);
                    }
                }
            }
        }
        view
    }

    fn kept(version: u32, seq: u64, id: &str) -> Value {
        json!({"version": version, "seq": seq, "truth": truth(id, version)})
    }

    #[test]
    fn a_truths_view_declared_under_another_name_or_key_is_caught() {
        let spec = truths_spec();
        assert_eq!(spec.name, "truths");
        assert_eq!(spec.version, 1);
        assert_eq!(
            spec.key,
            [FieldSpec {
                name: "story".into(),
                kind: FieldKind::Integer,
            }]
        );
        assert!(spec.indexes.is_empty());
        assert_eq!(TruthsProjector::new().handles(), ["story.refined"]);
    }

    #[test]
    fn a_kept_version_lost_or_kept_twice_is_caught() {
        let d5 = "5c".repeat(32);
        let d6 = "6d".repeat(32);
        let d7 = "7e".repeat(32);
        let view = fold(&[
            refined(5, 1, S5, &d5, &[("t1", 1), ("t2", 1)]),
            refined(6, 1, S6, &d6, &[("t1", 1), ("t2", 2), ("t3", 1)]),
            refined(7, 1, S7, &d7, &[("t1", 1), ("t2", 2)]),
        ]);
        assert_eq!(view.len(), 1);
        assert_eq!(
            view[&DocKey(vec![KeyValue::Integer(1)])],
            json!({
                "story": 1,
                "set": S7,
                "submission": d7,
                "seq": 7,
                "recorded_at": "2026-10-22T09:00:07Z",
                "truths": [truth("t1", 1), truth("t2", 2)],
                "kept": {
                    "t1": [kept(1, 5, "t1")],
                    "t2": [kept(1, 5, "t2"), kept(2, 6, "t2")],
                    "t3": [kept(1, 6, "t3")]
                }
            })
        );
    }

    #[test]
    fn a_refinement_of_another_story_mixed_in_is_caught() {
        let d5 = "5c".repeat(32);
        let d6 = "6d".repeat(32);
        let view = fold(&[
            refined(5, 1, S5, &d5, &[("t1", 1)]),
            refined(6, 2, S6, &d6, &[("t2", 1)]),
        ]);
        assert_eq!(view.len(), 2);
        let one = &view[&DocKey(vec![KeyValue::Integer(1)])];
        assert_eq!(one["truths"], json!([truth("t1", 1)]));
        assert_eq!(one["kept"], json!({"t1": [kept(1, 5, "t1")]}));
        let two = &view[&DocKey(vec![KeyValue::Integer(2)])];
        assert_eq!(two["story"], 2);
        assert_eq!(two["truths"], json!([truth("t2", 1)]));
        assert_eq!(two["kept"], json!({"t2": [kept(1, 6, "t2")]}));
    }
}
