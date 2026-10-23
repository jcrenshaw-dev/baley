//! A story's refinement read back (design 0005 sections 6 and 7, design 0004
//! PRJ-R20's refinement reader, D-10): the approved description, the story's
//! roadmap entry, the phase it is committed to and its truths with every kept
//! version. `context-intake`, the `document` kind `refinement` and `baley
//! story refinement <id>` all read through here, so the session and the owner
//! see the same thing.
//!
//! Keyed by story, not by phase, so a backlog story with no phase can be read
//! before it is committed. Nothing is appended and no order is issued.

use std::collections::BTreeMap;

use baley_core::scope::{PHASE_VIEW, phase_key};
use baley_core::truth::{TRUTHS_VIEW, truths_key};
use baley_store::{DocKey, ProjectId, Views};
use serde::{Deserialize, Serialize};

use crate::scope::read::{Description, ReadError, Story, roadmap};

/// Story `id`'s refinement as it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoryRefinement {
    /// The approved project description, or none before the start.
    pub description: Option<Description>,
    /// The story's roadmap entry, a dropped story with its status.
    pub story: Story,
    /// The phase the story is committed to, or none in the backlog.
    pub phase: Option<CommittedPhase>,
    /// Its truths, or none before its first refinement.
    pub truths: Option<Truths>,
}

/// The phase a story is committed to, from its `phase` document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommittedPhase {
    /// Its number.
    pub number: u32,
    /// Its name.
    pub name: String,
    /// Its goal.
    pub goal: String,
    /// Its status, such as `declared`.
    pub status: String,
}

/// A story's truths: the current ones and every version ever approved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Truths {
    /// The current truths in submitted order, each at its version.
    pub current: Vec<VersionedTruth>,
    /// Every version ever approved, by truth id, each id's oldest first.
    pub kept: Vec<VersionedTruth>,
}

/// One truth at one version, as `story.refined` carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionedTruth {
    /// Its id within the story.
    pub id: String,
    /// Its version.
    pub version: u32,
    /// `when` or `for-any`.
    pub form: String,
    /// The one trigger, or a `for-any` truth's input.
    pub trigger: String,
    /// The one observer.
    pub observer: String,
    /// `sees`, `gets` or `is refused`.
    pub verb: String,
    /// The one outcome.
    pub outcome: String,
    /// `literal` or `property`.
    pub kind: String,
    /// Attested as observable from outside.
    pub observable: bool,
    /// Attested as having a fixed oracle.
    pub fixed_oracle: bool,
    /// The tool or live service a person needs to prove it, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_verify: Option<String>,
}

/// Reads story `id`'s refinement in `project`, recording nothing: the
/// description and the story from one read of the roadmap, so the
/// description is the approved one and never a pending draft's (PRJ-R20);
/// then the committed phase's `phase` document; then the story's `truths`
/// document. A story the roadmap does not hold is `ReadError::NoSuchStory`;
/// a store or document fault comes back for the caller to answer.
pub fn read<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    id: u32,
) -> Result<StoryRefinement, ReadError> {
    let roadmap = roadmap(store, project)?;
    let story = roadmap
        .stories
        .into_iter()
        .find(|story| story.id == id)
        .ok_or(ReadError::NoSuchStory)?;
    let phase = match story.phase {
        Some(number) => Some(
            document::<CommittedPhase, _>(store, project, PHASE_VIEW, &phase_key(number))?
                .ok_or_else(|| unreadable(PHASE_VIEW))?,
        ),
        None => None,
    };
    let truths = document::<TruthsDocument, _>(store, project, TRUTHS_VIEW, &truths_key(id))?
        .map(Truths::from);
    Ok(StoryRefinement {
        description: roadmap.description,
        story,
        phase,
        truths,
    })
}

/// The `truths` document as its projector writes it; the members a read
/// does not need are left out.
#[derive(Deserialize)]
struct TruthsDocument {
    truths: Vec<VersionedTruth>,
    kept: BTreeMap<String, Vec<KeptEntry>>,
}

/// One kept version: the truth as the refinement that first carried it.
#[derive(Deserialize)]
struct KeptEntry {
    truth: VersionedTruth,
}

impl From<TruthsDocument> for Truths {
    fn from(document: TruthsDocument) -> Self {
        // Each id's versions are kept in the order approved, which is
        // oldest first; ids come in id order.
        let kept = document
            .kept
            .into_values()
            .flatten()
            .map(|entry| entry.truth)
            .collect();
        Self {
            current: document.truths,
            kept,
        }
    }
}

/// One view document read as `T`, or none when the view holds no such key.
fn document<T: for<'de> Deserialize<'de>, S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    view: &str,
    key: &DocKey,
) -> Result<Option<T>, ReadError> {
    match store.get(project, view, key).map_err(ReadError::Store)? {
        Some(document) => serde_json::from_value(document.body)
            .map(Some)
            .map_err(|_| unreadable(view)),
        None => Ok(None),
    }
}

fn unreadable(view: &str) -> ReadError {
    ReadError::Unreadable(format!("the {view} view document cannot be read"))
}

#[cfg(test)]
pub(crate) mod tests {
    //! Stores are real SQLite stores in fresh temporary directories, with
    //! times supplied; expected values are written out from design 0005
    //! PLN-R4, D-02, D-03, D-10 and the plan's data S1, T1, T2, T2', T3 and
    //! T3', never from running this code.

    use baley_core::questions::judge_set;
    use baley_core::scope::ChangeKind;
    use baley_store::{Hash, RequestId};
    use baley_store_sqlite::SqliteStore;
    use serde_json::{Value, json};

    use super::*;
    use crate::questions::answer::tests::ADA;
    use crate::refine::order::tests::approved;
    use crate::refine::questions::tests::{D, Q, refine, submitted};
    use crate::refine::submit::tests::{t1, t2, truths_submitted, with};
    use crate::scope::command::approve_command;
    use crate::scope::fixtures::*;

    fn rid(n: u8) -> String {
        format!("00000000-0000-4000-8000-{n:012x}")
    }

    /// T2 with its outcome changed.
    pub(crate) fn t2_changed() -> Value {
        with(t2(), "outcome", "the same round and the same waiting list")
    }

    /// T1's slots under id `t3` with another outcome.
    pub(crate) fn t3() -> Value {
        with(
            with(t1(), "id", "t3"),
            "outcome",
            "stale-draft naming the first differing member",
        )
    }

    /// T3 with another trigger.
    fn t3_changed() -> Value {
        with(
            t3(),
            "trigger",
            "the owner approves a draft by an unknown digest",
        )
    }

    /// `truth` as a read gives it at `version`.
    pub(crate) fn at(truth: Value, version: u32) -> Value {
        let mut truth = truth;
        truth["version"] = json!(version);
        truth
    }

    /// Approves the truth draft `digest` through the two steps as the owner
    /// `Ada Owner` at `at`.
    pub(crate) fn approve(store: &SqliteStore, digest: &str, request: &str, at: &str) {
        let digest = Hash::from_hex(digest).expect("a digest");
        let judged = crate::refine::approve::judge(store, &project(), digest)
            .unwrap_or_else(|refusal| panic!("{refusal}"));
        let command =
            approve_command(&project(), 0, digest, RequestId(request.into()), at).unwrap();
        let answer = crate::scope::submit::answer(
            crate::refine::approve::record(store, &command, Some(&judged), ADA),
            store,
        );
        assert_eq!(answer["status"], "ok", "{answer}");
    }

    /// A store holding S1, approved, with story 1's empty set submitted
    /// after `story-refine` and so closed at once with drafting order [`D`].
    /// Gives the store and the set's id.
    pub(crate) fn drafting_from_empty() -> (tempfile::TempDir, SqliteStore, String) {
        let (dir, store) = approved();
        refine(&store, 1, Q, R1, T1);
        let receipt = submitted(&store, 1, Q, &judge_set(&json!([])).unwrap(), R2, T2);
        assert_eq!(receipt["drafting"]["work_order"], D, "{receipt}");
        let set = receipt["set"].as_str().expect("a set").to_owned();
        (dir, store, set)
    }

    /// Submits `truths` for story 1 under [`D`] naming `set` and approves
    /// them, with request ids `n` and `n + 1`.
    pub(crate) fn revise(store: &SqliteStore, set: &str, truths: Value, n: u8, at: &str) {
        let receipt = truths_submitted(store, 1, set, &truths, D, &rid(n), at);
        assert_eq!(receipt["status"], "ok", "{receipt}");
        approve(store, receipt["digest"].as_str().unwrap(), &rid(n + 1), at);
    }

    fn truths_of(store: &SqliteStore) -> Value {
        let read = read(store, &project(), 1).unwrap();
        serde_json::to_value(read.truths.expect("story 1 is refined")).unwrap()
    }

    #[test]
    fn a_truth_version_bumped_unchanged_restarted_on_readding_or_left_unread_is_caught() {
        let (_dir, store, set) = drafting_from_empty();
        assert_eq!(read(&store, &project(), 1).unwrap().truths, None);
        revise(&store, &set, json!([t1(), t2()]), 10, T3);
        revise(&store, &set, json!([t1(), t2_changed(), t3()]), 20, T4);
        revise(&store, &set, json!([t1(), t2_changed()]), 30, T5);
        assert_eq!(
            truths_of(&store),
            json!({
                "current": [at(t1(), 1), at(t2_changed(), 2)],
                "kept": [at(t1(), 1), at(t2(), 1), at(t2_changed(), 2), at(t3(), 1)]
            })
        );
        revise(
            &store,
            &set,
            json!([t1(), t2_changed(), t3_changed()]),
            40,
            T6,
        );
        assert_eq!(
            truths_of(&store),
            json!({
                "current": [at(t1(), 1), at(t2_changed(), 2), at(t3_changed(), 2)],
                "kept": [
                    at(t1(), 1),
                    at(t2(), 1),
                    at(t2_changed(), 2),
                    at(t3(), 1),
                    at(t3_changed(), 2)
                ]
            })
        );
    }

    #[test]
    fn a_backlog_story_unreadable_or_given_a_phase_is_caught() {
        let (_dir, store) = approved();
        let read_3 = read(&store, &project(), 3).expect("story 3 is read");
        assert_eq!(
            read_3.story,
            Story {
                id: 3,
                sentence: "Teammates see the backlog".into(),
                status: "deferred".into(),
                phase: None,
                truths: 0,
            }
        );
        assert_eq!(read_3.phase, None);
        assert_eq!(read_3.truths, None);
        assert_eq!(read(&store, &project(), 9), Err(ReadError::NoSuchStory));
    }

    #[test]
    fn a_refinement_read_showing_a_pending_description_is_caught() {
        let (_dir, store) = approved();
        submit_change(&store, ChangeKind::ProjectDescribe, &d1(), R1, T4).unwrap();
        let read_1 = read(&store, &project(), 1).expect("story 1 is read");
        assert_eq!(
            read_1.description,
            Some(Description {
                what: "A ledger for agent work".into(),
                core_value: "Nothing moves on an unproven claim".into(),
                constraints: vec!["One binary".into()],
            })
        );
    }
}
