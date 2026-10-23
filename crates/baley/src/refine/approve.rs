//! The owner's approval of a truth draft by its digest (design 0005 section
//! 5, PLN-R4, PLN-R15, PLN-R26, PRJ-R14 to PRJ-R16), in two steps that only
//! `baley story truths approve` calls: no truth approval is served over MCP
//! (D-06, ADR 0044).
//!
//! The first step reads and judges outside any transaction, in D-17's
//! order, so a refusal records nothing at all, and gives each truth the
//! version it gets against the story's current truths (D-02, D-03). The
//! second step records one `story.refined` as the owner with the story's
//! whole truth list, confirming inside its transaction that nothing the
//! first step read has moved. A truth revision is never recorded as
//! `scope.approved` (design 0004).

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use baley_core::scope::{
    DRAFT_VIEW, PROJECT_STREAM, PriorApproval, ROADMAP_STREAM, SCOPE_SUBMITTED, draft_key,
};
use baley_core::truth::{
    STORY_REFINED, STORY_REFINED_VERSION, STORY_TRUTHS, Truth, TruthApprovalFacts,
    TruthApprovalRefusal, TruthSubmission, judge_truth_approval, judge_truths, refined_payload,
    versions,
};
use baley_store::{
    Absence, Actor, Command, Decision, Document, EVENT_PAGE_BOUND, EventMatch, Hash, HistoryFilter,
    Ledger, NewEvent, Observed, ObservedDocument, OutcomeKind, PageRequest, Payloads, ProjectId,
    Recorded, StaleInput, StoreError, StreamName, Transaction, Views,
};
use serde_json::{Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::refine::order::{backlog_read, held_story, open_questions, phase_dispatch};
use crate::refine::submit::current_truths;
use crate::scope::approve::{newest_query, submitted_at};
use crate::scope::read::{Preview, PreviewError, preview, preview_of};
use crate::scope::submit::NOT_RECORDED_BUSY;

/// What the first step read and judged, which the second step confirms and
/// records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judged {
    /// The approved draft's digest.
    digest: Hash,
    /// The sequence of its latest submission.
    seq: u64,
    /// The newest `story-truths` draft of its story, digest and sequence.
    newest: (Hash, u64),
    /// Whether a `story.refined` naming the digest was recorded before its
    /// latest submission.
    approved_before: bool,
    /// The story.
    story: u32,
    /// What its drafts are filed under, `story/<id>`.
    target: String,
    /// The question set the truths were drafted from.
    set: String,
    /// Every truth of the draft with the version approval gives it.
    truths: Vec<(Truth, u32)>,
    /// The `backlog`, latest set, committed phase and `truths` documents, at
    /// the sequence that produced each.
    documents: Vec<ObservedDocument>,
}

/// One recorded approval of a digest.
struct Approval {
    seq: u64,
    owner: String,
    at: String,
}

/// The first step: reads the draft with `digest` (none, or one that is not a
/// story's truth draft, is `unknown-draft`) and judges its body again, a body
/// that no longer judges being a ledger fault; then its `scope.submitted` at
/// its sequence, the newest truth draft of its story, every earlier
/// `story.refined` naming it, the story, its latest set and that set's open
/// questions, whether a worker is dispatched on the phase it is committed to,
/// and its current truths. It judges in D-17's order, each refusal at slot
/// `digest`, and gives the truths with their versions against the current
/// truths now (D-02, D-03). It runs outside any transaction, since a
/// transaction has no payload read. Nothing is recorded.
pub fn judge<S: Ledger + Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    digest: Hash,
) -> Result<Judged, Value> {
    let busy = |error: StoreError| store_failed(&error, NOT_RECORDED_BUSY);
    let (submission, seq) = match preview(store, project, digest) {
        Ok(read) if read.kind == STORY_TRUTHS => accept(&read)?,
        Ok(_) | Err(PreviewError::UnknownDraft) => {
            return Err(refused(&TruthApprovalRefusal::UnknownDraft));
        }
        Err(error) => return Err(preview_failed(error)),
    };
    let recorded = submitted_at(store, project, digest, seq).map_err(busy)?;
    let newest = match store
        .find(
            project,
            DRAFT_VIEW,
            &newest_query(STORY_TRUTHS, submission.target()),
        )
        .map_err(busy)?
        .items
        .into_iter()
        .next()
    {
        Some(document) => Some(newest_of(store, &document)?),
        None => None,
    };
    let approvals = approvals_of(store, project, digest).map_err(busy)?;
    let prior = approvals.iter().find(|approval| approval.seq > seq);
    let story = submission.story();
    let (held, _) = held_story(store, project, story)?;
    let mut documents = Vec::new();
    let mut latest = None;
    let mut open = Vec::new();
    let mut dispatch_active = false;
    // A story the roadmap no longer holds has no backlog entry to read.
    if let Some(entry) = &held {
        let read = backlog_read(store, project, story)?;
        documents.extend(read.documents);
        if let Some((id, set)) = &read.latest {
            latest = Some(id.clone());
            open = open_questions(&set.facts)
                .into_iter()
                .map(str::to_owned)
                .collect();
        }
        if let Some(phase) = entry.phase {
            let (active, document) = phase_dispatch(store, project, phase)?;
            dispatch_active = active;
            documents.push(document);
        }
    }
    let current = current_truths(store, project, story)?;
    documents.push(current.document.clone());
    let facts = TruthApprovalFacts {
        target: Some(&submission),
        recorded,
        newest: newest.as_ref().map(|(newest, _)| newest),
        approved: prior.map(|approval| PriorApproval {
            owner: &approval.owner,
            at: &approval.at,
        }),
        story_held: held.is_some(),
        latest_set: latest.as_deref(),
        open_questions: &open,
        dispatch_active,
    };
    judge_truth_approval(&facts).map_err(|refusal| refused(&refusal))?;
    // An approved judgement had a newest draft.
    let (newest, newest_seq) =
        newest.ok_or_else(|| refused(&TruthApprovalRefusal::UnknownDraft))?;
    let given = versions(submission.truths(), &current.truths, &current.kept).map_err(|_| {
        failed(
            LEDGER_UNAVAILABLE,
            "a truth of this story has no version number left, so nothing was approved",
            PLACE_LEDGER,
        )
    })?;
    let truths = submission
        .truths()
        .iter()
        .cloned()
        .zip(given.truths.iter().map(|truth| truth.version))
        .collect();
    Ok(Judged {
        digest,
        seq,
        newest: (newest.digest(), newest_seq),
        approved_before: approvals.iter().any(|approval| approval.seq < seq),
        story,
        target: submission.target().into(),
        set: submission.set().into(),
        truths,
        documents,
    })
}

/// The second step: records the approval judged by the first step under a
/// copy of `command` whose actor is the owner, as `scope::approve::record`
/// does, with `owner` as the owner's name and the command's time as Baley's
/// (D-06). Before it appends anything the decision confirms that the draft
/// is at the sequence read with its `scope.submitted`, that the newest truth
/// draft of the story is the one read, and that no `story.refined` naming the
/// digest landed since; it observes the `backlog`, latest set, committed
/// phase and `truths` documents the first step read, so anything moved since
/// makes the call `ledger-busy` with nothing recorded (D-17). Then it appends
/// one `story.refined` on `roadmap` with the story, the set, the digest, the
/// owner, the time and the story's whole truth list with versions (D-04). No
/// `scope.approved` is appended. A retry of a request the store holds is
/// answered from its record before any decision runs; a decision asked to run
/// without the first step's reads records nothing.
///
/// The receipt: `status`, `kind`, `digest`, `story`, `set`, `owner`, `at` and
/// `truths`, each truth's `{id, version}`.
pub fn record<L: Ledger + ?Sized>(
    store: &L,
    command: &Command,
    judged: Option<&Judged>,
    owner: &str,
) -> Result<Recorded, StoreError> {
    let command = Command {
        actor: Actor::Owner,
        ..command.clone()
    };
    let at = command.recorded_at.clone();
    store.transact(&command, &mut |tx| {
        let Some(judged) = judged else {
            return Err(StoreError::Unavailable(
                "an approval reached its transaction without its first step's reads".into(),
            ));
        };
        confirm_draft(tx, judged)?;
        // A transaction cannot ask for an approval after a sequence, so when
        // one preceded the latest submission a second is left to the
        // `truths` document, which every `story.refined` of the story moves.
        let approved = approval_of(judged.digest);
        if !judged.approved_before && tx.event_exists(&approved)? {
            return Err(StoreError::Stale(StaleInput::Absence(Absence::Event(
                approved,
            ))));
        }
        confirm_newest(tx, judged)?;
        tx.append(NewEvent {
            stream: StreamName(ROADMAP_STREAM.into()),
            type_name: STORY_REFINED.into(),
            type_version: STORY_REFINED_VERSION,
            git: None,
            payload: refined_payload(
                judged.story,
                &judged.set,
                judged.digest,
                owner,
                &at,
                &judged.truths,
            ),
            attachments: Vec::new(),
        })?;
        let truths: Vec<Value> = judged
            .truths
            .iter()
            .map(|(truth, version)| json!({"id": truth.id, "version": version}))
            .collect();
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: json!({
                "status": "ok",
                "kind": STORY_TRUTHS,
                "digest": judged.digest.to_hex(),
                "story": judged.story,
                "set": judged.set,
                "owner": owner,
                "at": at,
                "truths": truths,
            }),
            sensitive: false,
            observed: Observed {
                documents: judged.documents.clone(),
                ..Observed::default()
            },
            git: None,
        })
    })
}

/// A recorded truth draft's body judged again. One that no longer judges
/// clean, or whose digest is not the draft's, is a ledger fault.
fn accept(read: &Preview) -> Result<(TruthSubmission, u64), Value> {
    let body = &read.submission;
    let story = body
        .get("story")
        .and_then(Value::as_u64)
        .and_then(|story| u32::try_from(story).ok())
        .and_then(NonZeroU32::new);
    let set = body.get("set").and_then(Value::as_str);
    let judged = match (story, set, body.get("truths")) {
        (Some(story), Some(set), Some(truths)) => judge_truths(story, set, truths).ok(),
        _ => None,
    };
    match judged {
        Some(submission) if submission.digest() == read.digest => Ok((submission, read.seq)),
        _ => Err(failed(
            LEDGER_UNAVAILABLE,
            "the draft is recorded, but its body no longer judges as the truths it was",
            PLACE_LEDGER,
        )),
    }
}

/// The newest truth draft's accepted submission and sequence.
fn newest_of<P: Payloads + ?Sized>(
    payloads: &P,
    document: &Document,
) -> Result<(TruthSubmission, u64), Value> {
    accept(&preview_of(payloads, document).map_err(preview_failed)?)
}

fn preview_failed(error: PreviewError) -> Value {
    match error {
        PreviewError::Store(error) => store_failed(&error, NOT_RECORDED_BUSY),
        PreviewError::UnknownDraft | PreviewError::Unreadable(_) => failed(
            LEDGER_UNAVAILABLE,
            "the draft is recorded, but its document or body cannot be read",
            PLACE_LEDGER,
        ),
    }
}

/// Every `story.refined` on `roadmap` naming `digest` as its submission,
/// oldest first, read from the events.
fn approvals_of<L: Ledger + ?Sized>(
    store: &L,
    project: &ProjectId,
    digest: Hash,
) -> Result<Vec<Approval>, StoreError> {
    let filter = HistoryFilter {
        types: vec![STORY_REFINED.into()],
        git_commit: None,
    };
    let digest = json!(digest.to_hex());
    let text = |value: &Value, name: &str| {
        value
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let mut approvals = Vec::new();
    let mut after = None;
    loop {
        let page = store.history(
            project,
            1..=u64::MAX,
            &filter,
            PageRequest {
                limit: EVENT_PAGE_BOUND,
                after,
            },
        )?;
        approvals.extend(
            page.items
                .iter()
                .filter(|event| {
                    event.stream == ROADMAP_STREAM
                        && event.payload.get("submission") == Some(&digest)
                })
                .map(|event| Approval {
                    seq: event.seq,
                    owner: text(&event.payload, "owner"),
                    at: text(&event.payload, "at"),
                }),
        );
        match page.next {
            Some(next) => after = Some(next),
            None => return Ok(approvals),
        }
    }
}

/// A `story.refined` naming `digest`.
fn approval_of(digest: Hash) -> EventMatch {
    EventMatch {
        type_name: STORY_REFINED.into(),
        stream: Some(StreamName(ROADMAP_STREAM.into())),
        fields: BTreeMap::from([("submission".to_string(), json!(digest.to_hex()))]),
    }
}

/// The draft is at the sequence read and its `scope.submitted` is there.
fn confirm_draft(tx: &mut dyn Transaction, judged: &Judged) -> Result<(), StoreError> {
    let key = draft_key(judged.digest);
    let now = tx.get(DRAFT_VIEW, &key)?.as_ref().and_then(sequence);
    let submitted = EventMatch {
        type_name: SCOPE_SUBMITTED.into(),
        stream: Some(StreamName(PROJECT_STREAM.into())),
        fields: BTreeMap::from([("digest".to_string(), json!(judged.digest.to_hex()))]),
    };
    if now != Some(judged.seq) || !tx.event_exists(&submitted)? {
        return Err(StoreError::Stale(StaleInput::Document {
            view: DRAFT_VIEW.into(),
            key,
            seen: Some(judged.seq),
            now,
        }));
    }
    Ok(())
}

/// The newest truth draft of the story is the one the first step read, by
/// digest and sequence, never by time: two submissions can share a time.
fn confirm_newest(tx: &mut dyn Transaction, judged: &Judged) -> Result<(), StoreError> {
    let newest = tx
        .find(DRAFT_VIEW, &newest_query(STORY_TRUTHS, &judged.target))?
        .items
        .into_iter()
        .next();
    let now = newest.as_ref().and_then(|document| {
        let digest = document
            .body
            .get("digest")
            .and_then(Value::as_str)
            .and_then(Hash::from_hex)?;
        Some((digest, sequence(document)?))
    });
    if now != Some(judged.newest) {
        return Err(StoreError::Stale(StaleInput::Document {
            view: DRAFT_VIEW.into(),
            key: draft_key(judged.newest.0),
            seen: Some(judged.newest.1),
            now: now.map(|(_, seq)| seq),
        }));
    }
    Ok(())
}

/// The latest submission's sequence a `draft` document holds.
fn sequence(document: &Document) -> Option<u64> {
    document.body.get("seq").and_then(Value::as_u64)
}

/// The `refused` answer for a truth approval, slot `digest`, with what the
/// refusal names in its details.
fn refused(refusal: &TruthApprovalRefusal) -> Value {
    let reason = match refusal {
        TruthApprovalRefusal::UnknownDraft => {
            "no story's truth draft has this digest, so nothing was approved"
        }
        TruthApprovalRefusal::StaleDraft { .. } => {
            "a newer truth draft of the same story was submitted after this one, so it was not approved. The details name the first part that differs"
        }
        TruthApprovalRefusal::DraftApproved { .. } => {
            "this draft was already approved after its latest submission, so it was not approved again. The details name that approval"
        }
        TruthApprovalRefusal::NoSuchStory => {
            "the draft's story is not on the roadmap or is dropped, so nothing was approved"
        }
        TruthApprovalRefusal::SetAbandoned => {
            "the draft was drafted from a question set that is no longer the story's latest, so it was not approved. A draft from the latest set can be"
        }
        TruthApprovalRefusal::QuestionOpen { .. } => {
            "a question of the story's latest set is open, so the truths were not approved. The details name each open question"
        }
        TruthApprovalRefusal::DispatchActive => {
            "a worker is dispatched on the phase the story is committed to, so the truths were not approved"
        }
    };
    let answer = Refusal::new(refusal.code(), reason).slot("digest");
    match refusal.details() {
        Some(details) => answer.details(details),
        None => answer,
    }
    .value()
}

#[cfg(test)]
mod tests {
    //! Stores are real SQLite stores in fresh temporary directories, with
    //! times supplied; expected answers are written out from design 0005,
    //! design 0004, D-02, D-04, D-06, D-11, D-17, D-18, D-21 and the plan's
    //! data S1, Q1, Q3, T1 and T2, never from running this code.

    use baley_core::scope::{ChangeKind, ROADMAP_STREAM};
    use baley_store::{Actor, Event, NewEvent, RequestId, StreamName};
    use baley_store_sqlite::SqliteStore;
    use serde_json::json;

    use super::*;
    use crate::questions::answer::ANSWER_COMMAND;
    use crate::questions::answer::tests::{ADA, D1, relay, run, take};
    use crate::questions::record::tests::q1;
    use crate::refine::order::check as refine_check;
    use crate::refine::order::tests::{approved, input_body, input_names, issue_checked};
    use crate::refine::questions::tests::{D, Q, q3, submitted};
    use crate::refine::submit::tests::{t1, t2, truths_submitted};
    use crate::scope::command::approve_command;
    use crate::scope::fixtures::*;

    /// A request id of its own for each step of a flow.
    fn rid(n: u8) -> String {
        format!("00000000-0000-4000-8000-{n:012x}")
    }

    /// Approves the draft with digest `digest` through the two steps under
    /// the command `baley story truths approve` builds, as [`ADA`] at `at`.
    fn approve(store: &SqliteStore, digest: &str, request: &str, at: &str) -> Value {
        let digest = Hash::from_hex(digest).expect("a digest");
        match judge(store, &project(), digest) {
            Ok(judged) => {
                let command =
                    approve_command(&project(), 0, digest, RequestId(request.into()), at).unwrap();
                crate::scope::submit::answer(record(store, &command, Some(&judged), ADA), store)
            }
            Err(refusal) => refusal,
        }
    }

    /// Takes `question` of `set` at the terminal: the check and the
    /// recording with no drafting order, under an owner's command with no
    /// caller.
    fn take_at_terminal(store: &SqliteStore, set: &str, question: &str, request: &str, at: &str) {
        let mut command = command(ANSWER_COMMAND, request, 4, at);
        command.caller = None;
        command.actor = Actor::Owner;
        let answer = run(store, set, &json!([take(question)]), &command, ADA);
        assert_eq!(answer["status"], "ok", "{answer}");
    }

    fn text(value: &Value) -> String {
        value.as_str().expect("text").to_owned()
    }

    fn after(store: &SqliteStore, seq: u64) -> Vec<Event> {
        history(store)
            .into_iter()
            .filter(|event| event.seq > seq && event.type_name != "command.completed")
            .collect()
    }

    fn refused(answer: &Value, code: &str) {
        assert_eq!(answer["status"], "refused", "{answer}");
        assert_eq!(answer["code"], code, "{answer}");
        assert_eq!(answer["slot"], "digest", "{answer}");
    }

    #[test]
    fn a_truth_approval_refused_too_late_recorded_on_a_refusal_or_served_over_mcp_is_caught() {
        use crate::mcp::operations::{APPLY_OPERATIONS, QUERY_OPERATIONS};
        use crate::scope::read::{Draft, draft};

        let (dir, store) = approved();
        let checked = refine_check(&store, &project(), 1).unwrap();
        assert_eq!(
            issue_checked(&store, &checked, Q, &rid(1), T1)["status"],
            "ok"
        );
        let s_a = text(&submitted(&store, 1, Q, &q1(), &rid(2), T2)["set"]);
        relay(&store, &s_a, &json!([take("q1")]), &rid(3), T3);
        let close = relay(&store, &s_a, &json!([take("q2")]), &rid(4), T3);
        assert_eq!(close["drafting"]["work_order"], D1, "{close}");
        let x_a = truths_submitted(&store, 1, &s_a, &json!([t1(), t2()]), D1, &rid(5), T4);
        assert_eq!(x_a["status"], "ok", "{x_a}");
        let x_a = text(&x_a["digest"]);
        let s_b = submitted(&store, 1, Q, &q3(), &rid(6), T4);
        assert_eq!(s_b["status"], "ok", "{s_b}");
        let s_b = text(&s_b["set"]);

        let before = head(&store);
        refused(
            &approve(&store, &x_a, &rid(7), T5),
            "question-set-abandoned",
        );
        assert_eq!(head(&store), before);

        let x_b = truths_submitted(&store, 1, &s_b, &json!([t1(), t2()]), D1, &rid(8), T5);
        assert_eq!(x_b["status"], "ok", "{x_b}");
        let x_b = text(&x_b["digest"]);
        drop(store);
        let store = crate::ledger::open::store(
            &dir.path().join("home"),
            T6,
            crate::ledger::open::options(),
        )
        .unwrap();
        let Draft::Held(preview) =
            draft(&store, &project(), Hash::from_hex(&x_b).unwrap()).unwrap()
        else {
            panic!("the draft is held");
        };
        assert_eq!(preview.kind, "story-truths");
        assert_eq!(preview.submission["set"], s_b.as_str());

        let before = head(&store);
        let open = approve(&store, &x_b, &rid(9), T6);
        refused(&open, "question-open");
        assert_eq!(open["details"], json!({"questions": ["q3"]}));
        assert_eq!(head(&store), before);

        take_at_terminal(&store, &s_b, "q3", &rid(10), T6);
        let approvals_before = of_type(&history(&store), "scope.approved").len();
        let before = head(&store);
        let approved = approve(&store, &x_b, &rid(11), T7);
        assert_eq!(approved["status"], "ok", "{approved}");
        let new = after(&store, before);
        assert_eq!(new.len(), 1, "{new:?}");
        assert_eq!(new[0].type_name, "story.refined");
        assert_eq!(new[0].stream, "roadmap");
        assert_eq!(new[0].actor, Actor::Owner);
        assert_eq!(new[0].caller, None);
        assert_eq!(new[0].payload["owner"], ADA);
        assert_eq!(new[0].payload["at"], T7);
        assert_eq!(
            of_type(&history(&store), "scope.approved").len(),
            approvals_before
        );

        for operation in APPLY_OPERATIONS.iter().chain(QUERY_OPERATIONS) {
            assert!(!operation.name.contains("approve"), "{}", operation.name);
        }
    }

    #[test]
    fn a_refinement_record_missing_its_set_digest_or_truths_or_an_input_read_late_is_caught() {
        use crate::scope::read::{backlog, roadmap};

        let (_dir, store) = approved();
        let states = |store: &SqliteStore| -> Vec<(String, Option<u32>, u64)> {
            backlog(store, &project())
                .unwrap()
                .into_iter()
                .filter(|story| story.id <= 2)
                .map(|story| (story.refinement, story.round, story.truths))
                .collect()
        };
        let state = |name: &str, round: Option<u32>, truths: u64| (name.to_owned(), round, truths);
        let unrefined = state("unrefined", None, 0);
        assert_eq!(states(&store), [unrefined.clone(), unrefined.clone()]);

        let checked = refine_check(&store, &project(), 1).unwrap();
        assert_eq!(
            issue_checked(&store, &checked, Q, &rid(1), T1)["status"],
            "ok"
        );
        let s_a = text(&submitted(&store, 1, Q, &q1(), &rid(2), T2)["set"]);
        assert_eq!(
            states(&store),
            [state("questions-open", Some(1), 0), unrefined.clone()]
        );
        relay(&store, &s_a, &json!([take("q1")]), &rid(3), T3);
        relay(&store, &s_a, &json!([take("q2")]), &rid(4), T3);
        assert_eq!(
            states(&store),
            [state("drafting", None, 0), unrefined.clone()]
        );

        let describe = receipt(
            &submit_change(&store, ChangeKind::ProjectDescribe, &d1(), &rid(5), T4).unwrap(),
        )["digest"]
            .as_str()
            .and_then(Hash::from_hex)
            .unwrap();
        let judged = crate::scope::approve::judge(&store, &project(), describe).unwrap();
        crate::scope::approve::record(
            &store,
            &command(crate::scope::approve::APPROVE_COMMAND, &rid(6), 6, T4),
            Some(&judged),
            ADA,
        )
        .unwrap();

        let x_a = truths_submitted(&store, 1, &s_a, &json!([t1(), t2()]), D1, &rid(7), T4);
        assert_eq!(x_a["status"], "ok", "{x_a}");
        let s_b = text(&submitted(&store, 1, Q, &q3(), &rid(8), T5)["set"]);
        assert_eq!(
            states(&store),
            [state("questions-open", Some(1), 0), unrefined.clone()]
        );
        let x_b = truths_submitted(&store, 1, &s_b, &json!([t1(), t2()]), D1, &rid(9), T5);
        let x_b = text(&x_b["digest"]);
        take_at_terminal(&store, &s_b, "q3", &rid(10), T6);
        let before = head(&store);
        let approved = approve(&store, &x_b, &rid(11), T7);
        assert_eq!(approved["status"], "ok", "{approved}");
        assert_eq!(
            states(&store),
            [state("refined", None, 2), unrefined.clone()]
        );

        let refined = after(&store, before);
        let versioned = |truth: Value| {
            let mut truth = truth;
            truth["version"] = json!(1);
            truth
        };
        assert_eq!(
            refined[0].payload,
            json!({"story": 1, "set": s_b, "submission": x_b, "owner": ADA, "at": T7,
                "truths": [versioned(t1()), versioned(t2())]})
        );

        let events = history(&store);
        let issued = of_type(&events, "work_order.issued");
        let drafting = issued
            .iter()
            .find(|event| event.payload["id"] == D1)
            .expect("D is issued");
        assert_eq!(
            input_body(&store, drafting, "description"),
            json!({"what": "A ledger for agent work",
                "core_value": "Nothing moves on an unproven claim",
                "constraints": ["One binary"]})
        );

        const NEXT: &str = "8b9c0d1e-2f3a-4b4c-9d5e-7f8a9b0c1d2e";
        let checked = refine_check(&store, &project(), 1).unwrap();
        assert_eq!(
            issue_checked(&store, &checked, NEXT, &rid(12), T8)["status"],
            "ok"
        );
        let events = history(&store);
        let next = of_type(&events, "work_order.issued")
            .into_iter()
            .find(|event| event.payload["id"] == NEXT)
            .expect("the next order is issued");
        assert_eq!(
            input_names(next),
            ["story", "description", "roadmap", "truths"]
        );
        assert_eq!(input_body(&store, next, "description"), d1());
        assert_eq!(
            input_body(&store, next, "roadmap"),
            serde_json::to_value(roadmap(&store, &project()).unwrap()).unwrap()
        );
        assert_eq!(
            input_body(&store, next, "truths"),
            json!([versioned(t1()), versioned(t2())])
        );
    }

    /// Appends a `story.refined` for story 1 from set `set` by hand, naming
    /// `submission`, with `truths` as written, under request `n`.
    fn refined_by_hand(
        store: &SqliteStore,
        n: u8,
        set: &str,
        submission: &str,
        truths: Value,
        at: &str,
    ) {
        use baley_core::truth::{STORY_REFINED, STORY_REFINED_VERSION};

        append(
            store,
            &rid(n),
            at,
            vec![NewEvent {
                stream: StreamName(ROADMAP_STREAM.into()),
                type_name: STORY_REFINED.into(),
                type_version: STORY_REFINED_VERSION,
                git: None,
                payload: json!({"story": 1, "set": set, "submission": submission,
                    "owner": ADA, "at": at, "truths": truths}),
                attachments: Vec::new(),
            }],
        );
    }

    fn at_version(truth: Value, version: u32) -> Value {
        let mut truth = truth;
        truth["version"] = json!(version);
        truth
    }

    #[test]
    fn a_stale_or_repeated_truth_approval_recorded_is_caught() {
        use crate::refine::submit::tests::{drafting, with};

        let (_dir, store, a) = drafting();
        let x1 = truths_submitted(&store, 1, &a, &json!([t1(), t2()]), D, &rid(1), T4);
        let x2 = truths_submitted(
            &store,
            1,
            &a,
            &json!([t1(), with(t2(), "outcome", "a later round")]),
            D,
            &rid(2),
            T5,
        );
        let (x1, x2) = (text(&x1["digest"]), text(&x2["digest"]));

        let before = head(&store);
        let stale = approve(&store, &x1, &rid(3), T6);
        refused(&stale, "stale-draft");
        assert_eq!(stale["details"], json!({"path": "truths[1].outcome"}));
        assert_eq!(head(&store), before);

        let first = approve(&store, &x2, &rid(4), T7);
        assert_eq!(first["status"], "ok", "{first}");
        let before = head(&store);
        let again = approve(&store, &x2, &rid(5), T8);
        refused(&again, "draft-approved");
        assert_eq!(again["details"], json!({"owner": ADA, "at": T7}));
        assert_eq!(head(&store), before);
    }

    #[test]
    fn a_truth_approval_over_a_moved_record_is_caught() {
        use crate::questions::record::tests::{open_set, story};
        use crate::refine::submit::tests::{drafting, with};

        // One change to the ledger between the two steps.
        type Move = dyn Fn(&SqliteStore, &str);
        let moves: [&Move; 3] = [
            &|store, set| {
                refined_by_hand(
                    store,
                    200,
                    set,
                    &"5c".repeat(32),
                    json!([at_version(t1(), 1)]),
                    T5,
                );
            },
            &|store, _| {
                open_set(store, &rid(100), &q3(), &story(1));
            },
            &|store, set| {
                let newer = json!([t1(), with(t2(), "outcome", "a later round")]);
                let receipt = truths_submitted(store, 1, set, &newer, D, &rid(101), T5);
                assert_eq!(receipt["status"], "ok", "{receipt}");
            },
        ];
        for (case, moved) in moves.iter().enumerate() {
            let (_dir, store, a) = drafting();
            let x1 = truths_submitted(&store, 1, &a, &json!([t1(), t2()]), D, &rid(1), T4);
            let digest = Hash::from_hex(&text(&x1["digest"])).unwrap();
            let judged = judge(&store, &project(), digest).expect("X1 is approvable");
            moved(&store, &a);
            let before = head(&store);
            let command = approve_command(&project(), 0, digest, RequestId(rid(2)), T6).unwrap();
            let answer =
                crate::scope::submit::answer(record(&store, &command, Some(&judged), ADA), &store);
            assert_eq!(answer["status"], "failed", "{case}: {answer}");
            assert_eq!(answer["code"], "ledger-busy", "{case}: {answer}");
            assert_eq!(head(&store), before, "{case}");
        }
    }

    #[test]
    fn a_scope_digest_approved_as_truths_is_caught() {
        let (_dir, store) = approved();
        let edit = receipt(
            &submit_change(
                &store,
                ChangeKind::PhaseEdit,
                &json!({"phase": 1, "goal": "Every step is recorded"}),
                &rid(1),
                T4,
            )
            .unwrap(),
        )["digest"]
            .as_str()
            .unwrap()
            .to_owned();
        let before = head(&store);
        refused(&approve(&store, &edit, &rid(2), T5), "unknown-draft");
        assert_eq!(head(&store), before);
    }

    #[test]
    fn versions_assigned_at_submit_or_ignoring_the_record_are_caught() {
        use crate::refine::submit::tests::{drafting, with};

        let (_dir, store, a) = drafting();
        refined_by_hand(
            &store,
            200,
            &a,
            &"5c".repeat(32),
            json!([at_version(t1(), 1), at_version(t2(), 1)]),
            T4,
        );
        let x = truths_submitted(
            &store,
            1,
            &a,
            &json!([t1(), with(t2(), "outcome", "a later round")]),
            D,
            &rid(1),
            T5,
        );
        assert_eq!(
            x["marks"],
            json!([{"id": "t1", "mark": "unchanged"}, {"id": "t2", "mark": "changed"}])
        );
        refined_by_hand(
            &store,
            201,
            &a,
            &"6d".repeat(32),
            json!([
                at_version(t1(), 1),
                at_version(
                    with(t2(), "outcome", "the same round and the same waiting list"),
                    2
                )
            ]),
            T6,
        );
        let before = head(&store);
        let approved = approve(&store, &text(&x["digest"]), &rid(2), T7);
        assert_eq!(approved["status"], "ok", "{approved}");
        let refined = after(&store, before);
        let versions: Vec<(&str, u64)> = refined[0].payload["truths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|truth| {
                (
                    truth["id"].as_str().unwrap(),
                    truth["version"].as_u64().unwrap(),
                )
            })
            .collect();
        assert_eq!(versions, [("t1", 1), ("t2", 3)]);
        assert_eq!(
            approved["truths"],
            json!([{"id": "t1", "version": 1}, {"id": "t2", "version": 3}])
        );
    }
}
