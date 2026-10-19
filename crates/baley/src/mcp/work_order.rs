//! The work-order operations on `baley_apply` (design 0004 section 4,
//! design 0012). `project-start` records one `work_order.issued` for the
//! start planner on the `roadmap` stream, with the brief as a `record`
//! payload, after the arguments, the start check and preparation pass. A
//! refusal records nothing.

use baley_core::capture::is_blank;
use baley_store::{CommandKind, Hash, InstructionEvidence, RequestId, request_digest};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::envelope::Refusal;
use crate::mcp::client::MAX_ECHOED_BYTES;
use crate::mcp::prepare::WriteRequest;

/// The command kind a project start is recorded under.
pub const START_COMMAND: &str = "project.start";

/// The code for a brief that is empty or only whitespace.
pub const BRIEF_BLANK: &str = "brief-blank";

/// The code for a brief over [`MAX_BRIEF_BYTES`].
pub const BRIEF_TOO_LARGE: &str = "brief-too-large";

/// The most bytes of UTF-8 a brief may hold (D-01).
pub const MAX_BRIEF_BYTES: usize = 65_536;

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `project-start` takes beside its `operation`. Unknown
/// fields are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StartShape {
    /// A fresh UUID for this start in lowercase hyphenated form. A retry of
    /// the same start sends the same id.
    pub request_id: String,
    /// The project brief as text, which the session reads and sends whole.
    /// Baley reads no file for it. It must not be blank, and it holds at
    /// most 65536 bytes of UTF-8.
    pub brief: String,
    /// The identity of the instruction the session followed, if it followed
    /// one.
    pub instruction: Option<String>,
}

/// The tagged form a call's arguments arrive in, so `operation` is read and
/// left out of the shape.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum StartCall {
    #[serde(rename = "project-start")]
    Start(StartShape),
}

/// A project start whose arguments passed every check made before
/// preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedStart {
    /// The write preparation carries into the domain command.
    pub request: WriteRequest,
    /// The brief, as sent.
    pub brief: String,
    /// The registry's evidence for the instruction the session named, if any.
    pub instruction: Option<InstructionEvidence>,
}

/// Judges `project-start`'s arguments before anything is prepared or
/// recorded: the shape, then the request id, the brief and the instruction,
/// in that order. A fault is the `refused` answer, which records nothing.
pub fn judge_start_arguments(arguments: &Value) -> Result<JudgedStart, Value> {
    let shape = match serde_json::from_value::<StartCall>(arguments.clone()) {
        Ok(StartCall::Start(shape)) => shape,
        Err(error) => {
            return Err(
                Refusal::new("invalid-arguments", bounded(error.to_string()))
                    .slot("arguments")
                    .value(),
            );
        }
    };
    // One spelling per id, so a retry cannot miss its replay by changing
    // case or wrapping.
    let canonical = uuid::Uuid::parse_str(&shape.request_id)
        .map(|parsed| parsed.hyphenated().to_string())
        .ok();
    if canonical.as_deref() != Some(shape.request_id.as_str()) {
        return Err(Refusal::new(
            "invalid-arguments",
            "request_id must be a UUID in lowercase hyphenated form, such as 9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f. A retry of the same start sends the same id",
        )
        .slot("request_id")
        .value());
    }
    if is_blank(&shape.brief) {
        return Err(Refusal::new(
            BRIEF_BLANK,
            "the brief is empty or only whitespace, so nothing was recorded. Send the brief's text",
        )
        .slot("brief")
        .value());
    }
    let size = shape.brief.len();
    if size > MAX_BRIEF_BYTES {
        return Err(Refusal::new(
            BRIEF_TOO_LARGE,
            format!(
                "the brief is {size} bytes of UTF-8, over the limit of {MAX_BRIEF_BYTES} bytes, so nothing was recorded. Send a shorter brief"
            ),
        )
        .slot("brief")
        .value());
    }
    let instruction = match shape
        .instruction
        .as_deref()
        .map(crate::instruction::evidence)
    {
        None => None,
        Some(Ok(evidence)) => Some(evidence),
        // The registry's refusal names its read's `identity` argument; here
        // the session sent the identity as `instruction`.
        Some(Err(mut refusal)) => {
            refusal["slot"] = json!("instruction");
            return Err(refusal);
        }
    };
    // The brief's hash stands for it, so the same brief under the same id
    // replays and another brief is a reused id.
    let brief = Hash(Sha256::digest(shape.brief.as_bytes()).into());
    let digest = request_digest(&json!({"command": START_COMMAND, "brief": brief.to_hex()}))
        .expect("text is canonical");
    Ok(JudgedStart {
        request: WriteRequest {
            kind: CommandKind(START_COMMAND.into()),
            request_id: RequestId(shape.request_id),
            digest,
        },
        brief: shape.brief,
        instruction,
    })
}

/// Serde's text for a shape fault, cut to the echo limit with a closing
/// `...`, since it repeats an unknown field's name at whatever length the
/// caller sent.
fn bounded(text: String) -> String {
    const CUT: &str = "...";
    if text.len() <= MAX_ECHOED_BYTES {
        return text;
    }
    let mut end = MAX_ECHOED_BYTES - CUT.len();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{CUT}", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    const R1: &str = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";
    const B1: &str = "Build a ledger for agent work";
    const B2: &str = "Build a ledger for agent work.";

    fn start(request_id: &str, brief: &str) -> Value {
        json!({"operation": "project-start", "request_id": request_id, "brief": brief})
    }

    fn refused(arguments: &Value) -> Value {
        let refusal = judge_start_arguments(arguments).expect_err("refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        refusal
    }

    #[test]
    fn a_brief_judged_blank_or_oversized_by_the_wrong_measure_is_caught() {
        let refusal = refused(&start(R1, " \n\t"));
        assert_eq!(refusal["code"], "brief-blank");
        assert_eq!(refusal["slot"], "brief");

        for brief in ["x".repeat(65_536), "é".repeat(32_768)] {
            let judged = judge_start_arguments(&start(R1, &brief)).expect("accepted");
            assert_eq!(judged.brief, brief);
        }

        let refusal = refused(&start(R1, &"é".repeat(32_769)));
        assert_eq!(refusal["code"], "brief-too-large");
        assert_eq!(refusal["slot"], "brief");
        let reason = refusal["reason"].as_str().unwrap();
        assert!(reason.contains("65536"), "{reason}");
        assert!(reason.contains("65538"), "{reason}");
    }

    #[test]
    fn a_start_request_digest_blind_to_the_brief_is_caught() {
        let request = |brief| judge_start_arguments(&start(R1, brief)).unwrap().request;
        let first = request(B1);
        let second = request(B2);
        assert_eq!(
            first.digest.to_hex(),
            "3d83fcc2baedc54269e332c5201f37226efbff6ce62eea636385255a4df2705f"
        );
        assert_eq!(
            second.digest.to_hex(),
            "2631f59ab1fa07a995b568fa83dd7fadc433faa40473627dc35dc1c9db55c16b"
        );
        for request in [&first, &second] {
            assert_eq!(request.kind, CommandKind("project.start".into()));
            assert_eq!(request.request_id, RequestId(R1.into()));
        }

        let mut path = start(R1, B1);
        path["path"] = json!("/real/r/BRIEF.md");
        let refusal = refused(&path);
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "arguments");

        let refusal = refused(&start(&R1.to_uppercase(), B1));
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "request_id");
    }
}
