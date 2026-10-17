//! The `backlog` view: one document per project listing its stories in
//! priority order (design 0005's view row).

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, KeyValue, Projector, ProjectorError, ViewSpec,
};
use serde_json::{Value, json};

use super::event::{
    PHASE_DECLARED, STORY_CORRECTED, STORY_DECLARED, STORY_DROPPED, STORY_REASSIGNED,
    STORY_REPRIORITIZED,
};
use super::roadmap::reprioritized;

/// The view's name.
pub const BACKLOG_VIEW: &str = "backlog";

/// The key field the one document holds.
const KEY_FIELD: &str = "backlog";
/// The one document's key text.
const PROJECT_KEY: &str = "project";

/// The view's declaration: one text key, no indexes, and page bound 1,
/// since every read is a `get` of the one document.
pub fn backlog_spec() -> ViewSpec {
    ViewSpec {
        name: BACKLOG_VIEW.into(),
        version: 2,
        key: vec![FieldSpec {
            name: KEY_FIELD.into(),
            kind: FieldKind::Text,
        }],
        indexes: Vec::new(),
        page_bound: 1,
    }
}

/// The key of the project's backlog document.
pub fn backlog_key() -> DocKey {
    DocKey(vec![KeyValue::Text(PROJECT_KEY.into())])
}

/// Keeps the `backlog` view current from the story events and
/// `phase.declared`.
///
/// The stories are kept in declaration order, which is priority order
/// until stories are reprioritized. Each is unrefined with no truths and no
/// size until refinement and sizing record them. A later declaration of an
/// id already held replaces that entry in place, so it keeps its priority.
/// A correction, a move and a drop change a story in place; a dropped
/// story stays with status `dropped` and no phase. A reprioritization puts
/// the stories it lists first and every other story after them in its
/// current relative order. A phase declaration commits each story it lists
/// to that phase and returns to the backlog each story it no longer lists.
pub struct BacklogProjector {
    spec: ViewSpec,
}

impl BacklogProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self {
            spec: backlog_spec(),
        }
    }
}

impl Default for BacklogProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for BacklogProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[
            STORY_DECLARED,
            PHASE_DECLARED,
            STORY_CORRECTED,
            STORY_REASSIGNED,
            STORY_DROPPED,
            STORY_REPRIORITIZED,
        ]
    }

    fn keys(&self, _event: &Event) -> Vec<DocKey> {
        vec![backlog_key()]
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
        let fault = || refuse("a member is missing or of the wrong type");
        let no_story = || refuse("the story is not declared");
        let whole = |name: &str| payload.get(name).and_then(Value::as_u64).ok_or_else(fault);
        let text = |name: &str| payload.get(name).and_then(Value::as_str).ok_or_else(fault);
        let phase = || match payload.get("phase") {
            Some(Value::Null) => Ok(Value::Null),
            Some(phase) => Ok(json!(phase.as_u64().ok_or_else(fault)?)),
            None => Err(fault()),
        };
        let mut stories = match documents.first() {
            Some((_, document)) => document
                .get("stories")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| refuse("the backlog document holds no story list"))?,
            None => Vec::new(),
        };
        match event.type_name.as_str() {
            STORY_DECLARED => {
                let id = whole("id")?;
                let entry = json!({
                    "id": id,
                    "sentence": text("sentence")?,
                    "status": text("status")?,
                    "phase": phase()?,
                    "refinement": "unrefined",
                    "truths": 0,
                    "size": null,
                });
                match stories.iter().position(|story| story["id"] == id) {
                    Some(at) => stories[at] = entry,
                    None => stories.push(entry),
                }
            }
            PHASE_DECLARED => {
                let number = whole("number")?;
                let listed: Vec<u64> = payload
                    .get("stories")
                    .and_then(Value::as_array)
                    .and_then(|ids| ids.iter().map(Value::as_u64).collect())
                    .ok_or_else(fault)?;
                for story in &mut stories {
                    let held = story["id"].as_u64().is_some_and(|id| listed.contains(&id));
                    if held {
                        story["phase"] = json!(number);
                    } else if story["phase"] == number {
                        story["phase"] = Value::Null;
                    }
                }
            }
            STORY_CORRECTED => {
                let (sentence, status) = (text("sentence")?, text("status")?);
                let story = story_mut(&mut stories, whole("id")?).ok_or_else(no_story)?;
                story["sentence"] = json!(sentence);
                story["status"] = json!(status);
            }
            STORY_REASSIGNED => {
                let (status, phase) = (text("status")?, phase()?);
                let story = story_mut(&mut stories, whole("id")?).ok_or_else(no_story)?;
                story["status"] = json!(status);
                story["phase"] = phase;
            }
            STORY_DROPPED => {
                let status = text("status")?;
                let story = story_mut(&mut stories, whole("id")?).ok_or_else(no_story)?;
                story["status"] = json!(status);
                story["phase"] = Value::Null;
            }
            STORY_REPRIORITIZED => {
                let order: Vec<u64> = payload
                    .get("order")
                    .and_then(Value::as_array)
                    .and_then(|ids| ids.iter().map(Value::as_u64).collect())
                    .ok_or_else(fault)?;
                stories = reprioritized(stories, &order).ok_or_else(no_story)?;
            }
            _ => return Err(refuse("not a backlog event")),
        }
        Ok(vec![Change::Put {
            key: backlog_key(),
            body: json!({KEY_FIELD: PROJECT_KEY, "stories": stories}),
        }])
    }
}

/// The entry of story `id`.
fn story_mut(stories: &mut [Value], id: u64) -> Option<&mut Value> {
    stories.iter_mut().find(|story| story["id"] == id)
}

#[cfg(test)]
mod tests {
    //! Expected documents are written out from design 0005's backlog shape
    //! and the plan's approved S1, never from running this code.

    use std::collections::BTreeMap;

    use baley_store::{Actor, Hash, PayloadRef, ProjectId, RequestId, RetentionClass};
    use serde_json::json;

    use super::super::event::{
        phase_declared_payload, story_corrected_payload, story_declared_payload,
        story_dropped_payload, story_reassigned_payload, story_reprioritized_payload,
    };
    use super::super::submission::StoryStatus;
    use super::*;

    fn story(seq: u64, id: u32, sentence: &str, status: StoryStatus, phase: Option<u32>) -> Event {
        event(
            seq,
            "story.declared",
            story_declared_payload(id, sentence, status, phase, Hash([0x5c; 32])),
        )
    }

    fn event(seq: u64, type_name: &str, payload: Value) -> Event {
        Event {
            project_id: ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into()),
            seq,
            stream: "roadmap".into(),
            stream_version: seq,
            type_name: type_name.into(),
            type_version: 1,
            actor: Actor::Owner,
            caller: None,
            recorded_at: "2026-10-13T09:00:02Z".into(),
            request_id: RequestId("9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f".into()),
            git: None,
            policy_version: 0,
            payload,
            prev_hash: None,
            hash: Hash([0; 32]),
        }
    }

    /// The approved S1's three story declarations.
    fn approved_s1() -> Vec<Event> {
        vec![
            story(
                11,
                1,
                "Every step is recorded",
                StoryStatus::Active,
                Some(1),
            ),
            story(
                12,
                2,
                "The owner approves scope",
                StoryStatus::Active,
                Some(2),
            ),
            story(
                13,
                3,
                "Teammates see the backlog",
                StoryStatus::Deferred,
                None,
            ),
        ]
    }

    /// The stories after `events`, each applied to what the one before it
    /// left, as the store hands documents over.
    fn fold(events: &[Event]) -> Value {
        let projector = BacklogProjector::new();
        let mut view = BTreeMap::new();
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
        assert_eq!(view.len(), 1, "one document per project");
        view.remove(&backlog_key()).expect("the project's document")["stories"].clone()
    }

    #[test]
    fn a_backlog_out_of_priority_order_or_missing_refinement_and_truths_is_caught() {
        // Ids declared out of ascending order, so a sort by id cannot pass
        // for declaration order.
        let events = [
            story(
                11,
                3,
                "Teammates see the backlog",
                StoryStatus::Deferred,
                None,
            ),
            story(
                12,
                1,
                "Every step is recorded",
                StoryStatus::Active,
                Some(1),
            ),
            story(
                13,
                2,
                "The owner approves scope",
                StoryStatus::Active,
                Some(2),
            ),
        ];
        assert_eq!(
            fold(&events),
            json!([
                {"id": 3, "sentence": "Teammates see the backlog", "status": "deferred",
                    "phase": null, "refinement": "unrefined", "truths": 0, "size": null},
                {"id": 1, "sentence": "Every step is recorded", "status": "active", "phase": 1,
                    "refinement": "unrefined", "truths": 0, "size": null},
                {"id": 2, "sentence": "The owner approves scope", "status": "active",
                    "phase": 2, "refinement": "unrefined", "truths": 0, "size": null}
            ])
        );
    }

    #[test]
    fn a_redeclared_story_appended_to_the_backlog_a_second_time_is_caught() {
        let mut events = approved_s1();
        events.push(story(
            14,
            2,
            "The owner approves scope",
            StoryStatus::Deferred,
            None,
        ));
        let stories = fold(&events);
        let listed: Vec<(u64, &str, &Value)> = stories
            .as_array()
            .unwrap()
            .iter()
            .map(|story| {
                (
                    story["id"].as_u64().unwrap(),
                    story["status"].as_str().unwrap(),
                    &story["phase"],
                )
            })
            .collect();
        assert_eq!(
            listed,
            [
                (1, "active", &json!(1)),
                (2, "deferred", &Value::Null),
                (3, "deferred", &Value::Null)
            ]
        );
    }

    /// Phase `number` declared with `stories`.
    fn phase(seq: u64, number: u32, stories: &[u32]) -> Event {
        let detail = PayloadRef {
            hash: Hash([0xab; 32]),
            bytes: 24,
            class: RetentionClass::Record,
        };
        event(
            seq,
            "phase.declared",
            phase_declared_payload(
                number,
                "Phase",
                "A goal",
                &detail,
                &[],
                stories,
                Hash([0x5c; 32]),
            ),
        )
    }

    /// The stories after the approved S1, then `events`.
    fn after_s1(events: Vec<Event>) -> Value {
        let mut all = approved_s1();
        all.extend(events);
        fold(&all)
    }

    fn ids(stories: &Value) -> Vec<u64> {
        stories
            .as_array()
            .unwrap()
            .iter()
            .map(|story| story["id"].as_u64().unwrap())
            .collect()
    }

    fn entry(stories: &Value, id: u64) -> Value {
        stories
            .as_array()
            .unwrap()
            .iter()
            .find(|story| story["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("story {id} in {stories}"))
    }

    #[test]
    fn a_backlog_changed_by_an_event_it_does_not_handle_is_caught() {
        assert_eq!(
            BacklogProjector::new().handles(),
            [
                "story.declared",
                "phase.declared",
                "story.corrected",
                "story.reassigned",
                "story.dropped",
                "story.reprioritized"
            ]
        );
        assert_eq!(backlog_spec().version, 2);
    }

    #[test]
    fn a_backlog_story_change_lost_or_moving_its_place_is_caught() {
        let digest = Hash([0x5c; 32]);
        let stories = after_s1(vec![
            event(
                14,
                "story.corrected",
                story_corrected_payload(
                    3,
                    "Teammates read the backlog",
                    StoryStatus::Deferred,
                    digest,
                ),
            ),
            event(
                15,
                "story.reassigned",
                story_reassigned_payload(3, StoryStatus::Active, Some(2), digest),
            ),
            event(16, "story.dropped", story_dropped_payload(1, digest)),
        ]);
        assert_eq!(ids(&stories), [1, 2, 3]);
        let third = entry(&stories, 3);
        assert_eq!(third["sentence"], "Teammates read the backlog");
        assert_eq!(third["status"], "active");
        assert_eq!(third["phase"], 2);
        let first = entry(&stories, 1);
        assert_eq!(first["status"], "dropped");
        assert_eq!(first["phase"], Value::Null);
        for story in stories.as_array().unwrap() {
            assert_eq!(story["refinement"], "unrefined", "{story}");
            assert_eq!(story["truths"], 0, "{story}");
            assert_eq!(story["size"], Value::Null, "{story}");
        }
    }

    #[test]
    fn a_reprioritized_backlog_out_of_order_or_losing_a_dropped_story_is_caught() {
        let digest = Hash([0x5c; 32]);
        let stories = after_s1(vec![
            event(14, "story.dropped", story_dropped_payload(3, digest)),
            event(
                15,
                "story.reprioritized",
                story_reprioritized_payload(&[2, 1], digest),
            ),
        ]);
        assert_eq!(ids(&stories), [2, 1, 3]);
        assert_eq!(entry(&stories, 3)["status"], "dropped");
    }

    #[test]
    fn a_story_a_declaration_commits_left_in_the_backlog_is_caught() {
        let stories = after_s1(vec![
            story(14, 4, "Reports show progress", StoryStatus::Active, None),
            phase(15, 3, &[4]),
            phase(16, 2, &[]),
        ]);
        assert_eq!(entry(&stories, 4)["phase"], 3);
        assert_eq!(entry(&stories, 2)["phase"], Value::Null);
    }
}
