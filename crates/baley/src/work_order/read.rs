//! Reads of a project's work orders, recording nothing: an order by its id,
//! the pending start planner order a start is checked against (D-02), the
//! input references this project's purges released (D-16) and a start
//! order's accepted result (D-03). A store error comes back unchanged for
//! the caller to answer.

use baley_core::policy::Role;
use baley_core::scope::{PROJECT_STREAM, SCOPE_APPROVED, START_KIND};
use baley_core::work_order::{
    PENDING, START_SUBJECT, WORK_ORDER_LIVE_INDEX, WORK_ORDER_VIEW, work_order_key,
};
use baley_store::{
    Document, EVENT_PAGE_BOUND, Hash, HistoryFilter, IndexQuery, KeyValue, Ledger, PAYLOAD_PURGED,
    PageRequest, ProjectId, PurgedEvent, RETENTION_STREAM, StoreError, StreamName, Views,
};
use serde_json::{Value, json};

/// The document of order `id` in `project`, or `None` when the view holds
/// none.
pub fn order<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    id: &str,
) -> Result<Option<Value>, StoreError> {
    order_document(store, project, id).map(|found| found.map(|document| document.body))
}

/// The view document of order `id` in `project` with the sequence that last
/// produced it, so a decision can observe it unchanged, or `None`.
pub fn order_document<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    id: &str,
) -> Result<Option<Document>, StoreError> {
    store.get(project, WORK_ORDER_VIEW, &work_order_key(id))
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

/// One reference a purge of this project released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The sequence of the event that held the reference.
    pub seq: u64,
    /// The released body's hash.
    pub hash: Hash,
    /// The reason the purge recorded.
    pub reason: String,
}

/// The references a project's purges released, and the version of its
/// `retention` stream they were read up to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Releases {
    /// Every released reference, in the order the purges recorded them.
    pub released: Vec<Release>,
    /// The `retention` stream's version when it was read, 0 when it holds
    /// nothing.
    pub version: u64,
}

impl Releases {
    /// The `(seq, hash)` pairs, as the retry judge takes them.
    pub fn pairs(&self) -> Vec<(u64, Hash)> {
        self.released
            .iter()
            .map(|release| (release.seq, release.hash))
            .collect()
    }

    /// This project's purge of the reference to `hash` at `seq`, if any.
    pub fn find(&self, seq: u64, hash: &Hash) -> Option<&Release> {
        self.released
            .iter()
            .find(|release| release.seq == seq && release.hash == *hash)
    }
}

/// The references `project`'s own purges released, read from its
/// `retention` stream outside any transaction. A body's status is shared by
/// every project holding the bytes, so only this project's releases say its
/// reference is gone. A transaction cannot page a stream, so a decision that
/// depends on these `expect`s the version given here.
pub fn releases<L: Ledger + ?Sized>(
    store: &L,
    project: &ProjectId,
) -> Result<Releases, StoreError> {
    let stream = StreamName(RETENTION_STREAM.into());
    let mut read = Releases {
        released: Vec::new(),
        version: 0,
    };
    let mut after = None;
    loop {
        let page = store.stream(
            project,
            &stream,
            1,
            PageRequest {
                limit: EVENT_PAGE_BOUND,
                after,
            },
        )?;
        for event in page.items {
            read.version = read.version.max(event.stream_version);
            if event.type_name != PAYLOAD_PURGED {
                continue;
            }
            let Some(purge) = PurgedEvent::from_value(&event.payload) else {
                return Err(StoreError::Unavailable(format!(
                    "the purge at seq {} cannot be read",
                    event.seq
                )));
            };
            read.released
                .extend(purge.released.into_iter().map(|(seq, hash)| Release {
                    seq,
                    hash,
                    reason: purge.reason.clone(),
                }));
        }
        match page.next {
            Some(next) => after = Some(next),
            None => return Ok(read),
        }
    }
}

/// The accepted result of a start order (D-03): the digest of `project`'s
/// start approval when it is among `order`'s drafts, or `None`. An approval
/// names only a digest, so the view cannot reach an order from it; the
/// approval is read from the events, as `start_approved` reads it.
pub fn accepted<L: Ledger + ?Sized>(
    store: &L,
    project: &ProjectId,
    order: &Value,
) -> Result<Option<String>, StoreError> {
    let filter = HistoryFilter {
        types: vec![SCOPE_APPROVED.into()],
        git_commit: None,
    };
    let start = json!(START_KIND);
    let mut after = None;
    let approved = loop {
        let page = store.history(
            project,
            1..=u64::MAX,
            &filter,
            PageRequest {
                limit: EVENT_PAGE_BOUND,
                after,
            },
        )?;
        let found = page.items.into_iter().find_map(|event| {
            (event.stream == PROJECT_STREAM && event.payload.get("kind") == Some(&start))
                .then(|| event.payload.get("submission").cloned())
                .flatten()
        });
        if found.is_some() {
            break found;
        }
        match page.next {
            Some(next) => after = Some(next),
            None => break None,
        }
    };
    let Some(Value::String(digest)) = approved else {
        return Ok(None);
    };
    let listed = order
        .get("drafts")
        .and_then(Value::as_array)
        .is_some_and(|drafts| drafts.iter().any(|draft| draft.as_str() == Some(&digest)));
    Ok(listed.then_some(digest))
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
            documents: &[],
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

    /// A lowercase UUID numbered `n`, distinct for each `n` and `tag`.
    fn numbered(tag: u8, n: u32) -> String {
        format!("{n:08x}-{tag:02x}00-4000-8000-000000000000")
    }

    /// Records a planner order `id` about `start` in `project` with `brief`
    /// as its input and no absence declared, so several may be pending.
    fn issue_brief(store: &SqliteStore, project: &ProjectId, id: &str, brief: &[u8]) {
        let policy =
            effective_policy(Schema::standard(), Some(Host::ClaudeCode), None, None).unwrap();
        let names = AcceptedNames {
            names: BTreeSet::new(),
            version: 0,
        };
        let route = compose(
            &policy,
            1,
            Host::ClaudeCode,
            Role::Planner,
            NonZeroU32::MIN,
            &names,
        )
        .unwrap();
        let further = serde_json::Map::new();
        let inputs = [Input::Bytes {
            name: "brief",
            bytes: brief,
        }];
        let issue = Issue {
            id,
            request: id,
            parent: None,
            subject: START_SUBJECT,
            route: &route,
            instructions: &[],
            inputs: &inputs,
            absences: &[],
            streams: &[],
            documents: &[],
            further: &further,
        };
        let command = command_in(project, "project.start", id, 1, T1);
        let answer = issue::answer(issue::record(store, &command, Some(&issue)), store);
        assert_eq!(answer["status"], "ok", "{answer}");
    }

    /// Releases `project`'s references to `hash` under purge request
    /// `request_id`.
    fn purge_as(
        store: &SqliteStore,
        project: &ProjectId,
        hash: Hash,
        reason: &str,
        request_id: &str,
    ) {
        use baley_store::{Actor, Command, CommandKind, RequestId};
        let purge = Command {
            project: project.clone(),
            kind: CommandKind("payload.purge".into()),
            request_id: RequestId(request_id.into()),
            digest: Hash([9; 32]),
            scope: Vec::new(),
            policy_version: 0,
            recorded_at: T3.into(),
            actor: Actor::Owner,
            caller: None,
        };
        store.purge(&purge, &[hash], reason).unwrap();
    }

    fn sha256(bytes: &[u8]) -> Hash {
        use sha2::{Digest, Sha256};
        Hash(Sha256::digest(bytes).into())
    }

    /// Records `value` as a start submission in `project` answering order
    /// `id`, under `request_id`, and gives its digest.
    fn answer_order(
        store: &SqliteStore,
        project: &ProjectId,
        id: &str,
        value: &serde_json::Value,
        request_id: &str,
    ) -> Hash {
        let mut command = command_in(
            project,
            crate::scope::submit::SUBMIT_COMMAND,
            request_id,
            2,
            T2,
        );
        command.caller = Some(Caller::Server(caller().with_work_order(id).unwrap()));
        let submission = judge_submission(value).unwrap();
        crate::scope::submit::record(store, &command, &submission).unwrap();
        submission.digest()
    }

    #[test]
    fn an_accepted_result_named_before_approval_or_for_another_orders_draft_is_caught() {
        let (_dir, store) = two_projects();
        let m_digest = M_DIGEST.to_string();
        issue_in(&store, &project(), I0, I0, None, 1, &brief(), R1, T1);
        let m_hash = answer_order(&store, &project(), I0, &m(), R2);
        assert_eq!(m_hash.to_hex(), M_DIGEST);
        let held = |project: &ProjectId, id: &str| order(&store, project, id).unwrap().unwrap();
        assert_eq!(
            accepted(&store, &project(), &held(&project(), I0)).unwrap(),
            None
        );
        approve_draft_in(&store, &project(), m_hash);
        assert_eq!(
            accepted(&store, &project(), &held(&project(), I0)).unwrap(),
            Some(m_digest.clone())
        );

        issue_in(&store, &project_b(), I1, I1, None, 1, &brief(), R1, T1);
        answer_order(&store, &project_b(), I1, &m(), R2);
        issue_in(&store, &project_b(), I2, I2, None, 1, &brief(), R3, T2);
        let s1_hash = answer_order(&store, &project_b(), I2, &s1(), R4);
        approve_draft_in(&store, &project_b(), s1_hash);
        assert_eq!(
            accepted(&store, &project_b(), &held(&project_b(), I1)).unwrap(),
            None
        );
        assert_eq!(
            accepted(&store, &project_b(), &held(&project_b(), I2)).unwrap(),
            Some(s1_hash.to_hex())
        );

        use baley_store::Admin;
        let third = ProjectId("8b3e4c6a-0d1f-4a2b-9c3d-4e5f6a7b8c9d".into());
        store.create_project(&third, "third", T0).unwrap();
        issue_in(&store, &third, I0, I0, None, 1, &brief(), R1, T1);
        answer_order(&store, &third, I0, &m(), R2);
        answer_order(&store, &third, I0, &s1(), R3);
        answer_order(&store, &third, I0, &m(), R4);
        approve_draft_in(&store, &third, m_hash);
        let document = held(&third, I0);
        assert_eq!(document["drafts"], json!([M_DIGEST, s1_hash.to_hex()]));
        assert_eq!(accepted(&store, &third, &document).unwrap(), Some(m_digest));
    }

    #[test]
    fn an_input_this_project_purged_read_as_held_or_another_projects_purge_taken_is_caught() {
        let (_dir, store) = two_projects();
        issue_in(&store, &project(), I0, I0, None, 1, &brief(), R1, T1);
        issue_in(&store, &project_b(), I1, I1, None, 1, &brief(), R1, T1);
        let b1 = Hash::from_hex(B1_SHA256).unwrap();
        purge_in(&store, &project_b(), b1, "owner purge");

        assert_eq!(
            releases(&store, &project()).unwrap(),
            Releases {
                released: Vec::new(),
                version: 0
            }
        );
        let i1_seq = order(&store, &project_b(), I1).unwrap().unwrap()["seq"]
            .as_u64()
            .unwrap();
        let first = Release {
            seq: i1_seq,
            hash: b1,
            reason: "owner purge".into(),
        };
        assert_eq!(
            releases(&store, &project_b()).unwrap(),
            Releases {
                released: vec![first.clone()],
                version: 1
            }
        );

        let bound = EVENT_PAGE_BOUND;
        for n in 0..bound {
            let text = format!("another brief {n}");
            issue_brief(&store, &project_b(), &numbered(1, n), text.as_bytes());
            purge_as(
                &store,
                &project_b(),
                sha256(text.as_bytes()),
                "later purge",
                &numbered(2, n),
            );
        }
        let read = releases(&store, &project_b()).unwrap();
        assert_eq!(read.version, u64::from(bound) + 1);
        assert_eq!(read.released.len(), bound as usize + 1);
        assert!(read.released.contains(&first), "{read:?}");
        let last = sha256(format!("another brief {}", bound - 1).as_bytes());
        assert!(
            read.released
                .iter()
                .any(|release| release.hash == last && release.reason == "later purge"),
            "{read:?}"
        );
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
