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
//! the owner's name travels in each event's payload.

use std::cell::RefCell;

use baley_core::questions::{Call, judge_call};
use baley_store::{CommandKind, ProjectId, RequestId, request_digest};
use baley_store_sqlite::SqliteStore;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::envelope::Refusal;
use crate::mcp::handler::Preparation;
use crate::mcp::prepare::WriteRequest;
use crate::mcp::work_order::{judge_request_id, prepared_write, shape_fault};
use crate::providers::identity::{self, OWNER_IDENTITY_MISSING};
use crate::questions::answer::{self, ANSWER_COMMAND};

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

/// Serves one `questions-answer` call for the session's `preparation`. The
/// arguments are judged first; then the write is prepared under the
/// session's caller with the answer check as its own check; then the owner
/// is read from the write's policy, falling back to git's `user.name`, and
/// the call is recorded over the set the check read. A replay goes to the
/// recording with no read, and the store answers it from its record.
pub fn serve(arguments: &Value, preparation: &Preparation) -> Value {
    let judged = match judge_arguments(arguments) {
        Ok(judged) => judged,
        Err(refusal) => return refusal,
    };
    // Kept for the recording to observe. A replay skips the check.
    let kept = RefCell::new(None);
    let check = |store: &SqliteStore, project: &ProjectId| {
        *kept.borrow_mut() = Some(answer::check(store, project, &judged.set, &judged.call)?);
        Ok(())
    };
    let (store, command, stepped) =
        match prepared_write(preparation, &preparation.caller, &judged.request, &check) {
            Ok(prepared) => prepared,
            Err(answer) => return answer,
        };
    // A replay ran no steps and is answered from its record, so it needs no
    // owner.
    let owner = match stepped {
        Some(stepped) => match identity::owner(
            identity::owner_name(&stepped.policy),
            &mut crate::process::System,
        ) {
            Ok(owner) => owner,
            Err(reason) => return Refusal::new(OWNER_IDENTITY_MISSING, reason).value(),
        },
        None => String::new(),
    };
    answer::answer(
        answer::record(store, &command, &owner, kept.borrow().as_ref()),
        store,
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
