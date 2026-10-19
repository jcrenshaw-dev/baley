//! The work-order operations on `baley_apply` (design 0004 section 4,
//! design 0012). `project-start` records one `work_order.issued` for the
//! start planner on the `roadmap` stream, with the brief as a `record`
//! payload, after the arguments, the start check and preparation pass. A
//! refusal records nothing.

use std::num::NonZeroU32;
use std::path::Path;

use baley_core::capture::is_blank;
use baley_core::policy::{EffectivePolicy, Layer, Role};
use baley_core::work_order::{START_ORDER_PENDING, START_SUBJECT};
use baley_store::{
    CommandKind, Hash, InstructionEvidence, Ledger, ProjectId, RequestId, ServerCaller, Views,
    request_digest,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::checkout::{holds_existing_code, tracked_files};
use crate::discovery::PROJECT_FILE;
use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::client::MAX_ECHOED_BYTES;
use crate::mcp::handler::Preparation;
use crate::mcp::prepare::{
    CHECKOUT_FACTS_UNAVAILABLE, LEDGER_UNAVAILABLE, Prepared, Stepped, WriteRequest,
    prepare_checked,
};
use crate::process::Process;
use crate::scope::submit::{NOT_RECORDED_BUSY, start_once};
use crate::work_order::issue::{self, Input, Issue, start_absences};
use crate::work_order::read::pending_start;
use crate::work_order::route::{compose, host_names};

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

/// Project start's check, run by preparation after the replay lookup and
/// before the settings, recording nothing: an approved start is
/// `project-already-started` (D-19), and a pending start planner order is
/// `start-order-pending`, naming it (D-02). A read fault is the store
/// error's `failed` answer.
pub fn start_check<S: Ledger + Views + ?Sized>(
    store: &S,
    project: &ProjectId,
) -> Result<(), Value> {
    start_once(store, project)?;
    let pending =
        pending_start(store, project).map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))?;
    let Some(order) = pending else {
        return Ok(());
    };
    let id = order.get("id").and_then(Value::as_str).unwrap_or_default();
    Err(Refusal::new(
        START_ORDER_PENDING,
        format!(
            "the start planner's work order {id} is still pending, so no other start was recorded. Read that order with `document`, or fail it with `work-order-fail` and retry it with `work-order-retry`"
        ),
    )
    .slot("operation")
    .details(json!({"work_order": id}))
    .value())
}

/// The lines a start's answer carries when settings are missing (D-18):
/// one when `policy`, the host's effective policy, has no global file, and
/// one when no setting comes from the project file. Each says the defaults
/// apply and names `baley config interview`; neither runs it.
pub fn settings_lines(policy: &EffectivePolicy) -> Vec<String> {
    let mut lines = Vec::new();
    if policy.global.is_none() {
        lines.push(
            "There is no global settings file, so Baley's defaults apply. The owner can run `baley config interview` to choose settings".to_string(),
        );
    }
    let from_project = policy
        .settings
        .values()
        .any(|setting| matches!(setting.source.layer, Layer::Project | Layer::ProjectHost));
    if !from_project {
        lines.push(
            "The project file sets nothing beyond [project], so Baley's defaults apply to this project. The owner can run `baley config interview` to choose settings".to_string(),
        );
    }
    lines
}

/// The instruction a start order carries for its planner.
const START_PLANNER: &str = "bal-start-planner";

/// The instruction a start order adds when the checkout holds code.
const START_SURVEY: &str = "bal-start-survey";

/// Serves one `project-start` call for the session's `preparation`. The
/// arguments are judged first, and a refusal is answered before anything is
/// prepared. Then the write is prepared under the caller with the
/// instruction evidence attached, with [`start_check`] as its own check, so
/// a start already approved or a start order still pending ends the call
/// before checkout admission and the policy step.
///
/// For a write that ran its steps, the settings lines come from the
/// prepared policy, and the checkout is judged for existing code from the
/// root commit and, when there is one, HEAD's listing. The order carries the
/// start planner's instruction, then the survey's when the checkout holds
/// code (D-20, D-24), and runs under the route for `planner` at attempt 1 at
/// the step's policy version (D-11). A fresh UUID is both its id and its
/// request, and the brief is its one input. A replay goes to the recording
/// with no order, and the store answers it from its record. No
/// acknowledgement is checked (D-12) and no `forge.checked` is recorded
/// (D-21).
pub fn serve_start(arguments: &Value, preparation: &Preparation) -> Value {
    let judged = match judge_start_arguments(arguments) {
        Ok(judged) => judged,
        Err(refusal) => return refusal,
    };
    let ledger = preparation.ledger.as_deref();
    // With no ledger, preparation fails at the project check, before any
    // settings are read from the config folder or the check is asked.
    let config = ledger.map_or(Path::new(""), |ledger| ledger.config.as_path());
    let check = |project: &ProjectId| match ledger {
        Some(ledger) => start_check(&ledger.store, project),
        None => Err(failed(
            LEDGER_UNAVAILABLE,
            "the write was checked without a ledger",
            PLACE_LEDGER,
        )),
    };
    let caller = evidenced(&preparation.caller, judged.instruction.as_ref());
    let mut process = crate::process::System;
    let prepared = match prepare_checked(
        ledger.map(|ledger| &ledger.store),
        &caller,
        &judged.request,
        config,
        preparation.host,
        &preparation.at,
        &mut process,
        &check,
    ) {
        Ok(prepared) => prepared,
        Err(answer) => return answer,
    };
    let (Some(ledger), Some(command)) = (ledger, prepared.command.as_ref()) else {
        return failed(
            LEDGER_UNAVAILABLE,
            "the write was prepared without a ledger or a command",
            PLACE_LEDGER,
        );
    };
    let store = &ledger.store;
    let Some(stepped) = &prepared.stepped else {
        return issue::answer(issue::record(store, command, None), store);
    };
    let existing = match existing_code(&prepared, stepped, &mut process) {
        Ok(existing) => existing,
        Err(text) => return failed(CHECKOUT_FACTS_UNAVAILABLE, text, "git"),
    };
    let mut instructions = Vec::new();
    for identity in std::iter::once(START_PLANNER).chain(existing.then_some(START_SURVEY)) {
        match crate::instruction::evidence(identity) {
            Ok(evidence) => instructions.push(evidence),
            Err(refusal) => return refusal,
        }
    }
    let names = match host_names(store, preparation.host) {
        Ok(names) => names,
        Err(error) => return store_failed(&error, NOT_RECORDED_BUSY),
    };
    let route = match compose(
        &stepped.policy,
        command.policy_version,
        preparation.host,
        Role::Planner,
        NonZeroU32::MIN,
        &names,
    ) {
        Ok(route) => route,
        Err(refusal) => return Refusal::new(refusal.code(), refusal.to_string()).value(),
    };
    let lines = settings_lines(&stepped.policy);
    let mut further = Map::new();
    if !lines.is_empty() {
        further.insert("settings".into(), json!(lines));
    }
    further.insert("existing_code".into(), json!(existing));
    let id = uuid::Uuid::new_v4().to_string();
    let inputs = [Input::Bytes {
        name: "brief",
        bytes: judged.brief.as_bytes(),
    }];
    let absences = start_absences();
    let issue = Issue {
        id: &id,
        request: &id,
        parent: None,
        subject: START_SUBJECT,
        route: &route,
        instructions: &instructions,
        inputs: &inputs,
        absences: &absences,
        streams: &[],
        further: &further,
    };
    issue::answer(issue::record(store, command, Some(&issue)), store)
}

/// Whether the prepared checkout holds existing code. HEAD's files are
/// listed only when it has a root commit, so an unborn HEAD runs no git.
fn existing_code(
    prepared: &Prepared,
    stepped: &Stepped,
    process: &mut dyn Process,
) -> Result<bool, String> {
    let Some(root_commit) = stepped.root_commit.as_deref() else {
        return Ok(false);
    };
    let listing = tracked_files(&prepared.root, process)?;
    let file = prepared
        .project_file
        .strip_prefix(&prepared.root)
        .unwrap_or(Path::new(PROJECT_FILE))
        .to_string_lossy();
    Ok(holds_existing_code(Some(root_commit), &listing, &file))
}

/// The caller a start is prepared under: `caller` with `instruction`'s
/// evidence attached, or as it is when the session named none, as `capture`
/// attaches it, so every event of the call carries it.
fn evidenced(caller: &ServerCaller, instruction: Option<&InstructionEvidence>) -> ServerCaller {
    match instruction {
        None => caller.clone(),
        Some(evidence) => caller
            .clone()
            .with_instructions(vec![evidence.clone()])
            .expect("one entry is within the caller's instruction limit"),
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const R1: &str = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";
    pub(crate) const B1: &str = "Build a ledger for agent work";
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

    fn policy(global: Option<&str>, project: &str) -> EffectivePolicy {
        use std::path::Path;

        use baley_core::policy::{Host, Schema, effective_policy};
        let global =
            global.map(|text| crate::settings::file(Path::new("/c/config.toml"), text.into()));
        let text = format!("[project]\nid = \"{ID}\"\nname = \"r\"\n{project}");
        let project = crate::settings::file(Path::new("/real/r/baley.toml"), text.into_bytes());
        effective_policy(
            Schema::standard(),
            Some(Host::ClaudeCode),
            global.as_ref(),
            Some(&project),
        )
        .unwrap()
    }

    const ID: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";
    const EFFORT_MAX: &str = "[roles.planner]\neffort = \"max\"\n";

    fn about(lines: &[String]) -> Vec<&'static str> {
        lines
            .iter()
            .map(|line| {
                assert!(line.contains("baley config interview"), "{line}");
                if line.contains("global settings file") {
                    "global"
                } else if line.contains("project file") {
                    "project"
                } else {
                    panic!("a line about neither file: {line}")
                }
            })
            .collect()
    }

    #[test]
    fn a_settings_line_shown_beside_settings_or_missing_without_them_is_caught() {
        let cases = [
            (None, "", vec!["global", "project"]),
            (Some(EFFORT_MAX), "", vec!["project"]),
            (None, "[roles.planner]\neffort = \"low\"\n", vec!["global"]),
            (
                None,
                "[host.claude-code]\nescalate_on_failure = true\n",
                vec!["global"],
            ),
            (
                Some(EFFORT_MAX),
                "[roles.planner]\neffort = \"low\"\n",
                vec![],
            ),
        ];
        for (global, project, expected) in cases {
            let lines = settings_lines(&policy(global, project));
            assert_eq!(about(&lines), expected, "{global:?} {project:?}: {lines:?}");
        }
    }

    /// Records a pending start planner order `id` in `project` under
    /// `request_id` at `at`, as project start records one.
    pub(crate) fn issue_start(
        store: &baley_store_sqlite::SqliteStore,
        project: &ProjectId,
        id: &str,
        request_id: &str,
        at: &str,
    ) {
        use std::collections::BTreeSet;
        use std::num::NonZeroU32;

        use baley_core::policy::{AcceptedNames, Host, Role, Schema, effective_policy};
        use baley_core::work_order::START_SUBJECT;

        use crate::scope::fixtures::command_in;
        use crate::work_order::issue::{self, Input, Issue, start_absences};

        let policy =
            effective_policy(Schema::standard(), Some(Host::ClaudeCode), None, None).unwrap();
        let names = AcceptedNames {
            names: BTreeSet::new(),
            version: 0,
        };
        let attempt = NonZeroU32::new(1).unwrap();
        let route = crate::work_order::route::compose(
            &policy,
            1,
            Host::ClaudeCode,
            Role::Planner,
            attempt,
            &names,
        )
        .unwrap();
        let absences = start_absences();
        let further = serde_json::Map::new();
        let inputs = [Input::Bytes {
            name: "brief",
            bytes: B1.as_bytes(),
        }];
        let issue = Issue {
            id,
            request: id,
            parent: None,
            subject: START_SUBJECT,
            route: &route,
            instructions: &[],
            inputs: &inputs,
            absences: &absences,
            streams: &[],
            further: &further,
        };
        let command = command_in(project, START_COMMAND, request_id, 1, at);
        let answer = issue::answer(issue::record(store, &command, Some(&issue)), store);
        assert_eq!(answer["status"], "ok", "{answer}");
    }

    pub(crate) const I0: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";

    #[test]
    fn a_start_check_passing_an_approved_start_or_a_pending_order_is_caught() {
        use baley_core::work_order::{WORK_ORDER_FAILED, failed_payload};
        use baley_store::{NewEvent, StreamName};

        use crate::scope::fixtures::*;

        let (_dir, store) = two_projects();
        assert_eq!(start_check(&store, &project()), Ok(()));

        issue_start(&store, &project(), I0, R1, T1);
        let refusal = start_check(&store, &project()).unwrap_err();
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "start-order-pending", "{refusal}");
        assert_eq!(refusal["slot"], "operation", "{refusal}");
        assert_eq!(refusal["details"], json!({"work_order": I0}), "{refusal}");

        let digest = receipt(&submit_in(&store, &project_b(), &m(), R1, 1, T1).unwrap())["digest"]
            .as_str()
            .and_then(Hash::from_hex)
            .unwrap();
        approve_draft_in(&store, &project_b(), digest);
        let refusal = start_check(&store, &project_b()).unwrap_err();
        assert_eq!(refusal["code"], "project-already-started", "{refusal}");

        append(
            &store,
            R2,
            T4,
            vec![NewEvent {
                stream: StreamName("roadmap".into()),
                type_name: WORK_ORDER_FAILED.into(),
                type_version: 1,
                git: None,
                payload: failed_payload(I0, "the planner stopped"),
                attachments: vec![],
            }],
        );
        assert_eq!(start_check(&store, &project()), Ok(()));
    }
}
