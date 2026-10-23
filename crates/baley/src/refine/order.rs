//! A story's analyzer order (design 0005 `story refine`, PLN-R6, D-08,
//! D-09): the check that chooses the questions order or the drafting
//! order, judged before admission and recording nothing.
//!
//! A story has one live analyzer order, and no order is chosen while a
//! question of its latest set is open, so a set mid-answer is never
//! abandoned by a new one and truths are never drafted before the owner
//! decided.

use baley_core::policy::Role;
use baley_core::questions::{SetFacts, questions_key};
use baley_core::scope::{BACKLOG_VIEW, NO_SUCH_STORY, PHASE_VIEW, backlog_key, phase_key};
use baley_core::truth::{DISPATCH_ACTIVE, QUESTION_OPEN, Refinement, refinement};
use baley_core::work_order::{
    ANALYZER_ORDER_PENDING, PENDING, WORK_ORDER_LIVE_INDEX, WORK_ORDER_VIEW, story_subject,
};
use baley_store::{
    IndexQuery, KeyValue, ObservedDocument, PageRequest, ProjectId, StoreError, Views,
};
use serde_json::{Value, json};

use crate::envelope::{LEDGER_BUSY, Refusal};
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::questions::read::{SetRead, read as read_set};
use crate::scope::read::{ReadError, Story, story as read_story};
use crate::scope::submit::{NOT_RECORDED_BUSY, scope_unread};

/// The status a dropped story holds.
const DROPPED: &str = "dropped";

/// Which of the analyzer's two orders a story gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderKind {
    /// The first order, which returns questions.
    Questions,
    /// The second order, which drafts truths from the answers.
    Drafting,
}

impl OrderKind {
    /// The kind as a receipt spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Questions => "questions",
            Self::Drafting => "drafting",
        }
    }
}

/// A story's latest question set, as its facts were read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestSet {
    /// The set's id.
    pub id: String,
    /// What is recorded about it.
    pub facts: SetFacts,
}

/// Where a story's refinement stands when no question of its latest set is
/// open, with the documents that say so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefinementState {
    /// `Unrefined`, `Drafting` or `Refined`; never `QuestionsOpen`, which is
    /// refused.
    pub refinement: Refinement,
    /// The story's latest set, none before its first.
    pub latest: Option<LatestSet>,
    /// The `backlog` document and the latest set's `questions` document, at
    /// the sequence that produced each.
    pub documents: Vec<ObservedDocument>,
}

/// What `story-refine`'s check found, which the issue records and
/// observes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// The story, as the roadmap holds it.
    pub story: Story,
    /// The order it gets.
    pub kind: OrderKind,
    /// Its latest set, which a drafting order binds with every answer and
    /// deferral.
    pub latest: Option<LatestSet>,
    /// The documents the check read, at the sequence that produced each.
    pub documents: Vec<ObservedDocument>,
}

/// `story-refine`'s check for story `story` in `project`, run before
/// admission and recording nothing. In order: a story the roadmap does not
/// hold, or a dropped one, is `no-such-story`, while a deferred or
/// excluded story can be refined (D-09); a pending analyzer order of the
/// story is `analyzer-order-pending`, naming it (D-09); an open question of
/// its latest set is `question-open`, naming the set and the question
/// (owner ruling R3); then `dispatch-active` (PRJ-R16). A closed latest set
/// the story was not refined from gives the drafting order (D-08); no set,
/// or one the story was refined from, gives the questions order. A read
/// fault is the `failed` answer.
pub fn check<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    story: u32,
) -> Result<Checked, Value> {
    let entry = match read_story(store, project, story) {
        Ok(entry) if entry.status != DROPPED => entry,
        Ok(_) | Err(ReadError::NoSuchStory) => return Err(no_such_story(story)),
        Err(error) => return Err(scope_unread(&error)),
    };
    let pending = pending_analyzer(store, project, story)
        .map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))?;
    if let Some(order) = pending {
        return Err(analyzer_order_pending(
            story,
            &order,
            "so no other analyzer order was issued",
        ));
    }
    let state = refinement_state(store, project, story)?;
    // A story in the backlog has no phase, so nothing is dispatched on it.
    if let Some(phase) = entry.phase
        && dispatched(store, project, phase)?
    {
        return Err(Refusal::new(
            DISPATCH_ACTIVE,
            format!(
                "a worker is dispatched on phase {phase}, which story {story} is committed to, so no analyzer order was issued"
            ),
        )
        .slot("operation")
        .value());
    }
    let kind = match state.refinement {
        Refinement::Drafting => OrderKind::Drafting,
        _ => OrderKind::Questions,
    };
    Ok(Checked {
        story: entry,
        kind,
        latest: state.latest,
        documents: state.documents,
    })
}

/// The id of story `story`'s pending analyzer order in `project`, the newest
/// the `live` index gives for `analyzer`, the story's subject and
/// `pending`, or none. Both analyzer orders share the subject, so either
/// one is found.
pub fn pending_analyzer<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    story: u32,
) -> Result<Option<String>, StoreError> {
    let text = |value: &str| KeyValue::Text(value.into());
    let query = IndexQuery {
        index: WORK_ORDER_LIVE_INDEX.into(),
        equals: vec![
            text(Role::Analyzer.name()),
            text(&story_subject(story)),
            text(PENDING),
        ],
        page: PageRequest {
            limit: 1,
            after: None,
        },
    };
    let page = store.find(project, WORK_ORDER_VIEW, &query)?;
    Ok(page.items.into_iter().next().map(|document| {
        document
            .body
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }))
}

/// The refusal for story `story`'s pending analyzer order `pending`, with
/// `not_issued` saying what was not issued.
pub fn analyzer_order_pending(story: u32, pending: &str, not_issued: &str) -> Value {
    Refusal::new(
        ANALYZER_ORDER_PENDING,
        format!(
            "story {story}'s analyzer work order {pending} is still pending, {not_issued}, since a story has one live analyzer order. Read that order with `document`, or fail it with `work-order-fail` and retry it with `work-order-retry`"
        ),
    )
    .slot("operation")
    .details(json!({"work_order": pending}))
    .value()
}

/// Story `story`'s refinement state, worked out from its `backlog` entry
/// and its latest set's `questions` document through the same derivation
/// the backlog read uses. An open question of the latest set is refused
/// `question-open`, naming the set and its first open question in submitted
/// order, and records nothing. `story-refine` and a retry of an analyzer
/// order both ask here.
pub fn refinement_state<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    story: u32,
) -> Result<RefinementState, Value> {
    let busy = |error: StoreError| store_failed(&error, NOT_RECORDED_BUSY);
    let backlog = store
        .get(project, BACKLOG_VIEW, &backlog_key())
        .map_err(busy)?
        .ok_or_else(|| unreadable(BACKLOG_VIEW))?;
    let entry = backlog
        .body
        .get("stories")
        .and_then(Value::as_array)
        .and_then(|stories| {
            stories
                .iter()
                .find(|entry| entry.get("id").and_then(Value::as_u64) == Some(u64::from(story)))
        })
        .ok_or_else(|| unreadable(BACKLOG_VIEW))?;
    let set = optional_text(entry, "set").ok_or_else(|| unreadable(BACKLOG_VIEW))?;
    let refined = optional_text(entry, "refined").ok_or_else(|| unreadable(BACKLOG_VIEW))?;
    let mut documents = vec![ObservedDocument {
        view: BACKLOG_VIEW.into(),
        key: backlog_key(),
        produced_seq: Some(backlog.produced_seq),
    }];
    let held = match set {
        None => None,
        Some(id) => match read_set(store, project, id).map_err(busy)? {
            SetRead::Held(held) => {
                documents.push(ObservedDocument {
                    view: baley_core::questions::QUESTIONS_VIEW.into(),
                    key: questions_key(id),
                    produced_seq: Some(held.produced_seq),
                });
                Some((id, held))
            }
            // The backlog names a set only in the transaction that opens it.
            SetRead::NotHeld | SetRead::Unreadable(_) => {
                return Err(unreadable(baley_core::questions::QUESTIONS_VIEW));
            }
        },
    };
    let latest = held.as_ref().map(|(id, held)| (*id, &held.round));
    let Ok(state) = refinement(latest, refined) else {
        // Only a later set abandons a story's set, so the backlog moved
        // between the two reads.
        return Err(failed(
            LEDGER_BUSY,
            "the story's question sets moved while this call was prepared, so nothing was recorded. The same call can be sent again",
            PLACE_LEDGER,
        ));
    };
    let latest = held.map(|(id, held)| LatestSet {
        id: id.into(),
        facts: held.facts,
    });
    if let (Refinement::QuestionsOpen { .. }, Some(set)) = (state, &latest) {
        let open = first_open(&set.facts).unwrap_or_default();
        return Err(Refusal::new(
            QUESTION_OPEN,
            format!(
                "question {open} of story {story}'s question set {} is open, so no analyzer order was issued. The owner answers the set's questions in rounds, or defers each open one with a reason",
                set.id
            ),
        )
        .slot("operation")
        .details(json!({"set": set.id, "question": open}))
        .value());
    }
    Ok(RefinementState {
        refinement: state,
        latest,
        documents,
    })
}

/// The first question of `facts` in submitted order that is neither
/// answered nor deferred.
fn first_open(facts: &SetFacts) -> Option<&str> {
    facts
        .questions
        .iter()
        .map(|question| question.id.as_str())
        .find(|id| {
            !facts.answers.iter().any(|answer| answer.question == *id)
                && !facts
                    .deferrals
                    .iter()
                    .any(|deferral| deferral.question == *id)
        })
}

/// Whether a worker is dispatched on phase `phase`, read from its `phase`
/// document. Every status this build records says no dispatch; Build 5
/// records dispatch and adds the statuses that mean one here. Until then
/// an unknown status is a ledger fault, as the scope judgement treats one.
fn dispatched<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    phase: u32,
) -> Result<bool, Value> {
    let document = store
        .get(project, PHASE_VIEW, &phase_key(phase))
        .map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))?
        .ok_or_else(|| unreadable(PHASE_VIEW))?;
    dispatch_of(document.body.get("status").and_then(Value::as_str))
}

/// Whether a phase with `status` has a worker dispatched: no for every
/// status this build records, and a ledger fault for any other or none.
fn dispatch_of(status: Option<&str>) -> Result<bool, Value> {
    match status {
        Some("declared" | "withdrawn") => Ok(false),
        _ => Err(unreadable(PHASE_VIEW)),
    }
}

/// A member that is text or null: `Some(None)` for null or absent,
/// `None` for anything else.
fn optional_text<'a>(entry: &'a Value, name: &str) -> Option<Option<&'a str>> {
    match entry.get(name) {
        None | Some(Value::Null) => Some(None),
        Some(Value::String(text)) => Some(Some(text)),
        Some(_) => None,
    }
}

/// The refusal for a story the roadmap does not hold or that is dropped.
fn no_such_story(story: u32) -> Value {
    Refusal::new(
        NO_SUCH_STORY,
        format!("this project has no story {story} that can be refined, so nothing was recorded"),
    )
    .slot("story")
    .value()
}

fn unreadable(view: &str) -> Value {
    failed(
        LEDGER_UNAVAILABLE,
        format!("the {view} view document cannot be read, so nothing was recorded"),
        PLACE_LEDGER,
    )
}

#[cfg(test)]
mod tests {
    //! Stores are real SQLite stores in fresh temporary directories, with
    //! times supplied; expected answers are written out from design 0005,
    //! D-08, D-09, owner ruling R3 and the plan's data S1 and Q1, never
    //! from running this code.

    use std::collections::BTreeSet;
    use std::num::NonZeroU32;

    use baley_core::policy::{AcceptedNames, Host, Schema, effective_policy};
    use baley_core::questions::{
        QUESTION_ANSWERED, QUESTION_ANSWERED_VERSION, answered_payload, judge_set,
    };
    use baley_core::scope::{ChangeKind, ROADMAP_STREAM};
    use baley_core::truth::{STORY_REFINED, STORY_REFINED_VERSION};
    use baley_store::{Hash, NewEvent, StreamName};
    use baley_store_sqlite::SqliteStore;
    use serde_json::Map;

    use super::*;
    use crate::questions::record::tests::{open_set, q1, story as subject};
    use crate::scope::fixtures::*;
    use crate::work_order::issue::{self, Input, Issue};
    use crate::work_order::route::compose;

    pub(crate) const I0: &str = "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e";

    /// A store holding S1, approved.
    pub(crate) fn approved() -> (tempfile::TempDir, SqliteStore) {
        let (dir, store) = store();
        let digest = receipt(&submit(&store, &s1(), R1, 1, T1).unwrap())["digest"]
            .as_str()
            .and_then(Hash::from_hex)
            .unwrap();
        approve_draft(&store, digest);
        (dir, store)
    }

    /// Records a pending questions order `id` for story `story` by hand,
    /// with no check, under `request_id`.
    pub(crate) fn issue_by_hand(store: &SqliteStore, id: &str, story: u32, request_id: &str) {
        let policy =
            effective_policy(Schema::standard(), Some(Host::ClaudeCode), None, None).unwrap();
        let names = AcceptedNames {
            names: BTreeSet::new(),
            version: 0,
        };
        let route = compose(
            &policy,
            0,
            Host::ClaudeCode,
            Role::Analyzer,
            NonZeroU32::MIN,
            &names,
        )
        .unwrap();
        let instructions = [crate::instruction::evidence("bal-analyzer-questions").unwrap()];
        let inputs = [Input::Bytes {
            name: "story",
            bytes: b"{}",
        }];
        let subject = story_subject(story);
        let issue = Issue {
            id,
            request: id,
            parent: None,
            subject: &subject,
            route: &route,
            instructions: &instructions,
            inputs: &inputs,
            absences: &[],
            streams: &[],
            documents: &[],
            further: &Map::new(),
        };
        let command = command("story.refine", request_id, 8, T2);
        let answer = issue::answer(issue::record(store, &command, Some(&issue)), store);
        assert_eq!(answer["status"], "ok", "{answer}");
    }

    /// Records the owner's answer to `question` of `set` in `round` by hand.
    pub(crate) fn answer_by_hand(
        store: &SqliteStore,
        set: &str,
        question: &str,
        round: u32,
        request_id: &str,
    ) {
        append(
            store,
            request_id,
            T3,
            vec![NewEvent {
                stream: StreamName(ROADMAP_STREAM.into()),
                type_name: QUESTION_ANSWERED.into(),
                type_version: QUESTION_ANSWERED_VERSION,
                git: None,
                payload: answered_payload(set, question, round, "SQLite", true, "Ada Owner", T3),
                attachments: Vec::new(),
            }],
        );
    }

    fn kind(store: &SqliteStore, story: u32) -> Result<OrderKind, Value> {
        check(store, &project(), story).map(|checked| checked.kind)
    }

    fn refused(answer: Result<Checked, Value>) -> Value {
        let refusal = answer.expect_err("refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        refusal
    }

    #[test]
    fn a_drafting_order_chosen_with_a_question_open_is_caught() {
        let (_dir, store) = approved();
        assert_eq!(kind(&store, 1), Ok(OrderKind::Questions));

        let set = open_set(&store, R2, &q1(), &subject(1));
        assert_eq!(
            refused(check(&store, &project(), 1))["code"],
            "question-open"
        );

        answer_by_hand(&store, &set, "q1", 1, R3);
        answer_by_hand(&store, &set, "q2", 2, R4);
        assert_eq!(kind(&store, 1), Ok(OrderKind::Drafting));

        let refined = json!({"story": 1, "set": set, "submission": "5c".repeat(32),
            "owner": "Ada Owner", "at": T4, "truths": []});
        append(
            &store,
            R6,
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
        assert_eq!(kind(&store, 1), Ok(OrderKind::Questions));

        open_set(&store, R7, &judge_set(&json!([])).unwrap(), &subject(1));
        assert_eq!(kind(&store, 1), Ok(OrderKind::Drafting));
    }

    #[test]
    fn a_refine_over_an_open_set_let_through_or_judged_before_a_pending_order_is_caught() {
        let (_dir, store) = approved();
        let set = open_set(&store, R2, &q1(), &subject(1));
        answer_by_hand(&store, &set, "q1", 1, R3);

        let before = head(&store);
        let refusal = refused(check(&store, &project(), 1));
        assert_eq!(head(&store), before);
        assert_eq!(refusal["code"], "question-open");
        assert_eq!(refusal["slot"], "operation");
        assert_eq!(refusal["details"], json!({"set": set, "question": "q2"}));

        issue_by_hand(&store, I0, 1, R4);
        let before = head(&store);
        let refusal = refused(check(&store, &project(), 1));
        assert_eq!(head(&store), before);
        assert_eq!(refusal["code"], "analyzer-order-pending");
        assert_eq!(refusal["details"], json!({"work_order": I0}));
    }

    #[test]
    fn a_second_analyzer_order_or_a_dropped_story_let_through_is_caught() {
        use crate::scope::approve::{self, APPROVE_COMMAND};

        let (_dir, store) = approved();
        issue_by_hand(&store, I0, 1, R2);
        let drop = receipt(
            &submit_change(&store, ChangeKind::StoryDrop, &json!({"story": 3}), R3, T4).unwrap(),
        )["digest"]
            .as_str()
            .and_then(Hash::from_hex)
            .unwrap();
        let judged = approve::judge(&store, &project(), drop).unwrap();
        approve::record(
            &store,
            &command(APPROVE_COMMAND, R6, 6, T5),
            Some(&judged),
            "Ada Owner",
        )
        .unwrap();

        let before = head(&store);
        let refusal = refused(check(&store, &project(), 1));
        assert_eq!(refusal["code"], "analyzer-order-pending");
        assert_eq!(refusal["slot"], "operation");
        assert_eq!(
            refusal["details"],
            json!({"work_order": "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e"})
        );
        assert_eq!(kind(&store, 2), Ok(OrderKind::Questions));
        for story in [3, 9] {
            let refusal = refused(check(&store, &project(), story));
            assert_eq!(refusal["code"], "no-such-story", "{story}");
            assert_eq!(refusal["slot"], "story", "{story}");
        }
        assert_eq!(head(&store), before);
    }

    #[test]
    fn a_deferred_story_refused_or_an_unknown_phase_status_taken_for_no_dispatch_is_caught() {
        let (_dir, store) = approved();
        let deferred = check(&store, &project(), 3).expect("a deferred story can be refined");
        assert_eq!(deferred.story.status, "deferred");
        assert_eq!(deferred.story.phase, None);
        assert_eq!(deferred.kind, OrderKind::Questions);

        // No write records a status this build does not know, so the
        // observation is judged over the status text itself.
        assert_eq!(dispatch_of(Some("declared")), Ok(false));
        let fault = dispatch_of(Some("executing")).expect_err("an unknown status is a fault");
        assert_eq!(fault["status"], "failed");
        assert_eq!(fault["code"], "ledger-unavailable");
    }
}
