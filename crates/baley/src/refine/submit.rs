//! A truth draft as the session submits the analyzer's drafted truths
//! (design 0005 `story truths submit`, PLN-R2 to PLN-R5, D-02, D-05, D-18,
//! D-20): a check run before admission and recording nothing, then a
//! recording that stores the draft as a `record` payload and one
//! `scope.submitted` of kind `story-truths` on the `project` stream, as a
//! scope draft is recorded. The owner approves it by digest at the
//! terminal.
//!
//! A draft can be recorded while a question of its set is open; only the
//! approval is refused then (D-18). The marks a receipt carries say how each
//! truth stands against the story's current truths, never its version,
//! which is given at approval (D-02).

use std::collections::BTreeMap;

use baley_core::questions::QUESTION_SET_ABANDONED;
use baley_core::scope::{
    PROJECT_STREAM, SCOPE_SUBMITTED, SCOPE_SUBMITTED_VERSION, submitted_payload,
};
use baley_core::truth::{
    Mark, STORY_TRUTHS, TRUTHS_VIEW, Truth, TruthSubmission, Versions, truths_key, versions,
};
use baley_core::work_order::{NO_SUCH_WORK_ORDER, WRONG_WORK_ORDER};
use baley_store::{
    Command, Decision, InstructionEvidence, Ledger, NewEvent, Observed, ObservedDocument,
    OutcomeKind, ProjectId, Recorded, RetentionClass, StoreError, StreamName, Views,
};
use serde_json::{Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::questions::read::{SetRead, no_such_question_set, read as read_set};
use crate::refine::order::{
    OrderKind, backlog_read, is_analyzer_order, judged_story, order_evidence,
};
use crate::scope::submit::NOT_RECORDED_BUSY;
use crate::work_order::read::order_document;

/// What the check read, which the recording records and observes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// The drafting order's instruction evidence, which the submission's
    /// caller carries (D-19).
    pub evidence: Vec<InstructionEvidence>,
    /// How each submitted truth stands against the story's current truths,
    /// and the current truths the draft leaves out. Only the marks are
    /// given out; versions are given at approval (D-02).
    pub versions: Versions,
    /// The `backlog` and `truths` documents, at the sequence that produced
    /// each.
    documents: Vec<ObservedDocument>,
}

/// A story's current truths as its `truths` document holds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentTruths {
    /// Each current truth with its version, in the order approved.
    pub truths: Vec<(Truth, u32)>,
    /// Every version each id ever had.
    pub kept: BTreeMap<String, Vec<u32>>,
    /// The `truths` document, at the sequence that produced it, or absent
    /// before the story's first refinement.
    pub document: ObservedDocument,
}

/// Reads story `story`'s current truths from its `truths` document; none
/// before its first refinement. A document that cannot be read is a ledger
/// fault.
pub fn current_truths<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    story: u32,
) -> Result<CurrentTruths, Value> {
    let document = store
        .get(project, TRUTHS_VIEW, &truths_key(story))
        .map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))?;
    let observed = ObservedDocument {
        view: TRUTHS_VIEW.into(),
        key: truths_key(story),
        produced_seq: document.as_ref().map(|document| document.produced_seq),
    };
    let Some(document) = document else {
        return Ok(CurrentTruths {
            truths: Vec::new(),
            kept: BTreeMap::new(),
            document: observed,
        });
    };
    let unreadable = || {
        failed(
            LEDGER_UNAVAILABLE,
            format!("story {story}'s truths cannot be read, so nothing was recorded"),
            PLACE_LEDGER,
        )
    };
    let truths = document
        .body
        .get("truths")
        .and_then(Value::as_array)
        .ok_or_else(unreadable)?
        .iter()
        .map(versioned_truth)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(unreadable)?;
    let kept = document
        .body
        .get("kept")
        .and_then(Value::as_object)
        .ok_or_else(unreadable)?
        .iter()
        .map(|(id, entries)| {
            let numbers = entries
                .as_array()?
                .iter()
                .map(version_of)
                .collect::<Option<Vec<_>>>()?;
            Some((id.clone(), numbers))
        })
        .collect::<Option<BTreeMap<_, _>>>()
        .ok_or_else(unreadable)?;
    Ok(CurrentTruths {
        truths,
        kept,
        document: observed,
    })
}

/// One truth of a `truths` document with its version, or none when a member
/// cannot be read.
fn versioned_truth(entry: &Value) -> Option<(Truth, u32)> {
    let text = |name: &str| entry.get(name).and_then(Value::as_str).map(str::to_owned);
    let human_verify = match entry.get("human_verify") {
        None => None,
        Some(value) => Some(value.as_str()?.to_owned()),
    };
    let truth = Truth {
        id: text("id")?,
        form: text("form")?,
        trigger: text("trigger")?,
        observer: text("observer")?,
        verb: text("verb")?,
        outcome: text("outcome")?,
        kind: text("kind")?,
        human_verify,
    };
    Some((truth, version_of(entry)?))
}

/// The `version` member of an entry, when it is a whole number within u32.
fn version_of(entry: &Value) -> Option<u32> {
    entry
        .get("version")
        .and_then(Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
}

/// A mark as a receipt or a preview spells it.
pub fn mark_str(mark: Mark) -> &'static str {
    match mark {
        Mark::Unchanged => "unchanged",
        Mark::Changed => "changed",
        Mark::Added => "added",
    }
}

/// The check of the accepted truth `submission` answering work order
/// `work_order`, run before admission and recording nothing. In order: the
/// story, `no-such-story` for one not held or dropped; the set through its
/// read, `no-such-question-set` at slot `set` for one not held or another
/// story's, and `question-set-abandoned` at slot `set` for one that is not
/// the story's latest (D-18), while an open question refuses nothing here;
/// the order, none `no-such-work-order` and one that is not this story's
/// analyzer drafting order `wrong-work-order`, both at slot `work_order`, in
/// any state (D-20); then the story's current truths, which each submitted
/// truth is marked against. A read fault is the `failed` answer.
pub fn check<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    submission: &TruthSubmission,
    work_order: &str,
) -> Result<Checked, Value> {
    let busy = |error: StoreError| store_failed(&error, NOT_RECORDED_BUSY);
    let story = submission.story();
    judged_story(store, project, story)?;
    let backlog = backlog_read(store, project, story)?;
    match read_set(store, project, submission.set()).map_err(busy)? {
        SetRead::Held(held) if held.story == Some(story) => {}
        SetRead::Held(_) | SetRead::NotHeld => return Err(no_such_question_set("set")),
        SetRead::Unreadable(unreadable) => {
            return Err(failed(LEDGER_UNAVAILABLE, unreadable.0, PLACE_LEDGER));
        }
    }
    if backlog.latest.as_ref().map(|(id, _)| id.as_str()) != Some(submission.set()) {
        return Err(Refusal::new(
            QUESTION_SET_ABANDONED,
            format!(
                "that question set is not story {story}'s latest, so truths drafted from it were not recorded. Draft them from the story's latest set"
            ),
        )
        .slot("set")
        .value());
    }
    let order = order_document(store, project, work_order)
        .map_err(busy)?
        .ok_or_else(|| {
            Refusal::new(
                NO_SUCH_WORK_ORDER,
                "this project holds no work order with that id, so nothing was recorded",
            )
            .slot("work_order")
            .value()
        })?;
    if !is_analyzer_order(&order.body, story, OrderKind::Drafting) {
        return Err(Refusal::new(
            WRONG_WORK_ORDER,
            format!(
                "that work order is not story {story}'s analyzer drafting order, so its truths cannot answer it, and nothing was recorded"
            ),
        )
        .slot("work_order")
        .value());
    }
    let evidence = order_evidence(&order.body).ok_or_else(|| {
        failed(
            LEDGER_UNAVAILABLE,
            "the ledger holds a work order whose instructions cannot be read",
            PLACE_LEDGER,
        )
    })?;
    let current = current_truths(store, project, story)?;
    let versions = versions(submission.truths(), &current.truths, &current.kept).map_err(|_| {
        failed(
            LEDGER_UNAVAILABLE,
            "a truth of this story has no version number left, so nothing was recorded",
            PLACE_LEDGER,
        )
    })?;
    let mut documents: Vec<ObservedDocument> = backlog
        .documents
        .into_iter()
        .filter(|document| document.view != baley_core::questions::QUESTIONS_VIEW)
        .collect();
    documents.push(current.document);
    Ok(Checked {
        evidence,
        versions,
        documents,
    })
}

/// Records the accepted truth `submission` under `command`, the command its
/// caller prepared, carrying the drafting order it answers, in one
/// transaction: its body as a `record` payload and one `scope.submitted` of
/// kind `story-truths` with target `story/<id>` on the `project` stream
/// attaching it (D-05), so the `work_order` view lists the digest under the
/// order and marks it answered (D-20). Every call records, so the same
/// content submitted again is the newest draft again; only a retry of the
/// same request replays, before any decision runs. The decision observes the
/// `backlog` and `truths` documents the check read, so a set replaced or a
/// story refined since makes the call `ledger-busy` with nothing recorded; a
/// decision asked to run without the check's read records nothing.
///
/// The receipt: `status`, `digest`, `kind`, `story`, `set`, `recorded_at`,
/// `marks`, each truth's `{id, mark}` in submitted order, and `removed`, the
/// ids of the current truths the draft leaves out. No version is given.
pub fn record<L: Ledger + ?Sized>(
    store: &L,
    command: &Command,
    submission: &TruthSubmission,
    checked: Option<&Checked>,
) -> Result<Recorded, StoreError> {
    store.transact(command, &mut |tx| {
        let Some(checked) = checked else {
            return Err(StoreError::Unavailable(
                "a truth submission reached its transaction without its check's read".into(),
            ));
        };
        let body = tx.put_payload(submission.body(), RetentionClass::Record)?;
        tx.append(NewEvent {
            stream: StreamName(PROJECT_STREAM.into()),
            type_name: SCOPE_SUBMITTED.into(),
            type_version: SCOPE_SUBMITTED_VERSION,
            git: None,
            payload: submitted_payload(
                submission.digest(),
                STORY_TRUTHS,
                submission.target(),
                &body,
            ),
            attachments: vec![body],
        })?;
        let marks: Vec<Value> = checked
            .versions
            .truths
            .iter()
            .map(|truth| json!({"id": truth.id, "mark": mark_str(truth.mark)}))
            .collect();
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: json!({
                "status": "ok",
                "digest": submission.digest().to_hex(),
                "kind": STORY_TRUTHS,
                "story": submission.story(),
                "set": submission.set(),
                "recorded_at": command.recorded_at,
                "marks": marks,
                "removed": checked.versions.removed,
            }),
            sensitive: false,
            observed: Observed {
                documents: checked.documents.clone(),
                ..Observed::default()
            },
            git: None,
        })
    })
}

#[cfg(test)]
pub(crate) mod tests {
    //! Stores are real SQLite stores in fresh temporary directories, with
    //! times supplied; expected answers are written out from design 0005,
    //! D-02, D-05, D-18, D-20 and the plan's data S1, Q1, Q3, T1 and T2,
    //! never from running this code.

    use std::io::Read;
    use std::num::NonZeroU32;

    use baley_core::scope::{DRAFT_NEWEST_INDEX, DRAFT_VIEW, ROADMAP_STREAM};
    use baley_core::truth::{STORY_REFINED, STORY_REFINED_VERSION, judge_truths};
    use baley_store::{
        Caller, Hash, IndexQuery, KeyValue, PageRequest, PayloadBody, Payloads, canonical_json,
    };
    use baley_store_sqlite::SqliteStore;

    use super::*;
    use crate::questions::record::tests::q1;
    use crate::refine::order::tests::{answer_by_hand, approved, issue_by_hand};
    use crate::refine::questions::tests::{D, Q, Q2, q3, refine, submitted};
    use crate::scope::fixtures::*;
    use crate::scope::submit::SUBMIT_COMMAND;

    /// Story 2's questions order.
    pub(crate) const QC: &str = "5a6b7c8d-9e0f-4a1b-8c2d-4e5f6a7b8c9d";
    /// Story 2's drafting order.
    pub(crate) const D2: &str = "6b7c8d9e-0f1a-4b2c-9d3e-5f6a7b8c9d0e";
    const UNKNOWN: &str = "7c8d9e0f-1a2b-4c3d-8e4f-6a7b8c9d0e1f";

    /// T1, Plan 1's first truth.
    pub(crate) fn t1() -> Value {
        json!({"id": "t1", "form": "when",
            "trigger": "the owner approves a draft by a stale digest",
            "observer": "the owner", "verb": "is refused",
            "outcome": "stale-draft naming the first differing part", "kind": "literal",
            "observable": true, "fixed_oracle": true})
    }

    /// T2, Plan 1's second truth.
    pub(crate) fn t2() -> Value {
        json!({"id": "t2", "form": "for-any",
            "trigger": "order the answers of a set are recorded in",
            "observer": "a ledger reader", "verb": "gets", "outcome": "the same round",
            "kind": "property", "observable": true, "fixed_oracle": true})
    }

    /// `truth` with `member` set to `value`.
    pub(crate) fn with(truth: Value, member: &str, value: &str) -> Value {
        let mut truth = truth;
        truth[member] = json!(value);
        truth
    }

    /// Submits `truths` for story `story` drafted from set `set`, naming
    /// `order`, through the check and the recording under a command whose
    /// caller carries the order, as `context-submit` runs them below the
    /// session.
    pub(crate) fn truths_submitted(
        store: &SqliteStore,
        story: u32,
        set: &str,
        truths: &Value,
        order: &str,
        request_id: &str,
        at: &str,
    ) -> Value {
        let submission = judge_truths(NonZeroU32::new(story).unwrap(), set, truths).unwrap();
        let checked = match check(store, &project(), &submission, order) {
            Ok(checked) => checked,
            Err(refusal) => return refusal,
        };
        let mut command = command(SUBMIT_COMMAND, request_id, 5, at);
        command.caller = Some(Caller::Server(caller().with_work_order(order).unwrap()));
        crate::scope::submit::answer(record(store, &command, &submission, Some(&checked)), store)
    }

    /// A store holding S1, approved, with story 1's set A from Q1 closed by
    /// hand and its drafting order [`D`] issued through `story-refine`.
    /// Gives the store and A's id.
    pub(crate) fn drafting() -> (tempfile::TempDir, SqliteStore, String) {
        let (dir, store) = approved();
        refine(&store, 1, Q, R1, T1);
        let a = submitted(&store, 1, Q, &q1(), R2, T2);
        let a = a["set"].as_str().expect("A is opened").to_owned();
        answer_by_hand(&store, &a, "q1", 1, R3);
        answer_by_hand(&store, &a, "q2", 2, R4);
        refine(&store, 1, D, R5, T3);
        (dir, store, a)
    }

    fn body_bytes(store: &SqliteStore, event: &baley_store::Event) -> Vec<u8> {
        let hash = Hash::from_hex(event.payload["body"]["payload"].as_str().unwrap()).unwrap();
        let PayloadBody::Present(mut reader) = store.open(&hash).unwrap() else {
            panic!("the body is present");
        };
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    }

    fn truth_drafts(store: &SqliteStore) -> Vec<baley_store::Event> {
        history(store)
            .into_iter()
            .filter(|event| {
                event.type_name == "scope.submitted" && event.payload["kind"] == "story-truths"
            })
            .collect()
    }

    #[test]
    fn a_truth_draft_recorded_off_its_order_or_without_record_retention_is_caught() {
        let (_dir, store, a) = drafting();
        let receipt = truths_submitted(&store, 1, &a, &json!([t1(), t2()]), D, R6, T6);
        assert_eq!(receipt["status"], "ok", "{receipt}");

        let drafts = truth_drafts(&store);
        assert_eq!(drafts.len(), 1);
        let event = &drafts[0];
        assert_eq!(event.stream, "project");
        assert_eq!(event.payload["target"], "story/1");
        assert_eq!(event.payload["body"]["class"], "record");
        let expected = canonical_json(&json!({"kind": "story-truths", "set": a, "story": 1,
            "truths": [t1(), t2()]}))
        .unwrap();
        assert_eq!(body_bytes(&store, event), expected);
        let Some(Caller::Server(on)) = &event.caller else {
            panic!("a server caller: {:?}", event.caller);
        };
        assert_eq!(on.work_order(), Some(D));
        let digest = receipt["digest"].as_str().unwrap();
        assert_eq!(event.payload["digest"], digest);
        let order = crate::work_order::read::order(&store, &project(), D)
            .unwrap()
            .unwrap();
        assert_eq!(order["state"], "answered", "{order}");
        assert_eq!(order["drafts"], json!([digest]), "{order}");
        assert_eq!(
            receipt["marks"],
            json!([{"id": "t1", "mark": "added"}, {"id": "t2", "mark": "added"}])
        );
        assert_eq!(receipt["removed"], json!([]));
        assert_eq!(receipt["kind"], "story-truths");
        assert_eq!(receipt["story"], 1);
        assert_eq!(receipt["set"], a.as_str());
    }

    #[test]
    fn a_resubmitted_draft_left_stale_is_caught() {
        let (_dir, store, a) = drafting();
        let x = json!([t1(), t2()]);
        let y = json!([t1(), with(t2(), "outcome", "a later round")]);
        let first = truths_submitted(&store, 1, &a, &x, D, R6, T6);
        let second = truths_submitted(&store, 1, &a, &y, D, R7, T7);
        let third = truths_submitted(&store, 1, &a, &x, D, R8, T8);
        for receipt in [&first, &second, &third] {
            assert_eq!(receipt["status"], "ok", "{receipt}");
        }
        let drafts = truth_drafts(&store);
        assert_eq!(drafts.len(), 3);
        for draft in &drafts {
            assert_eq!(draft.payload["target"], "story/1");
        }
        assert_eq!(drafts[2].payload["digest"], first["digest"]);
        let newest = store
            .find(
                &project(),
                DRAFT_VIEW,
                &IndexQuery {
                    index: DRAFT_NEWEST_INDEX.into(),
                    equals: vec![
                        KeyValue::Text("story-truths".into()),
                        KeyValue::Text("story/1".into()),
                    ],
                    page: PageRequest {
                        limit: 1,
                        after: None,
                    },
                },
            )
            .unwrap();
        assert_eq!(newest.items[0].body["digest"], first["digest"]);
    }

    #[test]
    fn a_truth_submission_naming_the_wrong_set_or_order_recorded_is_caught() {
        let (_dir, store, a) = drafting();
        issue_by_hand(&store, Q2, 1, R6);
        let b = submitted(&store, 1, Q2, &q3(), R7, T4);
        let b = b["set"].as_str().expect("B is opened").to_owned();
        refine(&store, 2, QC, R8, T5);
        let c = submitted(&store, 2, QC, &q1(), R9, T5);
        let c = c["set"].as_str().expect("C is opened").to_owned();
        answer_by_hand(&store, &c, "q1", 1, "1b2c3d4e-5f6a-4b7c-8d9e-0f1a2b3c4d5e");
        answer_by_hand(&store, &c, "q2", 2, "2c3d4e5f-6a7b-4c8d-9e0f-1a2b3c4d5e6f");
        refine(&store, 2, D2, "3d4e5f6a-7b8c-4d9e-8f0a-2b3c4d5e6f7a", T6);

        let before = head(&store);
        let one = json!([t1()]);
        for (set, order, code, slot) in [
            (a.as_str(), D, "question-set-abandoned", "set"),
            (c.as_str(), D, "no-such-question-set", "set"),
            (UNKNOWN, D, "no-such-question-set", "set"),
            (b.as_str(), Q, "wrong-work-order", "work_order"),
            (b.as_str(), D2, "wrong-work-order", "work_order"),
        ] {
            let refusal = truths_submitted(&store, 1, set, &one, order, R1, T7);
            assert_eq!(refusal["status"], "refused", "{set} {order}: {refusal}");
            assert_eq!(refusal["code"], code, "{set} {order}: {refusal}");
            assert_eq!(refusal["slot"], slot, "{set} {order}: {refusal}");
        }
        assert_eq!(head(&store), before);
    }

    #[test]
    fn a_submission_refused_while_its_set_is_open_is_caught() {
        let (_dir, store, _a) = drafting();
        issue_by_hand(&store, Q2, 1, R6);
        let b = submitted(&store, 1, Q2, &q3(), R7, T4);
        assert_eq!(b["now"]["state"], "open", "{b}");
        let b = b["set"].as_str().unwrap().to_owned();
        let receipt = truths_submitted(&store, 1, &b, &json!([t1()]), D, R8, T5);
        assert_eq!(receipt["status"], "ok", "{receipt}");
        assert_eq!(truth_drafts(&store).len(), 1);
    }

    #[test]
    fn a_mark_reporting_a_version_or_missing_a_removal_is_caught() {
        let (_dir, store, a) = drafting();
        let versioned = |truth: Value| {
            let mut truth = truth;
            truth["version"] = json!(1);
            truth
        };
        append(
            &store,
            R6,
            T4,
            vec![NewEvent {
                stream: StreamName(ROADMAP_STREAM.into()),
                type_name: STORY_REFINED.into(),
                type_version: STORY_REFINED_VERSION,
                git: None,
                payload: json!({"story": 1, "set": a, "submission": "5c".repeat(32),
                    "owner": "Ada Owner", "at": T4,
                    "truths": [versioned(t1()), versioned(t2())]}),
                attachments: Vec::new(),
            }],
        );
        let t3 = with(t1(), "id", "t3");
        let changed = with(t1(), "outcome", "stale-draft naming the newest draft");
        let receipt = truths_submitted(&store, 1, &a, &json!([changed, t3]), D, R7, T5);
        assert_eq!(receipt["status"], "ok", "{receipt}");
        assert_eq!(
            receipt["marks"],
            json!([{"id": "t1", "mark": "changed"}, {"id": "t3", "mark": "added"}])
        );
        assert_eq!(receipt["removed"], json!(["t2"]));
        assert!(!receipt.to_string().contains("version"), "{receipt}");
        let drafts = truth_drafts(&store);
        let body = String::from_utf8(body_bytes(&store, &drafts[0])).unwrap();
        assert!(!body.contains("version"), "{body}");
    }
}
