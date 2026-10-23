//! `questions-answer` on `baley_apply`: a session relays the owner's answers
//! and deferrals for one round of a question set (design 0005 `questions
//! answer`, PLN-R24 to PLN-R26, HST-R9).
//!
//! The arguments, the request id and the call's grammar are judged before
//! anything is prepared. The write is then prepared with the answer check as
//! its own check, so every refusal comes before checkout admission and the
//! policy step and records nothing, and the call is recorded through the
//! same library function `baley questions answer` calls. The command is
//! Baley's with the session as its caller: over MCP Baley cannot tell
//! whether the owner gave the answer, so the record does not claim it, and
//! the owner's name travels in each event's payload. A call that closes a
//! story's set also issues the analyzer's drafting order with the answers
//! (D-08), which `baley questions answer` never does (ADR 0044).

use std::cell::RefCell;
use std::collections::BTreeSet;

use baley_core::policy::{AcceptedNames, EffectivePolicy, Host};
use baley_core::questions::{Call, SetFacts, judge_call};
use baley_store::{
    Command, CommandKind, Ledger, Payloads, ProjectId, RequestId, Views, request_digest,
};
use baley_store_sqlite::SqliteStore;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::store_failed;
use crate::mcp::handler::Preparation;
use crate::mcp::prepare::WriteRequest;
use crate::mcp::work_order::{judge_request_id, prepared_write, shape_fault};
use crate::providers::identity::{self, OWNER_IDENTITY_MISSING};
use crate::questions::answer::{self, ANSWER_COMMAND};
use crate::refine::order::{pending_analyzer, prepare_drafting};
use crate::scope::submit::NOT_RECORDED_BUSY;
use crate::work_order::route::host_names;

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `questions-answer` takes beside its `operation`. Unknown
/// fields are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnswerShape {
    /// A fresh UUID for this call in lowercase hyphenated form. A retry of
    /// the same call sends the same id.
    pub request_id: String,
    /// The question set's id, as its round read or an earlier receipt gave
    /// it.
    pub set: String,
    /// One item for each question the owner settles in this round:
    /// `{"question":"<id>","act":"take"}` takes the recommended answer,
    /// `{"question":"<id>","act":"answer","answer":"<the owner's words>"}`
    /// records the owner's own answer, and
    /// `{"question":"<id>","act":"defer","reason":"<the owner's reason>"}`
    /// leaves the question undecided. Each question at most once, every
    /// text at most 4096 bytes of UTF-8.
    // Typed as a list of objects in the schema, but taken as a value so the
    // call's own grammar judges it and a fault is `typed-content`.
    #[schemars(with = "Vec<Map<String, Value>>")]
    pub items: Value,
}

/// The tagged form `questions-answer`'s arguments arrive in.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum AnswerCall {
    #[serde(rename = "questions-answer")]
    Answer(AnswerShape),
}

/// An answer call whose arguments passed every check made before
/// preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedAnswer {
    /// The write preparation carries into the domain command.
    pub request: WriteRequest,
    /// The set answered.
    pub set: String,
    /// The call, its grammar judged.
    pub call: Call,
}

/// Judges `questions-answer`'s arguments before anything is prepared or
/// recorded: the shape, then the request id, then the call's grammar. A
/// fault is the `refused` answer, which records nothing. The request digest
/// covers the command kind, the set and the items and never the policy
/// version, so a retry after a policy edit replays.
pub fn judge_arguments(arguments: &Value) -> Result<JudgedAnswer, Value> {
    let shape = match serde_json::from_value::<AnswerCall>(arguments.clone()) {
        Ok(AnswerCall::Answer(shape)) => shape,
        Err(error) => return Err(shape_fault(&error)),
    };
    judge_request_id(&shape.request_id, "call")?;
    let call = judge_call(&shape.items).map_err(answer::call_fault)?;
    let digest =
        request_digest(&json!({"command": ANSWER_COMMAND, "set": shape.set, "items": shape.items}))
            .expect("judged items are canonical");
    Ok(JudgedAnswer {
        request: WriteRequest {
            kind: CommandKind(ANSWER_COMMAND.into()),
            request_id: RequestId(shape.request_id),
            digest,
        },
        set: shape.set,
        call,
    })
}

/// What a relayed answer call's check read: the answer check's read of the
/// set and, for a story's set, whether an analyzer order of the story was
/// pending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Relayed {
    /// The answer check's read.
    pub checked: answer::Checked,
    /// Whether an analyzer order of the set's story was pending at the check.
    pub analyzer_pending: bool,
}

impl Relayed {
    /// The story and its set's facts after the call when the call, recorded
    /// by `owner` at `at`, closes a story's set and no analyzer order of the
    /// story was pending at the check: the call then issues the drafting
    /// order (D-08). None otherwise: a pending order keeps the story's one
    /// live analyzer order (D-09), which the session's next `story-refine`
    /// follows once it is settled, and a plan draft's set gets T9's outputs.
    pub(crate) fn closing(&self, owner: &str, at: &str) -> Option<(u32, SetFacts)> {
        let story = self.checked.story?;
        if self.analyzer_pending {
            return None;
        }
        Some((story, self.checked.closed_facts(owner, at)?))
    }
}

/// A relayed answer call's check, run before admission and recording
/// nothing: the answer check, then for a story's set the story's pending
/// analyzer order. A refusal or a read fault is the answer to give.
pub(crate) fn relay_check<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    set: &str,
    call: &Call,
) -> Result<Relayed, Value> {
    let checked = answer::check(store, project, set, call)?;
    let analyzer_pending = match checked.story {
        Some(story) => pending_analyzer(store, project, story)
            .map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))?
            .is_some(),
        None => false,
    };
    Ok(Relayed {
        checked,
        analyzer_pending,
    })
}

/// Records a relayed call `relayed` under `command` as `owner` once the write
/// is prepared. When the call closes a story's set with no analyzer order of
/// the story pending, the drafting order is prepared as `drafting_id` under
/// `policy`, the write's effective policy, for `host` with the catalog
/// `names`, its `questions` input holding the set's answers and deferrals
/// with this call's own, and is issued with the answers. The answer is the
/// receipt, or why nothing was recorded.
#[allow(clippy::too_many_arguments)]
pub(crate) fn relay_record<S: Ledger + Views + Payloads>(
    store: &S,
    command: &Command,
    owner: &str,
    relayed: &Relayed,
    policy: &EffectivePolicy,
    host: Host,
    names: &AcceptedNames,
    drafting_id: &str,
) -> Value {
    let drafting = match relayed.closing(owner, &command.recorded_at) {
        Some((story, facts)) => {
            match prepare_drafting(
                store,
                command,
                story,
                &facts,
                policy,
                host,
                names,
                drafting_id,
            ) {
                Ok(prepared) => Some(prepared),
                Err(answer) => return answer,
            }
        }
        None => None,
    };
    answer::answer(
        answer::record(
            store,
            command,
            owner,
            Some(&relayed.checked),
            drafting.as_ref(),
        ),
        store,
    )
}

/// Serves one `questions-answer` call for the session's `preparation`. The
/// arguments are judged first; then the write is prepared under the
/// session's caller with the relayed check as its own check; then the owner
/// is read from the write's policy, falling back to git's `user.name`, and
/// the call is recorded over the set the check read, with the drafting order
/// when it closes a story's set (D-08). A replay goes to the recording with
/// no read, and the store answers it from its record.
pub fn serve(arguments: &Value, preparation: &Preparation) -> Value {
    let judged = match judge_arguments(arguments) {
        Ok(judged) => judged,
        Err(refusal) => return refusal,
    };
    // Kept for the recording to observe. A replay skips the check.
    let kept = RefCell::new(None);
    let check = |store: &SqliteStore, project: &ProjectId| {
        *kept.borrow_mut() = Some(relay_check(store, project, &judged.set, &judged.call)?);
        Ok(())
    };
    let (store, command, stepped) =
        match prepared_write(preparation, &preparation.caller, &judged.request, &check) {
            Ok(prepared) => prepared,
            Err(answer) => return answer,
        };
    let kept = kept.borrow();
    // A replay ran no steps and is answered from its record, so it needs no
    // owner.
    let (Some(stepped), Some(relayed)) = (stepped, kept.as_ref()) else {
        return answer::answer(answer::record(store, &command, "", None, None), store);
    };
    let owner = match identity::owner(
        identity::owner_name(&stepped.policy),
        &mut crate::process::System,
    ) {
        Ok(owner) => owner,
        Err(reason) => return Refusal::new(OWNER_IDENTITY_MISSING, reason).value(),
    };
    // The catalog is read only for a drafting order, and unused otherwise.
    let names = if relayed.closing(&owner, &command.recorded_at).is_some() {
        match host_names(store, preparation.host) {
            Ok(names) => names,
            Err(error) => return store_failed(&error, NOT_RECORDED_BUSY),
        }
    } else {
        AcceptedNames {
            names: BTreeSet::new(),
            version: 0,
        }
    };
    relay_record(
        store,
        &command,
        &owner,
        relayed,
        &stepped.policy,
        preparation.host,
        &names,
        &uuid::Uuid::new_v4().to_string(),
    )
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005 `questions answer`
    //! and design 0012's write rules, never from running this code.

    use super::*;

    const SET: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const OTHER: &str = "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e";
    const REQUEST: &str = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";

    fn arguments(set: &str, items: Value) -> Value {
        json!({"operation": "questions-answer", "request_id": REQUEST, "set": set,
            "items": items})
    }

    #[test]
    fn a_questions_answer_shape_or_request_id_fault_accepted_is_caught() {
        let take = json!([{"question": "q1", "act": "take"}]);
        let mut extra = arguments(SET, take.clone());
        extra["instruction"] = json!("bal-rounds");
        let mut upper = arguments(SET, take);
        upper["request_id"] = json!(REQUEST.to_uppercase());
        for (call, code, slot) in [
            (extra, "invalid-arguments", "arguments"),
            (upper, "invalid-arguments", "request_id"),
            (arguments(SET, json!("q1")), "typed-content", "items"),
        ] {
            let refusal = judge_arguments(&call).expect_err("refused");
            assert_eq!(refusal["status"], "refused", "{call}");
            assert_eq!(refusal["code"], code, "{call}");
            assert_eq!(refusal["slot"], slot, "{call}");
        }
    }

    fn digest(set: &str, items: Value) -> baley_store::Hash {
        judge_arguments(&arguments(set, items))
            .unwrap()
            .request
            .digest
    }

    #[test]
    fn a_relayed_answer_digest_covering_the_policy_or_missing_the_items_is_caught() {
        let take = |question: &str| json!([{"question": question, "act": "take"}]);
        let first = digest(SET, take("q1"));
        assert_eq!(digest(SET, take("q1")), first);
        assert_ne!(digest(SET, take("q2")), first);
        assert_ne!(digest(OTHER, take("q1")), first);
        let own = |answer: &str| json!([{"question": "q1", "act": "answer", "answer": answer}]);
        assert_ne!(digest(SET, own("SQLite")), digest(SET, own("Postgres")));
        let defer = |reason: &str| json!([{"question": "q2", "act": "defer", "reason": reason}]);
        assert_ne!(digest(SET, defer("a")), digest(SET, defer("b")));
    }
}
