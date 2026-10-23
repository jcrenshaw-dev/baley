//! A story's question set as the session submits the analyzer's return
//! (design 0005 `story questions submit`, PLN-R6, PLN-R25, D-07, D-08,
//! D-09, D-20): a check run before admission and recording nothing, then a
//! recording that opens the set, abandons the story's earlier set and, for
//! an empty set, issues the drafting order, all in one transaction.
//!
//! The story's earlier set is abandoned only once it is closed: while a
//! question of it is open the submit is refused, so a late or repeated
//! analyzer return never abandons a set mid-answer (owner ruling R3).

use baley_core::policy::{AcceptedNames, EffectivePolicy, Host};
use baley_core::questions::{
    Cause, QUESTIONS_VIEW, QuestionSet, SetFacts, Subject, questions_key, round_of,
};
use baley_core::work_order::{NO_SUCH_WORK_ORDER, PENDING, WRONG_WORK_ORDER, work_order_key};
use baley_store::{
    Command, Decision, InstructionEvidence, Ledger, Observed, ObservedDocument, OutcomeKind,
    Payloads, ProjectId, Recorded, StaleInput, StoreError, Views,
};
use serde_json::{Map, Value, json};

use crate::envelope::{LEDGER_BUSY, Refusal};
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::questions::answer::with_now;
use crate::questions::record::{AbandonError, abandon, open};
use crate::refine::order::{
    LatestSet, OrderKind, PreparedOrder, analyzer_absences, dispatch_refusal, is_analyzer_order,
    judged_story, order_evidence, other_pending, prepare_drafting, refinement_judged,
};
use crate::scope::submit::NOT_RECORDED_BUSY;
use crate::work_order::read::order_document;

/// The command kind a story's question set is recorded under.
pub const QUESTIONS_SUBMIT_COMMAND: &str = "story.questions.submit";

/// What a story's question set submission did not do when it is refused.
const NOT_RECORDED: &str = "the questions were not recorded";

/// What the check read, which the recording records and observes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// The story the set is for.
    pub story: u32,
    /// The analyzer's questions order the set answers.
    pub work_order: String,
    /// That order's instruction evidence, which the submission's caller
    /// carries (D-19).
    pub evidence: Vec<InstructionEvidence>,
    /// The story's latest set, closed, which the new set supersedes, or none.
    pub latest: Option<LatestSet>,
    /// Whether an analyzer order of the story other than the named one is
    /// pending, read for an empty set only.
    pub other_pending: bool,
    /// The named order's document when it was pending, which the set answers.
    order_pending: Option<ObservedDocument>,
    /// The `roadmap` document the story was judged on, the `backlog`
    /// document and the latest set's `questions` document.
    documents: Vec<ObservedDocument>,
}

impl Checked {
    /// Whether the set's submit issues the drafting order: an empty set is
    /// closed at once (D-08), unless another analyzer order of the story is
    /// pending, which keeps one live analyzer order (D-09).
    pub fn drafts(&self, set: &QuestionSet) -> bool {
        set.questions().is_empty() && !self.other_pending
    }
}

/// The facts of `set` just opened: its questions, nothing answered.
pub fn opened_facts(set: &QuestionSet) -> SetFacts {
    SetFacts {
        questions: set.questions().to_vec(),
        answers: Vec::new(),
        deferrals: Vec::new(),
        abandoned: None,
    }
}

/// The check of story `story`'s set `set`, already accepted by `judge_set`,
/// answering work order `work_order`, run before admission and recording
/// nothing. In order: the story as `story-refine` reads it, `no-such-story`
/// for one not held or dropped; the order, none `no-such-work-order` and one
/// that is not this story's analyzer questions order `wrong-work-order`, both
/// at slot `work_order`, in any state, as an answered start order passes a
/// start submission; an open question of the story's latest set
/// `question-open`, naming the set and the question (owner ruling R3); then
/// `dispatch-active`. For an empty set it also reads whether another
/// analyzer order of the story is pending. A read fault is the `failed`
/// answer.
pub fn check<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    story: u32,
    work_order: &str,
    set: &QuestionSet,
) -> Result<Checked, Value> {
    let busy = |error: StoreError| store_failed(&error, NOT_RECORDED_BUSY);
    let (entry, roadmap) = judged_story(store, project, story)?;
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
    if !is_analyzer_order(&order.body, story, OrderKind::Questions) {
        return Err(Refusal::new(
            WRONG_WORK_ORDER,
            format!(
                "that work order is not story {story}'s analyzer questions order, so its questions cannot answer it, and nothing was recorded"
            ),
        )
        .slot("work_order")
        .value());
    }
    let state = refinement_judged(store, project, story, NOT_RECORDED)?;
    dispatch_refusal(store, project, &entry, NOT_RECORDED)?;
    let evidence = order_evidence(&order.body).ok_or_else(|| {
        failed(
            LEDGER_UNAVAILABLE,
            "the ledger holds a work order whose instructions cannot be read",
            PLACE_LEDGER,
        )
    })?;
    let other_pending = set.questions().is_empty()
        && other_pending(store, project, story, work_order).map_err(busy)?;
    let pending = order.body.get("state").and_then(Value::as_str) == Some(PENDING);
    let mut documents = vec![roadmap];
    documents.extend(state.documents);
    Ok(Checked {
        story,
        work_order: work_order.into(),
        evidence,
        latest: state.latest,
        other_pending,
        order_pending: pending.then(|| ObservedDocument {
            view: baley_core::work_order::WORK_ORDER_VIEW.into(),
            key: work_order_key(work_order),
            produced_seq: Some(order.produced_seq),
        }),
        documents,
    })
}

/// Records `set` as the story's set under `command`, the command its caller
/// prepared, in one transaction: the story's latest set, which the check
/// found closed, abandoned with cause `superseded` (PLN-R25); the set opened
/// for the story and the questions order at the command's time; and
/// `drafting`, the drafting order prepared for an empty set, issued beside it
/// (D-08). The decision observes every document the check read; with a
/// drafting order it also observes the documents its inputs were read from
/// and, when the named order was pending at the check, that order's document,
/// since this set answers it, and otherwise declares that no analyzer order
/// of the story is pending (D-09). A request the store already holds is
/// replayed from its record before any decision runs; a decision asked to
/// run without the check's read records nothing.
///
/// The receipt: `status`, `set`, `story`, `recorded_at`, `now`, the new
/// set's round, and with a drafting order `drafting`, that order's receipt
/// members.
pub fn record<L: Ledger + ?Sized>(
    store: &L,
    command: &Command,
    set: &QuestionSet,
    checked: Option<&Checked>,
    drafting: Option<&PreparedOrder>,
) -> Result<Recorded, StoreError> {
    let at = command.recorded_at.as_str();
    store.transact(command, &mut |tx| {
        let Some(checked) = checked else {
            return Err(StoreError::Unavailable(
                "a question set reached its transaction without its check's read".into(),
            ));
        };
        if let Some(latest) = &checked.latest {
            abandon(tx, &latest.id, Cause::Superseded, at).map_err(|error| match error {
                AbandonError::Store(error) => error,
                // The check read it closed, so it moved since.
                AbandonError::NotHeld | AbandonError::Abandoned => {
                    StoreError::Stale(StaleInput::Document {
                        view: QUESTIONS_VIEW.into(),
                        key: questions_key(&latest.id),
                        seen: None,
                        now: None,
                    })
                }
            })?;
        }
        let subject = Subject::Story {
            story: checked.story,
            work_order: checked.work_order.clone(),
        };
        let id = open(tx, set, &subject, at)?;
        let mut receipt = Map::new();
        receipt.insert("status".into(), json!("ok"));
        receipt.insert("set".into(), json!(id));
        receipt.insert("story".into(), json!(checked.story));
        receipt.insert("recorded_at".into(), json!(at));
        let mut observed = Observed {
            documents: checked.documents.clone(),
            ..Observed::default()
        };
        if let Some(order) = drafting {
            let members = order.issue_within(tx, &id, at)?;
            receipt.insert("drafting".into(), Value::Object(members));
            observed.documents.extend(order.documents().iter().cloned());
            match &checked.order_pending {
                Some(document) => observed.documents.push(document.clone()),
                None => observed.absences = analyzer_absences(checked.story),
            }
        }
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: with_now(receipt, &id, &round_of(&opened_facts(set))),
            sensitive: false,
            observed,
            git: None,
        })
    })
}

/// Records `set`, checked as `checked`, under `command` once the write is
/// prepared: when the submit drafts, the drafting order is prepared as
/// `drafting_id` from the new set's facts, under `policy`, the write's
/// effective policy, for `host` with the catalog `names`, then the set and
/// the order are recorded together through [`record`]. The answer is the
/// receipt, or why nothing was recorded.
#[allow(clippy::too_many_arguments)]
pub fn submit<S: Ledger + Views + Payloads>(
    store: &S,
    command: &Command,
    set: &QuestionSet,
    checked: &Checked,
    policy: &EffectivePolicy,
    host: Host,
    names: &AcceptedNames,
    drafting_id: &str,
) -> Value {
    let drafting = if checked.drafts(set) {
        match prepare_drafting(
            store,
            command,
            checked.story,
            &opened_facts(set),
            policy,
            host,
            names,
            drafting_id,
        ) {
            Ok(prepared) => Some(prepared),
            Err(answer) => return answer,
        }
    } else {
        None
    };
    answer(
        record(store, command, set, Some(checked), drafting.as_ref()),
        store,
    )
}

/// The answer for a question set's transaction result. A story's sets, its
/// orders or its scope moved since the check is `ledger-busy`, since the
/// same call is judged afresh; the rest is answered as a scope call is.
pub fn answer(result: Result<Recorded, StoreError>, payloads: &dyn Payloads) -> Value {
    match result {
        Err(StoreError::Stale(_)) => failed(
            LEDGER_BUSY,
            "the story's question sets, its work orders or its scope moved while this call was prepared, so nothing was recorded. The same call can be sent again",
            PLACE_LEDGER,
        ),
        result => crate::scope::submit::answer(result, payloads),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    //! Stores are real SQLite stores in fresh temporary directories, with
    //! times supplied; expected answers are written out from design 0005,
    //! PLN-R25, D-08, D-09, D-20, owner ruling R3 and the plan's data S1, Q1
    //! and Q3, never from running this code.

    use std::collections::BTreeSet;

    use baley_core::policy::{AcceptedNames, Host, Schema, effective_policy};
    use baley_core::questions::{SetState, judge_set};
    use baley_store_sqlite::SqliteStore;

    use super::*;
    use crate::questions::read::{SetRead, read as read_set};
    use crate::questions::record::tests::{open_set, q1, story as subject};
    use crate::refine::order::tests::{answer_by_hand, approved, issue_by_hand, issue_checked};
    use crate::refine::order::{check as refine_check, pending_analyzer};
    use crate::scope::fixtures::*;

    pub(crate) const Q: &str = "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e";
    pub(crate) const Q2: &str = "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f";
    pub(crate) const D: &str = "2d3e4f5a-6b7c-4d8e-9f0a-1b2c3d4e5f6a";
    const UNKNOWN: &str = "3e4f5a6b-7c8d-4e9f-8a0b-2c3d4e5f6a7b";

    /// Set Q3, accepted.
    pub(crate) fn q3() -> QuestionSet {
        judge_set(&json!([{"id": "q3", "question": "Who reads the ledger?",
            "recommended": "The owner", "depends_on": []}]))
        .unwrap()
    }

    fn empty() -> QuestionSet {
        judge_set(&json!([])).unwrap()
    }

    /// Issues story `story`'s next analyzer order as `id` through
    /// `story-refine`'s check and issue.
    pub(crate) fn refine(store: &SqliteStore, story: u32, id: &str, request_id: &str, at: &str) {
        let checked = refine_check(store, &project(), story).unwrap();
        let issued = issue_checked(store, &checked, id, request_id, at);
        assert_eq!(issued["status"], "ok", "{issued}");
    }

    /// Submits `set` for story `story` naming `order` through the check and
    /// the submit, as `story-questions-submit` runs them below the session,
    /// with the default policy, which sets nothing for the analyzer, and an
    /// empty catalog; a drafting order is issued as [`D`].
    pub(crate) fn submitted(
        store: &SqliteStore,
        story: u32,
        order: &str,
        set: &QuestionSet,
        request_id: &str,
        at: &str,
    ) -> Value {
        let checked = match check(store, &project(), story, order, set) {
            Ok(checked) => checked,
            Err(refusal) => return refusal,
        };
        let policy =
            effective_policy(Schema::standard(), Some(Host::ClaudeCode), None, None).unwrap();
        let names = AcceptedNames {
            names: BTreeSet::new(),
            version: 0,
        };
        let command = command(QUESTIONS_SUBMIT_COMMAND, request_id, 9, at);
        super::submit(
            store,
            &command,
            set,
            &checked,
            &policy,
            Host::ClaudeCode,
            &names,
            D,
        )
    }

    fn held(store: &SqliteStore, set: &str) -> crate::questions::read::HeldSet {
        match read_set(store, &project(), set).unwrap() {
            SetRead::Held(held) => *held,
            other => panic!("{set}: {other:?}"),
        }
    }

    fn order(store: &SqliteStore, id: &str) -> Value {
        crate::work_order::read::order(store, &project(), id)
            .unwrap()
            .unwrap_or_else(|| panic!("order {id} is held"))
    }

    fn text(value: &Value) -> String {
        value.as_str().expect("text").to_owned()
    }

    #[test]
    fn an_earlier_set_left_standing_or_another_storys_set_superseded_is_caught() {
        let (_dir, store) = approved();
        refine(&store, 1, Q, R1, T1);
        let a = submitted(&store, 1, Q, &q1(), R2, T2);
        assert_eq!(a["status"], "ok", "{a}");
        let a = text(&a["set"]);
        answer_by_hand(&store, &a, "q1", 1, R3);
        answer_by_hand(&store, &a, "q2", 2, R4);
        issue_by_hand(&store, Q2, 1, R5);
        let b = submitted(&store, 1, Q2, &q3(), R6, T6);
        assert_eq!(b["status"], "ok", "{b}");
        assert_eq!(b["now"]["state"], "open");
        assert_eq!(b["now"]["round"], 1);
        let b = text(&b["set"]);

        let events = history(&store);
        let abandoned = of_type(&events, "questions.abandoned");
        assert_eq!(abandoned.len(), 1);
        assert_eq!(
            abandoned[0].payload,
            json!({"set": a, "cause": "superseded", "at": T6})
        );
        assert_eq!(abandoned[0].request_id.0, R6);
        let opened_b = of_type(&events, "questions.opened")
            .into_iter()
            .find(|event| event.payload["set"] == b.as_str())
            .expect("B is opened");
        assert_eq!(opened_b.request_id.0, R6);
        let round = held(&store, &b).round;
        assert_eq!(round.state, SetState::Open);
        assert_eq!(round.number, 1);
        let current: Vec<&str> = round
            .current
            .iter()
            .map(|asked| asked.id.as_str())
            .collect();
        assert_eq!(current, ["q3"]);

        refine(&store, 2, UNKNOWN, R7, T7);
        let c = submitted(&store, 2, UNKNOWN, &empty(), R8, T8);
        assert_eq!(c["status"], "ok", "{c}");
        assert_eq!(
            of_type(&history(&store), "questions.abandoned").len(),
            1,
            "story 2's set abandons nothing of story 1"
        );
        assert_eq!(held(&store, &a).round.state, SetState::Abandoned);
        assert_eq!(held(&store, &b).round.state, SetState::Open);

        let q = order(&store, Q);
        assert_eq!(q["state"], "answered", "{q}");
        assert_eq!(q["drafts"], json!([a]), "{q}");
    }

    #[test]
    fn a_questions_submission_abandoning_an_open_set_is_caught() {
        let (_dir, store) = approved();
        refine(&store, 1, Q, R1, T1);
        let a = submitted(&store, 1, Q, &q1(), R2, T2);
        let a = text(&a["set"]);
        answer_by_hand(&store, &a, "q1", 1, R3);

        let before = head(&store);
        let refusal = submitted(&store, 1, Q, &q3(), R4, T4);
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "question-open");
        assert_eq!(refusal["slot"], "operation");
        assert_eq!(refusal["details"], json!({"set": a, "question": "q2"}));
        assert_eq!(head(&store), before);
        let round = held(&store, &a).round;
        assert_eq!(round.state, SetState::Open);
        assert_eq!(round.abandoned, None);
    }

    #[test]
    fn a_questions_submission_naming_the_wrong_order_recorded_is_caught() {
        let (_dir, store) = approved();
        let set = open_set(&store, R1, &q1(), &subject(1));
        answer_by_hand(&store, &set, "q1", 1, R2);
        answer_by_hand(&store, &set, "q2", 2, R3);
        refine(&store, 1, D, R4, T4);
        assert_eq!(
            order(&store, D)["instructions"][0]["identity"],
            "bal-analyzer-drafting"
        );
        refine(&store, 2, Q2, R5, T5);

        let before = head(&store);
        for (named, code) in [
            (D, "wrong-work-order"),
            (Q2, "wrong-work-order"),
            (UNKNOWN, "no-such-work-order"),
        ] {
            let refusal = submitted(&store, 1, named, &q1(), R6, T6);
            assert_eq!(refusal["status"], "refused", "{named}: {refusal}");
            assert_eq!(refusal["code"], code, "{named}: {refusal}");
            assert_eq!(refusal["slot"], "work_order", "{named}: {refusal}");
        }
        assert_eq!(head(&store), before);
    }

    #[test]
    fn an_empty_set_left_without_its_drafting_order_or_beside_a_live_one_is_caught() {
        use crate::mcp::work_order::tests::fail_order;

        let (_dir, store) = approved();
        refine(&store, 1, Q, R1, T1);
        let first = submitted(&store, 1, Q, &empty(), R2, T2);
        assert_eq!(first["status"], "ok", "{first}");
        assert_eq!(first["now"]["state"], "closed", "{first}");
        assert_eq!(first["drafting"]["work_order"], D, "{first}");
        let drafting = order(&store, D);
        assert_eq!(drafting["state"], "pending", "{drafting}");
        assert_eq!(drafting["subject"], "story/1", "{drafting}");
        assert_eq!(
            drafting["instructions"][0]["identity"], "bal-analyzer-drafting",
            "{drafting}"
        );

        refine(&store, 2, Q2, R3, T3);
        fail_order(&store, &project(), Q2, R4, T4);
        issue_by_hand(&store, UNKNOWN, 2, R5);
        let second = submitted(&store, 2, Q2, &empty(), R6, T6);
        assert_eq!(second["status"], "ok", "{second}");
        assert_eq!(second["now"]["state"], "closed", "{second}");
        assert!(second.get("drafting").is_none(), "{second}");
        assert_eq!(
            pending_analyzer(&store, &project(), 2).unwrap().as_deref(),
            Some(UNKNOWN)
        );
    }
}
