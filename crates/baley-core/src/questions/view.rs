//! The `questions` view: one document per question set, keyed by its id,
//! holding the facts recorded about it (design 0001 Views, design 0005
//! section 6).
//!
//! The document stores facts only. Whether the set is open, closed or
//! abandoned, which questions wait and which are in the current round, and
//! the round number are worked out from these facts when read, never
//! stored, so no stored state can drift from the rule.

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, KeyValue, Projector, ProjectorError, ViewSpec,
};
use serde_json::{Map, Value, json};

use super::event::{QUESTION_ANSWERED, QUESTION_DEFERRED, QUESTIONS_ABANDONED, QUESTIONS_OPENED};

/// The view's name.
pub const QUESTIONS_VIEW: &str = "questions";

/// The key field: the set's id.
const SET_FIELD: &str = "set";

/// The view's declaration: the set id as a text key and no index.
pub fn questions_spec() -> ViewSpec {
    ViewSpec {
        name: QUESTIONS_VIEW.into(),
        version: 1,
        key: vec![FieldSpec {
            name: SET_FIELD.into(),
            kind: FieldKind::Text,
        }],
        indexes: Vec::new(),
        page_bound: 100,
    }
}

/// The key of the set with `id`.
pub fn questions_key(id: &str) -> DocKey {
    DocKey(vec![KeyValue::Text(id.to_owned())])
}

/// Keeps the `questions` view current from the four question events.
///
/// Opened: a new document with these members:
/// - `set`: the set's id.
/// - `subject`: `{kind: "story", story, work_order}` for a story's set or
///   `{kind: "plan", phase, plan_digest}` for a plan draft's.
/// - `stream`: the stream the opening went on, which every later record on
///   the set goes on too.
/// - `seq` and `recorded_at`: the opening event's sequence and time.
/// - `questions`: the questions as submitted, each
///   `{id, question, recommended, depends_on}`.
/// - `answers`: empty; each answer adds
///   `{question, answer, accepted, round, owner, at, seq}`.
/// - `deferrals`: empty; each deferral adds
///   `{question, reason, round, owner, at, seq}`.
/// - `abandoned`: null; an abandonment sets `{cause, at, seq}`.
///
/// `at` is the time the payload carries and `seq` the recording event's
/// sequence. Refused, so no recorded fact is ever replaced: an opening for a
/// set already held; an answer or deferral for a set not held, a question
/// the set lacks or a question already answered or deferred; an
/// abandonment of a set not held or already abandoned.
pub struct QuestionsProjector {
    spec: ViewSpec,
}

impl QuestionsProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self {
            spec: questions_spec(),
        }
    }
}

impl Default for QuestionsProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for QuestionsProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[
            QUESTIONS_OPENED,
            QUESTION_ANSWERED,
            QUESTION_DEFERRED,
            QUESTIONS_ABANDONED,
        ]
    }

    fn keys(&self, event: &Event) -> Vec<DocKey> {
        event
            .payload
            .get(SET_FIELD)
            .and_then(Value::as_str)
            .map(questions_key)
            .into_iter()
            .collect()
    }

    fn apply(
        &self,
        event: &Event,
        documents: &[(DocKey, Value)],
    ) -> Result<Vec<Change>, ProjectorError> {
        let refuse = |message: String| {
            ProjectorError(format!(
                "{} at seq {}: {message}",
                event.type_name, event.seq
            ))
        };
        let Some(payload) = event.payload.as_object() else {
            return Err(refuse("the payload is not an object".into()));
        };
        let text = |name: &str| {
            payload
                .get(name)
                .and_then(Value::as_str)
                .ok_or_else(|| refuse(format!("{name} is not text")))
        };
        let number = |name: &str| {
            payload
                .get(name)
                .and_then(Value::as_u64)
                .ok_or_else(|| refuse(format!("{name} is not a number")))
        };
        let set = text(SET_FIELD)?;
        let key = questions_key(set);
        let held = documents
            .iter()
            .find(|(at, _)| *at == key)
            .and_then(|(_, document)| document.as_object());

        if event.type_name == QUESTIONS_OPENED {
            if held.is_some() {
                return Err(refuse(format!("set {set} is already held")));
            }
            let subject = match text("subject")? {
                "story" => json!({"kind": "story", "story": number("story")?,
                    "work_order": text("work_order")?}),
                "plan" => json!({"kind": "plan", "phase": number("phase")?,
                    "plan_digest": text("plan_digest")?}),
                other => return Err(refuse(format!("subject {other} is not story or plan"))),
            };
            let Some(Value::Array(questions)) = payload.get("questions") else {
                return Err(refuse("questions is not a list".into()));
            };
            return Ok(vec![Change::Put {
                key,
                body: json!({
                    "set": set,
                    "subject": subject,
                    "stream": event.stream,
                    "seq": event.seq,
                    "recorded_at": event.recorded_at,
                    "questions": questions,
                    "answers": [],
                    "deferrals": [],
                    "abandoned": null,
                }),
            }]);
        }

        let Some(document) = held else {
            return Err(refuse(format!("set {set} is not held")));
        };
        let mut document = document.clone();
        if event.type_name == QUESTIONS_ABANDONED {
            if document.get("abandoned") != Some(&Value::Null) {
                return Err(refuse(format!("set {set} is already abandoned")));
            }
            document.insert(
                "abandoned".into(),
                json!({"cause": text("cause")?, "at": text("at")?, "seq": event.seq}),
            );
            return Ok(vec![Change::Put {
                key,
                body: Value::Object(document),
            }]);
        }

        let question = text("question")?;
        let asked = list(&document, "questions")
            .iter()
            .any(|entry| entry.get("id").and_then(Value::as_str) == Some(question));
        if !asked {
            return Err(refuse(format!("set {set} has no question {question}")));
        }
        let settled = ["answers", "deferrals"].iter().any(|member| {
            list(&document, member)
                .iter()
                .any(|entry| entry.get("question").and_then(Value::as_str) == Some(question))
        });
        if settled {
            return Err(refuse(format!("question {question} is already settled")));
        }
        let (member, entry) = if event.type_name == QUESTION_ANSWERED {
            let accepted = payload
                .get("accepted")
                .and_then(Value::as_bool)
                .ok_or_else(|| refuse("accepted is not a bool".into()))?;
            let entry = json!({"question": question, "answer": text("answer")?,
                "accepted": accepted, "round": number("round")?, "owner": text("owner")?,
                "at": text("at")?, "seq": event.seq});
            ("answers", entry)
        } else {
            let entry = json!({"question": question, "reason": text("reason")?,
                "round": number("round")?, "owner": text("owner")?, "at": text("at")?,
                "seq": event.seq});
            ("deferrals", entry)
        };
        let mut entries = list(&document, member).to_vec();
        entries.push(entry);
        document.insert(member.into(), Value::Array(entries));
        Ok(vec![Change::Put {
            key,
            body: Value::Object(document),
        }])
    }
}

/// The list member `name` of `document`, or none when it is not a list.
fn list<'d>(document: &'d Map<String, Value>, name: &str) -> &'d [Value] {
    document
        .get(name)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

#[cfg(test)]
mod tests {
    //! Expected documents are written out from the projector's documented
    //! members and the plan's test data, never from running this code.

    use std::collections::BTreeMap;

    use baley_store::{Actor, Hash, ProjectId, RequestId};

    use super::*;

    const SET: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const OTHER: &str = "7f3e4d5c-6071-4c8d-8e9f-1a2b3c4d5e6f";
    const ORDER: &str = "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e";
    const T0: &str = "2026-10-22T09:00:00Z";
    const T1: &str = "2026-10-22T09:00:01Z";
    const T2: &str = "2026-10-22T09:00:02Z";

    type View = BTreeMap<DocKey, Value>;

    fn event(seq: u64, stream: &str, type_name: &str, payload: Value) -> Event {
        Event {
            project_id: ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into()),
            seq,
            stream: stream.into(),
            stream_version: seq,
            type_name: type_name.into(),
            type_version: 1,
            actor: Actor::Baley,
            caller: None,
            recorded_at: T0.into(),
            request_id: RequestId("9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f".into()),
            git: None,
            policy_version: 0,
            payload,
            prev_hash: None,
            hash: Hash([0; 32]),
        }
    }

    /// Set Q1's questions as written in a payload.
    fn q1() -> Value {
        json!([
            {"id": "q1", "question": "Which store backs the ledger?", "recommended": "SQLite",
                "depends_on": []},
            {"id": "q2", "question": "Where does the ledger live?",
                "recommended": "Outside the checkout", "depends_on": ["q1"]}
        ])
    }

    fn plan_opened(seq: u64, set: &str) -> Event {
        event(
            seq,
            "phase/2",
            "questions.opened",
            json!({"set": set, "subject": "plan", "phase": 2,
                "plan_digest": "5c".repeat(32), "questions": q1(), "at": T0}),
        )
    }

    fn story_opened(seq: u64, set: &str) -> Event {
        event(
            seq,
            "roadmap",
            "questions.opened",
            json!({"set": set, "subject": "story", "story": 3, "work_order": ORDER,
                "questions": q1(), "at": T0}),
        )
    }

    fn answered(seq: u64, set: &str, question: &str, answer: &str) -> Event {
        event(
            seq,
            "phase/2",
            "question.answered",
            json!({"set": set, "question": question, "round": 1, "answer": answer,
                "accepted": true, "owner": "Ada Lovelace", "at": T1}),
        )
    }

    fn deferred(seq: u64, set: &str, question: &str, reason: &str) -> Event {
        event(
            seq,
            "phase/2",
            "question.deferred",
            json!({"set": set, "question": question, "round": 1, "reason": reason,
                "owner": "Ada Lovelace", "at": T1}),
        )
    }

    fn abandoned(seq: u64, set: &str, cause: &str, at: &str) -> Event {
        event(
            seq,
            "phase/2",
            "questions.abandoned",
            json!({"set": set, "cause": cause, "at": at}),
        )
    }

    /// Applies `event` to `view` as the store would, reading the documents
    /// at its keys first. A refusal leaves the view as it was.
    fn fold(view: &mut View, event: &Event) -> Result<(), ProjectorError> {
        let projector = QuestionsProjector::new();
        let read: Vec<(DocKey, Value)> = projector
            .keys(event)
            .into_iter()
            .filter_map(|key| view.get(&key).cloned().map(|body| (key, body)))
            .collect();
        for change in projector.apply(event, &read)? {
            match change {
                Change::Put { key, body } => {
                    view.insert(key, body);
                }
                Change::Delete { key } => {
                    view.remove(&key);
                }
            }
        }
        Ok(())
    }

    fn doc<'v>(view: &'v View, id: &str) -> &'v Value {
        &view[&DocKey(vec![KeyValue::Text(id.into())])]
    }

    #[test]
    fn a_questions_view_declared_under_another_name_or_key_is_caught() {
        let spec = questions_spec();
        assert_eq!(spec.name, "questions");
        assert_eq!(spec.version, 1);
        assert_eq!(
            spec.key,
            [FieldSpec {
                name: "set".into(),
                kind: FieldKind::Text,
            }]
        );
        assert!(spec.indexes.is_empty());
        assert_eq!(
            QuestionsProjector::new().handles(),
            [
                "questions.opened",
                "question.answered",
                "question.deferred",
                "questions.abandoned"
            ]
        );
    }

    #[test]
    fn an_opened_set_projected_without_its_stream_or_with_a_state_is_caught() {
        let mut view = View::new();
        fold(&mut view, &plan_opened(5, SET)).unwrap();
        fold(&mut view, &story_opened(6, OTHER)).unwrap();
        assert_eq!(view.len(), 2);
        assert_eq!(
            doc(&view, SET),
            &json!({"set": SET,
                "subject": {"kind": "plan", "phase": 2, "plan_digest": "5c".repeat(32)},
                "stream": "phase/2", "seq": 5, "recorded_at": T0, "questions": q1(),
                "answers": [], "deferrals": [], "abandoned": null})
        );
        assert_eq!(
            doc(&view, OTHER),
            &json!({"set": OTHER,
                "subject": {"kind": "story", "story": 3, "work_order": ORDER},
                "stream": "roadmap", "seq": 6, "recorded_at": T0, "questions": q1(),
                "answers": [], "deferrals": [], "abandoned": null})
        );
    }

    #[test]
    fn an_answer_or_deferral_projected_onto_no_set_an_unknown_question_or_a_settled_one_is_caught()
    {
        let mut view = View::new();
        fold(&mut view, &plan_opened(5, SET)).unwrap();
        assert!(fold(&mut view, &answered(6, OTHER, "q1", "SQLite")).is_err());
        assert!(fold(&mut view, &answered(6, SET, "q9", "SQLite")).is_err());
        fold(&mut view, &answered(6, SET, "q1", "SQLite")).unwrap();
        fold(&mut view, &deferred(7, SET, "q2", "after the spike")).unwrap();
        let settled = view.clone();
        assert!(fold(&mut view, &answered(8, SET, "q1", "Postgres")).is_err());
        assert!(fold(&mut view, &deferred(8, SET, "q1", "later")).is_err());
        assert!(fold(&mut view, &answered(8, SET, "q2", "Here")).is_err());
        assert!(fold(&mut view, &deferred(8, SET, "q2", "later still")).is_err());
        assert_eq!(view, settled);
        assert_eq!(view.len(), 1);
        assert_eq!(
            doc(&view, SET)["answers"],
            json!([{"question": "q1", "answer": "SQLite", "accepted": true, "round": 1,
                "owner": "Ada Lovelace", "at": T1, "seq": 6}])
        );
        assert_eq!(
            doc(&view, SET)["deferrals"],
            json!([{"question": "q2", "reason": "after the spike", "round": 1,
                "owner": "Ada Lovelace", "at": T1, "seq": 7}])
        );
    }

    #[test]
    fn an_abandonment_of_an_unknown_set_or_a_second_one_accepted_is_caught() {
        let mut view = View::new();
        fold(&mut view, &plan_opened(5, SET)).unwrap();
        assert!(fold(&mut view, &abandoned(6, OTHER, "superseded", T1)).is_err());
        fold(&mut view, &abandoned(6, SET, "superseded", T1)).unwrap();
        assert!(fold(&mut view, &abandoned(7, SET, "draft-replaced", T2)).is_err());
        assert_eq!(view.len(), 1);
        assert_eq!(
            doc(&view, SET)["abandoned"],
            json!({"cause": "superseded", "at": T1, "seq": 6})
        );
    }
}
