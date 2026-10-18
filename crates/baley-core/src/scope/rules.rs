//! The judgement of a scope change against accepted scope (design 0004
//! sections 5 and 6, PRJ-R13).
//!
//! A change whose shape was accepted is judged here against the scope the
//! `roadmap` document holds and the facts of each phase's progress, all
//! supplied as plain values. Nothing here reads a store or records
//! anything.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::change::{ChangeSubmission, ScopeChange};

/// The refusal code for withdrawing a phase whose execution must be undone
/// first.
pub const EXECUTION_PRESENT: &str = "execution-present";
/// The refusal code for withdrawing a complete phase or changing its
/// stories.
pub const PHASE_COMPLETE: &str = "phase-complete";
/// The refusal code for moving or dropping a story a complete phase served.
pub const STORY_SHIPPED: &str = "story-shipped";

/// What has happened in one phase, as PRJ-R13 judges a change to it. A
/// phase with no entry has neither execution nor completion.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhaseProgress {
    /// The undo the phase's execution needs, present only when execution
    /// is.
    pub undo: Option<String>,
    /// Whether the phase is complete.
    pub complete: bool,
}

/// One phase entry of accepted scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HeldPhase {
    pub(super) number: u32,
    pub(super) stories: Vec<u32>,
}

/// One story entry of accepted scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HeldStory {
    pub(super) id: u32,
    pub(super) phase: Option<u32>,
}

/// Accepted scope as the `roadmap` document holds it: every phase ever
/// declared, withdrawn ones included, and the stories in priority order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AcceptedScope {
    pub(super) phases: Vec<HeldPhase>,
    pub(super) stories: Vec<HeldStory>,
}

impl AcceptedScope {
    /// Reads the `roadmap` document's body, or the empty scope of a project
    /// with no document yet. `None` when the document cannot be read, which
    /// is a ledger fault rather than a judgement.
    pub fn read(document: Option<&Value>) -> Option<Self> {
        let Some(document) = document else {
            return Some(Self::default());
        };
        let document = document.as_object()?;
        let phases = entries(document, "phases")?
            .iter()
            .map(|phase| {
                let phase = phase.as_object()?;
                Some(HeldPhase {
                    number: whole(phase.get("number"))?,
                    stories: wholes(phase.get("stories"))?,
                })
            })
            .collect::<Option<_>>()?;
        let stories = entries(document, "stories")?
            .iter()
            .map(|story| {
                let story = story.as_object()?;
                Some(HeldStory {
                    id: whole(story.get("id"))?,
                    phase: match story.get("phase")? {
                        Value::Null => None,
                        phase => Some(whole(Some(phase))?),
                    },
                })
            })
            .collect::<Option<_>>()?;
        Some(Self { phases, stories })
    }

    /// Phase `number`, withdrawn or not.
    pub(super) fn phase(&self, number: u32) -> Option<&HeldPhase> {
        self.phases.iter().find(|phase| phase.number == number)
    }

    /// Story `id`.
    pub(super) fn story(&self, id: u32) -> Option<&HeldStory> {
        self.stories.iter().find(|story| story.id == id)
    }
}

fn entries<'a>(document: &'a Map<String, Value>, name: &str) -> Option<&'a Vec<Value>> {
    document.get(name)?.as_array()
}

fn whole(value: Option<&Value>) -> Option<u32> {
    value?.as_u64().and_then(|value| u32::try_from(value).ok())
}

fn wholes(value: Option<&Value>) -> Option<Vec<u32>> {
    value?
        .as_array()?
        .iter()
        .map(|value| whole(Some(value)))
        .collect()
}

/// Why a change is refused against accepted scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeRefusal {
    /// The phase has execution present, which `undo` must undo first.
    ExecutionPresent { phase: u32, undo: String },
    /// The phase is complete.
    PhaseComplete { phase: u32 },
    /// The story was served by `phase`, which is complete.
    StoryShipped { story: u32, phase: u32 },
}

impl ScopeRefusal {
    /// The refusal's public code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::ExecutionPresent { .. } => EXECUTION_PRESENT,
            Self::PhaseComplete { .. } => PHASE_COMPLETE,
            Self::StoryShipped { .. } => STORY_SHIPPED,
        }
    }

    /// What the refusal names, as its answer's details.
    pub fn details(&self) -> Option<Value> {
        Some(match self {
            Self::ExecutionPresent { phase, undo } => json!({"phase": phase, "undo": undo}),
            Self::PhaseComplete { phase } => json!({"phase": phase}),
            Self::StoryShipped { story, phase } => json!({"story": story, "phase": phase}),
        })
    }
}

/// Judges a change against what has happened in its phases (PRJ-R13): a
/// complete phase is never withdrawn and its stories never change, a phase
/// with execution present is withdrawn only once that execution is undone,
/// and a story a complete phase served is never moved or dropped. Every
/// other change passes.
///
/// Completion is judged before execution, since a complete phase is never
/// withdrawn whatever its execution.
pub fn judge_progress(
    change: &ChangeSubmission,
    scope: &AcceptedScope,
    progress: &BTreeMap<u32, PhaseProgress>,
) -> Result<(), ScopeRefusal> {
    let of = |phase: u32| progress.get(&phase).cloned().unwrap_or_default();
    match &change.change {
        ScopeChange::PhaseWithdraw { phase, .. } => {
            let progress = of(*phase);
            if progress.complete {
                return Err(ScopeRefusal::PhaseComplete { phase: *phase });
            }
            if let Some(undo) = progress.undo {
                return Err(ScopeRefusal::ExecutionPresent {
                    phase: *phase,
                    undo,
                });
            }
        }
        ScopeChange::PhaseEdit { phase, fields } => {
            let changes_stories = fields.stories.as_ref().is_some_and(|stories| {
                scope
                    .phase(*phase)
                    .is_none_or(|held| held.stories != *stories)
            });
            if changes_stories && of(*phase).complete {
                return Err(ScopeRefusal::PhaseComplete { phase: *phase });
            }
        }
        ScopeChange::StoryReassign { story, .. } | ScopeChange::StoryDrop { story } => {
            if let Some(phase) = scope.story(*story).and_then(|story| story.phase)
                && of(phase).complete
            {
                return Err(ScopeRefusal::StoryShipped {
                    story: *story,
                    phase,
                });
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Scopes are written out as the `roadmap` projector writes its
    //! document, and every expected code and detail is read off design 0004
    //! by hand.

    use std::collections::BTreeMap;

    use serde_json::{Value, json};

    use super::*;
    use crate::scope::{ChangeKind, ChangeSubmission, judge_change};

    /// A0, the roadmap after the approved S1.
    fn a0() -> Value {
        json!({"roadmap": "project",
            "description": {"what": "A ledger for agent work",
                "core_value": "Nothing moves on an unproven claim",
                "constraints": ["One binary"], "submission": "5c".repeat(32)},
            "phases": [
                {"number": 1, "name": "Records", "goal": "Steps are recorded",
                    "depends_on": [], "stories": [1], "status": "declared"},
                {"number": 2, "name": "Approval", "goal": "The owner approves",
                    "depends_on": [1], "stories": [2], "status": "declared"}],
            "order": [1, 2],
            "stories": [
                {"id": 1, "sentence": "Every step is recorded", "status": "active",
                    "phase": 1, "truths": 0},
                {"id": 2, "sentence": "The owner approves scope", "status": "active",
                    "phase": 2, "truths": 0},
                {"id": 3, "sentence": "Teammates see the backlog", "status": "deferred",
                    "phase": null, "truths": 0}],
            "highest_phase": 2, "highest_story": 3})
    }

    fn scope(document: &Value) -> AcceptedScope {
        AcceptedScope::read(Some(document)).expect("a readable roadmap")
    }

    fn change(kind: ChangeKind, value: Value) -> ChangeSubmission {
        judge_change(kind, &value).unwrap_or_else(|refusal| panic!("{value}: {refusal:?}"))
    }

    /// F0, nothing has happened in any phase.
    fn f0() -> BTreeMap<u32, PhaseProgress> {
        BTreeMap::new()
    }

    /// F1, phase 1 executing and phase 2 complete, having executed.
    fn f1() -> BTreeMap<u32, PhaseProgress> {
        BTreeMap::from([
            (
                1,
                PhaseProgress {
                    undo: Some("undo phase 1".into()),
                    complete: false,
                },
            ),
            (
                2,
                PhaseProgress {
                    undo: Some("undo phase 2".into()),
                    complete: true,
                },
            ),
        ])
    }

    /// F2, phase 1 complete with no execution present.
    fn f2() -> BTreeMap<u32, PhaseProgress> {
        BTreeMap::from([(
            1,
            PhaseProgress {
                undo: None,
                complete: true,
            },
        )])
    }

    /// A refusal as its public code and details.
    fn answer(refusal: &ScopeRefusal) -> (&'static str, Option<Value>) {
        (refusal.code(), refusal.details())
    }

    #[test]
    fn a_withdrawal_edit_or_story_move_on_a_complete_or_executing_phase_accepted_is_caught() {
        let a0 = scope(&a0());
        let withdraw_1 = change(
            ChangeKind::PhaseWithdraw,
            json!({"phase": 1, "decisions": [{"story": 1, "to": "drop"}]}),
        );
        let withdraw_2 = change(
            ChangeKind::PhaseWithdraw,
            json!({"phase": 2, "decisions": [{"story": 2, "to": 1}]}),
        );
        let edit_stories = change(ChangeKind::PhaseEdit, json!({"phase": 2, "stories": []}));
        let edit_goal = change(ChangeKind::PhaseEdit, json!({"phase": 2, "goal": "g"}));
        let reassign_2 = change(
            ChangeKind::StoryReassign,
            json!({"story": 2, "phase": null}),
        );
        let drop_2 = change(ChangeKind::StoryDrop, json!({"story": 2}));
        let reassign_1 = change(ChangeKind::StoryReassign, json!({"story": 1, "phase": 2}));
        let drop_1 = change(ChangeKind::StoryDrop, json!({"story": 1}));
        let judged = |change: &ChangeSubmission, progress: &BTreeMap<u32, PhaseProgress>| {
            judge_progress(change, &a0, progress).map_err(|refusal| answer(&refusal))
        };

        let f1 = f1();
        assert_eq!(
            judged(&withdraw_1, &f1),
            Err((
                "execution-present",
                Some(json!({"phase": 1, "undo": "undo phase 1"}))
            ))
        );
        assert_eq!(
            judged(&withdraw_2, &f1),
            Err(("phase-complete", Some(json!({"phase": 2}))))
        );
        assert_eq!(
            judged(&edit_stories, &f1),
            Err(("phase-complete", Some(json!({"phase": 2}))))
        );
        assert_eq!(judged(&edit_goal, &f1), Ok(()));
        for shipped in [&reassign_2, &drop_2] {
            assert_eq!(
                judged(shipped, &f1),
                Err(("story-shipped", Some(json!({"story": 2, "phase": 2})))),
                "{}",
                shipped.value()
            );
        }
        assert_eq!(judged(&reassign_1, &f1), Ok(()));
        assert_eq!(judged(&drop_1, &f1), Ok(()));

        // Completion is judged on its own, not only where execution is.
        assert_eq!(
            judged(&withdraw_1, &f2()),
            Err(("phase-complete", Some(json!({"phase": 1}))))
        );

        let f0 = f0();
        for passes in [
            &withdraw_1,
            &withdraw_2,
            &edit_stories,
            &edit_goal,
            &reassign_2,
            &drop_2,
            &reassign_1,
            &drop_1,
        ] {
            assert_eq!(judged(passes, &f0), Ok(()), "{}", passes.value());
        }
    }
}
