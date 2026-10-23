//! What an analyzer order binds at issue (D-21): the story, the approved
//! description, the accepted roadmap and, once the story was refined, its
//! current truths, each as canonical JSON a `record` payload holds, and for
//! a drafting order the set's questions with every answer and deferral.
//! Nothing here records anything. A retry carries its parent's inputs as
//! they were, so a later approval never changes what an order was given.

use baley_core::questions::SetFacts;
use baley_core::scope::{ROADMAP_VIEW, roadmap_key};
use baley_core::truth::{TRUTHS_VIEW, truths_key};
use baley_store::{ObservedDocument, ProjectId, Views, canonical_json};
use serde::Serialize;
use serde_json::{Value, json};

use crate::scope::read::{ReadError, Story, roadmap};

/// The input holding the story's roadmap entry.
pub const STORY_INPUT: &str = "story";
/// The input holding the approved description.
pub const DESCRIPTION_INPUT: &str = "description";
/// The input holding the accepted roadmap.
pub const ROADMAP_INPUT: &str = "roadmap";
/// The input holding the story's current truths with their versions.
pub const TRUTHS_INPUT: &str = "truths";
/// The input holding a drafting order's question set with every answer
/// and deferral.
pub const QUESTIONS_INPUT: &str = "questions";

/// What every analyzer order of one story binds, read outside any
/// transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderInputs {
    /// The story's roadmap entry.
    pub story: Story,
    /// Each input's name and canonical JSON bytes, in order: `story`,
    /// `description`, `roadmap` and, for a story refined before, `truths`.
    pub inputs: Vec<(&'static str, Vec<u8>)>,
    /// The `roadmap` document and the story's `truths` document, the latter
    /// absent before its first refinement, each at the sequence that
    /// produced it, so the issue can confirm neither moved.
    pub documents: Vec<ObservedDocument>,
}

/// Reads what an analyzer order of story `story` in `project` binds. The
/// story, the description and the roadmap come from one read of the
/// accepted roadmap, which only an approval changes, so a pending
/// `project-describe` draft never reaches an order. A story the roadmap
/// does not hold is [`ReadError::NoSuchStory`].
pub fn read<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    story: u32,
) -> Result<OrderInputs, ReadError> {
    // The sequences are read first, so a document that moves before its
    // content is read shows as moved when the issue confirms it.
    let roadmap_seq = store
        .get(project, ROADMAP_VIEW, &roadmap_key())
        .map_err(ReadError::Store)?
        .map(|document| document.produced_seq);
    let truths = store
        .get(project, TRUTHS_VIEW, &truths_key(story))
        .map_err(ReadError::Store)?;
    let accepted = roadmap(store, project)?;
    let entry = accepted
        .stories
        .iter()
        .find(|entry| entry.id == story)
        .cloned()
        .ok_or(ReadError::NoSuchStory)?;
    // A story is only declared by an approval, which also sets the
    // description, so a story without one is a broken document.
    let description = accepted
        .description
        .as_ref()
        .ok_or_else(|| unreadable(ROADMAP_VIEW))?;
    let mut inputs = vec![
        (STORY_INPUT, bytes(&entry)?),
        (DESCRIPTION_INPUT, bytes(description)?),
        (ROADMAP_INPUT, bytes(&accepted)?),
    ];
    if let Some(document) = &truths {
        let current = document
            .body
            .get("truths")
            .filter(|truths| truths.is_array())
            .ok_or_else(|| unreadable(TRUTHS_VIEW))?;
        inputs.push((TRUTHS_INPUT, bytes(current)?));
    }
    let documents = vec![
        ObservedDocument {
            view: ROADMAP_VIEW.into(),
            key: roadmap_key(),
            produced_seq: roadmap_seq,
        },
        ObservedDocument {
            view: TRUTHS_VIEW.into(),
            key: truths_key(story),
            produced_seq: truths.map(|document| document.produced_seq),
        },
    ];
    Ok(OrderInputs {
        story: entry,
        inputs,
        documents,
    })
}

/// The body of a drafting order's `questions` input for set `set`: its id,
/// its questions as submitted, and every answer and deferral with its
/// round, owner and time, each list in the questions' submitted order so
/// the body never depends on the order records were read in. The close of
/// a set builds the same body from its facts after the closing call.
pub fn questions_body(set: &str, facts: &SetFacts) -> Value {
    let position = |question: &str| {
        facts
            .questions
            .iter()
            .position(|asked| asked.id == question)
    };
    let mut answers: Vec<_> = facts.answers.iter().collect();
    answers.sort_by_key(|answer| position(&answer.question));
    let mut deferrals: Vec<_> = facts.deferrals.iter().collect();
    deferrals.sort_by_key(|deferral| position(&deferral.question));
    json!({
        "set": set,
        "questions": facts.questions.iter().map(|question| json!({
            "id": question.id,
            "question": question.question,
            "recommended": question.recommended,
            "depends_on": question.depends_on,
        })).collect::<Vec<_>>(),
        "answers": answers.iter().map(|answer| json!({
            "question": answer.question,
            "answer": answer.answer,
            "accepted": answer.accepted,
            "round": answer.round,
            "owner": answer.owner,
            "at": answer.at,
        })).collect::<Vec<_>>(),
        "deferrals": deferrals.iter().map(|deferral| json!({
            "question": deferral.question,
            "reason": deferral.reason,
            "round": deferral.round,
            "owner": deferral.owner,
            "at": deferral.at,
        })).collect::<Vec<_>>(),
    })
}

/// The canonical JSON bytes an input body is stored as. Every number these
/// bodies hold is a count, an id or a round, so a value outside the
/// canonical domain is a broken document.
pub fn bytes<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, ReadError> {
    let value = serde_json::to_value(value)
        .map_err(|error| ReadError::Unreadable(format!("an order input: {error}")))?;
    canonical_json(&value)
        .map_err(|error| ReadError::Unreadable(format!("an order input: {error}")))
}

fn unreadable(view: &str) -> ReadError {
    ReadError::Unreadable(format!("the {view} view document cannot be read"))
}

#[cfg(test)]
mod tests {
    //! Stores are real SQLite stores in fresh temporary directories, with
    //! times supplied; expected inputs are written out from D-21 and the
    //! plan's data S1, D1, T1 and Q1, never from running this code.

    use baley_core::questions::{Answer, Deferral, judge_set};
    use baley_core::scope::{ChangeKind, ROADMAP_STREAM};
    use baley_core::truth::{STORY_REFINED, STORY_REFINED_VERSION};
    use baley_store::{Hash, NewEvent, StreamName};
    use baley_store_sqlite::SqliteStore;

    use super::*;
    use crate::scope::fixtures::*;

    /// A store holding S1, approved.
    fn approved() -> (tempfile::TempDir, SqliteStore) {
        let (dir, store) = store();
        let digest = receipt(&submit(&store, &s1(), R1, 1, T1).unwrap())["digest"]
            .as_str()
            .and_then(Hash::from_hex)
            .unwrap();
        approve_draft(&store, digest);
        (dir, store)
    }

    fn names(read: &OrderInputs) -> Vec<&str> {
        read.inputs.iter().map(|(name, _)| *name).collect()
    }

    fn parsed(read: &OrderInputs, name: &str) -> Value {
        let (_, bytes) = read
            .inputs
            .iter()
            .find(|(input, _)| *input == name)
            .unwrap_or_else(|| panic!("no input {name}"));
        serde_json::from_slice(bytes).unwrap()
    }

    /// T1, the truth a hand-appended refinement of story 1 carries.
    fn t1() -> Value {
        json!({"id": "t1", "form": "when",
            "trigger": "the owner approves a draft by a stale digest",
            "observer": "the owner", "verb": "is refused",
            "outcome": "stale-draft naming the first differing part", "kind": "literal",
            "observable": true, "fixed_oracle": true, "version": 1})
    }

    #[test]
    fn an_order_input_read_from_a_pending_draft_or_missing_a_part_is_caught() {
        let (_dir, store) = approved();
        submit_change(&store, ChangeKind::ProjectDescribe, &d1(), R4, T4).unwrap();
        let read = read(&store, &project(), 1).unwrap();

        assert_eq!(names(&read), ["story", "description", "roadmap"]);
        assert_eq!(
            parsed(&read, "description"),
            json!({"what": "A ledger for agent work",
                "core_value": "Nothing moves on an unproven claim",
                "constraints": ["One binary"]})
        );
        assert_eq!(
            parsed(&read, "story"),
            json!({"id": 1, "sentence": "Every step is recorded", "status": "active",
                "phase": 1, "truths": 0})
        );
        assert_eq!(
            parsed(&read, "roadmap"),
            serde_json::to_value(roadmap(&store, &project()).unwrap()).unwrap()
        );
    }

    #[test]
    fn a_refined_story_issued_without_its_truths_is_caught() {
        let (_dir, store) = approved();
        let refined = json!({"story": 1, "set": "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d",
            "submission": "5c".repeat(32), "owner": "Ada Owner", "at": T4, "truths": [t1()]});
        append(
            &store,
            R4,
            T4,
            vec![NewEvent {
                stream: StreamName(ROADMAP_STREAM.into()),
                type_name: STORY_REFINED.into(),
                type_version: STORY_REFINED_VERSION,
                git: None,
                payload: refined,
                attachments: Vec::new(),
            }],
        );
        let read = read(&store, &project(), 1).unwrap();

        assert_eq!(names(&read), ["story", "description", "roadmap", "truths"]);
        assert_eq!(parsed(&read, "truths"), json!([t1()]));
        assert_eq!(parsed(&read, "story")["truths"], 1);
        assert_eq!(parsed(&read, "roadmap")["stories"][0]["truths"], 1);
        let truths = read
            .documents
            .iter()
            .find(|document| document.view == "truths")
            .expect("the truths document is observed");
        assert_eq!(truths.key, truths_key(1));
        assert!(truths.produced_seq.is_some(), "{truths:?}");
    }

    #[test]
    fn a_questions_body_dropping_an_answer_or_a_deferral_is_caught() {
        const SET: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
        let questions = judge_set(&json!([
            {"id": "q1", "question": "Which store backs the ledger?", "recommended": "SQLite",
                "depends_on": []},
            {"id": "q2", "question": "Where does the ledger live?",
                "recommended": "Outside the checkout", "depends_on": ["q1"]}
        ]))
        .unwrap();
        let facts = SetFacts {
            questions: questions.questions().to_vec(),
            answers: vec![Answer {
                question: "q1".into(),
                answer: "SQLite".into(),
                accepted: true,
                round: 1,
                owner: "Ada Owner".into(),
                at: T1.into(),
            }],
            deferrals: vec![Deferral {
                question: "q2".into(),
                reason: "after the spike".into(),
                round: 2,
                owner: "Ada Owner".into(),
                at: T2.into(),
            }],
            abandoned: None,
        };
        assert_eq!(
            questions_body(SET, &facts),
            json!({"set": SET,
                "questions": [
                    {"id": "q1", "question": "Which store backs the ledger?",
                        "recommended": "SQLite", "depends_on": []},
                    {"id": "q2", "question": "Where does the ledger live?",
                        "recommended": "Outside the checkout", "depends_on": ["q1"]}],
                "answers": [{"question": "q1", "answer": "SQLite", "accepted": true,
                    "round": 1, "owner": "Ada Owner", "at": T1}],
                "deferrals": [{"question": "q2", "reason": "after the spike", "round": 2,
                    "owner": "Ada Owner", "at": T2}]})
        );
    }
}
