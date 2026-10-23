//! `baley story truths approve <digest>`: the owner approves a story's
//! truth draft by its digest at the terminal, the only way truths are
//! approved (design 0005 section 5, D-06, ADR 0044).
//!
//! Gathering finds the project and opens the ledger; the approval's two
//! steps are `refine::approve`'s, and what is printed is judged by the pure
//! render below, every text from the ledger printed through `escaped`, so a
//! recorded line break cannot forge a line.

use serde_json::Value;

use crate::ledger::clock::SystemClock;
use crate::ledger::commands::new_request_id;
use crate::ledger::display::Render;
use crate::providers::identity;
use crate::refine::approve;
use crate::scope::command::{
    admitted, approve_command, checkout, escaped, judge_digest, open as open_ledger,
};
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
}
