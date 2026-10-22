//! A story's refinement state, worked out when read (design 0005 Figure 1,
//! D-11).
//!
//! The `backlog` view stores only a story's latest question set and the set
//! its latest `story.refined` names; the state comes from those and the
//! latest set's round, so no stored state can drift from `round_of`.

use crate::questions::{Round, SetState};

/// Where a story stands in its refinement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refinement {
    /// No question set was ever opened for it.
    Unrefined,
    /// Its latest set has an open question, in this round.
    QuestionsOpen {
        /// The round the set waits on.
        round: u32,
    },
    /// Its latest set is closed and no approved refinement names it yet.
    Drafting,
    /// Its latest set is closed and its latest approved refinement names it.
    Refined,
}

impl Refinement {
    /// The state as reads spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unrefined => "unrefined",
            Self::QuestionsOpen { .. } => "questions-open",
            Self::Drafting => "drafting",
            Self::Refined => "refined",
        }
    }
}

/// The story's latest set is abandoned. Only a later set abandons a story's
/// set, and that later set is then the latest, so the records disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbandonedLatestSet;

/// Works out a story's refinement state from its latest set's id and round,
/// none when no set was opened, and the set its latest `story.refined`
/// names, none when it was never refined. An open set is asked whatever the
/// marker says, since approved truths stay in force while new questions are
/// open.
pub fn refinement(
    latest: Option<(&str, &Round)>,
    refined: Option<&str>,
) -> Result<Refinement, AbandonedLatestSet> {
    let Some((set, round)) = latest else {
        return Ok(Refinement::Unrefined);
    };
    match round.state {
        SetState::Open => Ok(Refinement::QuestionsOpen {
            round: round.number,
        }),
        SetState::Closed if refined == Some(set) => Ok(Refinement::Refined),
        SetState::Closed => Ok(Refinement::Drafting),
        SetState::Abandoned => Err(AbandonedLatestSet),
    }
}

#[cfg(test)]
mod tests {
    //! Expected states are written out from design 0005 Figure 1 and D-11,
    //! never from running this code.

    use crate::questions::{Abandonment, Answer, Cause, Question, SetFacts, judge_set, round_of};
    use serde_json::json;

    use super::*;

    const S: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const EARLIER: &str = "7f3e4d5c-6071-4c8d-8e9f-1a2b3c4d5e6f";

    fn questions() -> Vec<Question> {
        judge_set(&json!([
            {"id": "q1", "question": "Which store backs the ledger?", "recommended": "SQLite",
                "depends_on": []},
            {"id": "q2", "question": "Where does the ledger live?",
                "recommended": "Outside the checkout", "depends_on": ["q1"]}
        ]))
        .unwrap()
        .questions()
        .to_vec()
    }

    fn answer(id: &str, round: u32) -> Answer {
        Answer {
            question: id.into(),
            answer: "yes".into(),
            accepted: false,
            round,
            owner: "Ada Owner".into(),
            at: "2026-10-22T09:00:00Z".into(),
        }
    }

    /// The round of the set with `answers` recorded.
    fn round(answers: Vec<Answer>, abandoned: bool) -> Round {
        round_of(&SetFacts {
            questions: questions(),
            answers,
            deferrals: Vec::new(),
            abandoned: abandoned.then(|| Abandonment {
                cause: Cause::Superseded,
                at: "2026-10-22T09:00:00Z".into(),
            }),
        })
    }

    #[test]
    fn a_refinement_state_worked_out_from_the_wrong_set_or_round_is_caught() {
        let open_in_two = round(vec![answer("q1", 1)], false);
        let open_in_one = round(Vec::new(), false);
        let closed = round(vec![answer("q1", 1), answer("q2", 2)], false);
        let abandoned = round(vec![answer("q1", 1)], true);

        assert_eq!(refinement(None, None), Ok(Refinement::Unrefined));
        assert_eq!(
            refinement(Some((S, &open_in_two)), None),
            Ok(Refinement::QuestionsOpen { round: 2 })
        );
        assert_eq!(
            refinement(Some((S, &closed)), None),
            Ok(Refinement::Drafting)
        );
        assert_eq!(
            refinement(Some((S, &closed)), Some(S)),
            Ok(Refinement::Refined)
        );
        assert_eq!(
            refinement(Some((S, &closed)), Some(EARLIER)),
            Ok(Refinement::Drafting)
        );
        assert_eq!(
            refinement(Some((S, &open_in_one)), Some(EARLIER)),
            Ok(Refinement::QuestionsOpen { round: 1 })
        );
        assert_eq!(
            refinement(Some((S, &abandoned)), None),
            Err(AbandonedLatestSet)
        );
    }
}
