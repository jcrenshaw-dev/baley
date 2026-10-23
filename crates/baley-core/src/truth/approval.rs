//! The judgement of an approval of a truth draft by digest (design 0005
//! section 5, PLN-R26, PRJ-R16).
//!
//! An approval is judged on facts the binary read, in one fixed order, so
//! one approval always gets one answer: the draft, then the story, then the
//! story's latest question set, then dispatch.

use serde_json::{Value, json};

use super::submission::{Truth, TruthSubmission};
use crate::questions::QUESTION_SET_ABANDONED;
use crate::scope::{DRAFT_APPROVED, NO_SUCH_STORY, PriorApproval, STALE_DRAFT, UNKNOWN_DRAFT};

/// The refusal code for approving truths while a question of the story's
/// latest set is open.
pub const QUESTION_OPEN: &str = "question-open";

/// The refusal code for approving truths while a worker is dispatched on
/// the phase that committed the story (PRJ-R16).
pub const DISPATCH_ACTIVE: &str = "dispatch-active";

/// Why an approval of a truth draft is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TruthApprovalRefusal {
    /// No recorded truth submission has the digest.
    UnknownDraft,
    /// A newer draft for the same story was submitted. `path` names the
    /// first part that differs from the newest, such as `truths[1].outcome`.
    StaleDraft { path: String },
    /// The draft was approved by `owner` at `at` after its latest
    /// submission.
    DraftApproved { owner: String, at: String },
    /// The story is not held, or it is dropped.
    NoSuchStory,
    /// The draft's question set is not the story's latest.
    SetAbandoned,
    /// These questions of the story's latest set are open, in submitted
    /// order.
    QuestionOpen { questions: Vec<String> },
    /// A worker is dispatched on the phase that committed the story.
    DispatchActive,
}

impl TruthApprovalRefusal {
    /// The refusal's public code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownDraft => UNKNOWN_DRAFT,
            Self::StaleDraft { .. } => STALE_DRAFT,
            Self::DraftApproved { .. } => DRAFT_APPROVED,
            Self::NoSuchStory => NO_SUCH_STORY,
            Self::SetAbandoned => QUESTION_SET_ABANDONED,
            Self::QuestionOpen { .. } => QUESTION_OPEN,
            Self::DispatchActive => DISPATCH_ACTIVE,
        }
    }

    /// What the refusal names, as its answer's details, or none.
    pub fn details(&self) -> Option<Value> {
        match self {
            Self::StaleDraft { path } => Some(json!({"path": path})),
            Self::DraftApproved { owner, at } => Some(json!({"owner": owner, "at": at})),
            Self::QuestionOpen { questions } => Some(json!({"questions": questions})),
            Self::UnknownDraft | Self::NoSuchStory | Self::SetAbandoned | Self::DispatchActive => {
                None
            }
        }
    }
}

/// What an approval of one truth draft is judged on.
#[derive(Debug, Clone, Copy)]
pub struct TruthApprovalFacts<'a> {
    /// The target draft's accepted submission, or `None` when no `draft`
    /// document has its digest.
    pub target: Option<&'a TruthSubmission>,
    /// Whether the target's `scope.submitted` was found among the events.
    pub recorded: bool,
    /// The newest `story-truths` draft of the same target, or `None`.
    pub newest: Option<&'a TruthSubmission>,
    /// The `story.refined` naming this digest recorded after its latest
    /// submission, or `None`.
    pub approved: Option<PriorApproval<'a>>,
    /// Whether the story is held and not dropped.
    pub story_held: bool,
    /// The id of the story's latest question set, or `None`.
    pub latest_set: Option<&'a str>,
    /// The ids of the latest set's open questions, in submitted order.
    pub open_questions: &'a [String],
    /// Whether a worker is dispatched on the phase that committed the story.
    /// The binary reads it as false until Build 5 records dispatch.
    pub dispatch_active: bool,
}

/// Judges an approval of a truth draft in D-17's order: `unknown-draft`
/// when the draft or its event is missing, or no newest draft is listed;
/// `stale-draft` when the newest draft is not this one, with the first
/// differing path; `draft-approved` naming the earlier approval;
/// `no-such-story`; `question-set-abandoned` when the draft's set is not the
/// story's latest; `question-open` naming every open question of that set;
/// and `dispatch-active`.
pub fn judge_truth_approval(facts: &TruthApprovalFacts) -> Result<(), TruthApprovalRefusal> {
    let Some(target) = facts.target.filter(|_| facts.recorded) else {
        return Err(TruthApprovalRefusal::UnknownDraft);
    };
    let Some(newest) = facts.newest else {
        return Err(TruthApprovalRefusal::UnknownDraft);
    };
    if newest.digest() != target.digest() {
        // A digest is taken over the story, the set and every truth member,
        // and both drafts share their target story, so they differ in one.
        let path = difference(target, newest).expect("differing digests differ in content");
        return Err(TruthApprovalRefusal::StaleDraft { path });
    }
    if let Some(approved) = facts.approved {
        return Err(TruthApprovalRefusal::DraftApproved {
            owner: approved.owner.into(),
            at: approved.at.into(),
        });
    }
    if !facts.story_held {
        return Err(TruthApprovalRefusal::NoSuchStory);
    }
    if facts.latest_set != Some(target.set()) {
        return Err(TruthApprovalRefusal::SetAbandoned);
    }
    if !facts.open_questions.is_empty() {
        return Err(TruthApprovalRefusal::QuestionOpen {
            questions: facts.open_questions.to_vec(),
        });
    }
    if facts.dispatch_active {
        return Err(TruthApprovalRefusal::DispatchActive);
    }
    Ok(())
}

/// The first part of two truth drafts that differs: `set`, then each truth
/// entry by entry in the truth's member order, or the first entry only one
/// list holds. `None` when they are equal.
fn difference(a: &TruthSubmission, b: &TruthSubmission) -> Option<String> {
    if a.set() != b.set() {
        return Some("set".into());
    }
    let (a, b) = (a.truths(), b.truths());
    (0..a.len().max(b.len())).find_map(|index| match (a.get(index), b.get(index)) {
        (Some(x), Some(y)) => member(x, y).map(|name| format!("truths[{index}].{name}")),
        _ => Some(format!("truths[{index}]")),
    })
}

/// The first member, in the truth's member order, that differs.
fn member(a: &Truth, b: &Truth) -> Option<&'static str> {
    [
        ("id", a.id == b.id),
        ("form", a.form == b.form),
        ("trigger", a.trigger == b.trigger),
        ("observer", a.observer == b.observer),
        ("verb", a.verb == b.verb),
        ("outcome", a.outcome == b.outcome),
        ("kind", a.kind == b.kind),
        ("human_verify", a.human_verify == b.human_verify),
    ]
    .into_iter()
    .find_map(|(name, equal)| (!equal).then_some(name))
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from D-17, design 0005's refusal
    //! table and the plan's U1, never from running this code.

    use std::num::NonZeroU32;

    use serde_json::json;

    use super::*;
    use crate::truth::judge_truths;

    const S: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const OTHER: &str = "7f3e4d5c-6071-4c8d-8e9f-1a2b3c4d5e6f";
    const T3: &str = "2026-10-22T09:00:03Z";

    fn t1() -> Value {
        json!({"id": "t1", "form": "when",
            "trigger": "the owner approves a draft by a stale digest",
            "observer": "the owner", "verb": "is refused",
            "outcome": "stale-draft naming the first differing part", "kind": "literal",
            "observable": true, "fixed_oracle": true})
    }

    fn t2() -> Value {
        json!({"id": "t2", "form": "for-any",
            "trigger": "order the answers of a set are recorded in",
            "observer": "a ledger reader", "verb": "gets", "outcome": "the same round",
            "kind": "property", "observable": true, "fixed_oracle": true})
    }

    fn draft(set: &str, truths: Value) -> TruthSubmission {
        judge_truths(NonZeroU32::new(1).unwrap(), set, &truths).unwrap()
    }

    fn u1() -> TruthSubmission {
        draft(S, json!([t1(), t2()]))
    }

    #[test]
    fn an_approval_judged_in_another_order_is_caught() {
        let u1 = u1();
        let mut changed = t2();
        changed["outcome"] = json!("the same round and the same waiting list");
        let u2 = draft(S, json!([t1(), changed]));
        let open = vec!["q2".to_owned(), "q3".to_owned()];
        let mut facts = TruthApprovalFacts {
            target: Some(&u1),
            recorded: false,
            newest: Some(&u2),
            approved: Some(PriorApproval {
                owner: "Ada Owner",
                at: T3,
            }),
            story_held: false,
            latest_set: Some(OTHER),
            open_questions: &open,
            dispatch_active: true,
        };
        let answer = |facts: &TruthApprovalFacts| {
            judge_truth_approval(facts).map_err(|refusal| (refusal.code(), refusal.details()))
        };

        assert_eq!(answer(&facts), Err(("unknown-draft", None)));
        facts.recorded = true;
        assert_eq!(
            answer(&facts),
            Err(("stale-draft", Some(json!({"path": "truths[1].outcome"}))))
        );
        facts.newest = Some(&u1);
        assert_eq!(
            answer(&facts),
            Err((
                "draft-approved",
                Some(json!({"owner": "Ada Owner", "at": T3}))
            ))
        );
        facts.approved = None;
        assert_eq!(answer(&facts), Err(("no-such-story", None)));
        facts.story_held = true;
        assert_eq!(answer(&facts), Err(("question-set-abandoned", None)));
        facts.latest_set = Some(S);
        assert_eq!(
            answer(&facts),
            Err(("question-open", Some(json!({"questions": ["q2", "q3"]}))))
        );
        facts.open_questions = &[];
        assert_eq!(answer(&facts), Err(("dispatch-active", None)));
        facts.dispatch_active = false;
        assert_eq!(answer(&facts), Ok(()));
    }

    #[test]
    fn a_stale_path_naming_the_wrong_member_or_entry_is_caught() {
        let u1 = u1();
        let stale = |newest: &TruthSubmission| {
            judge_truth_approval(&TruthApprovalFacts {
                target: Some(&u1),
                recorded: true,
                newest: Some(newest),
                approved: None,
                story_held: true,
                latest_set: Some(S),
                open_questions: &[],
                dispatch_active: false,
            })
        };
        let mut t3 = t1();
        t3["id"] = json!("t3");
        assert_eq!(
            stale(&draft(OTHER, json!([t1(), t2()]))),
            Err(TruthApprovalRefusal::StaleDraft { path: "set".into() })
        );
        assert_eq!(
            stale(&draft(S, json!([t1(), t2(), t3]))),
            Err(TruthApprovalRefusal::StaleDraft {
                path: "truths[2]".into()
            })
        );
    }
}
