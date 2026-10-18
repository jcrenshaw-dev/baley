//! `scope-submit` on `baley_apply`: a start submission judged whole and
//! recorded as a draft the owner can approve by its digest (design 0004
//! section 5). A submission refused on its content, or after the project's
//! start was approved, records nothing.

use std::path::Path;

use baley_core::scope::{ChangeKind, ChangeSubmission, Submission, judge_change, judge_submission};
use baley_store::{CommandKind, Hash, ProjectId, RequestId, request_digest};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed};
use crate::mcp::client::MAX_ECHOED_BYTES;
use crate::mcp::handler::Preparation;
use crate::mcp::prepare::{LEDGER_UNAVAILABLE, WriteRequest, prepare_checked};
use crate::scope::submit::{self, SUBMIT_COMMAND};

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `scope-submit` takes beside its `operation`. Unknown fields
/// are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubmitShape {
    /// A fresh UUID for this submission in lowercase hyphenated form. A retry
    /// of the same submission sends the same id.
    pub request_id: String,
    /// The whole start submission: `description` with `what`, `core_value`
    /// and `constraints`; `stories`, each with a `label`, a `sentence` and a
    /// `status` of `active`, `deferred` or `excluded`; and `roadmap`, the
    /// phases in order, each with a `name`, a `goal`, a `detail`,
    /// `depends_on` as positions in `roadmap` counted from 1, and the
    /// `stories` it delivers by label.
    pub submission: Value,
}

/// The tagged form a call's arguments arrive in, so `operation` is read and
/// left out of the shape.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum SubmitCall {
    #[serde(rename = "scope-submit")]
    Submit(SubmitShape),
}

/// One request shape per change operation, each taking a `request_id` and a
/// `submission`, and the tagged form their calls arrive in. A spelling is
/// written once here, beside the kind it names.
macro_rules! change_shapes {
    ($($shape:ident, $spelling:literal, $kind:ident, $doc:literal, $submission:literal;)*) => {
        $(
            #[doc = $doc]
            #[derive(Debug, Deserialize, JsonSchema)]
            #[serde(deny_unknown_fields)]
            pub struct $shape {
                /// A fresh UUID for this change in lowercase hyphenated form.
                /// A retry of the same change sends the same id.
                pub request_id: String,
                #[doc = $submission]
                pub submission: Value,
            }
        )*

        /// The kinds a change operation names, in the order `baley_apply`
        /// lists them. Each operation is spelled as its kind.
        pub const CHANGE_KINDS: &[ChangeKind] = &[$(ChangeKind::$kind),*];

        #[derive(Deserialize)]
        #[serde(tag = "operation")]
        enum ChangeCall {
            $(
                #[serde(rename = $spelling)]
                $kind($shape),
            )*
        }

        impl ChangeCall {
            /// The kind the spelling names, the request id and the submission.
            fn parts(self) -> (ChangeKind, String, Value) {
                match self {
                    $(Self::$kind(shape) => (ChangeKind::$kind, shape.request_id, shape.submission),)*
                }
            }
        }
    };
}

change_shapes! {
    ProjectDescribeShape, "project-describe", ProjectDescribe,
    "The arguments `project-describe` takes beside its `operation`. Unknown fields are refused.",
    "The revised project description, as a start's `description` holds it: `what` and `core_value`, each text, and `constraints`, a list of text. Baley adds the change's kind.";
    PhaseDeclareShape, "phase-declare", PhaseDeclare,
    "The arguments `phase-declare` takes beside its `operation`. Unknown fields are refused.",
    "The new phase: `name`, `goal` and `detail`, each text; `depends_on`, the numbers of the phases it depends on; `stories`, the ids of the stories it serves; and `position`, its place in the phase order counted from 1, last when left out. Phases are named by number and stories by id. Baley adds the change's kind.";
    PhaseEditShape, "phase-edit", PhaseEdit,
    "The arguments `phase-edit` takes beside its `operation`. Unknown fields are refused.",
    "The phase to change by its number in `phase`, and only the fields that change, at least one: `name`, `goal` or `detail` as text, `depends_on` as phase numbers and `stories` as story ids. Phases are named by number and stories by id. Baley adds the change's kind.";
    PhaseReorderShape, "phase-reorder", PhaseReorder,
    "The arguments `phase-reorder` takes beside its `operation`. Unknown fields are refused.",
    "`order`, the number of every phase in the roadmap's order, each once, in the new order. Phases are named by number. Baley adds the change's kind.";
    PhaseWithdrawShape, "phase-withdraw", PhaseWithdraw,
    "The arguments `phase-withdraw` takes beside its `operation`. Unknown fields are refused.",
    "`phase`, the number of the phase to withdraw, and `decisions`, one for each story it serves, each with the story's id in `story` and, in `to`, the number of the phase that serves it instead or `drop`. Phases are named by number and stories by id. Baley adds the change's kind.";
    StoryDeclareShape, "story-declare", StoryDeclare,
    "The arguments `story-declare` takes beside its `operation`. Unknown fields are refused.",
    "The new story: `sentence`, its text; `status`, `active`, `deferred` or `excluded`; and `phase`, the number of the phase that serves it, or null for the backlog. Only an active story is served by a phase. Phases are named by number. Baley adds the change's kind.";
    StoryEditShape, "story-edit", StoryEdit,
    "The arguments `story-edit` takes beside its `operation`. Unknown fields are refused.",
    "`story`, the id of the story to correct, and `sentence`, its corrected text. Stories are named by id. Baley adds the change's kind.";
    StoryReassignShape, "story-reassign", StoryReassign,
    "The arguments `story-reassign` takes beside its `operation`. Unknown fields are refused.",
    "`story`, the id of the story to move, and `phase`, the number of the phase that serves it now, or null for the backlog. Phases are named by number and stories by id. Baley adds the change's kind.";
    StoryDropShape, "story-drop", StoryDrop,
    "The arguments `story-drop` takes beside its `operation`. Unknown fields are refused.",
    "`story`, the id of the story to drop. Stories are named by id. Baley adds the change's kind.";
    BacklogReorderShape, "backlog-reorder", BacklogReorder,
    "The arguments `backlog-reorder` takes beside its `operation`. Unknown fields are refused.",
    "`order`, the id of every active and deferred story, each once, in the new priority order. Stories are named by id. Baley adds the change's kind.";
}

/// The change kind a `baley_apply` spelling names, if it is a change
/// operation.
pub fn change_kind(spelling: &str) -> Option<ChangeKind> {
    CHANGE_KINDS
        .iter()
        .copied()
        .find(|kind| kind.as_str() == spelling)
}

/// A change call whose arguments passed every check made before
/// preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedChange {
    /// The write preparation carries into the domain command.
    pub request: WriteRequest,
    /// The accepted change.
    pub change: ChangeSubmission,
}

/// A submission whose arguments passed every check made before preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedSubmission {
    /// The write preparation carries into the domain command.
    pub request: WriteRequest,
    /// The accepted submission.
    pub submission: Submission,
}

/// Judges `scope-submit`'s arguments before anything is prepared or
/// recorded: the shape, then the request id, then the submission whole. A
/// fault is the `refused` answer, which records nothing. A fault in the
/// submission is never `invalid-arguments` (D-08).
pub fn judge_arguments(arguments: &Value) -> Result<JudgedSubmission, Value> {
    let shape = match serde_json::from_value::<SubmitCall>(arguments.clone()) {
        Ok(SubmitCall::Submit(shape)) => shape,
        Err(error) => return Err(shape_refused(&error)),
    };
    let request_id = judge_request_id(shape.request_id)?;
    let submission = judge_submission(&shape.submission)
        .map_err(|refusal| submit::submission_refused(&refusal))?;
    Ok(JudgedSubmission {
        request: write_request(request_id, submission.digest()),
        submission,
    })
}

/// Judges a change operation's arguments before anything is prepared or
/// recorded, in `judge_arguments`' order: the shape, then the request id,
/// then the submission whole under the kind the spelling names. A fault is
/// the `refused` answer, which records nothing. A fault in the submission is
/// `malformed-submission`, never `invalid-arguments` (D-08), and a `kind`
/// the caller sent is one, since the spelling names the kind.
pub fn judge_change_arguments(arguments: &Value) -> Result<JudgedChange, Value> {
    let (kind, request_id, submission) =
        match serde_json::from_value::<ChangeCall>(arguments.clone()) {
            Ok(call) => call.parts(),
            Err(error) => return Err(shape_refused(&error)),
        };
    let request_id = judge_request_id(request_id)?;
    let change =
        judge_change(kind, &submission).map_err(|refusal| submit::submission_refused(&refusal))?;
    Ok(JudgedChange {
        request: write_request(request_id, change.digest()),
        change,
    })
}

/// The `invalid-arguments` answer for arguments that do not fit the shape.
fn shape_refused(error: &serde_json::Error) -> Value {
    Refusal::new("invalid-arguments", bounded(error.to_string()))
        .slot("arguments")
        .value()
}

/// The request id when it is written in the one spelling a retry repeats,
/// so a retry cannot miss its replay by changing case or wrapping.
fn judge_request_id(request_id: String) -> Result<RequestId, Value> {
    let canonical = uuid::Uuid::parse_str(&request_id)
        .map(|parsed| parsed.hyphenated().to_string())
        .ok();
    if canonical.as_deref() != Some(request_id.as_str()) {
        return Err(Refusal::new(
            "invalid-arguments",
            "request_id must be a UUID in lowercase hyphenated form, such as 9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f. A retry of the same submission sends the same id",
        )
        .slot("request_id")
        .value());
    }
    Ok(RequestId(request_id))
}

/// The write a scope submission prepares. The draft digest stands for the
/// content, and for a change its kind too, so the same content under the same
/// id replays and other content, or another kind, is a reused id.
fn write_request(request_id: RequestId, draft: Hash) -> WriteRequest {
    let digest = request_digest(&json!({
        "command": SUBMIT_COMMAND,
        "submission": draft.to_hex(),
    }))
    .expect("text is canonical");
    WriteRequest {
        kind: CommandKind(SUBMIT_COMMAND.into()),
        request_id,
        digest,
    }
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

/// Serves one `scope-submit` call for the session's `preparation`: the
/// arguments are judged first, then the write is prepared with the
/// start-once read as its own check, so a start already approved ends the
/// call before checkout admission and the policy step. Then the submission
/// runs in its own transaction. An answer that ends preparation is returned
/// as it is.
pub fn serve(arguments: &Value, preparation: &Preparation) -> Value {
    let judged = match judge_arguments(arguments) {
        Ok(judged) => judged,
        Err(refusal) => return refusal,
    };
    let ledger = preparation.ledger.as_deref();
    // With no ledger, preparation fails at the project check, before any
    // settings are read from the config folder or the check is asked.
    let config = ledger.map_or(Path::new(""), |ledger| ledger.config.as_path());
    let check = |project: &ProjectId| match ledger {
        Some(ledger) => submit::start_once(&ledger.store, project),
        None => Err(failed(
            LEDGER_UNAVAILABLE,
            "the write was checked without a ledger",
            PLACE_LEDGER,
        )),
    };
    let prepared = match prepare_checked(
        ledger.map(|ledger| &ledger.store),
        &preparation.caller,
        &judged.request,
        config,
        preparation.host,
        &preparation.at,
        &mut crate::process::System,
        &check,
    ) {
        Ok(prepared) => prepared,
        Err(answer) => return answer,
    };
    let (Some(ledger), Some(command)) = (ledger, prepared.command) else {
        return failed(
            LEDGER_UNAVAILABLE,
            "the write was prepared without a ledger or a command",
            PLACE_LEDGER,
        );
    };
    submit::answer(
        submit::record(&ledger.store, &command, &judged.submission),
        &ledger.store,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::fixtures::{R1, m, s1};

    fn arguments(request_id: &str, submission: Value) -> Value {
        json!({"operation": "scope-submit", "request_id": request_id,
            "submission": submission})
    }

    fn refused(arguments: &Value) -> Value {
        let refusal = judge_arguments(arguments).expect_err("refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        refusal
    }

    #[test]
    fn a_request_id_in_another_spelling_or_an_unknown_argument_accepted_is_caught() {
        let refusal = refused(&arguments("9D0C1B7E-2F4A-4B6C-8D1E-3A5B7C9D0E2F", m()));
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "request_id");

        let mut extra = arguments(R1, m());
        extra["owner"] = json!("Ada Owner");
        let refusal = refused(&extra);
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "arguments");
    }

    #[test]
    fn an_unknown_argument_name_echoed_at_any_length_is_caught() {
        // Three bytes a character, so the cut cannot land on a whole one by luck.
        let mut extra = arguments(R1, m());
        extra["名".repeat(10_000)] = json!(true);
        let refusal = refused(&extra);
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "arguments");
        let reason = refusal["reason"].as_str().expect("a reason");
        assert!(reason.len() <= MAX_ECHOED_BYTES, "{} bytes", reason.len());
    }

    #[test]
    fn a_malformed_submission_answered_as_invalid_arguments_is_caught() {
        let refusal = refused(&arguments(R1, json!({"description": 1})));
        assert_eq!(refusal["code"], "malformed-submission");
        assert_eq!(refusal["slot"], "submission");
        assert_eq!(
            refusal["details"],
            json!({"section": "description", "field": "description"})
        );
    }

    #[test]
    fn a_write_request_digest_that_ignores_the_submission_is_caught() {
        let judged = |submission: Value| judge_arguments(&arguments(R1, submission)).unwrap();
        let minimal = judged(m());
        assert_ne!(minimal.request.digest, judged(s1()).request.digest);
        let reversed = json!({
            "roadmap": [{"stories": ["s"], "depends_on": [], "detail": "d", "goal": "g",
                "name": "n"}],
            "stories": [{"status": "active", "sentence": "t", "label": "s"}],
            "description": {"constraints": [], "core_value": "v", "what": "w"}});
        assert_eq!(judged(reversed).request.digest, minimal.request.digest);
        assert_eq!(minimal.request.kind, CommandKind("scope.submit".into()));
        assert_eq!(minimal.request.request_id, RequestId(R1.into()));
    }

    fn change_call(operation: &str, request_id: &str, submission: Value) -> Value {
        json!({"operation": operation, "request_id": request_id, "submission": submission})
    }

    fn change_refused(arguments: &Value) -> Value {
        let refusal = judge_change_arguments(arguments).expect_err("refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        refusal
    }

    #[test]
    fn a_change_operation_judged_as_another_kind_or_taking_a_sent_kind_is_caught() {
        let digest = |operation| {
            judge_change_arguments(&change_call(operation, R1, json!({"order": [2, 1]})))
                .unwrap()
                .change
                .digest()
                .to_hex()
        };
        assert_eq!(
            digest("phase-reorder"),
            "5c4b7650561455ec8d35bfe5be8bdca5b426260fb4ada5934e9a2b0ea0de6619"
        );
        assert_eq!(
            digest("backlog-reorder"),
            "cd1d1badf514e109dbb4cf81d97a87ec909b603aabd5876bba434c560c084159"
        );

        let sent = json!({"kind": "phase-reorder", "order": [2, 1]});
        let refusal = change_refused(&change_call("phase-reorder", R1, sent));
        assert_eq!(refusal["code"], "malformed-submission");
        assert_eq!(refusal["slot"], "submission");
        assert_eq!(
            refusal["details"],
            json!({"section": "kind", "field": "kind"})
        );
    }

    #[test]
    fn a_change_argument_in_another_form_accepted_is_caught() {
        let order = || json!({"order": [2, 1]});
        let mut extra = change_call("phase-reorder", R1, order());
        extra["extra"] = json!(true);
        let refusal = change_refused(&extra);
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "arguments");

        let upper = change_call(
            "phase-reorder",
            "9D0C1B7E-2F4A-4B6C-8D1E-3A5B7C9D0E2F",
            order(),
        );
        let refusal = change_refused(&upper);
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "request_id");

        let none = json!({"operation": "phase-reorder", "request_id": R1});
        let refusal = change_refused(&none);
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "arguments");
    }

    #[test]
    fn a_change_request_digest_that_ignores_the_change_is_caught() {
        let judged = |operation, order: Value| {
            judge_change_arguments(&change_call(operation, R1, json!({"order": order})))
                .unwrap()
                .request
        };
        let first = judged("phase-reorder", json!([2, 1]));
        let again = judged("phase-reorder", json!([2, 1]));
        let changed = judged("phase-reorder", json!([1, 2]));
        let backlog = judged("backlog-reorder", json!([2, 1]));
        for request in [&first, &again, &changed, &backlog] {
            assert_eq!(request.kind, CommandKind("scope.submit".into()));
            assert_eq!(request.request_id, RequestId(R1.into()));
        }
        assert_eq!(first.digest, again.digest);
        assert_ne!(changed.digest, first.digest);
        assert_ne!(backlog.digest, first.digest);
    }
}
