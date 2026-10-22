//! The owner's `baley questions` commands (design 0005 `questions round`
//! and `questions answer`, ADR 0044): `baley questions round <set>` prints
//! a set's round through the same read as `document`, recording nothing,
//! and `baley questions answer <set>` records the owner's answers and
//! deferrals for one round through the same check and recording as
//! `questions-answer`, with the owner as actor and no caller, so the record
//! shows which answers the owner gave at the terminal.
//!
//! Gathering finds the project and opens the ledger through the scope
//! commands' own steps; what is printed is worked out by the pure functions
//! below. Every text from the ledger is printed escaped, so a recorded line
//! break or terminal escape cannot forge a line.

use std::process::ExitCode;

use baley_core::questions::{Round, judge_call};
use baley_store::{CanonicalError, Command, ProjectId, RequestId, StoreError, request_digest};
use serde_json::{Value, json};

use crate::ledger::clock::SystemClock;
use crate::ledger::commands::new_request_id;
use crate::ledger::display::{self, Render};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::providers::identity;
use crate::questions::answer::{self as answers, ANSWER_COMMAND, call_fault};
use crate::questions::read::{self, NO_SUCH_QUESTION_SET, SetRead, round_value};
use crate::scope::command::{
    admitted, answer_render, checkout, emit, escaped, open, owner_command, reading,
};

/// `baley questions`: the owner's read of a question set's rounds, and the
/// owner's answers to one.
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
    /// Record answers and deferrals for questions of the current round, as the
    /// owner. Every item is judged before anything is recorded, and one
    /// refused item records none.
    Answer {
        /// The set's id, as its round read or an answer's receipt gives it.
        #[arg(value_name = "SET")]
        set: String,
        /// Take the recommended answer to QUESTION.
        #[arg(long = "take", value_name = "QUESTION")]
        take: Vec<String>,
        // Each occurrence takes exactly two values, kept in pairs in order.
        /// Answer QUESTION in the owner's own words, TEXT.
        #[arg(long = "answer", num_args = 2, value_names = ["QUESTION", "TEXT"])]
        answer: Vec<String>,
        /// Leave QUESTION undecided for REASON; a waiting question may be
        /// deferred too.
        #[arg(long = "defer", num_args = 2, value_names = ["QUESTION", "REASON"])]
        defer: Vec<String>,
    },
}

/// Runs one `baley questions` command and prints its result.
pub fn run(args: QuestionsArgs) -> ExitCode {
    emit(match args.command {
        QuestionsCommand::Round { set } => {
            reading(|store, project| round_render(read::read(store, project, &set)))
        }
        QuestionsCommand::Answer {
            set,
            take,
            answer,
            defer,
        } => record_answers(&set, &take, &answer, &defer),
    })
}

/// `questions answer`: the words are judged into items before anything is
/// read; the call is checked against the set before checkout admission and
/// the policy step, so a refused call records nothing at all; the owner is
/// read over the policy the step recorded; and the call is recorded as the
/// owner under the version in force, through the same library function as
/// `questions-answer`.
fn record_answers(
    set: &str,
    take: &[String],
    answer: &[String],
    defer: &[String],
) -> Result<Render, Render> {
    let items = terminal_items(take, answer, defer);
    let call = judge_call(&items).map_err(|fault| receipt_render(&call_fault(fault), set))?;
    let checkout = checkout()?;
    let store = open(&checkout)?;
    let project = &checkout.project;
    let checked = answers::check(&store, project, set, &call)
        .map_err(|refusal| receipt_render(&refusal, set))?;
    let (policy, version) = admitted(&store, &checkout)?;

    let owner = identity::owner(identity::owner_name(&policy), &mut crate::process::System)
        .map_err(Render::refusal)?;
    let command = answer_command(
        project,
        version,
        set,
        &items,
        new_request_id(),
        &SystemClock::now(),
    )
    .map_err(|e| Render::refusal(e.to_string()))?;
    let recorded = answers::record(&store, &command, &owner, Some(&checked));
    Ok(receipt_render(&answers::answer(recorded, &store), set))
}

/// The items a session sends for the same acts: takes first, then own
/// answers, then deferrals, each in the order given. `answer` and `defer`
/// hold their words in pairs, question first.
pub(crate) fn terminal_items(take: &[String], answer: &[String], defer: &[String]) -> Value {
    let takes = take
        .iter()
        .map(|question| json!({"question": question, "act": "take"}));
    let answers = answer
        .chunks(2)
        .map(|pair| json!({"question": pair[0], "act": "answer", "answer": pair[1]}));
    let defers = defer
        .chunks(2)
        .map(|pair| json!({"question": pair[0], "act": "defer", "reason": pair[1]}));
    Value::Array(takes.chain(answers).chain(defers).collect())
}

/// The command a terminal answer call is recorded under: the same kind as
/// `questions-answer`, built as an approval's is, with the owner as actor,
/// no caller, Baley's clock and `policy_version` in both its digest and
/// itself. The version is safe in the digest because a command-line request
/// id is always fresh.
pub(crate) fn answer_command(
    project: &ProjectId,
    policy_version: u64,
    set: &str,
    items: &Value,
    request_id: RequestId,
    at: &str,
) -> Result<Command, CanonicalError> {
    let call = request_digest(&json!({"command": ANSWER_COMMAND, "set": set, "items": items}))?;
    owner_command(
        ANSWER_COMMAND,
        project,
        policy_version,
        call,
        request_id,
        at,
    )
}

/// What an answer call's answer prints: the round recorded and its time,
/// then the round as it now stands, or, when the receipt names the read
/// instead of carrying the round, that `baley questions round <set>` shows
/// it, exit 0; a refusal as `<code>: <reason>`, exit 2; a failure the same
/// way, exit 1.
pub(crate) fn receipt_render(answer: &Value, set: &str) -> Render {
    if answer["status"] != "ok" {
        return answer_render(answer);
    }
    let mut lines = vec![format!(
        "recorded round {} at {}",
        answer["round"],
        escaped(answer["recorded_at"].as_str().unwrap_or_default())
    )];
    let now = &answer["now"];
    if now.get("read").is_some() {
        lines.push(format!(
            "question set {}, round {}",
            escaped(now["state"].as_str().unwrap_or_default()),
            now["round"]
        ));
        lines.push(format!(
            "the round is too long to print here; baley questions round {} shows it",
            escaped(set)
        ));
    } else {
        lines.extend(shape_lines(now));
    }
    Render {
        lines,
        code: 0,
        error: false,
    }
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

/// The lines of one round, printed from the shape a session reads it in.
pub(crate) fn round_lines(round: &Round) -> Vec<String> {
    shape_lines(&round_value(round))
}

/// The lines of a round in `questions::read`'s shape: the set's state and
/// round number, the questions to ask with their recommended answers, each
/// waiting question with what it waits on, each answer marked as the
/// recommendation or the owner's own, each deferral with its reason, and an
/// abandonment's cause. The relay sentence is for a session and is left
/// out.
fn shape_lines(round: &Value) -> Vec<String> {
    let text = |value: &Value, name: &str| escaped(value[name].as_str().unwrap_or_default());
    let list = |name: &str| round[name].as_array().cloned().unwrap_or_default();
    let mut lines = vec![match round["state"].as_str() {
        Some("open") => format!("question set open, round {}", round["round"]),
        Some("closed") => "question set closed: every question is answered or deferred".into(),
        _ => "question set abandoned: it takes no answer".into(),
    }];
    let abandoned = &round["abandoned"];
    if abandoned.is_object() {
        lines.push(format!(
            "abandoned {} at {}",
            text(abandoned, "cause"),
            text(abandoned, "at")
        ));
    }
    for asked in list("current") {
        lines.push(format!(
            "ask {}: {} (recommended: {})",
            text(&asked, "id"),
            text(&asked, "question"),
            text(&asked, "recommended")
        ));
    }
    for waiting in list("waiting") {
        let on: Vec<String> = waiting["waits_on"]
            .as_array()
            .map(|on| {
                on.iter()
                    .map(|on| format!("{} {}", text(on, "id"), text(on, "state")))
                    .collect()
            })
            .unwrap_or_default();
        lines.push(format!(
            "waiting {}: {} (waits on {})",
            text(&waiting, "id"),
            text(&waiting, "question"),
            on.join(", ")
        ));
    }
    for answer in list("answered") {
        let whose = if answer["accepted"] == true {
            "took the recommendation"
        } else {
            "the owner's own answer"
        };
        lines.push(format!(
            "answered {} in round {} by {} at {}: {} ({whose})",
            text(&answer, "id"),
            answer["round"],
            text(&answer, "owner"),
            text(&answer, "at"),
            text(&answer, "answer")
        ));
    }
    for deferral in list("deferred") {
        lines.push(format!(
            "deferred {} in round {} by {} at {}: {}",
            text(&deferral, "id"),
            deferral["round"],
            text(&deferral, "owner"),
            text(&deferral, "at"),
            text(&deferral, "reason")
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    //! Rounds are written out by hand from design 0005 and the plan's set
    //! R2; expected lines are checked for what they must show, never
    //! produced by the code under test.

    use baley_core::questions::{
        Abandonment, Answer, Asked, Cause, Deferral, Pending, SetState, Waiting,
    };

    use super::*;

    const OWNER: &str = "Ada Lovelace";
    const AT: &str = "2026-10-22T09:00:01Z";
    const SET: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";

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

    fn words(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn terminal_answer_words_judged_into_other_items_than_a_sessions_is_caught() {
        let items = terminal_items(
            &words(&["q1"]),
            &words(&["q2", "Use Postgres"]),
            &words(&["q3", "after the spike"]),
        );
        assert_eq!(
            items,
            serde_json::json!([{"question":"q1","act":"take"},
                {"question":"q2","act":"answer","answer":"Use Postgres"},
                {"question":"q3","act":"defer","reason":"after the spike"}])
        );
        for items in [
            terminal_items(&words(&["q1", "q1"]), &[], &[]),
            terminal_items(&[], &[], &[]),
        ] {
            let fault = judge_call(&items).expect_err("refused");
            assert_eq!(call_fault(fault)["code"], "typed-content", "{items}");
        }
    }

    #[test]
    fn a_terminal_answer_command_built_without_the_owner_or_its_policy_version_is_caught() {
        let project = crate::scope::fixtures::project();
        let items = serde_json::json!([{"question": "q1", "act": "take"}]);
        let build = |version| {
            answer_command(
                &project,
                version,
                SET,
                &items,
                RequestId("9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f".into()),
                AT,
            )
            .unwrap()
        };
        let at_three = build(3);
        assert_eq!(at_three.actor, baley_store::Actor::Owner);
        assert_eq!(at_three.caller, None);
        assert_eq!(at_three.policy_version, 3);
        assert_eq!(at_three.kind.0, "questions.answer");
        assert_ne!(build(4).digest, at_three.digest);
    }

    #[test]
    fn an_answer_receipt_printed_without_its_round_or_with_a_raw_line_break_is_caught() {
        let open = serde_json::json!({"status": "ok", "set": SET, "round": 1,
            "recorded_at": AT, "now": {"state": "open", "round": 2,
                "current": [
                    {"id": "q2", "question": "Do sessions\nshare one ledger?",
                        "recommended": "Yes, one per user"},
                    {"id": "q3", "question": "Where does the ledger live?",
                        "recommended": "Outside the checkout"}],
                "waiting": [], "answered": [], "deferred": [], "abandoned": null,
                "relay": "bal-rounds"}});
        let render = receipt_render(&open, SET);
        assert_eq!(render.code, 0);
        assert!(!raw(&render.lines), "{:?}", render.lines);
        assert!(
            holding(&render.lines, &["round 1", AT]).is_some(),
            "{:?}",
            render.lines
        );
        assert!(
            holding(&render.lines, &["q2", "Do sessions\\nshare one ledger?"]).is_some(),
            "{:?}",
            render.lines
        );
        assert!(
            holding(&render.lines, &["q3"]).is_some(),
            "{:?}",
            render.lines
        );

        let closed = serde_json::json!({"status": "ok", "set": SET, "round": 2,
            "recorded_at": AT, "now": {"state": "closed", "round": 3, "current": [],
                "waiting": [], "answered": [], "deferred": [], "abandoned": null,
                "relay": "bal-rounds"}});
        let render = receipt_render(&closed, SET);
        assert!(
            holding(&render.lines, &["closed"]).is_some(),
            "{:?}",
            render.lines
        );

        let named = serde_json::json!({"status": "ok", "set": SET, "round": 1,
            "recorded_at": AT, "now": {"state": "open", "round": 2,
                "read": {"kind": "questions", "set": SET}, "relay": "bal-rounds"}});
        let render = receipt_render(&named, SET);
        let command = format!("baley questions round {SET}");
        assert!(
            holding(&render.lines, &[command.as_str()]).is_some(),
            "{:?}",
            render.lines
        );

        let refused = serde_json::json!({"status": "refused", "code": "question-closed",
            "reason": "question `q1` is already answered or deferred", "slot": "items",
            "id": "q1"});
        let render = receipt_render(&refused, SET);
        assert_eq!(render.code, 2);
        assert_eq!(
            render.lines,
            ["question-closed: question `q1` is already answered or deferred"]
        );
        let busy = serde_json::json!({"status": "failed", "code": "ledger-busy",
            "reason": "the question set moved", "place": "ledger"});
        let render = receipt_render(&busy, SET);
        assert_eq!(render.code, 1);
        assert!(render.error);
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

    mod over_the_store {
        //! Sets are opened and abandoned with `questions::record` over the
        //! real store, and calls recorded under each surface's own command
        //! builder: `prepared_command` for a session's, `answer_command` for
        //! the terminal's. Expected records come from design 0005 section 6,
        //! D-04 and the plan's set R2.

        use baley_core::questions::{Cause, SetState, judge_call};
        use baley_store::{Actor, Caller, RequestId, ServerCaller, StoreError};
        use baley_store_sqlite::SqliteStore;
        use serde_json::{Value, json};

        use super::super::*;
        use crate::mcp::prepare::prepared_command;
        use crate::questions::answer::tests::{OWNER, defer, on_stream, own, r2, take};
        use crate::questions::read::HeldSet;
        use crate::questions::record::tests::{open_set, story, transact};
        use crate::scope::fixtures::*;

        /// Session A, the one that opened the set: the fixtures' caller.
        const SESSION_A: &str = "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f";
        const SESSION_B: &str = "1c8f5b63-4d2e-4a7b-9f0c-2b3c4d5e6f7a";
        const AT2: &str = "2026-10-22T09:00:02Z";
        const AT3: &str = "2026-10-22T09:00:03Z";
        const AT4: &str = "2026-10-22T09:00:04Z";

        /// One call relayed by session B, as `questions-answer` runs it.
        fn relayed(
            store: &SqliteStore,
            set: &str,
            items: &Value,
            request: &str,
            at: &str,
        ) -> Value {
            let caller =
                ServerCaller::new("/real/r", "/real/r/p", "claude-code", SESSION_B, &json!(7))
                    .unwrap();
            let judged = crate::mcp::questions::judge_arguments(&json!({
                "operation": "questions-answer", "request_id": request, "set": set,
                "items": items}))
            .unwrap();
            let command = prepared_command(&project(), &judged.request, 0, at, &caller);
            match answers::check(store, &project(), set, &judged.call) {
                Ok(checked) => answers::answer(
                    answers::record(store, &command, OWNER, Some(&checked)),
                    store,
                ),
                Err(refusal) => refusal,
            }
        }

        /// One call given at the terminal at `version`, as `questions answer`
        /// runs it after admission.
        fn at_terminal(
            store: &SqliteStore,
            set: &str,
            items: &Value,
            version: u64,
            request: &str,
            at: &str,
        ) -> Value {
            let call = judge_call(items).unwrap();
            let command = answer_command(
                &project(),
                version,
                set,
                items,
                RequestId(request.into()),
                at,
            )
            .unwrap();
            match answers::check(store, &project(), set, &call) {
                Ok(checked) => answers::answer(
                    answers::record(store, &command, OWNER, Some(&checked)),
                    store,
                ),
                Err(refusal) => refusal,
            }
        }

        fn held(store: &SqliteStore, set: &str) -> HeldSet {
            match read::read(store, &project(), set).unwrap() {
                SetRead::Held(held) => *held,
                other => panic!("{other:?}"),
            }
        }

        #[test]
        fn a_set_lost_to_a_session_exit_or_answered_after_abandonment_is_caught() {
            let (dir, store) = store();
            assert_eq!(caller().baley_session(), SESSION_A);
            let set = open_set(&store, R1, &r2(), &story(1));
            drop(store);
            let store = crate::ledger::open::store(
                &dir.path().join("home"),
                AT2,
                crate::ledger::open::options(),
            )
            .unwrap();

            let first = relayed(&store, &set, &json!([take("q1")]), R2, AT2);
            assert_eq!(first["status"], "ok", "{first}");
            assert_eq!(first["round"], 1, "{first}");
            let second = at_terminal(
                &store,
                &set,
                &json!([own("q2", "No, one per project")]),
                3,
                R3,
                AT3,
            );
            assert_eq!(second["status"], "ok", "{second}");
            assert_eq!(second["round"], 2, "{second}");
            let round = held(&store, &set).round;
            let answered: Vec<(&str, &str, bool, u32)> = round
                .answered
                .iter()
                .map(|a| (a.question.as_str(), a.answer.as_str(), a.accepted, a.round))
                .collect();
            assert_eq!(
                answered,
                [
                    ("q1", "SQLite", true, 1),
                    ("q2", "No, one per project", false, 2)
                ]
            );
            let current: Vec<&str> = round.current.iter().map(|q| q.id.as_str()).collect();
            assert_eq!(current, ["q3"]);

            transact(&store, R4, &mut |tx| {
                crate::questions::record::abandon(tx, &set, Cause::Superseded, AT4)
                    .map_err(|error| StoreError::Unavailable(format!("{error:?}")))
            })
            .unwrap();
            for (items, request) in [
                (json!([take("q3")]), R5),
                (json!([defer("q3", "later")]), R6),
            ] {
                let before = head(&store);
                let refused = relayed(&store, &set, &items, request, AT4);
                assert_eq!(refused["status"], "refused", "{refused}");
                assert_eq!(refused["code"], "question-set-abandoned", "{refused}");
                assert_eq!(head(&store), before);
            }
            let round = held(&store, &set).round;
            assert_eq!(round.state, SetState::Abandoned);
            let listed: Vec<&str> = round.answered.iter().map(|a| a.question.as_str()).collect();
            assert_eq!(listed, ["q1", "q2"]);
        }

        #[test]
        fn a_relayed_answer_recorded_as_the_owners_or_a_terminal_answer_as_baleys_is_caught() {
            let (_dir, store) = store();
            let set = open_set(&store, R1, &r2(), &story(1));
            let relayed = relayed(&store, &set, &json!([take("q1")]), R2, AT2);
            assert_eq!(relayed["status"], "ok", "{relayed}");
            let terminal = at_terminal(
                &store,
                &set,
                &json!([defer("q2", "after the spike")]),
                3,
                R3,
                AT3,
            );
            assert_eq!(terminal["status"], "ok", "{terminal}");

            let recorded = on_stream(&store, "roadmap");
            assert_eq!(recorded.len(), 2);
            let q1 = &recorded[0];
            assert_eq!(q1.payload["question"], "q1");
            assert_eq!(q1.actor, Actor::Baley);
            let Some(Caller::Server(caller)) = &q1.caller else {
                panic!("a relayed answer without its session: {:?}", q1.caller);
            };
            assert_eq!(caller.baley_session(), SESSION_B);
            assert_eq!(q1.payload["owner"], OWNER);
            assert_eq!(q1.payload["at"], AT2);
            let q2 = &recorded[1];
            assert_eq!(q2.payload["question"], "q2");
            assert_eq!(q2.actor, Actor::Owner);
            assert_eq!(q2.caller, None);
            assert_eq!(q2.payload["owner"], OWNER);
            assert_eq!(q2.payload["at"], AT3);
        }
    }
}
