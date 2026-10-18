//! The judgement of a scope change against accepted scope (design 0004
//! sections 5 and 6, PRJ-R13).
//!
//! A change whose shape was accepted is judged here against the scope the
//! `roadmap` document holds and the facts of each phase's progress, all
//! supplied as plain values. Each kind's rules run in a fixed order, so one
//! change always gets one answer. Nothing here reads a store or records
//! anything.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use super::change::{ChangeSubmission, Decision, Destination, PhaseDeclaration, ScopeChange};
use super::submission::{
    DEPENDENCY_CYCLE, MALFORMED_SUBMISSION, STORY_DOUBLE_ASSIGNED, StoryStatus,
};
use crate::capture::NO_SUCH_PHASE;

/// The refusal code for withdrawing a phase whose execution must be undone
/// first.
pub const EXECUTION_PRESENT: &str = "execution-present";
/// The refusal code for withdrawing a complete phase or changing its
/// stories.
pub const PHASE_COMPLETE: &str = "phase-complete";
/// The refusal code for moving or dropping a story a complete phase served.
pub const STORY_SHIPPED: &str = "story-shipped";
/// The refusal code for an order that leaves out or repeats an entry it
/// must hold.
pub const ORDER_INCOMPLETE: &str = "order-incomplete";
/// The refusal code for a phase placed before a phase it depends on.
pub const DEPENDENCY_ORDER: &str = "dependency-order";
/// The refusal code for a story id no story has.
pub const NO_SUCH_STORY: &str = "no-such-story";
/// The refusal code for a change submitted or approved before the
/// project's start is approved.
pub const PROJECT_NOT_STARTED: &str = "project-not-started";
/// The refusal code for a withdrawal that leaves a story of the phase
/// without a decision.
pub const STORY_UNDECIDED: &str = "story-undecided";
/// The refusal code for withdrawing a phase other declared phases depend
/// on.
pub const PHASE_HAS_DEPENDENTS: &str = "phase-has-dependents";
/// The refusal code for changing a story that is dropped or excluded, both
/// final.
pub const STORY_NOT_SERVED: &str = "story-not-served";

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

/// A story's status in accepted scope: one a submission states, or
/// `dropped`, which only an approval gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HeldStatus {
    Active,
    Deferred,
    Excluded,
    Dropped,
}

impl HeldStatus {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "active" => Some(Self::Active),
            "deferred" => Some(Self::Deferred),
            "excluded" => Some(Self::Excluded),
            "dropped" => Some(Self::Dropped),
            _ => None,
        }
    }

    /// The status as a submission states it, or `None` for `dropped`.
    pub(super) fn stated(self) -> Option<StoryStatus> {
        match self {
            Self::Active => Some(StoryStatus::Active),
            Self::Deferred => Some(StoryStatus::Deferred),
            Self::Excluded => Some(StoryStatus::Excluded),
            Self::Dropped => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Deferred => "deferred",
            Self::Excluded => "excluded",
            Self::Dropped => "dropped",
        }
    }
}

/// One phase entry of accepted scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HeldPhase {
    pub(super) number: u32,
    pub(super) name: String,
    pub(super) goal: String,
    pub(super) depends_on: Vec<u32>,
    pub(super) stories: Vec<u32>,
    pub(super) withdrawn: bool,
}

/// One story entry of accepted scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HeldStory {
    pub(super) id: u32,
    pub(super) status: HeldStatus,
    pub(super) phase: Option<u32>,
}

/// Accepted scope as the `roadmap` document holds it: every phase ever
/// declared, withdrawn ones included, the phase order, the stories in
/// priority order, and the highest phase number and story id ever
/// declared.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AcceptedScope {
    pub(super) phases: Vec<HeldPhase>,
    pub(super) order: Vec<u32>,
    pub(super) stories: Vec<HeldStory>,
    pub(super) highest_phase: u32,
    pub(super) highest_story: u32,
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
                    name: phase.get("name")?.as_str()?.to_owned(),
                    goal: phase.get("goal")?.as_str()?.to_owned(),
                    depends_on: wholes(phase.get("depends_on"))?,
                    stories: wholes(phase.get("stories"))?,
                    withdrawn: match phase.get("status")?.as_str()? {
                        "declared" => false,
                        "withdrawn" => true,
                        _ => return None,
                    },
                })
            })
            .collect::<Option<_>>()?;
        let stories = entries(document, "stories")?
            .iter()
            .map(|story| {
                let story = story.as_object()?;
                Some(HeldStory {
                    id: whole(story.get("id"))?,
                    status: HeldStatus::parse(story.get("status")?.as_str()?)?,
                    phase: match story.get("phase")? {
                        Value::Null => None,
                        phase => Some(whole(Some(phase))?),
                    },
                })
            })
            .collect::<Option<_>>()?;
        Some(Self {
            phases,
            order: wholes(document.get("order"))?,
            stories,
            highest_phase: whole(document.get("highest_phase"))?,
            highest_story: whole(document.get("highest_story"))?,
        })
    }

    /// Phase `number`, withdrawn or not.
    pub(super) fn phase(&self, number: u32) -> Option<&HeldPhase> {
        self.phases.iter().find(|phase| phase.number == number)
    }

    /// Phase `number` when it is declared and not withdrawn, since a
    /// withdrawn phase is no phase a change can name (D-07).
    pub(super) fn declared(&self, number: u32) -> Option<&HeldPhase> {
        self.phase(number).filter(|phase| !phase.withdrawn)
    }

    /// The phases not withdrawn.
    fn declared_phases(&self) -> impl Iterator<Item = &HeldPhase> {
        self.phases.iter().filter(|phase| !phase.withdrawn)
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
    /// No start is approved, so there is no accepted scope to change.
    ProjectNotStarted,
    /// No declared phase has this number.
    NoSuchPhase { phase: u32 },
    /// No story has this id.
    NoSuchStory { story: u32 },
    /// A member breaks a rule the start's shape also holds. `section` and
    /// `field` name it as a start's refusal does.
    Malformed { section: String, field: String },
    /// The story is served by another declared phase.
    StoryDoubleAssigned { story: u32, phase: u32 },
    /// Every declared phase on a dependency cycle, ascending.
    DependencyCycle { phases: Vec<u32> },
    /// `phase` is placed before `depends_on`, a phase it depends on.
    DependencyOrder { phase: u32, depends_on: u32 },
    /// The order does not hold each of `order` exactly once.
    OrderIncomplete { order: Vec<u32> },
    /// The withdrawn phase serves this story and the withdrawal gives it
    /// no decision.
    StoryUndecided { story: u32 },
    /// These declared phases, ascending, depend on the withdrawn one.
    PhaseHasDependents { phases: Vec<u32> },
    /// The story is dropped or excluded, so nothing changes it.
    StoryNotServed { story: u32, status: &'static str },
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
            Self::ProjectNotStarted => PROJECT_NOT_STARTED,
            Self::NoSuchPhase { .. } => NO_SUCH_PHASE,
            Self::NoSuchStory { .. } => NO_SUCH_STORY,
            Self::Malformed { .. } => MALFORMED_SUBMISSION,
            Self::StoryDoubleAssigned { .. } => STORY_DOUBLE_ASSIGNED,
            Self::DependencyCycle { .. } => DEPENDENCY_CYCLE,
            Self::DependencyOrder { .. } => DEPENDENCY_ORDER,
            Self::OrderIncomplete { .. } => ORDER_INCOMPLETE,
            Self::StoryUndecided { .. } => STORY_UNDECIDED,
            Self::PhaseHasDependents { .. } => PHASE_HAS_DEPENDENTS,
            Self::StoryNotServed { .. } => STORY_NOT_SERVED,
            Self::ExecutionPresent { .. } => EXECUTION_PRESENT,
            Self::PhaseComplete { .. } => PHASE_COMPLETE,
            Self::StoryShipped { .. } => STORY_SHIPPED,
        }
    }

    /// What the refusal names, as its answer's details, or none.
    pub fn details(&self) -> Option<Value> {
        Some(match self {
            Self::ProjectNotStarted => return None,
            Self::NoSuchPhase { phase } | Self::PhaseComplete { phase } => {
                json!({"phase": phase})
            }
            Self::NoSuchStory { story } | Self::StoryUndecided { story } => {
                json!({"story": story})
            }
            Self::Malformed { section, field } => json!({"section": section, "field": field}),
            Self::StoryDoubleAssigned { story, phase } | Self::StoryShipped { story, phase } => {
                json!({"story": story, "phase": phase})
            }
            Self::DependencyCycle { phases } | Self::PhaseHasDependents { phases } => {
                json!({"phases": phases})
            }
            Self::DependencyOrder { phase, depends_on } => {
                json!({"phase": phase, "depends_on": depends_on})
            }
            Self::OrderIncomplete { order } => json!({"order": order}),
            Self::StoryNotServed { story, status } => json!({"story": story, "status": status}),
            Self::ExecutionPresent { phase, undo } => json!({"phase": phase, "undo": undo}),
        })
    }
}

/// Judges an accepted change against accepted scope, whether the start is
/// approved and what has happened in each phase.
///
/// With no start approved every change is `project-not-started`, since a
/// phase or story declared before the start would be left out of the
/// start's order (D-05). Then each kind's rules run in a fixed order:
/// the phase or story it names must be one a change may name, then
/// PRJ-R13's progress, then the change's own members.
pub fn judge_scope_change(
    change: &ChangeSubmission,
    scope: &AcceptedScope,
    started: bool,
    progress: &BTreeMap<u32, PhaseProgress>,
) -> Result<(), ScopeRefusal> {
    if !started {
        return Err(ScopeRefusal::ProjectNotStarted);
    }
    match &change.change {
        ScopeChange::PhaseDeclare(declaration) => judge_declaration(declaration, scope),
        ScopeChange::PhaseEdit { phase, fields } => {
            let phase = *phase;
            if scope.declared(phase).is_none() {
                return Err(ScopeRefusal::NoSuchPhase { phase });
            }
            judge_progress(change, scope, progress)?;
            judge_members(
                scope,
                fields.depends_on.as_deref().unwrap_or(&[]),
                fields.stories.as_deref().unwrap_or(&[]),
                Some(phase),
            )?;
            if let Some(depends_on) = &fields.depends_on {
                let phases = on_cycle_with(scope, phase, depends_on);
                if !phases.is_empty() {
                    return Err(ScopeRefusal::DependencyCycle { phases });
                }
                placed_after(&scope.order, phase, depends_on)?;
            }
            Ok(())
        }
        ScopeChange::PhaseReorder { order } => judge_reorder(order, scope),
        ScopeChange::PhaseWithdraw { phase, decisions } => {
            let phase = *phase;
            let Some(held) = scope.declared(phase) else {
                return Err(ScopeRefusal::NoSuchPhase { phase });
            };
            judge_progress(change, scope, progress)?;
            judge_withdrawal(held, decisions, scope)
        }
        ScopeChange::StoryDeclare {
            phase: Some(phase), ..
        } if scope.declared(*phase).is_none() => Err(ScopeRefusal::NoSuchPhase { phase: *phase }),
        ScopeChange::StoryEdit { story, .. } => served(scope, *story),
        ScopeChange::StoryReassign { story, phase } => {
            served(scope, *story)?;
            judge_progress(change, scope, progress)?;
            match phase {
                Some(phase) if scope.declared(*phase).is_none() => {
                    Err(ScopeRefusal::NoSuchPhase { phase: *phase })
                }
                _ => Ok(()),
            }
        }
        ScopeChange::StoryDrop { story } => {
            served(scope, *story)?;
            judge_progress(change, scope, progress)
        }
        ScopeChange::BacklogReorder { order } => judge_backlog_order(order, scope),
        // A description has no rule of its own past the start's.
        ScopeChange::ProjectDescribe(_) | ScopeChange::StoryDeclare { .. } => Ok(()),
    }
}

/// Story `id` exists and is still served: one dropped or excluded is final
/// (design 0004 Figure 2, PRJ-R17).
fn served(scope: &AcceptedScope, id: u32) -> Result<(), ScopeRefusal> {
    let Some(story) = scope.story(id) else {
        return Err(ScopeRefusal::NoSuchStory { story: id });
    };
    match story.status {
        HeldStatus::Dropped | HeldStatus::Excluded => Err(ScopeRefusal::StoryNotServed {
            story: id,
            status: story.status.as_str(),
        }),
        HeldStatus::Active | HeldStatus::Deferred => Ok(()),
    }
}

/// A full backlog order: every id a story, and every active and deferred
/// story exactly once with no other (D-17).
fn judge_backlog_order(order: &[u32], scope: &AcceptedScope) -> Result<(), ScopeRefusal> {
    if let Some(&story) = order.iter().find(|story| scope.story(**story).is_none()) {
        return Err(ScopeRefusal::NoSuchStory { story });
    }
    let ordered: BTreeSet<u32> = scope
        .stories
        .iter()
        .filter(|story| matches!(story.status, HeldStatus::Active | HeldStatus::Deferred))
        .map(|story| story.id)
        .collect();
    let held: BTreeSet<u32> = order.iter().copied().collect();
    if held != ordered || order.len() != ordered.len() {
        return Err(ScopeRefusal::OrderIncomplete {
            order: ordered.into_iter().collect(),
        });
    }
    Ok(())
}

/// A new phase's dependencies and stories, then its position, then whether
/// that position comes after every phase it depends on.
fn judge_declaration(
    declaration: &PhaseDeclaration,
    scope: &AcceptedScope,
) -> Result<(), ScopeRefusal> {
    judge_members(scope, &declaration.depends_on, &declaration.stories, None)?;
    let last = scope.order.len() + 1;
    let at = match declaration.position {
        None => last,
        Some(position) => usize::try_from(position)
            .ok()
            .filter(|position| *position <= last)
            .ok_or_else(|| ScopeRefusal::Malformed {
                section: "position".into(),
                field: "position".into(),
            })?,
    };
    // With no number left the approval fails for want of one (D-14), so
    // there is no phase to name.
    let Some(number) = scope.highest_phase.checked_add(1) else {
        return Ok(());
    };
    let mut order = scope.order.clone();
    order.insert(at - 1, number);
    placed_after(&order, number, &declaration.depends_on)
}

/// The dependencies and stories a declaration or an edit gives: each
/// dependency a declared phase, each story one that exists, is active, and
/// no other declared phase serves. `edited` is the phase an edit changes,
/// whose own stories do not count as another's.
fn judge_members(
    scope: &AcceptedScope,
    depends_on: &[u32],
    stories: &[u32],
    edited: Option<u32>,
) -> Result<(), ScopeRefusal> {
    if let Some(&phase) = depends_on
        .iter()
        .find(|phase| scope.declared(**phase).is_none())
    {
        return Err(ScopeRefusal::NoSuchPhase { phase });
    }
    if let Some(&story) = stories.iter().find(|story| scope.story(**story).is_none()) {
        return Err(ScopeRefusal::NoSuchStory { story });
    }
    // Only an active story is promised, so only one may be served, as at
    // the start.
    if let Some(at) = stories.iter().position(|story| {
        scope
            .story(*story)
            .is_some_and(|story| story.status != HeldStatus::Active)
    }) {
        return Err(ScopeRefusal::Malformed {
            section: "stories".into(),
            field: format!("stories[{at}]"),
        });
    }
    for &story in stories {
        if let Some(other) = scope
            .declared_phases()
            .find(|phase| Some(phase.number) != edited && phase.stories.contains(&story))
        {
            return Err(ScopeRefusal::StoryDoubleAssigned {
                story,
                phase: other.number,
            });
        }
    }
    Ok(())
}

/// A withdrawal of `withdrawn`: no declared phase depends on it, so the
/// owner edits those dependencies first (D-04); every story it serves has a
/// decision (PRJ-R12); each decision is for one of those stories; and a
/// story moved goes to another declared phase (D-07).
fn judge_withdrawal(
    withdrawn: &HeldPhase,
    decisions: &[Decision],
    scope: &AcceptedScope,
) -> Result<(), ScopeRefusal> {
    let phase = withdrawn.number;
    let dependents: BTreeSet<u32> = scope
        .declared_phases()
        .filter(|held| held.number != phase && held.depends_on.contains(&phase))
        .map(|held| held.number)
        .collect();
    if !dependents.is_empty() {
        return Err(ScopeRefusal::PhaseHasDependents {
            phases: dependents.into_iter().collect(),
        });
    }
    if let Some(&story) = withdrawn
        .stories
        .iter()
        .find(|story| !decisions.iter().any(|decision| decision.story == **story))
    {
        return Err(ScopeRefusal::StoryUndecided { story });
    }
    if let Some(decision) = decisions
        .iter()
        .find(|decision| !withdrawn.stories.contains(&decision.story))
    {
        return Err(ScopeRefusal::NoSuchStory {
            story: decision.story,
        });
    }
    if let Some(to) = decisions.iter().find_map(|decision| match decision.to {
        Destination::Phase(to) if to == phase || scope.declared(to).is_none() => Some(to),
        _ => None,
    }) {
        return Err(ScopeRefusal::NoSuchPhase { phase: to });
    }
    Ok(())
}

/// A full phase order: every number a declared phase, every declared phase
/// once, and no phase before one it depends on.
fn judge_reorder(order: &[u32], scope: &AcceptedScope) -> Result<(), ScopeRefusal> {
    if let Some(&phase) = order.iter().find(|phase| scope.declared(**phase).is_none()) {
        return Err(ScopeRefusal::NoSuchPhase { phase });
    }
    let declared: BTreeSet<u32> = scope.declared_phases().map(|phase| phase.number).collect();
    let held: BTreeSet<u32> = order.iter().copied().collect();
    // Every entry is declared, so a full order is one with no repeat and
    // as many entries as there are declared phases.
    if held.len() != order.len() || order.len() != declared.len() {
        return Err(ScopeRefusal::OrderIncomplete {
            order: declared.into_iter().collect(),
        });
    }
    for &phase in order {
        if let Some(held) = scope.declared(phase) {
            placed_after(order, phase, &held.depends_on)?;
        }
    }
    Ok(())
}

/// `dependency-order` naming the first of `depends_on`, in its own order,
/// that `order` places after `phase`.
fn placed_after(order: &[u32], phase: u32, depends_on: &[u32]) -> Result<(), ScopeRefusal> {
    let place = |number: u32| order.iter().position(|held| *held == number);
    let Some(at) = place(phase) else {
        return Ok(());
    };
    match depends_on
        .iter()
        .find(|depends_on| place(**depends_on).is_some_and(|place| place > at))
    {
        Some(&depends_on) => Err(ScopeRefusal::DependencyOrder { phase, depends_on }),
        None => Ok(()),
    }
}

/// Every declared phase on a dependency cycle once `edited` depends on
/// `depends_on`, ascending, a phase depending on itself included. Accepted
/// scope has no cycle, so every cycle runs through the edited phase, and
/// its phases are those the edited phase reaches that also reach it.
fn on_cycle_with(scope: &AcceptedScope, edited: u32, depends_on: &[u32]) -> Vec<u32> {
    let mut edges: BTreeMap<u32, Vec<u32>> = scope
        .declared_phases()
        .map(|phase| (phase.number, phase.depends_on.clone()))
        .collect();
    edges.insert(edited, depends_on.to_vec());
    let mut reverse: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (&phase, depends_on) in &edges {
        for &dependency in depends_on {
            reverse.entry(dependency).or_default().push(phase);
        }
    }
    let reached = reach(edited, &edges);
    let reaching = reach(edited, &reverse);
    reached.intersection(&reaching).copied().collect()
}

/// Every phase one or more steps from `from` along `edges`. Kept iterative
/// so a long chain cannot exhaust the stack.
fn reach(from: u32, edges: &BTreeMap<u32, Vec<u32>>) -> BTreeSet<u32> {
    let next = |phase: u32| edges.get(&phase).into_iter().flatten().copied();
    let mut seen = BTreeSet::new();
    let mut pending: Vec<u32> = next(from).collect();
    while let Some(phase) = pending.pop() {
        if seen.insert(phase) {
            pending.extend(next(phase));
        }
    }
    seen
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
pub(super) mod tests {
    //! Scopes are written out as the `roadmap` projector writes its
    //! document, and every expected code and detail is read off design 0004
    //! by hand.

    use std::collections::BTreeMap;

    use serde_json::{Value, json};

    use super::*;
    use crate::scope::{ChangeKind, ChangeSubmission, judge_change};

    /// A0, the roadmap after the approved S1.
    pub(in crate::scope) fn a0() -> Value {
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

    pub(in crate::scope) fn scope(document: &Value) -> AcceptedScope {
        AcceptedScope::read(Some(document)).expect("a readable roadmap")
    }

    pub(in crate::scope) fn change(kind: ChangeKind, value: Value) -> ChangeSubmission {
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

    /// `document` with each JSON pointer set to its value.
    pub(in crate::scope) fn with(document: Value, changes: &[(&str, Value)]) -> Value {
        let mut document = document;
        for (pointer, to) in changes {
            *document
                .pointer_mut(pointer)
                .unwrap_or_else(|| panic!("{pointer}")) = to.clone();
        }
        document
    }

    /// A1, A0 after phase 2 is withdrawn with story 2 moved to phase 1.
    pub(in crate::scope) fn a1() -> Value {
        with(
            a0(),
            &[
                ("/phases/1/status", json!("withdrawn")),
                ("/phases/1/stories", json!([])),
                ("/phases/0/stories", json!([1, 2])),
                ("/stories/1/phase", json!(1)),
                ("/order", json!([1])),
            ],
        )
    }

    /// A3, A0 with a third phase depending on nothing.
    fn a3() -> Value {
        let mut a3 = with(
            a0(),
            &[("/order", json!([1, 2, 3])), ("/highest_phase", json!(3))],
        );
        a3["phases"]
            .as_array_mut()
            .unwrap()
            .push(json!({"number": 3,
            "name": "Reports", "goal": "The owner reads progress",
            "depends_on": [], "stories": [], "status": "declared"}));
        a3
    }

    /// P, a new phase depending on phase 1, with each member set.
    pub(in crate::scope) fn p(changes: &[(&str, Value)]) -> ChangeSubmission {
        let mut value = json!({"name": "Reports", "goal": "The owner reads progress",
            "detail": "Progress from the ledger", "depends_on": [1], "stories": []});
        for (member, to) in changes {
            value[*member] = to.clone();
        }
        change(ChangeKind::PhaseDeclare, value)
    }

    /// D1, a revised description.
    pub(in crate::scope) fn d1() -> ChangeSubmission {
        change(
            ChangeKind::ProjectDescribe,
            json!({"what": "A ledger for agent work",
                "core_value": "Nothing moves on an unproven claim",
                "constraints": ["One binary", "No server"]}),
        )
    }

    /// The judgement of `change` against `document` after a start, over F0.
    fn judged(change: &ChangeSubmission, document: &Value) -> Result<(), (&'static str, Value)> {
        judged_over(change, document, &f0())
    }

    fn judged_over(
        change: &ChangeSubmission,
        document: &Value,
        progress: &BTreeMap<u32, PhaseProgress>,
    ) -> Result<(), (&'static str, Value)> {
        judge_scope_change(change, &scope(document), true, progress)
            .map_err(|refusal| (refusal.code(), refusal.details().unwrap_or(Value::Null)))
    }

    fn malformed(section: &str, field: &str) -> Result<(), (&'static str, Value)> {
        Err((
            "malformed-submission",
            json!({"section": section, "field": field}),
        ))
    }

    #[test]
    fn a_declaration_naming_a_missing_or_withdrawn_phase_or_story_accepted_is_caught() {
        assert_eq!(
            judged(&p(&[("depends_on", json!([9]))]), &a0()),
            Err(("no-such-phase", json!({"phase": 9})))
        );
        assert_eq!(
            judged(&p(&[("depends_on", json!([2]))]), &a1()),
            Err(("no-such-phase", json!({"phase": 2})))
        );
        assert_eq!(
            judged(&p(&[("stories", json!([9]))]), &a0()),
            Err(("no-such-story", json!({"story": 9})))
        );
        assert_eq!(judged(&p(&[]), &a0()), Ok(()));
    }

    #[test]
    fn a_declaration_serving_an_inactive_or_taken_story_accepted_is_caught() {
        assert_eq!(
            judged(&p(&[("stories", json!([3]))]), &a0()),
            malformed("stories", "stories[0]")
        );
        assert_eq!(
            judged(&p(&[("stories", json!([1]))]), &a0()),
            Err(("story-double-assigned", json!({"story": 1, "phase": 1})))
        );
        assert_eq!(
            judged(&p(&[("position", json!(4))]), &a0()),
            malformed("position", "position")
        );
        assert_eq!(judged(&p(&[("position", json!(3))]), &a0()), Ok(()));
    }

    #[test]
    fn an_edit_of_a_withdrawn_phase_or_one_closing_a_cycle_accepted_is_caught() {
        let edit = |value: Value| change(ChangeKind::PhaseEdit, value);
        assert_eq!(
            judged(&edit(json!({"phase": 2, "goal": "g"})), &a1()),
            Err(("no-such-phase", json!({"phase": 2})))
        );
        assert_eq!(
            judged(&edit(json!({"phase": 1, "depends_on": [2]})), &a0()),
            Err(("dependency-cycle", json!({"phases": [1, 2]})))
        );
        assert_eq!(
            judged(&edit(json!({"phase": 2, "stories": [1]})), &a0()),
            Err(("story-double-assigned", json!({"story": 1, "phase": 1})))
        );
        assert_eq!(
            judged(&edit(json!({"phase": 2, "stories": [2]})), &a0()),
            Ok(())
        );
    }

    #[test]
    fn a_reorder_missing_repeating_or_inverting_a_phase_accepted_is_caught() {
        let reorder = |order: Value| change(ChangeKind::PhaseReorder, json!({"order": order}));
        for (order, expected) in [
            (
                json!([1]),
                Err(("order-incomplete", json!({"order": [1, 2]}))),
            ),
            (
                json!([1, 2, 2]),
                Err(("order-incomplete", json!({"order": [1, 2]}))),
            ),
            (
                json!([1, 2, 3]),
                Err(("no-such-phase", json!({"phase": 3}))),
            ),
            (
                json!([2, 1]),
                Err(("dependency-order", json!({"phase": 2, "depends_on": 1}))),
            ),
            (json!([1, 2]), Ok(())),
        ] {
            assert_eq!(judged(&reorder(order.clone()), &a0()), expected, "{order}");
        }
        assert_eq!(
            judged(&reorder(json!([1, 2])), &a1()),
            Err(("no-such-phase", json!({"phase": 2})))
        );
    }

    #[test]
    fn a_declaration_or_edit_placing_a_phase_before_its_dependency_accepted_is_caught() {
        let edit = |value: Value| change(ChangeKind::PhaseEdit, value);
        assert_eq!(
            judged(&p(&[("position", json!(1))]), &a0()),
            Err(("dependency-order", json!({"phase": 3, "depends_on": 1})))
        );
        assert_eq!(judged(&p(&[("position", json!(2))]), &a0()), Ok(()));
        assert_eq!(
            judged(&edit(json!({"phase": 1, "depends_on": [3]})), &a3()),
            Err(("dependency-order", json!({"phase": 1, "depends_on": 3})))
        );
        assert_eq!(
            judged(&edit(json!({"phase": 3, "depends_on": [1]})), &a3()),
            Ok(())
        );
    }

    #[test]
    fn a_change_before_the_start_accepted_is_caught() {
        let unstarted = AcceptedScope::read(None).expect("the empty scope");
        for before in [
            d1(),
            p(&[("depends_on", json!([]))]),
            change(
                ChangeKind::StoryDeclare,
                json!({"sentence": "s", "status": "active", "phase": null}),
            ),
            change(ChangeKind::PhaseReorder, json!({"order": [1]})),
        ] {
            let refusal = judge_scope_change(&before, &unstarted, false, &f0())
                .expect_err("refused before the start");
            assert_eq!(
                (refusal.code(), refusal.details()),
                ("project-not-started", None),
                "{}",
                before.value()
            );
        }
        assert_eq!(judged(&d1(), &a0()), Ok(()));
    }

    fn withdraw(value: Value) -> ChangeSubmission {
        change(ChangeKind::PhaseWithdraw, value)
    }

    #[test]
    fn a_withdrawal_leaving_a_story_undecided_or_dependents_behind_accepted_is_caught() {
        assert_eq!(
            judged(
                &withdraw(json!({"phase": 1, "decisions": [{"story": 1, "to": "drop"}]})),
                &a0()
            ),
            Err(("phase-has-dependents", json!({"phases": [2]})))
        );
        assert_eq!(
            judged(&withdraw(json!({"phase": 2, "decisions": []})), &a0()),
            Err(("story-undecided", json!({"story": 2})))
        );
        for decided in [json!(1), json!("drop")] {
            assert_eq!(
                judged(
                    &withdraw(json!({"phase": 2, "decisions": [{"story": 2, "to": decided}]})),
                    &a0()
                ),
                Ok(()),
                "{decided}"
            );
        }
    }

    #[test]
    fn a_withdrawal_moving_a_story_nowhere_or_deciding_another_phases_story_accepted_is_caught() {
        for (value, expected) in [
            (
                json!({"phase": 2, "decisions": [{"story": 2, "to": 2}]}),
                ("no-such-phase", json!({"phase": 2})),
            ),
            (
                json!({"phase": 2, "decisions": [{"story": 2, "to": 9}]}),
                ("no-such-phase", json!({"phase": 9})),
            ),
            (
                json!({"phase": 2,
                    "decisions": [{"story": 2, "to": 1}, {"story": 1, "to": "drop"}]}),
                ("no-such-story", json!({"story": 1})),
            ),
            (
                json!({"phase": 9, "decisions": []}),
                ("no-such-phase", json!({"phase": 9})),
            ),
        ] {
            assert_eq!(
                judged(&withdraw(value.clone()), &a0()),
                Err(expected),
                "{value}"
            );
        }
    }

    #[test]
    fn a_withdrawal_judged_before_its_facts_is_caught() {
        assert_eq!(
            judged_over(
                &withdraw(json!({"phase": 2, "decisions": []})),
                &a0(),
                &f1()
            ),
            Err(("phase-complete", json!({"phase": 2})))
        );
    }

    /// A2, A0 with story 3 dropped.
    fn a2() -> Value {
        with(a0(), &[("/stories/2/status", json!("dropped"))])
    }

    /// A4, A0 with story 3 excluded.
    fn a4() -> Value {
        with(a0(), &[("/stories/2/status", json!("excluded"))])
    }

    #[test]
    fn an_edit_reassign_or_drop_of_a_missing_dropped_or_excluded_story_accepted_is_caught() {
        assert_eq!(
            judged(
                &change(ChangeKind::StoryEdit, json!({"story": 9, "sentence": "s"})),
                &a0()
            ),
            Err(("no-such-story", json!({"story": 9})))
        );
        let changes = [
            change(ChangeKind::StoryEdit, json!({"story": 3, "sentence": "s"})),
            change(ChangeKind::StoryReassign, json!({"story": 3, "phase": 2})),
            change(ChangeKind::StoryDrop, json!({"story": 3})),
        ];
        for (document, status) in [(a2(), "dropped"), (a4(), "excluded")] {
            for unserved in &changes {
                assert_eq!(
                    judged(unserved, &document),
                    Err(("story-not-served", json!({"story": 3, "status": status}))),
                    "{} over {status}",
                    unserved.value()
                );
            }
        }
    }

    #[test]
    fn a_story_moved_to_a_missing_or_withdrawn_phase_accepted_is_caught() {
        let reassign = |phase: Value| {
            change(
                ChangeKind::StoryReassign,
                json!({"story": 3, "phase": phase}),
            )
        };
        assert_eq!(
            judged(&reassign(json!(9)), &a0()),
            Err(("no-such-phase", json!({"phase": 9})))
        );
        assert_eq!(
            judged(
                &change(
                    ChangeKind::StoryDeclare,
                    json!({"sentence": "s", "status": "active", "phase": 9})
                ),
                &a0()
            ),
            Err(("no-such-phase", json!({"phase": 9})))
        );
        assert_eq!(
            judged(&reassign(json!(2)), &a1()),
            Err(("no-such-phase", json!({"phase": 2})))
        );
        assert_eq!(judged(&reassign(Value::Null), &a0()), Ok(()));
    }

    #[test]
    fn a_backlog_order_missing_repeating_or_holding_a_dropped_story_accepted_is_caught() {
        let reorder = |order: Value| change(ChangeKind::BacklogReorder, json!({"order": order}));
        for (order, document, expected) in [
            (
                json!([1, 2]),
                a0(),
                Err(("order-incomplete", json!({"order": [1, 2, 3]}))),
            ),
            (
                json!([1, 2, 3, 9]),
                a0(),
                Err(("no-such-story", json!({"story": 9}))),
            ),
            (
                json!([1, 2, 3]),
                a2(),
                Err(("order-incomplete", json!({"order": [1, 2]}))),
            ),
            (json!([2, 1]), a2(), Ok(())),
        ] {
            assert_eq!(
                judged(&reorder(order.clone()), &document),
                expected,
                "{order}"
            );
        }
    }

    #[test]
    fn a_shipped_story_moved_after_its_facts_is_caught() {
        for shipped in [
            change(
                ChangeKind::StoryReassign,
                json!({"story": 1, "phase": null}),
            ),
            change(ChangeKind::StoryDrop, json!({"story": 1})),
        ] {
            assert_eq!(
                judged_over(&shipped, &a2(), &f2()),
                Err(("story-shipped", json!({"story": 1, "phase": 1}))),
                "{}",
                shipped.value()
            );
        }
    }
}
