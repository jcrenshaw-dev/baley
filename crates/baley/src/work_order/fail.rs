//! The recording of one order's failure (design 0012, decision 2): one
//! `work_order.failed` on the stream the order was issued on, in one domain
//! transaction under the command its caller prepared.
//!
//! A session reports a failure; nothing infers one from silence. The caller
//! checks the order is pending first and keeps what it read, and the
//! decision observes the order's document at that read, so a submission or
//! another failure landing since makes the store refuse the command as stale
//! with nothing recorded.

use baley_core::work_order::{
    FAILED, WORK_ORDER_FAILED, WORK_ORDER_FAILED_VERSION, WORK_ORDER_VIEW, failed_payload,
    order_stream, work_order_key,
};
use baley_store::{
    Command, Decision, Ledger, NewEvent, Observed, ObservedDocument, OutcomeKind, Recorded,
    StoreError, StreamName,
};
use serde_json::json;

/// What a failure's check read of its pending order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// The order's id.
    pub id: String,
    /// What the order is about, which names the stream it was issued on.
    pub subject: String,
    /// The sequence that last produced the order's document.
    pub produced_seq: u64,
}

/// The sentence a failure's receipt carries, naming the next step.
pub const NEXT: &str =
    "Issue the next attempt of this order with `work-order-retry`, naming this order's id.";

/// Records the failure of the order `pending` names, with `reason`, under
/// `command`. A request the store already holds is replayed from its record
/// before any decision runs; a decision asked to run without the check's
/// read records nothing. The store's result comes back unchanged.
pub fn record<L: Ledger + ?Sized>(
    store: &L,
    command: &Command,
    reason: &str,
    pending: Option<&Pending>,
) -> Result<Recorded, StoreError> {
    store.transact(command, &mut |tx| {
        let Some(pending) = pending else {
            return Err(StoreError::Unavailable(
                "a work order's failure reached its transaction without its check's read".into(),
            ));
        };
        let Some(stream) = order_stream(&pending.subject) else {
            return Err(StoreError::Unavailable(format!(
                "no stream records orders about {}",
                pending.subject
            )));
        };
        tx.append(NewEvent {
            stream: StreamName(stream.into()),
            type_name: WORK_ORDER_FAILED.into(),
            type_version: WORK_ORDER_FAILED_VERSION,
            git: None,
            payload: failed_payload(&pending.id, reason),
            attachments: Vec::new(),
        })?;
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: json!({"status": "ok", "work_order": pending.id, "state": FAILED,
                "next": NEXT, "recorded_at": command.recorded_at}),
            sensitive: false,
            observed: Observed {
                documents: vec![ObservedDocument {
                    view: WORK_ORDER_VIEW.into(),
                    key: work_order_key(&pending.id),
                    produced_seq: Some(pending.produced_seq),
                }],
                ..Observed::default()
            },
            git: None,
        })
    })
}

#[cfg(test)]
mod tests {
    //! Orders are issued through project start's recording over the real
    //! store, and checked through the failure's own check.

    use baley_core::scope::judge_submission;
    use baley_store::{Caller, Ledger, PageRequest, ProjectId, StreamName};
    use baley_store_sqlite::SqliteStore;
    use serde_json::{Value, json};

    use super::*;
    use crate::mcp::work_order::fail_check;
    use crate::mcp::work_order::tests::{I0, issue_start};
    use crate::scope::fixtures::*;
    use crate::work_order::issue;
    use crate::work_order::read::order;

    const I1: &str = "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e";
    const I2: &str = "7f3e4d5c-6071-4c8d-8e9f-1a2b3c4d5e6f";
    const REASON: &str = "the worker exited without a return";

    fn fail_in(project: &ProjectId, request_id: &str, at: &str) -> Command {
        command_in(project, "work_order.fail", request_id, 4, at)
    }

    fn head_of(store: &SqliteStore, project: &ProjectId) -> u64 {
        store.head(project).unwrap().map_or(0, |head| head.seq)
    }

    fn roadmap(store: &SqliteStore, project: &ProjectId) -> Vec<baley_store::Event> {
        let page = PageRequest {
            limit: 100,
            after: None,
        };
        store
            .stream(project, &StreamName("roadmap".into()), 1, page)
            .unwrap()
            .items
    }

    #[test]
    fn a_failure_recorded_off_its_stream_or_over_a_moved_order_is_caught() {
        let (_dir, store) = two_projects();
        issue_start(&store, &project(), I0, R1, T1);
        let kept = fail_check(&store, &project(), I0).unwrap();
        let command = fail_in(&project(), R2, T2);
        let answer = issue::answer(record(&store, &command, REASON, Some(&kept)), &store);
        let next: Value = json!(NEXT);
        assert!(NEXT.contains("work-order-retry"), "{NEXT}");
        assert_eq!(
            answer,
            json!({"status": "ok", "work_order": I0, "state": "failed", "next": next,
                "recorded_at": T2})
        );
        let failures = |store: &SqliteStore| -> Vec<baley_store::Event> {
            roadmap(store, &project())
                .into_iter()
                .filter(|event| event.type_name == "work_order.failed")
                .collect()
        };
        let recorded = failures(&store);
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].type_version, 1);
        assert_eq!(recorded[0].payload, json!({"id": I0, "reason": REASON}));
        assert_eq!(
            order(&store, &project(), I0).unwrap().unwrap()["state"],
            "failed"
        );

        let again = issue::answer(record(&store, &command, REASON, Some(&kept)), &store);
        assert_eq!(again, answer);
        assert_eq!(failures(&store).len(), 1);

        issue_start(&store, &project_b(), I1, R1, T1);
        let kept = fail_check(&store, &project_b(), I1).unwrap();
        let mut submit = command_in(
            &project_b(),
            crate::scope::submit::SUBMIT_COMMAND,
            R2,
            2,
            T2,
        );
        submit.caller = Some(Caller::Server(caller().with_work_order(I1).unwrap()));
        crate::scope::submit::record(&store, &submit, &judge_submission(&m()).unwrap()).unwrap();
        let before = head_of(&store, &project_b());
        let stale = issue::answer(
            record(&store, &fail_in(&project_b(), R3, T3), REASON, Some(&kept)),
            &store,
        );
        assert_eq!(stale["status"], "failed", "{stale}");
        assert_eq!(stale["code"], "ledger-busy", "{stale}");
        assert_eq!(head_of(&store, &project_b()), before);

        issue_start(&store, &project_b(), I2, R4, T4);
        let before = head_of(&store, &project_b());
        let unchecked = record(&store, &fail_in(&project_b(), R5, T5), REASON, None);
        assert!(unchecked.is_err(), "{unchecked:?}");
        assert_eq!(head_of(&store, &project_b()), before);
        assert_eq!(
            order(&store, &project_b(), I2).unwrap().unwrap()["state"],
            "pending"
        );
    }
}
