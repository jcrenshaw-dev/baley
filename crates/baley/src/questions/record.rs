//! Opening and abandoning a question set inside a caller's transaction
//! (design 0005 section 6, PLN-R25).
//!
//! Neither function runs a transaction of its own: a set opens, or is
//! abandoned, together with the caller's own records or not at all. The
//! callers to come are T7's `story questions submit`, which opens a story's
//! set and abandons the one it supersedes, and T9's `plan submit`, which
//! opens a plan draft's set and abandons the replaced draft's. No session
//! exit or shutdown path calls either: only an explicit abandonment ends a
//! set.

use baley_core::questions::{
    Cause, QUESTIONS_ABANDONED, QUESTIONS_ABANDONED_VERSION, QUESTIONS_OPENED,
    QUESTIONS_OPENED_VERSION, QUESTIONS_VIEW, QuestionSet, Subject, abandoned_payload,
    opened_payload, questions_key, set_stream,
};
use baley_store::{NewEvent, StoreError, StreamName, Transaction};
use serde_json::Value;

/// Why a set was not abandoned. Nothing was appended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbandonError {
    /// The `questions` view holds no set with that id.
    NotHeld,
    /// The set is already abandoned.
    Abandoned,
    /// The store failed, or the set's document could not be read.
    Store(StoreError),
}

impl From<StoreError> for AbandonError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// Opens the accepted `set` as a set of `subject` at `at`, appending
/// `questions.opened` on the subject's stream inside `tx`, and returns the
/// set's id, a fresh lowercase UUID. Called by T7's `story questions submit`
/// and T9's `plan submit` inside their own transactions.
pub fn open(
    tx: &mut dyn Transaction,
    set: &QuestionSet,
    subject: &Subject,
    at: &str,
) -> Result<String, StoreError> {
    // Minted fresh, never derived from the content, so two sets with the
    // same questions never share an id.
    let id = uuid::Uuid::new_v4().to_string();
    tx.append(NewEvent {
        stream: StreamName(set_stream(subject)),
        type_name: QUESTIONS_OPENED.into(),
        type_version: QUESTIONS_OPENED_VERSION,
        git: None,
        payload: opened_payload(&id, subject, set, at),
        attachments: Vec::new(),
    })?;
    Ok(id)
}

/// Abandons the set `set` for `cause` at `at`, appending
/// `questions.abandoned` inside `tx` on the stream the set's document
/// records. A set the view does not hold, or one already abandoned, appends
/// nothing and is answered as an error the caller can refuse with. Called
/// by T7's `story questions submit` and T9's `plan submit` in the
/// transaction that records the later set or draft.
pub fn abandon(
    tx: &mut dyn Transaction,
    set: &str,
    cause: Cause,
    at: &str,
) -> Result<(), AbandonError> {
    let Some(document) = tx.get(QUESTIONS_VIEW, &questions_key(set))? else {
        return Err(AbandonError::NotHeld);
    };
    if document.body.get("abandoned") != Some(&Value::Null) {
        return Err(AbandonError::Abandoned);
    }
    let Some(stream) = document.body.get("stream").and_then(Value::as_str) else {
        return Err(AbandonError::Store(StoreError::Unavailable(format!(
            "question set {set} records no stream"
        ))));
    };
    tx.append(NewEvent {
        stream: StreamName(stream.into()),
        type_name: QUESTIONS_ABANDONED.into(),
        type_version: QUESTIONS_ABANDONED_VERSION,
        git: None,
        payload: abandoned_payload(set, cause, at),
        attachments: Vec::new(),
    })?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    //! Sets are opened and abandoned in test transactions over the real
    //! store; expected records come from design 0005 section 6.

    use baley_core::questions::judge_set;
    use baley_store::{
        Decision, Hash, Ledger, Observed, OutcomeKind, PageRequest, StreamName, Views,
    };
    use baley_store_sqlite::SqliteStore;
    use serde_json::json;

    use super::*;
    use crate::scope::fixtures::*;

    /// The work order the story's sets name.
    pub(crate) const ORDER: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";

    /// Set Q1, accepted.
    pub(crate) fn q1() -> QuestionSet {
        judge_set(&json!([
            {"id": "q1", "question": "Which store backs the ledger?", "recommended": "SQLite",
                "depends_on": []},
            {"id": "q2", "question": "Where does the ledger live?",
                "recommended": "Outside the checkout", "depends_on": ["q1"]}
        ]))
        .unwrap()
    }

    pub(crate) fn story(story: u32) -> Subject {
        Subject::Story {
            story,
            work_order: ORDER.into(),
        }
    }

    fn plan(phase: u32) -> Subject {
        Subject::Plan {
            phase,
            plan_digest: Hash([0x5c; 32]),
        }
    }

    /// Runs `decide` inside a test command's transaction under `request`,
    /// committing what it appended when it returns `Ok`.
    pub(crate) fn transact<T>(
        store: &SqliteStore,
        request: &str,
        decide: &mut dyn FnMut(&mut dyn Transaction) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let mut out = None;
        store.transact(&command("test.questions", request, 3, T1), &mut |tx| {
            out = Some(decide(tx)?);
            Ok(Decision {
                kind: OutcomeKind::Done,
                answer: json!({"status": "ok"}),
                sensitive: false,
                observed: Observed::default(),
                git: None,
            })
        })?;
        Ok(out.expect("the decision ran"))
    }

    /// Opens `set` as a set of `subject` in its own test transaction.
    pub(crate) fn open_set(
        store: &SqliteStore,
        request: &str,
        set: &QuestionSet,
        subject: &Subject,
    ) -> String {
        transact(store, request, &mut |tx| open(tx, set, subject, T1)).unwrap()
    }

    /// Declares story `story` of `project` in its own test transaction, as
    /// an approval would, since the `backlog` and `roadmap` projectors refuse
    /// a set or a refinement of a story they do not hold.
    pub(crate) fn declare(store: &SqliteStore, project: &baley_store::ProjectId, story: u32) {
        use baley_core::scope::{
            ROADMAP_STREAM, STORY_DECLARED, STORY_DECLARED_VERSION, StoryStatus,
            story_declared_payload,
        };

        let payload = story_declared_payload(
            story,
            "Every step is recorded",
            StoryStatus::Active,
            None,
            Hash([0x5c; 32]),
        );
        let request = format!("2b3c4d5e-6f70-4a8b-9c0d-{story:012x}");
        let command = command_in(project, "test.declare", &request, 4, T1);
        store
            .transact(&command, &mut |tx| {
                tx.append(NewEvent {
                    stream: StreamName(ROADMAP_STREAM.into()),
                    type_name: STORY_DECLARED.into(),
                    type_version: STORY_DECLARED_VERSION,
                    git: None,
                    payload: payload.clone(),
                    attachments: Vec::new(),
                })?;
                Ok(Decision {
                    kind: OutcomeKind::Done,
                    answer: json!({"status": "ok"}),
                    sensitive: false,
                    observed: Observed::default(),
                    git: None,
                })
            })
            .unwrap();
    }

    fn on_stream(store: &SqliteStore, stream: &str, type_name: &str) -> Vec<baley_store::Event> {
        let page = PageRequest {
            limit: 100,
            after: None,
        };
        store
            .stream(&project(), &StreamName(stream.into()), 1, page)
            .unwrap()
            .items
            .into_iter()
            .filter(|event| event.type_name == type_name)
            .collect()
    }

    fn document(store: &SqliteStore, id: &str) -> Option<Value> {
        store
            .get(&project(), "questions", &questions_key(id))
            .unwrap()
            .map(|document| document.body)
    }

    fn lowercase_uuid(id: &str) -> bool {
        id.len() == 36
            && id.char_indices().all(|(at, c)| match at {
                8 | 13 | 18 | 23 => c == '-',
                _ => c.is_ascii_digit() || ('a'..='f').contains(&c),
            })
    }

    #[test]
    fn a_set_opened_off_its_subjects_stream_or_under_another_id_than_returned_is_caught() {
        let (_dir, store) = store();
        declare(&store, &project(), 1);
        let story_set = open_set(&store, R1, &q1(), &story(1));
        let plan_set = open_set(&store, R2, &q1(), &plan(2));
        assert!(lowercase_uuid(&story_set), "{story_set}");
        assert!(lowercase_uuid(&plan_set), "{plan_set}");

        let on_roadmap = on_stream(&store, "roadmap", "questions.opened");
        assert_eq!(on_roadmap.len(), 1);
        assert_eq!(on_roadmap[0].payload["set"], story_set);
        assert_eq!(on_roadmap[0].payload["story"], 1);
        let on_phase = on_stream(&store, "phase/2", "questions.opened");
        assert_eq!(on_phase.len(), 1);
        assert_eq!(on_phase[0].payload["set"], plan_set);
        assert_eq!(on_phase[0].payload["phase"], 2);

        assert_eq!(document(&store, &story_set).unwrap()["stream"], "roadmap");
        assert_eq!(document(&store, &plan_set).unwrap()["stream"], "phase/2");
    }

    #[test]
    fn two_sets_with_the_same_questions_given_one_id_is_caught() {
        let (_dir, store) = store();
        declare(&store, &project(), 1);
        let first = open_set(&store, R1, &q1(), &story(1));
        let second = open_set(&store, R2, &q1(), &story(1));
        assert_ne!(first, second);
        assert!(document(&store, &first).is_some());
        assert!(document(&store, &second).is_some());
    }

    #[test]
    fn an_abandonment_off_its_sets_stream_or_of_an_unknown_or_abandoned_set_recorded_is_caught() {
        let (_dir, store) = store();
        let set = open_set(&store, R1, &q1(), &plan(2));
        transact(&store, R2, &mut |tx| {
            abandon(tx, &set, Cause::DraftReplaced, T2)
                .map_err(|error| StoreError::Unavailable(format!("{error:?}")))
        })
        .unwrap();
        let abandoned = on_stream(&store, "phase/2", "questions.abandoned");
        assert_eq!(abandoned.len(), 1);
        assert_eq!(
            abandoned[0].payload,
            json!({"set": set, "cause": "draft-replaced", "at": T2})
        );
        assert!(on_stream(&store, "roadmap", "questions.abandoned").is_empty());

        for (request, id, expected) in [
            (
                R3,
                "7f3e4d5c-6071-4c8d-8e9f-1a2b3c4d5e6f",
                AbandonError::NotHeld,
            ),
            (R4, set.as_str(), AbandonError::Abandoned),
        ] {
            let before = head(&store);
            let mut seen = None;
            let refused = transact(&store, request, &mut |tx| {
                let at_start = tx.head()?;
                let answer = abandon(tx, id, Cause::DraftReplaced, T3);
                seen = Some((answer, at_start == tx.head()?));
                Err::<(), _>(StoreError::Unavailable("refused".into()))
            });
            assert!(refused.is_err());
            assert_eq!(seen, Some((Err(expected), true)), "{id}");
            assert_eq!(head(&store), before, "{id}");
        }
    }

    #[test]
    fn a_set_opened_outside_its_callers_transaction_is_caught() {
        let (_dir, store) = store();
        declare(&store, &project(), 1);
        let before = head(&store);
        let mut opened = None;
        let refused = transact(&store, R1, &mut |tx| {
            let id = open(tx, &q1(), &story(1), T1)?;
            let held = tx.get(QUESTIONS_VIEW, &questions_key(&id))?.is_some();
            opened = Some((id, held));
            Err::<(), _>(StoreError::Unavailable(
                "the caller's own check failed".into(),
            ))
        });
        assert!(refused.is_err());
        let (id, held_inside) = opened.expect("the decision ran");
        assert!(held_inside, "the set opens inside the caller's transaction");
        assert_eq!(head(&store), before);
        assert_eq!(document(&store, &id), None);
    }
}
