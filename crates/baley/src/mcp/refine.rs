//! `story-refine` on `baley_apply`: a session asks for the analyzer's next
//! order for one story (design 0005 `story refine`, PLN-R6, D-08, D-09,
//! D-19).
//!
//! The arguments, the request id and the instruction are judged before
//! anything is prepared. The write is then prepared with the refine check as
//! its own check, so every refusal comes before checkout admission and the
//! policy step and records nothing, and the order the check chose is issued
//! through the same library function the owner's commands would call.

use std::cell::RefCell;

use baley_store::{CommandKind, InstructionEvidence, ProjectId, RequestId, request_digest};
use baley_store_sqlite::SqliteStore;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::mcp::capture::store_failed;
use crate::mcp::handler::Preparation;
use crate::mcp::prepare::WriteRequest;
use crate::mcp::work_order::{
    evidenced, judge_instruction, judge_request_id, prepared_write, shape_fault,
};
use crate::refine::order;
use crate::scope::submit::NOT_RECORDED_BUSY;
use crate::work_order::issue as recording;
use crate::work_order::route::host_names;

/// The command kind a story refine is recorded under.
pub const REFINE_COMMAND: &str = "story.refine";

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `story-refine` takes beside its `operation`. Unknown
/// fields are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RefineShape {
    /// A fresh UUID for this call in lowercase hyphenated form. A retry of
    /// the same call sends the same id.
    pub request_id: String,
    /// The id of the story to refine, as the roadmap or the backlog gives
    /// it.
    pub story: u32,
    /// The identity of the instruction the session followed, if it followed
    /// one.
    pub instruction: Option<String>,
}

/// The tagged form `story-refine`'s arguments arrive in.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum RefineCall {
    #[serde(rename = "story-refine")]
    Refine(RefineShape),
}

/// A story refine whose arguments passed every check made before
/// preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedRefine {
    /// The write preparation carries into the domain command.
    pub request: WriteRequest,
    /// The story to refine.
    pub story: u32,
    /// The registry's evidence for the instruction the session named, if any.
    pub instruction: Option<InstructionEvidence>,
}

/// Judges `story-refine`'s arguments before anything is prepared or
/// recorded: the shape, then the request id, then the instruction. A fault
/// is the `refused` answer, which records nothing. The request digest covers
/// the command kind and the story, so a retry replays and the same request
/// id for another story is a reused id.
pub fn judge_arguments(arguments: &Value) -> Result<JudgedRefine, Value> {
    let shape = match serde_json::from_value::<RefineCall>(arguments.clone()) {
        Ok(RefineCall::Refine(shape)) => shape,
        Err(error) => return Err(shape_fault(&error)),
    };
    judge_request_id(&shape.request_id, "call")?;
    let instruction = judge_instruction(shape.instruction.as_deref())?;
    let digest = request_digest(&json!({"command": REFINE_COMMAND, "story": shape.story}))
        .expect("a story id is canonical");
    Ok(JudgedRefine {
        request: WriteRequest {
            kind: CommandKind(REFINE_COMMAND.into()),
            request_id: RequestId(shape.request_id),
            digest,
        },
        story: shape.story,
        instruction,
    })
}

/// Serves one `story-refine` call for the session's `preparation`. The
/// arguments are judged first; then the write is prepared under the caller
/// with the instruction evidence attached and the refine check as its own
/// check; then, for a write that ran its steps, the host catalog is read and
/// the order the check chose is issued under a fresh lowercase UUID with the
/// prepared policy. A replay goes to the recording with no order, and the
/// store answers it from its record.
pub fn serve(arguments: &Value, preparation: &Preparation) -> Value {
    let judged = match judge_arguments(arguments) {
        Ok(judged) => judged,
        Err(refusal) => return refusal,
    };
    // Kept for the issue to observe. A replay skips the check.
    let kept = RefCell::new(None);
    let check = |store: &SqliteStore, project: &ProjectId| {
        *kept.borrow_mut() = Some(order::check(store, project, judged.story)?);
        Ok(())
    };
    let caller = evidenced(&preparation.caller, judged.instruction.as_ref());
    let (store, command, stepped) =
        match prepared_write(preparation, &caller, &judged.request, &check) {
            Ok(prepared) => prepared,
            Err(answer) => return answer,
        };
    let kept = kept.borrow();
    let (Some(stepped), Some(checked)) = (stepped, kept.as_ref()) else {
        return recording::answer(recording::record(store, &command, None), store);
    };
    let names = match host_names(store, preparation.host) {
        Ok(names) => names,
        Err(error) => return store_failed(&error, NOT_RECORDED_BUSY),
    };
    order::issue(
        store,
        &command,
        checked,
        &stepped.policy,
        preparation.host,
        &names,
        &uuid::Uuid::new_v4().to_string(),
    )
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, D-19 and
    //! HST-R5, never from running this code.

    use baley_core::policy::Host;
    use baley_store::ServerCaller;

    use super::*;
    use crate::mcp::gate::Called;
    use crate::mcp::operations::{APPLY_OPERATIONS, Lookup, Tool, lookup, schema_answer};

    const R1: &str = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";

    fn refine(request_id: &str, story: u32) -> Value {
        json!({"operation": "story-refine", "request_id": request_id, "story": story})
    }

    fn refused(arguments: &Value) -> Value {
        let refusal = judge_arguments(arguments).expect_err("refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        refusal
    }

    #[test]
    fn a_story_refine_call_judged_loosely_or_replayed_as_another_is_caught() {
        let first = judge_arguments(&refine(R1, 1)).expect("accepted");
        assert_eq!(first.request.kind, CommandKind("story.refine".into()));
        assert_eq!(first.story, 1);

        let mut unknown = refine(R1, 1);
        unknown["phase"] = json!(1);
        let refusal = refused(&unknown);
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "arguments");

        let refusal = refused(&refine(&R1.to_uppercase(), 1));
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "request_id");

        let mut nope = refine(R1, 1);
        nope["instruction"] = json!("bal-nope");
        let refusal = refused(&nope);
        assert_eq!(refusal["code"], "unknown-instruction");
        assert_eq!(refusal["slot"], "instruction");

        let other = judge_arguments(&refine(R1, 2)).expect("accepted");
        assert_ne!(other.request.digest, first.request.digest);
    }

    #[test]
    fn story_refine_left_out_of_the_baseline_or_unserved_is_caught() {
        let names: Vec<&str> = APPLY_OPERATIONS.iter().map(|op| op.name).collect();
        assert_eq!(
            names[names.len() - 2..],
            ["questions-answer", "story-refine"]
        );
        assert_eq!(
            lookup(Tool::Apply, Some("story-refine")),
            Lookup::Available {
                needs_project: true
            }
        );
        let answer =
            schema_answer(&json!({"operation": "schema", "tool": "apply", "for": "story-refine"}));
        assert_eq!(answer["status"], "ok", "{answer}");
        let required = answer["schema"]["required"].as_array().unwrap();
        for field in ["request_id", "story"] {
            assert!(required.contains(&json!(field)), "{field} in {answer}");
        }
        assert!(!required.contains(&json!("instruction")), "{answer}");
        assert!(
            answer["schema"]["properties"].get("instruction").is_some(),
            "{answer}"
        );

        let caller = ServerCaller::new(
            "/real/r",
            "/real/r",
            "claude-code",
            "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f",
            &json!(7),
        )
        .unwrap();
        // No ledger and no checkout to discover, so a served call reaches
        // preparation and answers `failed` at its project check, never
        // `unknown-operation`.
        let preparation = Preparation {
            ledger: None,
            host: Host::ClaudeCode,
            caller,
            at: "2026-10-22T09:00:00Z".into(),
        };
        let answer = crate::mcp::handler::operate(
            &Called::Operation(Tool::Apply, "story-refine".into()),
            Some(&refine(R1, 1)),
            Some(&preparation),
        );
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["recorded"], false, "{answer}");
        assert!(
            ["project-context-invalid", "not-a-project"]
                .contains(&answer["code"].as_str().unwrap()),
            "{answer}"
        );
    }
}
