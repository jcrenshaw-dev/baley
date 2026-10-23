//! `baley story truths approve <digest>`: the owner approves a story's
//! truth draft by its digest at the terminal, the only way truths are
//! approved (design 0005 section 5, D-06, ADR 0044). `baley story
//! refinement <id>` prints a story's refinement through `refine::read`, the
//! read `context-intake` and `document` give, recording nothing (D-10).
//!
//! Gathering finds the project and opens the ledger; the approval's two
//! steps are `refine::approve`'s, and what is printed is judged by the pure
//! renders below, every text from the ledger printed through `escaped`, so a
//! recorded line break cannot forge a line.

use serde_json::Value;

use crate::ledger::clock::SystemClock;
use crate::ledger::commands::new_request_id;
use crate::ledger::display::Render;
use crate::providers::identity;
use crate::refine::approve;
use crate::refine::read::{self, StoryRefinement, VersionedTruth};
use crate::scope::command::{
    admitted, approve_command, checkout, describe, escaped, judge_digest, judge_number,
    open as open_ledger, printed, read_error, reading,
};
use crate::scope::read::ReadError;
use crate::scope::submit;

/// `baley story truths`: the owner's commands on a story's truths.
#[derive(clap::Args, Debug)]
pub struct TruthsArgs {
    #[command(subcommand)]
    pub(crate) command: TruthsCommand,
}

#[derive(clap::Subcommand, Debug)]
pub(crate) enum TruthsCommand {
    /// Approve a story's truth draft by its digest as the owner, recording
    /// the story's truths with their versions.
    Approve {
        /// The draft's digest, as its submission's receipt gives it.
        #[arg(value_name = "DIGEST")]
        digest: String,
    },
}

/// Runs one `baley story truths` command.
pub(crate) fn run(args: TruthsArgs) -> Result<Render, Render> {
    match args.command {
        TruthsCommand::Approve { digest } => approve(&digest),
    }
}

/// `story truths approve`, as `baley scope approve` runs a scope approval:
/// the digest is judged before anything is read; then the checkout and the
/// ledger; the approval's first step, whose refusal prints and records
/// nothing; checkout admission and the policy step; the owner over the
/// policy the step recorded; the owner's command; the second step; and the
/// answer printed.
pub(crate) fn approve(digest: &str) -> Result<Render, Render> {
    let digest = judge_digest(digest)?;
    let checkout = checkout()?;
    let store = open_ledger(&checkout)?;
    let project = &checkout.project;
    let judged =
        approve::judge(&store, project, digest).map_err(|answer| approval_render(&answer))?;
    let (policy, version) = admitted(&store, &checkout)?;

    let owner = identity::owner(identity::owner_name(&policy), &mut crate::process::System)
        .map_err(Render::refusal)?;
    let command = approve_command(
        project,
        version,
        digest,
        new_request_id(),
        &SystemClock::now(),
    )
    .map_err(|e| Render::refusal(e.to_string()))?;
    let recorded = approve::record(&store, &command, Some(&judged), &owner);
    Ok(approval_render(&submit::answer(recorded, &store)))
}

/// What a truth approval's answer prints: an `ok` receipt as the approval,
/// the story and its set, and one line for each truth's version, exit 0; a
/// refusal as `<code>: <reason>`, with the open questions for
/// `question-open`, the first differing part for `stale-draft` and the
/// earlier owner and time for `draft-approved`, exit 2; a failure the same
/// way, exit 1.
pub(crate) fn approval_render(answer: &Value) -> Render {
    let text = |value: &Value| escaped(value.as_str().unwrap_or_default());
    if answer["status"] == "ok" {
        let mut lines = vec![
            format!(
                "approved the {} draft {} as {} at {}",
                text(&answer["kind"]),
                text(&answer["digest"]),
                text(&answer["owner"]),
                text(&answer["at"])
            ),
            format!(
                "story {} refined from question set {}",
                answer["story"],
                text(&answer["set"])
            ),
        ];
        let truths = answer["truths"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        lines.extend(
            truths
                .iter()
                .map(|truth| format!("truth {}: version {}", text(&truth["id"]), truth["version"])),
        );
        return Render {
            lines,
            code: 0,
            error: false,
        };
    }
    let mut line = format!("{}: {}", text(&answer["code"]), text(&answer["reason"]));
    let details = &answer["details"];
    if let Some(questions) = details["questions"].as_array() {
        let open: Vec<String> = questions.iter().map(text).collect();
        line = format!("{line}: {}", open.join(", "));
    }
    if let Some(path) = details["path"].as_str() {
        line = format!("{line}: {}", escaped(path));
    }
    if let (Some(owner), Some(at)) = (details["owner"].as_str(), details["at"].as_str()) {
        line = format!("{line}: approved by {} at {}", escaped(owner), escaped(at));
    }
    match answer["status"].as_str() {
        Some("refused") => Render::refusal(line),
        _ => Render {
            lines: vec![line],
            code: 1,
            error: true,
        },
    }
}

/// `baley story refinement <id>`: the story id is judged before anything is
/// read; then the checkout and the ledger, only to read, and the story's
/// refinement printed. Nothing is recorded and no order is issued (D-10,
/// ADR 0044).
pub(crate) fn refinement(id: &str) -> Result<Render, Render> {
    let id = judge_number(id)?;
    reading(|store, project| refinement_render(read::read(store, project, id)))
}

/// What `baley story refinement` prints for a refinement read: the
/// description as `scope show` prints it; the story line and its `truths:`
/// line as `story show` prints them, but not its `phase:` line, so exactly
/// one line names the phase; the committed phase as `phase <n>: <name>`
/// with its goal and status, or `phase: backlog`; each current truth as
/// `truth <id> version <v>:` and its sentence, then a line with its kind,
/// its attestations and any human check; then `earlier versions:` and each
/// kept version that is not current the same way; or `truths: none` before
/// refinement. A refusal prints `<code>: <reason>`, exit 2, and a failure
/// exit 1. Every text from the ledger is printed through `escaped`.
pub(crate) fn refinement_render(read: Result<StoryRefinement, ReadError>) -> Render {
    let refinement = match read {
        Ok(refinement) => refinement,
        Err(error) => return read_error(error),
    };
    let mut lines = Vec::new();
    if let Some(description) = &refinement.description {
        describe(
            &mut lines,
            &description.what,
            &description.core_value,
            &description.constraints,
        );
    }
    let story = &refinement.story;
    lines.push(format!(
        "story {} ({}): {}",
        story.id,
        escaped(&story.status),
        escaped(&story.sentence)
    ));
    lines.push(format!("truths: {}", story.truths));
    match &refinement.phase {
        Some(phase) => lines.extend([
            format!("phase {}: {}", phase.number, escaped(&phase.name)),
            format!("  goal: {}", escaped(&phase.goal)),
            format!("  status: {}", escaped(&phase.status)),
        ]),
        None => lines.push("phase: backlog".into()),
    }
    let Some(truths) = &refinement.truths else {
        lines.push("truths: none".into());
        return printed(lines);
    };
    for truth in &truths.current {
        truth_lines(&mut lines, truth);
    }
    let earlier: Vec<&VersionedTruth> = truths
        .kept
        .iter()
        .filter(|kept| {
            !truths
                .current
                .iter()
                .any(|current| current.id == kept.id && current.version == kept.version)
        })
        .collect();
    if earlier.is_empty() {
        lines.push("earlier versions: none".into());
    } else {
        lines.push("earlier versions:".into());
        for truth in earlier {
            truth_lines(&mut lines, truth);
        }
    }
    printed(lines)
}

/// One truth's two lines: `truth <id> version <v>:` and its sentence, read
/// as the owner approved it, then its detail line.
pub(crate) fn truth_lines(lines: &mut Vec<String>, truth: &VersionedTruth) {
    lines.push(format!(
        "truth {} version {}: {}",
        escaped(&truth.id),
        truth.version,
        sentence(
            &truth.form,
            &truth.trigger,
            &truth.observer,
            &truth.verb,
            &truth.outcome
        )
    ));
    lines.push(truth_detail(
        &truth.kind,
        truth.observable,
        truth.fixed_oracle,
        truth.human_verify.as_deref(),
    ));
}

/// A truth's detail line: its kind, its two attestations and, when it has
/// one, the tool or service a person needs to prove it.
pub(crate) fn truth_detail(
    kind: &str,
    observable: bool,
    fixed_oracle: bool,
    human_verify: Option<&str>,
) -> String {
    let mut detail = format!(
        "  kind: {}, observable: {observable}, fixed oracle: {fixed_oracle}",
        escaped(kind)
    );
    if let Some(human_verify) = human_verify {
        detail.push_str(&format!(", human verify: {}", escaped(human_verify)));
    }
    detail
}

/// A truth's sentence in its form: `When <trigger>, <observer> <verb>
/// <outcome>.` or `For any <trigger>, <observer> <verb> <outcome>.`, each
/// part escaped.
pub(crate) fn sentence(
    form: &str,
    trigger: &str,
    observer: &str,
    verb: &str,
    outcome: &str,
) -> String {
    let opening = if form == "for-any" { "For any" } else { "When" };
    format!(
        "{opening} {}, {} {} {}.",
        escaped(trigger),
        escaped(observer),
        escaped(verb),
        escaped(outcome)
    )
}

#[cfg(test)]
mod tests {
    //! Expected lines are written out from design 0005 section 5 and the
    //! plan's data, never from running this code. No test opens a ledger or
    //! a checkout.

    use clap::Parser;
    use serde_json::json;

    use super::*;
    use crate::scope::command::{StoryArgs, StoryCommand};

    const X: &str = "c85deb14471b95d360f3d94b276751e67770ea1ce447597bce8cec83436e2bab";
    const S: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const T7: &str = "2026-10-13T09:00:07Z";

    fn receipt(truths: serde_json::Value) -> Value {
        json!({"status": "ok", "kind": "story-truths", "digest": X, "story": 1, "set": S,
            "owner": "Ada Owner", "at": T7, "truths": truths})
    }

    #[test]
    fn a_truth_approval_printed_without_its_versions_or_refusal_details_is_caught() {
        let ok = approval_render(&receipt(
            json!([{"id": "t1", "version": 1}, {"id": "t2", "version": 3}]),
        ));
        assert_eq!(
            ok.lines,
            [
                format!("approved the story-truths draft {X} as Ada Owner at {T7}"),
                format!("story 1 refined from question set {S}"),
                "truth t1: version 1".to_owned(),
                "truth t2: version 3".to_owned(),
            ]
        );
        assert_eq!(ok.code, 0);
        assert!(!ok.error);

        let open = approval_render(&json!({"status": "refused", "code": "question-open",
            "reason": "a question of the story's latest set is open, so the truths were not approved",
            "slot": "digest", "details": {"questions": ["q3", "q4"]}}));
        assert_eq!(open.lines.len(), 1, "{:?}", open.lines);
        for part in ["question-open", "q3", "q4"] {
            assert!(open.lines[0].contains(part), "{part}: {:?}", open.lines);
        }
        assert_eq!(open.code, 2);
        assert!(open.error);

        let forged = approval_render(&receipt(
            json!([{"id": "t1\nbaley: approved", "version": 1}]),
        ));
        assert_eq!(forged.lines.len(), 3, "{:?}", forged.lines);
        assert_eq!(forged.lines[2], "truth t1\\nbaley: approved: version 1");
    }

    /// The `baley story` part of the command line, over the library's own
    /// arguments.
    #[derive(Parser)]
    struct Line {
        #[command(subcommand)]
        top: Top,
    }

    #[derive(clap::Subcommand)]
    enum Top {
        Story(StoryArgs),
    }

    #[test]
    fn a_truths_approve_command_parsed_otherwise_is_caught() {
        let Top::Story(args) = Line::try_parse_from(["baley", "story", "truths", "approve", X])
            .expect("parsed")
            .top;
        let StoryCommand::Truths(TruthsArgs {
            command: TruthsCommand::Approve { digest },
        }) = args.command
        else {
            panic!("a truths approval: {:?}", args.command);
        };
        assert_eq!(digest, X);

        let usage = Line::try_parse_from(["baley", "story", "truths", "approve"])
            .err()
            .expect("a usage error");
        assert_eq!(
            usage.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );

        let refused = approve("ABC").expect_err("refused");
        assert_eq!(refused.code, 2);
        assert!(refused.error);
        assert!(
            refused.lines[0].contains("64 lowercase hexadecimal"),
            "{:?}",
            refused.lines
        );
    }

    #[test]
    fn a_refinement_read_that_records_issues_an_order_or_differs_by_surface_is_caught() {
        use crate::mcp::document::{DocumentShape, Identity, read as document_read};
        use crate::mcp::refine::{IntakeShape, intake_answer, judge_context_arguments};
        use crate::refine::order::pending_analyzer;
        use crate::refine::questions::tests::D;
        use crate::refine::read::tests::{approve as approve_truths, at, drafting_from_empty};
        use crate::refine::submit::tests::{t1, t2, truths_submitted};
        use crate::scope::fixtures::{R3, R4, T3, head, project};
        use crate::work_order::read::order_document;

        let (_dir, store, set) = drafting_from_empty();
        let a = truths_submitted(&store, 1, &set, &json!([t1(), t2()]), D, R3, T3);
        assert_eq!(a["status"], "ok", "{a}");
        let a_digest = a["digest"].as_str().unwrap().to_owned();
        approve_truths(&store, &a_digest, R4, T3);

        let order_seq = |store| {
            order_document(store, &project(), D)
                .unwrap()
                .map(|document| document.produced_seq)
        };
        let before = (
            head(&store),
            pending_analyzer(&store, &project(), 1).unwrap(),
            order_seq(&store),
        );

        let intake = intake_answer(
            &store,
            &project(),
            &IntakeShape {
                story: 1,
                part: None,
            },
        );
        let expected = json!({
            "status": "ok",
            "description": {"what": "A ledger for agent work",
                "core_value": "Nothing moves on an unproven claim",
                "constraints": ["One binary"]},
            "story": {"id": 1, "sentence": "Every step is recorded", "status": "active",
                "phase": 1, "truths": 2},
            "phase": {"number": 1, "name": "Records", "goal": "Steps are recorded",
                "status": "declared"},
            "truths": {"current": [at(t1(), 1), at(t2(), 1)],
                "kept": [at(t1(), 1), at(t2(), 1)]}
        });
        assert_eq!(intake, expected);

        let mut document = document_read(
            &store,
            &project(),
            &DocumentShape {
                identity: Identity::Refinement { story: 1 },
                part: None,
            },
        );
        let identity = document
            .as_object_mut()
            .unwrap()
            .remove("identity")
            .expect("an identity");
        assert_eq!(identity, json!({"kind": "refinement", "story": 1}));
        assert_eq!(document, intake);

        let printed = refinement_render(read::read(&store, &project(), 1));
        assert_eq!(
            printed.lines,
            [
                "what: A ledger for agent work",
                "core value: Nothing moves on an unproven claim",
                "constraint: One binary",
                "story 1 (active): Every step is recorded",
                "truths: 2",
                "phase 1: Records",
                "  goal: Steps are recorded",
                "  status: declared",
                "truth t1 version 1: When the owner approves a draft by a stale digest, the owner is refused stale-draft naming the first differing part.",
                "  kind: literal, observable: true, fixed oracle: true",
                "truth t2 version 1: For any order the answers of a set are recorded in, a ledger reader gets the same round.",
                "  kind: property, observable: true, fixed oracle: true",
                "earlier versions: none",
            ]
        );
        assert_eq!(printed.code, 0);

        let after = (
            head(&store),
            pending_analyzer(&store, &project(), 1).unwrap(),
            order_seq(&store),
        );
        assert_eq!(after, before);
        assert_eq!(after.1, None);

        let judged = judge_context_arguments(&json!({"operation": "context-submit",
            "request_id": "00000000-0000-4000-8000-0000000000aa", "work_order": D,
            "story": 1, "set": set, "truths": [t1(), t2()]}))
        .expect("accepted");
        assert_eq!(judged.submission.digest().to_hex(), a_digest);
        let judge = baley_core::truth::judge_truths(
            std::num::NonZeroU32::new(1).unwrap(),
            &set,
            &json!([t1(), t2()]),
        )
        .unwrap();
        assert_eq!(judge.digest().to_hex(), a_digest);
    }

    /// Story 1's refinement read as the owner would see it after T2 was
    /// revised to T2' with a human check: t1 at 1, t2 at 2, and t2 1 kept.
    fn refinement_of(t2_outcome: &str) -> StoryRefinement {
        use crate::refine::read::tests::at;
        use crate::refine::read::{CommittedPhase, Truths};
        use crate::refine::submit::tests::{t1, t2, with};
        use crate::scope::read::Story;

        let truth = |value: Value| -> VersionedTruth { serde_json::from_value(value).unwrap() };
        let revised = with(
            with(t2(), "outcome", t2_outcome),
            "human_verify",
            "a live endpoint",
        );
        StoryRefinement {
            description: None,
            story: Story {
                id: 1,
                sentence: "Every step is recorded".into(),
                status: "active".into(),
                phase: Some(1),
                truths: 2,
            },
            phase: Some(CommittedPhase {
                number: 1,
                name: "Records".into(),
                goal: "Steps are recorded".into(),
                status: "declared".into(),
            }),
            truths: Some(Truths {
                current: vec![truth(at(t1(), 1)), truth(at(revised.clone(), 2))],
                kept: vec![
                    truth(at(t1(), 1)),
                    truth(at(t2(), 1)),
                    truth(at(revised, 2)),
                ],
            }),
        }
    }

    #[test]
    fn a_refinement_printed_without_its_versions_or_forms_is_caught() {
        let printed = refinement_render(Ok(refinement_of(
            "the same round and the same waiting list",
        )));
        let lines = &printed.lines;
        assert_eq!(printed.code, 0);
        assert!(lines.contains(&"story 1 (active): Every step is recorded".to_owned()));
        assert!(lines.contains(&"truths: 2".to_owned()));
        let phases: Vec<&String> = lines.iter().filter(|l| l.starts_with("phase")).collect();
        assert_eq!(phases, ["phase 1: Records"], "{lines:?}");
        let t1_line = "truth t1 version 1: When the owner approves a draft by a stale digest, the owner is refused stale-draft naming the first differing part.";
        let t2_line = "truth t2 version 2: For any order the answers of a set are recorded in, a ledger reader gets the same round and the same waiting list.";
        let at = |line: &str| {
            lines
                .iter()
                .position(|l| l == line)
                .unwrap_or_else(|| panic!("{line} in {lines:?}"))
        };
        at(t1_line);
        let t2_at = at(t2_line);
        assert!(
            lines[t2_at + 1].contains("human verify: a live endpoint"),
            "{lines:?}"
        );
        let earlier = at("earlier versions:");
        assert!(earlier > t2_at, "{lines:?}");
        assert!(
            lines[earlier + 1].starts_with(
                "truth t2 version 1: For any order the answers of a set are recorded in, a ledger reader gets the same round."
            ),
            "{lines:?}"
        );
        assert_eq!(lines.len(), earlier + 3, "{lines:?}");

        let forged = refinement_render(Ok(refinement_of("the same round\nbaley: approved")));
        assert_eq!(forged.lines.len(), lines.len(), "{:?}", forged.lines);
        assert!(forged.lines.iter().all(|line| !line.contains('\n')));
        assert!(
            forged
                .lines
                .iter()
                .any(|line| line.contains("the same round\\nbaley: approved")),
            "{:?}",
            forged.lines
        );
    }

    #[test]
    fn a_story_refinement_command_parsed_otherwise_is_caught() {
        let Top::Story(args) = Line::try_parse_from(["baley", "story", "refinement", "1"])
            .expect("parsed")
            .top;
        let StoryCommand::Refinement { id } = args.command else {
            panic!("a refinement read: {:?}", args.command);
        };
        assert_eq!(id, "1");
        for id in ["0", "x"] {
            let refused = refinement(id).expect_err("refused");
            assert_eq!(refused.code, 2, "{id}");
            assert!(refused.error, "{id}");
            assert!(
                refused.lines[0].contains("whole numbers from 1"),
                "{id}: {:?}",
                refused.lines
            );
        }
    }
}
