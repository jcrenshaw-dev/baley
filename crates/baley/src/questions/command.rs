//! The owner's `baley questions` commands (design 0005 `questions round`,
//! ADR 0044): `baley questions round <set>` prints a set's round through
//! the same read as `document`, recording nothing.
//!
//! Gathering finds the project and opens the ledger through the scope
//! commands' own steps; what is printed is worked out by the pure functions
//! below. Every text from the ledger is printed escaped, so a recorded line
//! break or terminal escape cannot forge a line.

use std::process::ExitCode;

use baley_core::questions::{Pending, Round, SetState};
use baley_store::StoreError;

use crate::ledger::display::{self, Render};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::questions::read::{self, NO_SUCH_QUESTION_SET, SetRead};
use crate::scope::command::{emit, escaped, reading};

/// `baley questions`: the owner's read of a question set's rounds.
#[derive(clap::Args, Debug)]
pub struct QuestionsArgs {
    /// The questions command to run.
    #[command(subcommand)]
    pub command: QuestionsCommand,
}

/// One `baley questions` command.
#[derive(clap::Subcommand, Debug, PartialEq, Eq)]
pub enum QuestionsCommand {
    /// Print a question set's current round with each recommended answer,
    /// what each waiting question waits on, and every answer and deferral
    /// recorded. Nothing is recorded.
    Round {
        /// The set's id, as its round read or an answer's receipt gives it.
        #[arg(value_name = "SET")]
        set: String,
    },
}

/// Runs one `baley questions` command and prints its result.
pub fn run(args: QuestionsArgs) -> ExitCode {
    emit(match args.command {
        QuestionsCommand::Round { set } => {
            reading(|store, project| round_render(read::read(store, project, &set)))
        }
    })
}

/// What a round read prints: the set's lines, exit 0; a set this project
/// does not hold as `no-such-question-set`, exit 2, echoing none of the id;
/// an unreadable record, exit 1.
pub(crate) fn round_render(read: Result<SetRead, StoreError>) -> Render {
    match read {
        Ok(SetRead::Held(held)) => Render {
            lines: round_lines(&held.round),
            code: 0,
            error: false,
        },
        Ok(SetRead::NotHeld) => Render::refusal(format!(
            "{NO_SUCH_QUESTION_SET}: this project holds no question set with that id"
        )),
        Ok(SetRead::Unreadable(unreadable)) => Render {
            lines: vec![format!("{LEDGER_UNAVAILABLE}: {}", unreadable.0)],
            code: 1,
            error: true,
        },
        Err(error) => display::store_error(&error, None),
    }
}

/// The lines of one round: the set's state and round number, the questions
/// to ask with their recommended answers, each waiting question with what it
/// waits on, each answer marked as the recommendation or the owner's own,
/// each deferral with its reason, and an abandonment's cause.
pub(crate) fn round_lines(round: &Round) -> Vec<String> {
    let mut lines = vec![match round.state {
        SetState::Open => format!("question set open, round {}", round.number),
        SetState::Closed => "question set closed: every question is answered or deferred".into(),
        SetState::Abandoned => "question set abandoned: it takes no answer".into(),
    }];
    if let Some(abandoned) = &round.abandoned {
        lines.push(format!(
            "abandoned {} at {}",
            abandoned.cause.as_str(),
            escaped(&abandoned.at)
        ));
    }
    for asked in &round.current {
        lines.push(format!(
            "ask {}: {} (recommended: {})",
            escaped(&asked.id),
            escaped(&asked.question),
            escaped(&asked.recommended)
        ));
    }
    for waiting in &round.waiting {
        let on: Vec<String> = waiting
            .on
            .iter()
            .map(|(id, pending)| {
                let state = match pending {
                    Pending::Open => "open",
                    Pending::Deferred => "deferred",
                };
                format!("{} {state}", escaped(id))
            })
            .collect();
        lines.push(format!(
            "waiting {}: {} (waits on {})",
            escaped(&waiting.id),
            escaped(&waiting.question),
            on.join(", ")
        ));
    }
    for answer in &round.answered {
        let whose = if answer.accepted {
            "took the recommendation"
        } else {
            "the owner's own answer"
        };
        lines.push(format!(
            "answered {} in round {} by {} at {}: {} ({whose})",
            escaped(&answer.question),
            answer.round,
            escaped(&answer.owner),
            escaped(&answer.at),
            escaped(&answer.answer)
        ));
    }
    for deferral in &round.deferred {
        lines.push(format!(
            "deferred {} in round {} by {} at {}: {}",
            escaped(&deferral.question),
            deferral.round,
            escaped(&deferral.owner),
            escaped(&deferral.at),
            escaped(&deferral.reason)
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    //! Rounds are written out by hand from design 0005 and the plan's set
    //! R2; expected lines are checked for what they must show, never
    //! produced by the code under test.

    use baley_core::questions::{Abandonment, Answer, Asked, Cause, Deferral, Waiting};

    use super::*;

    const OWNER: &str = "Ada Lovelace";
    const AT: &str = "2026-10-22T09:00:01Z";

    fn asked(id: &str, question: &str, recommended: &str) -> Asked {
        Asked {
            id: id.into(),
            question: question.into(),
            recommended: recommended.into(),
        }
    }

    fn answer(id: &str, text: &str, accepted: bool) -> Answer {
        Answer {
            question: id.into(),
            answer: text.into(),
            accepted,
            round: 1,
            owner: OWNER.into(),
            at: AT.into(),
        }
    }

    fn deferral(id: &str, reason: &str) -> Deferral {
        Deferral {
            question: id.into(),
            reason: reason.into(),
            round: 1,
            owner: OWNER.into(),
            at: AT.into(),
        }
    }

    fn round(state: SetState) -> Round {
        Round {
            number: 2,
            state,
            current: Vec::new(),
            waiting: Vec::new(),
            answered: Vec::new(),
            deferred: Vec::new(),
            abandoned: None,
        }
    }

    fn raw(lines: &[String]) -> bool {
        lines
            .iter()
            .any(|line| line.contains('\n') || line.contains('\u{1b}'))
    }

    fn holding<'a>(lines: &'a [String], needles: &[&str]) -> Option<&'a String> {
        lines
            .iter()
            .find(|line| needles.iter().all(|needle| line.contains(needle)))
    }

    #[test]
    fn a_round_printed_with_a_raw_line_break_or_missing_a_question_is_caught() {
        let answered = Round {
            current: vec![
                asked("q2", "Do sessions share one ledger?", "\u{1b}[2J"),
                asked("q3", "Where does the ledger live?", "Outside the checkout"),
            ],
            answered: vec![answer("q1", "SQLite\nphase: 9", false)],
            ..round(SetState::Open)
        };
        let lines = round_lines(&answered);
        assert!(!raw(&lines), "{lines:?}");
        assert!(holding(&lines, &["round 2"]).is_some(), "{lines:?}");
        assert!(
            holding(
                &lines,
                &["q2", "Do sessions share one ledger?", "\\u{1b}[2J"]
            )
            .is_some(),
            "{lines:?}"
        );
        assert!(
            holding(
                &lines,
                &["q3", "Where does the ledger live?", "Outside the checkout"]
            )
            .is_some(),
            "{lines:?}"
        );
        assert!(
            holding(
                &lines,
                &["q1", "SQLite\\nphase: 9", "the owner's own answer"]
            )
            .is_some(),
            "{lines:?}"
        );

        let deferred = Round {
            current: vec![asked(
                "q2",
                "Do sessions share one ledger?",
                "Yes, one per user",
            )],
            waiting: vec![Waiting {
                id: "q3".into(),
                question: "Where does the ledger live?".into(),
                on: vec![("q1".into(), Pending::Deferred)],
            }],
            deferred: vec![deferral("q1", "after\nthe spike")],
            ..round(SetState::Open)
        };
        let lines = round_lines(&deferred);
        assert!(!raw(&lines), "{lines:?}");
        assert!(
            holding(&lines, &["waiting q3", "q1 deferred"]).is_some(),
            "{lines:?}"
        );
        assert!(
            holding(&lines, &["deferred q1", "after\\nthe spike"]).is_some(),
            "{lines:?}"
        );
    }

    #[test]
    fn a_closed_or_abandoned_set_printed_as_open_is_caught() {
        let lines = round_lines(&round(SetState::Closed));
        assert!(holding(&lines, &["closed"]).is_some(), "{lines:?}");
        assert!(holding(&lines, &["open"]).is_none(), "{lines:?}");

        let abandoned = Round {
            answered: vec![answer("q1", "SQLite", true)],
            deferred: vec![deferral("q2", "after the spike")],
            abandoned: Some(Abandonment {
                cause: Cause::Superseded,
                at: AT.into(),
            }),
            ..round(SetState::Abandoned)
        };
        let lines = round_lines(&abandoned);
        assert!(holding(&lines, &["abandoned"]).is_some(), "{lines:?}");
        assert!(holding(&lines, &["superseded"]).is_some(), "{lines:?}");
        assert!(
            holding(&lines, &["answered q1", "SQLite"]).is_some(),
            "{lines:?}"
        );
        assert!(
            holding(&lines, &["deferred q2", "after the spike"]).is_some(),
            "{lines:?}"
        );
    }

    #[test]
    fn an_unknown_set_printed_as_success_is_caught() {
        let render = round_render(Ok(SetRead::NotHeld));
        assert_eq!(render.code, 2);
        assert!(render.error);
        assert_eq!(render.lines.len(), 1);
        assert!(
            render.lines[0].starts_with("no-such-question-set: "),
            "{:?}",
            render.lines
        );
    }
}
