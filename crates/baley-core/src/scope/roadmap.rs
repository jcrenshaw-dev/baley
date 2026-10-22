//! The `roadmap` view: one document per project holding its accepted
//! scope (design 0004 section 6).

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, KeyValue, Projector, ProjectorError, ViewSpec,
};
use serde_json::{Map, Value, json};

use super::event::{
    PHASE_DECLARED, PHASE_REORDERED, PHASE_WITHDRAWN, PROJECT_DESCRIBED, STORY_CORRECTED,
    STORY_DECLARED, STORY_DROPPED, STORY_REASSIGNED, STORY_REPRIORITIZED,
};
use crate::truth::STORY_REFINED;

/// The view's name.
pub const ROADMAP_VIEW: &str = "roadmap";

/// The key field the one document holds.
const KEY_FIELD: &str = "roadmap";
/// The one document's key text.
const PROJECT_KEY: &str = "project";

/// The view's declaration: one text key, no indexes, and page bound 1,
/// since every read is a `get` of the one document.
pub fn roadmap_spec() -> ViewSpec {
    ViewSpec {
        name: ROADMAP_VIEW.into(),
        version: 3,
        key: vec![FieldSpec {
            name: KEY_FIELD.into(),
            kind: FieldKind::Text,
        }],
        indexes: Vec::new(),
        page_bound: 1,
    }
}

/// The key of the project's roadmap document.
pub fn roadmap_key() -> DocKey {
    DocKey(vec![KeyValue::Text(PROJECT_KEY.into())])
}

/// Keeps the `roadmap` view current from the events an approval records.
///
/// The document holds the latest description, the declared phases by
/// number, the latest phase order, the stories in priority order, and the
/// highest phase number and story id ever declared. Those two never go
/// down, so an approval numbers past every declaration, withdrawn phases
/// included. A later declaration of a number or id already held replaces
/// that entry in place. Submissions and approvals themselves are not
/// handled: accepted scope changes only on the events an approval records.
///
/// A phase's `stories` are the stories it serves now, and each story's
/// `phase` the phase serving it, kept in step by every event that moves a
/// story. A withdrawn phase keeps its entry with status `withdrawn`; it
/// leaves the order only on the `phase.reordered` its approval records. A
/// dropped story keeps its entry with status `dropped` and no phase, and a
/// reprioritization puts the stories it lists first and every other story
/// after them in its current relative order.
///
/// A story's `truths` is 0 when it is declared and, once refined, the
/// length of the truth list of its latest `story.refined`, which replaces
/// the count before it. A refinement of a story not held is refused.
pub struct RoadmapProjector {
    spec: ViewSpec,
}

impl RoadmapProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self {
            spec: roadmap_spec(),
        }
    }
}

impl Default for RoadmapProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for RoadmapProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[
            PROJECT_DESCRIBED,
            STORY_DECLARED,
            PHASE_DECLARED,
            PHASE_REORDERED,
            PHASE_WITHDRAWN,
            STORY_CORRECTED,
            STORY_REASSIGNED,
            STORY_DROPPED,
            STORY_REPRIORITIZED,
            STORY_REFINED,
        ]
    }

    fn keys(&self, _event: &Event) -> Vec<DocKey> {
        vec![roadmap_key()]
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
        let Value::Object(payload) = &event.payload else {
            return Err(refuse("the payload is not an object"));
        };
        let mut document = match documents.first() {
            Some((_, Value::Object(document))) => document.clone(),
            Some(_) => return Err(refuse("the roadmap document is not an object")),
            None => empty(),
        };
        let fault = || refuse("a member is missing or of the wrong type");
        let no_story = || refuse("the story is not declared");
        match event.type_name.as_str() {
            PROJECT_DESCRIBED => {
                let description = json!({
                    "what": text(payload, "what").ok_or_else(fault)?,
                    "core_value": text(payload, "core_value").ok_or_else(fault)?,
                    "constraints": texts(payload, "constraints").ok_or_else(fault)?,
                    "submission": text(payload, "submission").ok_or_else(fault)?,
                });
                document.insert("description".into(), description);
            }
            STORY_DECLARED => {
                let id = whole(payload, "id").ok_or_else(fault)?;
                let phase = phase_or_null(payload).ok_or_else(fault)?;
                let entry = json!({
                    "id": id,
                    "sentence": text(payload, "sentence").ok_or_else(fault)?,
                    "status": text(payload, "status").ok_or_else(fault)?,
                    "phase": phase,
                    "truths": 0,
                });
                let stories = entries(&mut document, "stories").ok_or_else(fault)?;
                match stories.iter().position(|story| story["id"] == id) {
                    Some(at) => stories[at] = entry,
                    None => stories.push(entry),
                }
                raise(&mut document, "highest_story", id);
                // At a start the phase is declared after its stories and
                // lists them itself.
                if let Some(entry) = phase
                    .as_u64()
                    .and_then(|number| phase_mut(&mut document, number))
                {
                    serve(entry, id);
                }
            }
            PHASE_DECLARED => {
                let number = whole(payload, "number").ok_or_else(fault)?;
                let listed = wholes(payload, "stories").ok_or_else(fault)?;
                let entry = json!({
                    "number": number,
                    "name": text(payload, "name").ok_or_else(fault)?,
                    "goal": text(payload, "goal").ok_or_else(fault)?,
                    "depends_on": wholes(payload, "depends_on").ok_or_else(fault)?,
                    "stories": listed,
                    "status": "declared",
                });
                let phases = entries(&mut document, "phases").ok_or_else(fault)?;
                let at = phases.partition_point(|phase| {
                    phase["number"].as_u64().is_some_and(|held| held < number)
                });
                if phases
                    .get(at)
                    .is_some_and(|phase| phase["number"] == number)
                {
                    phases[at] = entry;
                } else {
                    phases.insert(at, entry);
                }
                raise(&mut document, "highest_phase", number);
                // An edit that leaves a story out returns it to the backlog.
                for story in entries(&mut document, "stories").ok_or_else(fault)? {
                    let listed = story["id"].as_u64().is_some_and(|id| listed.contains(&id));
                    if listed {
                        story["phase"] = json!(number);
                    } else if story["phase"] == number {
                        story["phase"] = Value::Null;
                    }
                }
            }
            PHASE_REORDERED => {
                let order = wholes(payload, "order").ok_or_else(fault)?;
                document.insert("order".into(), json!(order));
            }
            PHASE_WITHDRAWN => {
                let number = whole(payload, "number").ok_or_else(fault)?;
                let entry = phase_mut(&mut document, number)
                    .ok_or_else(|| refuse("the phase is not declared"))?;
                entry["status"] = json!("withdrawn");
            }
            STORY_CORRECTED => {
                let id = whole(payload, "id").ok_or_else(fault)?;
                let sentence = text(payload, "sentence").ok_or_else(fault)?;
                let status = text(payload, "status").ok_or_else(fault)?;
                let story = story_mut(&mut document, id).ok_or_else(no_story)?;
                story["sentence"] = json!(sentence);
                story["status"] = json!(status);
            }
            STORY_REASSIGNED => {
                let id = whole(payload, "id").ok_or_else(fault)?;
                let status = text(payload, "status").ok_or_else(fault)?;
                let phase = phase_or_null(payload).ok_or_else(fault)?;
                unserve(&mut document, id);
                let story = story_mut(&mut document, id).ok_or_else(no_story)?;
                story["status"] = json!(status);
                story["phase"] = phase.clone();
                if let Some(number) = phase.as_u64() {
                    let entry = phase_mut(&mut document, number)
                        .ok_or_else(|| refuse("the story's new phase is not declared"))?;
                    serve(entry, id);
                }
            }
            STORY_DROPPED => {
                let id = whole(payload, "id").ok_or_else(fault)?;
                let status = text(payload, "status").ok_or_else(fault)?;
                unserve(&mut document, id);
                let story = story_mut(&mut document, id).ok_or_else(no_story)?;
                story["status"] = json!(status);
                story["phase"] = Value::Null;
            }
            STORY_REPRIORITIZED => {
                let order = wholes(payload, "order").ok_or_else(fault)?;
                let stories = entries(&mut document, "stories").ok_or_else(fault)?;
                *stories = reprioritized(std::mem::take(stories), &order).ok_or_else(no_story)?;
            }
            STORY_REFINED => {
                let id = whole(payload, "story").ok_or_else(fault)?;
                let Some(Value::Array(truths)) = payload.get("truths") else {
                    return Err(fault());
                };
                let count = truths.len();
                let story = story_mut(&mut document, id).ok_or_else(no_story)?;
                story["truths"] = json!(count);
            }
            _ => return Err(refuse("not a roadmap event")),
        }
        Ok(vec![Change::Put {
            key: roadmap_key(),
            body: Value::Object(document),
        }])
    }
}

/// The stories `order` lists first, in its order, then every other story
/// in its current relative order. None when the order names a story not
/// held.
pub(super) fn reprioritized(mut stories: Vec<Value>, order: &[u64]) -> Option<Vec<Value>> {
    let mut ordered = Vec::with_capacity(stories.len());
    for id in order {
        let at = stories.iter().position(|story| story["id"] == *id)?;
        ordered.push(stories.remove(at));
    }
    ordered.extend(stories);
    Some(ordered)
}

/// The payload's `phase`: a number, or null for the backlog.
fn phase_or_null(payload: &Map<String, Value>) -> Option<Value> {
    match payload.get("phase") {
        Some(Value::Null) => Some(Value::Null),
        _ => whole(payload, "phase").map(|phase| json!(phase)),
    }
}

/// The entry of phase `number`.
fn phase_mut(document: &mut Map<String, Value>, number: u64) -> Option<&mut Value> {
    entries(document, "phases")?
        .iter_mut()
        .find(|phase| phase["number"] == number)
}

/// The entry of story `id`.
fn story_mut(document: &mut Map<String, Value>, id: u64) -> Option<&mut Value> {
    entries(document, "stories")?
        .iter_mut()
        .find(|story| story["id"] == id)
}

/// Lists story `id` last among the phase entry's stories, unless listed.
fn serve(phase: &mut Value, id: u64) {
    if let Some(stories) = phase.get_mut("stories").and_then(Value::as_array_mut)
        && !stories.iter().any(|story| *story == id)
    {
        stories.push(json!(id));
    }
}

/// Takes story `id` off every phase entry that lists it.
fn unserve(document: &mut Map<String, Value>, id: u64) {
    for phase in entries(document, "phases").into_iter().flatten() {
        if let Some(stories) = phase.get_mut("stories").and_then(Value::as_array_mut) {
            stories.retain(|story| *story != id);
        }
    }
}

/// The document of a project with no accepted scope yet.
fn empty() -> Map<String, Value> {
    let Value::Object(document) = json!({
        KEY_FIELD: PROJECT_KEY,
        "description": null,
        "phases": [],
        "order": [],
        "stories": [],
        "highest_phase": 0,
        "highest_story": 0,
    }) else {
        unreachable!("an object literal")
    };
    document
}

fn text<'a>(payload: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    payload.get(name).and_then(Value::as_str)
}

fn texts<'a>(payload: &'a Map<String, Value>, name: &str) -> Option<Vec<&'a str>> {
    payload
        .get(name)?
        .as_array()?
        .iter()
        .map(Value::as_str)
        .collect()
}

fn whole(payload: &Map<String, Value>, name: &str) -> Option<u64> {
    payload.get(name).and_then(Value::as_u64)
}

fn wholes(payload: &Map<String, Value>, name: &str) -> Option<Vec<u64>> {
    payload
        .get(name)?
        .as_array()?
        .iter()
        .map(Value::as_u64)
        .collect()
}

/// The document's list `name`, which the empty document always has.
fn entries<'a>(document: &'a mut Map<String, Value>, name: &str) -> Option<&'a mut Vec<Value>> {
    document.get_mut(name)?.as_array_mut()
}

/// Raises `name` to `value`, never lowering it.
fn raise(document: &mut Map<String, Value>, name: &str, value: u64) {
    let held = document.get(name).and_then(Value::as_u64).unwrap_or(0);
    document.insert(name.into(), json!(held.max(value)));
}

#[cfg(test)]
mod tests {
    //! Expected documents are written out from design 0004's roadmap shape
    //! and the approved S1 of the plan, never from running this code.

    use std::collections::BTreeMap;

    use baley_store::{Actor, Hash, PayloadRef, ProjectId, RequestId, RetentionClass};
    use serde_json::json;

    use super::super::event::{
        described_payload, phase_declared_payload, phase_reordered_payload,
        phase_withdrawn_payload, story_corrected_payload, story_declared_payload,
        story_dropped_payload, story_reassigned_payload, story_reprioritized_payload,
    };
    use super::super::submission::{Description, StoryStatus};
    use super::*;

    const AT: &str = "2026-10-13T09:00:02Z";

    fn digest() -> Hash {
        Hash([0x5c; 32])
    }

    fn event(type_name: &str, seq: u64, payload: Value) -> Event {
        Event {
            project_id: ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into()),
            seq,
            stream: "roadmap".into(),
            stream_version: seq,
            type_name: type_name.into(),
            type_version: 1,
            actor: Actor::Owner,
            caller: None,
            recorded_at: AT.into(),
            request_id: RequestId("9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f".into()),
            git: None,
            policy_version: 0,
            payload,
            prev_hash: None,
            hash: Hash([0; 32]),
        }
    }

    fn detail() -> PayloadRef {
        PayloadRef {
            hash: Hash([0xab; 32]),
            bytes: 24,
            class: RetentionClass::Record,
        }
    }

    fn story(seq: u64, id: u32, sentence: &str, status: StoryStatus, phase: Option<u32>) -> Event {
        event(
            "story.declared",
            seq,
            story_declared_payload(id, sentence, status, phase, digest()),
        )
    }

    fn phase(
        seq: u64,
        number: u32,
        name: &str,
        goal: &str,
        depends_on: &[u32],
        stories: &[u32],
    ) -> Event {
        event(
            "phase.declared",
            seq,
            phase_declared_payload(number, name, goal, &detail(), depends_on, stories, digest()),
        )
    }

    /// The document after `events`, each applied to what the one before it
    /// left, as the store hands documents over.
    fn fold(events: &[Event]) -> Value {
        let projector = RoadmapProjector::new();
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
        view.remove(&roadmap_key()).expect("the project's document")
    }

    #[test]
    fn an_approved_start_projected_out_of_order_or_without_its_description_is_caught() {
        let description = Description {
            what: "A ledger for agent work".into(),
            core_value: "Nothing moves on an unproven claim".into(),
            constraints: vec!["One binary".into()],
        };
        let document = fold(&[
            event(
                "project.described",
                10,
                described_payload(&description, digest()),
            ),
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
            phase(14, 1, "Records", "Steps are recorded", &[], &[1]),
            phase(15, 2, "Approval", "The owner approves", &[1], &[2]),
            event(
                "phase.reordered",
                16,
                phase_reordered_payload(&[1, 2], digest()),
            ),
            // A start's order is always 1 to n, so a later insertion gives
            // the order a sort by number cannot reproduce.
            phase(17, 3, "Reads", "Teammates read the roadmap", &[], &[]),
            event(
                "phase.reordered",
                18,
                phase_reordered_payload(&[3, 1, 2], digest()),
            ),
        ]);
        let submission = "5c".repeat(32);
        assert_eq!(
            document,
            json!({
                "roadmap": "project",
                "description": {"what": "A ledger for agent work",
                    "core_value": "Nothing moves on an unproven claim",
                    "constraints": ["One binary"], "submission": submission},
                "phases": [
                    {"number": 1, "name": "Records", "goal": "Steps are recorded",
                        "depends_on": [], "stories": [1], "status": "declared"},
                    {"number": 2, "name": "Approval", "goal": "The owner approves",
                        "depends_on": [1], "stories": [2], "status": "declared"},
                    {"number": 3, "name": "Reads", "goal": "Teammates read the roadmap",
                        "depends_on": [], "stories": [], "status": "declared"}
                ],
                "order": [3, 1, 2],
                "stories": [
                    {"id": 1, "sentence": "Every step is recorded", "status": "active",
                        "phase": 1, "truths": 0},
                    {"id": 2, "sentence": "The owner approves scope", "status": "active",
                        "phase": 2, "truths": 0},
                    {"id": 3, "sentence": "Teammates see the backlog", "status": "deferred",
                        "phase": null, "truths": 0}
                ],
                "highest_phase": 3,
                "highest_story": 3
            })
        );
    }

    #[test]
    fn a_submission_projected_as_accepted_scope_is_caught() {
        assert_eq!(
            RoadmapProjector::new().handles(),
            [
                "project.described",
                "story.declared",
                "phase.declared",
                "phase.reordered",
                "phase.withdrawn",
                "story.corrected",
                "story.reassigned",
                "story.dropped",
                "story.reprioritized",
                "story.refined"
            ]
        );
    }

    #[test]
    fn a_redeclared_phase_or_story_listed_twice_or_lowering_the_highest_value_is_caught() {
        let document = fold(&[
            phase(1, 2, "Approval", "The owner approves", &[1], &[2]),
            phase(2, 1, "Records", "Steps are recorded", &[], &[1]),
            phase(3, 1, "Records", "New goal", &[], &[1]),
            story(
                4,
                3,
                "Teammates see the backlog",
                StoryStatus::Deferred,
                None,
            ),
            story(5, 1, "Every step is recorded", StoryStatus::Active, Some(1)),
            story(6, 1, "New sentence", StoryStatus::Active, Some(1)),
        ]);
        let phases: Vec<(u64, &str)> = document["phases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|phase| {
                (
                    phase["number"].as_u64().unwrap(),
                    phase["goal"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(phases, [(1, "New goal"), (2, "The owner approves")]);
        assert_eq!(document["highest_phase"], 2);
        let stories: Vec<(u64, &str)> = document["stories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|story| {
                (
                    story["id"].as_u64().unwrap(),
                    story["sentence"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            stories,
            [(3, "Teammates see the backlog"), (1, "New sentence")]
        );
        assert_eq!(document["highest_story"], 3);
    }

    /// The approved S1's events: stories 1 and 2 active in phases 1 and 2,
    /// story 3 deferred in the backlog, phase 2 depending on phase 1, and
    /// the order `[1, 2]`.
    fn approved_s1() -> Vec<Event> {
        let description = Description {
            what: "A ledger for agent work".into(),
            core_value: "Nothing moves on an unproven claim".into(),
            constraints: vec!["One binary".into()],
        };
        vec![
            event(
                "project.described",
                10,
                described_payload(&description, digest()),
            ),
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
            phase(14, 1, "Records", "Steps are recorded", &[], &[1]),
            phase(15, 2, "Approval", "The owner approves", &[1], &[2]),
            event(
                "phase.reordered",
                16,
                phase_reordered_payload(&[1, 2], digest()),
            ),
        ]
    }

    /// The document after the approved S1, then `events`.
    fn after_s1(events: Vec<Event>) -> Value {
        let mut all = approved_s1();
        all.extend(events);
        fold(&all)
    }

    fn reassigned(seq: u64, id: u32, phase: Option<u32>) -> Event {
        event(
            "story.reassigned",
            seq,
            story_reassigned_payload(id, StoryStatus::Active, phase, digest()),
        )
    }

    fn dropped(seq: u64, id: u32) -> Event {
        event("story.dropped", seq, story_dropped_payload(id, digest()))
    }

    /// The entry of phase `number`.
    fn phase_entry(document: &Value, number: u64) -> Value {
        document["phases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|phase| phase["number"] == number)
            .cloned()
            .unwrap_or_else(|| panic!("phase {number} in {document}"))
    }

    /// The entry of story `id`.
    fn story_entry(document: &Value, id: u64) -> Value {
        document["stories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|story| story["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("story {id} in {document}"))
    }

    fn story_ids(document: &Value) -> Vec<u64> {
        document["stories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|story| story["id"].as_u64().unwrap())
            .collect()
    }

    #[test]
    fn a_withdrawn_phase_dropped_from_the_roadmap_or_left_declared_is_caught() {
        let document = after_s1(vec![
            event("phase.withdrawn", 17, phase_withdrawn_payload(2, digest())),
            reassigned(18, 2, Some(1)),
            event(
                "phase.reordered",
                19,
                phase_reordered_payload(&[1], digest()),
            ),
        ]);
        let withdrawn = phase_entry(&document, 2);
        assert_eq!(withdrawn["status"], "withdrawn");
        assert_eq!(withdrawn["stories"], json!([]));
        assert_eq!(phase_entry(&document, 1)["stories"], json!([1, 2]));
        assert_eq!(story_entry(&document, 2)["phase"], 1);
        assert_eq!(document["order"], json!([1]));
        assert_eq!(document["highest_phase"], 2);
    }

    #[test]
    fn a_reassigned_or_dropped_story_left_on_its_old_phase_is_caught() {
        let mut events = vec![reassigned(17, 3, Some(2))];
        let first = after_s1(events.clone());
        assert_eq!(story_entry(&first, 3)["status"], "active");
        assert_eq!(story_entry(&first, 3)["phase"], 2);
        assert_eq!(phase_entry(&first, 2)["stories"], json!([2, 3]));

        events.push(dropped(18, 2));
        let second = after_s1(events.clone());
        assert_eq!(story_entry(&second, 2)["status"], "dropped");
        assert_eq!(story_entry(&second, 2)["phase"], Value::Null);
        assert_eq!(phase_entry(&second, 2)["stories"], json!([3]));

        events.push(reassigned(19, 1, None));
        let third = after_s1(events);
        assert_eq!(story_entry(&third, 1)["phase"], Value::Null);
        assert_eq!(phase_entry(&third, 1)["stories"], json!([]));
    }

    #[test]
    fn a_corrected_story_moved_or_given_another_status_is_caught() {
        let document = after_s1(vec![event(
            "story.corrected",
            17,
            story_corrected_payload(
                3,
                "Teammates read the backlog",
                StoryStatus::Deferred,
                digest(),
            ),
        )]);
        assert_eq!(story_ids(&document), [1, 2, 3]);
        let story = story_entry(&document, 3);
        assert_eq!(story["sentence"], "Teammates read the backlog");
        assert_eq!(story["status"], "deferred");
        assert_eq!(story["phase"], Value::Null);
    }

    #[test]
    fn a_reprioritized_roadmap_misordered_or_losing_a_dropped_story_is_caught() {
        let document = after_s1(vec![
            dropped(17, 3),
            event(
                "story.reprioritized",
                18,
                story_reprioritized_payload(&[2, 1], digest()),
            ),
        ]);
        assert_eq!(story_ids(&document), [2, 1, 3]);
        assert_eq!(story_entry(&document, 3)["status"], "dropped");
    }

    #[test]
    fn a_declaration_leaving_a_released_story_committed_or_a_declared_story_unserved_is_caught() {
        let mut events = vec![phase(17, 2, "Approval", "The owner approves", &[1], &[])];
        let released = after_s1(events.clone());
        assert_eq!(story_entry(&released, 2)["phase"], Value::Null);
        assert_eq!(phase_entry(&released, 2)["stories"], json!([]));

        events.push(story(
            18,
            4,
            "Reports show progress",
            StoryStatus::Active,
            Some(2),
        ));
        let declared = after_s1(events);
        assert_eq!(story_ids(&declared), [1, 2, 3, 4]);
        assert_eq!(story_entry(&declared, 4)["phase"], 2);
        assert_eq!(phase_entry(&declared, 2)["stories"], json!([4]));
    }

    /// A `story.refined` for story `id` carrying `truths`.
    fn refined(seq: u64, id: u32, truths: &[Value]) -> Event {
        event(
            "story.refined",
            seq,
            json!({"story": id, "set": "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d",
                "submission": "5c".repeat(32), "owner": "Ada Owner",
                "at": "2026-10-22T09:00:00Z", "truths": truths}),
        )
    }

    #[test]
    fn a_story_truth_count_left_at_zero_or_summed_across_revisions_is_caught() {
        assert_eq!(roadmap_spec().version, 3);
        let t1 = json!({"id": "t1", "form": "when",
            "trigger": "the owner approves a draft by a stale digest",
            "observer": "the owner", "verb": "is refused",
            "outcome": "stale-draft naming the first differing part", "kind": "literal",
            "observable": true, "fixed_oracle": true, "version": 1});
        let t2 = json!({"id": "t2", "form": "for-any",
            "trigger": "order the answers of a set are recorded in",
            "observer": "a ledger reader", "verb": "gets", "outcome": "the same round",
            "kind": "property", "observable": true, "fixed_oracle": true, "version": 1});
        let mut events = vec![refined(17, 1, &[t1.clone(), t2])];
        let first = after_s1(events.clone());
        assert_eq!(story_entry(&first, 1)["truths"], 2);
        assert_eq!(story_entry(&first, 2)["truths"], 0);
        assert_eq!(story_entry(&first, 3)["truths"], 0);

        events.push(refined(18, 1, std::slice::from_ref(&t1)));
        let second = after_s1(events);
        assert_eq!(story_entry(&second, 1)["truths"], 1);

        let undeclared = refined(19, 9, &[t1]);
        assert!(
            RoadmapProjector::new()
                .apply(&undeclared, &[(roadmap_key(), second)])
                .is_err()
        );
    }
}
