//! Reads of a project's `work_order` view, recording nothing: an order by
//! its id, and the pending start planner order a start is checked against
//! (D-02). A store error comes back unchanged for the caller to answer.

use baley_core::policy::Role;
use baley_core::work_order::{
    PENDING, START_SUBJECT, WORK_ORDER_LIVE_INDEX, WORK_ORDER_VIEW, work_order_key,
};
use baley_store::{IndexQuery, KeyValue, PageRequest, ProjectId, StoreError, Views};
use serde_json::Value;

/// The document of order `id` in `project`, or `None` when the view holds
/// none.
pub fn order<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    id: &str,
) -> Result<Option<Value>, StoreError> {
    store
        .get(project, WORK_ORDER_VIEW, &work_order_key(id))
        .map(|found| found.map(|document| document.body))
}

/// The pending start planner order of `project`, the newest the `live`
/// index gives for `planner`, `start`, `pending`, or `None` when there is
/// none.
pub fn pending_start<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
) -> Result<Option<Value>, StoreError> {
    let text = |value: &str| KeyValue::Text(value.into());
    let query = IndexQuery {
        index: WORK_ORDER_LIVE_INDEX.into(),
        equals: vec![
            text(Role::Planner.name()),
            text(START_SUBJECT),
            text(PENDING),
        ],
        page: PageRequest {
            limit: 1,
            after: None,
        },
    };
    let page = store.find(project, WORK_ORDER_VIEW, &query)?;
    Ok(page.items.into_iter().next().map(|document| document.body))
}

#[cfg(test)]
mod tests {
    //! Orders are recorded through `issue::record` over the real store, and
    //! the other events are appended by test commands as other writes would
    //! leave them.

    use std::collections::BTreeSet;
    use std::num::NonZeroU32;

    use baley_core::policy::{AcceptedNames, Host, Schema, effective_policy};
    use baley_core::scope::judge_submission;
    use baley_core::work_order::{
        OrderInput, WORK_ORDER_FAILED, WORK_ORDER_ISSUED, failed_payload,
    };
    use baley_store::{Caller, Hash, NewEvent, PayloadRef, RetentionClass, StreamName};
    use baley_store_sqlite::SqliteStore;
    use serde_json::json;

    use super::*;
    use crate::scope::fixtures::*;
    use crate::work_order::issue::{self, Input, Issue, start_absences};
    use crate::work_order::route::compose;

    const I0: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const I1: &str = "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e";
    const I2: &str = "7f3e4d5c-6071-4c8d-8e9f-1a2b3c4d5e6f";
    const B1: &str = "Build a ledger for agent work";
    const B1_SHA256: &str = "04fe1dc2bdd0a95df1e6d96c47379024f73df0d9584025e1431b6b5c3162ac9b";

    /// Records the planner's start order `id` in `project` at `attempt`,
    /// asking `request` and retrying `parent`, under `request_id` at `at`.
    #[allow(clippy::too_many_arguments)]
    fn issue_in(
        store: &SqliteStore,
        project: &ProjectId,
        id: &str,
        request: &str,
        parent: Option<&str>,
        attempt: u32,
        inputs: &[Input<'_>],
        request_id: &str,
        at: &str,
    ) {
        let policy =
            effective_policy(Schema::standard(), Some(Host::ClaudeCode), None, None).unwrap();
        let names = AcceptedNames {
            names: BTreeSet::new(),
            version: 0,
        };
        let attempt = NonZeroU32::new(attempt).unwrap();
        let route = compose(&policy, 1, Host::ClaudeCode, Role::Planner, attempt, &names).unwrap();
        // A retry child is recorded beside its pending parent here, so it
        // declares no absence.
        let absences = if parent.is_none() {
            start_absences()
        } else {
            Vec::new()
        };
        let further = serde_json::Map::new();
        let issue = Issue {
            id,
            request,
            parent,
            subject: START_SUBJECT,
            route: &route,
            instructions: &[],
            inputs,
            absences: &absences,
            streams: &[],
            further: &further,
        };
        let command = command_in(project, "project.start", request_id, 1, at);
        let answer = issue::answer(issue::record(store, &command, Some(&issue)), store);
        assert_eq!(answer["status"], "ok", "{answer}");
    }

    fn brief() -> [Input<'static>; 1] {
        [Input::Bytes {
            name: "brief",
            bytes: B1.as_bytes(),
        }]
    }

    #[test]
    fn an_order_read_from_another_project_or_under_another_id_is_caught() {
        let (_dir, store) = two_projects();
        issue_in(&store, &project(), I0, I0, None, 1, &brief(), R1, T1);
        let held = OrderInput {
            name: "brief".into(),
            body: PayloadRef {
                hash: Hash::from_hex(B1_SHA256).unwrap(),
                bytes: 29,
                class: RetentionClass::Record,
            },
        };
        issue_in(
            &store,
            &project(),
            I2,
            I0,
            Some(I0),
            2,
            &[Input::Held(&held)],
            R2,
            T2,
        );

        let first = order(&store, &project(), I0).unwrap().expect("I0 is held");
        assert_eq!(first["id"], I0);
        assert_eq!(first["state"], "pending");
        let child = order(&store, &project(), I2).unwrap().expect("I2 is held");
        assert_eq!(child["id"], I2);
        assert_eq!(child["request"], I0);
        assert_eq!(order(&store, &project_b(), I0).unwrap(), None);
        assert_eq!(order(&store, &project(), I1).unwrap(), None);
    }

    /// A pending order of `role` about `subject`, as a test command
    /// appends it.
    fn issued_by_hand(id: &str, role: &str, subject: &str) -> NewEvent {
        NewEvent {
            stream: StreamName("roadmap".into()),
            type_name: WORK_ORDER_ISSUED.into(),
            type_version: 1,
            git: None,
            payload: json!({"id": id, "request": id, "parent": null, "role": role,
                "subject": subject, "attempt": 1, "route": {}, "agent": "bal-checker-high",
                "instructions": [], "inputs": []}),
            attachments: vec![],
        }
    }

    #[test]
    fn a_failed_or_answered_start_order_taken_as_pending_is_caught() {
        let (_dir, store) = two_projects();
        issue_in(&store, &project(), I0, I0, None, 1, &brief(), R1, T1);
        let pending = pending_start(&store, &project())
            .unwrap()
            .expect("I0 is pending");
        assert_eq!(pending["id"], I0);

        append(
            &store,
            R2,
            T2,
            vec![NewEvent {
                stream: StreamName("roadmap".into()),
                type_name: WORK_ORDER_FAILED.into(),
                type_version: 1,
                git: None,
                payload: failed_payload(I0, "the planner stopped"),
                attachments: vec![],
            }],
        );
        assert_eq!(pending_start(&store, &project()).unwrap(), None);

        append(
            &store,
            R3,
            T3,
            vec![issued_by_hand(
                "8a4f5e6d-7182-4d9e-9fa0-2b3c4d5e6f70",
                "checker",
                "start",
            )],
        );
        append(
            &store,
            R4,
            T4,
            vec![issued_by_hand(
                "9b5a6f7e-8293-4eaf-8ab1-3c4d5e6f7081",
                "planner",
                "phase/2",
            )],
        );
        assert_eq!(pending_start(&store, &project()).unwrap(), None);

        issue_in(&store, &project_b(), I1, I1, None, 1, &brief(), R1, T1);
        let mut command = command_in(
            &project_b(),
            crate::scope::submit::SUBMIT_COMMAND,
            R2,
            2,
            T2,
        );
        command.caller = Some(Caller::Server(caller().with_work_order(I1).unwrap()));
        crate::scope::submit::record(&store, &command, &judge_submission(&m()).unwrap()).unwrap();
        assert_eq!(pending_start(&store, &project_b()).unwrap(), None);
    }
}
