//! The judgement of a scope change submission, its digest and its target
//! (design 0004 sections 5 and 6).
//!
//! Each change kind has its own shape, judged whole as a start is: the
//! submission must be an object, an unknown member is refused before any
//! other fault, then each member is judged in the shape's order. Only the
//! shape is judged here; whether the change fits accepted scope is judged
//! against the roadmap elsewhere. Nothing here records anything.

use std::collections::HashSet;

use baley_store::{Hash, canonical_json, request_digest};
use serde_json::{Map, Value};

use super::submission::{
    DESCRIPTION_MEMBERS, Description, Judged, StoryStatus, description_members, list, malformed,
    object, text, unknown_member,
};

/// The member Baley adds to a change body, holding the kind's spelling.
const KIND_MEMBER: &str = "kind";
/// The target both reorders are filed under: the one order each keeps.
const ORDER_TARGET: &str = "order";
/// The target a description revision is filed under.
const DESCRIPTION_TARGET: &str = "description";
/// What a withdrawal decision's `to` holds for a story to drop.
const DROP: &str = "drop";

const PHASE_DECLARE_MEMBERS: &[&str] = &[
    "name",
    "goal",
    "detail",
    "depends_on",
    "stories",
    "position",
];
const PHASE_EDIT_MEMBERS: &[&str] = &["phase", "name", "goal", "detail", "depends_on", "stories"];
const ORDER_MEMBERS: &[&str] = &["order"];
const PHASE_WITHDRAW_MEMBERS: &[&str] = &["phase", "decisions"];
pub(super) const DECISION_MEMBERS: &[&str] = &["story", "to"];
const STORY_DECLARE_MEMBERS: &[&str] = &["sentence", "status", "phase"];
const STORY_EDIT_MEMBERS: &[&str] = &["story", "sentence"];
const STORY_REASSIGN_MEMBERS: &[&str] = &["story", "phase"];
const STORY_DROP_MEMBERS: &[&str] = &["story"];

/// A kind of scope change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// A revised project description.
    ProjectDescribe,
    /// A new phase.
    PhaseDeclare,
    /// Changed fields of one phase.
    PhaseEdit,
    /// A new phase order.
    PhaseReorder,
    /// One phase withdrawn, with a decision for each story it served.
    PhaseWithdraw,
    /// A new story.
    StoryDeclare,
    /// One story's corrected wording.
    StoryEdit,
    /// One story moved to another phase or the backlog.
    StoryReassign,
    /// One story dropped.
    StoryDrop,
    /// A new priority order of the backlog.
    BacklogReorder,
}

impl ChangeKind {
    /// The kind as change bodies and `scope.approved` spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProjectDescribe => "project-describe",
            Self::PhaseDeclare => "phase-declare",
            Self::PhaseEdit => "phase-edit",
            Self::PhaseReorder => "phase-reorder",
            Self::PhaseWithdraw => "phase-withdraw",
            Self::StoryDeclare => "story-declare",
            Self::StoryEdit => "story-edit",
            Self::StoryReassign => "story-reassign",
            Self::StoryDrop => "story-drop",
            Self::BacklogReorder => "backlog-reorder",
        }
    }

    /// The members a change of this kind may hold, in the shape's order.
    pub(super) fn members(self) -> &'static [&'static str] {
        match self {
            Self::ProjectDescribe => DESCRIPTION_MEMBERS,
            Self::PhaseDeclare => PHASE_DECLARE_MEMBERS,
            Self::PhaseEdit => PHASE_EDIT_MEMBERS,
            Self::PhaseReorder | Self::BacklogReorder => ORDER_MEMBERS,
            Self::PhaseWithdraw => PHASE_WITHDRAW_MEMBERS,
            Self::StoryDeclare => STORY_DECLARE_MEMBERS,
            Self::StoryEdit => STORY_EDIT_MEMBERS,
            Self::StoryReassign => STORY_REASSIGN_MEMBERS,
            Self::StoryDrop => STORY_DROP_MEMBERS,
        }
    }
}

/// A new phase as a `phase-declare` states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseDeclaration {
    /// The phase's name.
    pub name: String,
    /// Its goal.
    pub goal: String,
    /// Its detail text.
    pub detail: String,
    /// The phase numbers it depends on, none repeated.
    pub depends_on: Vec<u32>,
    /// The story ids it serves, none repeated.
    pub stories: Vec<u32>,
    /// Its 1-based place in the order, or last when absent.
    pub position: Option<u32>,
}

/// The fields a `phase-edit` changes; an absent one keeps the declaration
/// in force.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhaseFields {
    /// A new name.
    pub name: Option<String>,
    /// A new goal.
    pub goal: Option<String>,
    /// A new detail text.
    pub detail: Option<String>,
    /// New dependencies, none repeated.
    pub depends_on: Option<Vec<u32>>,
    /// New stories, none repeated.
    pub stories: Option<Vec<u32>>,
}

/// Where a withdrawal sends one story of the withdrawn phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    /// Served by this phase number instead.
    Phase(u32),
    /// Dropped.
    Drop,
}

/// One story's decision in a `phase-withdraw`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    /// The story's id.
    pub story: u32,
    /// Where it goes.
    pub to: Destination,
}

/// A change's members as typed values. Text is kept as written, never
/// trimmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeChange {
    /// `project-describe`: the whole new description.
    ProjectDescribe(Description),
    /// `phase-declare`.
    PhaseDeclare(PhaseDeclaration),
    /// `phase-edit` of phase `phase`.
    PhaseEdit { phase: u32, fields: PhaseFields },
    /// `phase-reorder`: phase numbers in their new order.
    PhaseReorder { order: Vec<u32> },
    /// `phase-withdraw` of phase `phase`, no story decided twice.
    PhaseWithdraw {
        phase: u32,
        decisions: Vec<Decision>,
    },
    /// `story-declare`: a story served by `phase`, or in the backlog.
    StoryDeclare {
        sentence: String,
        status: StoryStatus,
        phase: Option<u32>,
    },
    /// `story-edit` of story `story`.
    StoryEdit { story: u32, sentence: String },
    /// `story-reassign` of story `story` to `phase`, or to the backlog.
    StoryReassign { story: u32, phase: Option<u32> },
    /// `story-drop` of story `story`.
    StoryDrop { story: u32 },
    /// `backlog-reorder`: story ids in their new priority order.
    BacklogReorder { order: Vec<u32> },
}

impl ScopeChange {
    /// The change's kind.
    pub fn kind(&self) -> ChangeKind {
        match self {
            Self::ProjectDescribe(_) => ChangeKind::ProjectDescribe,
            Self::PhaseDeclare(_) => ChangeKind::PhaseDeclare,
            Self::PhaseEdit { .. } => ChangeKind::PhaseEdit,
            Self::PhaseReorder { .. } => ChangeKind::PhaseReorder,
            Self::PhaseWithdraw { .. } => ChangeKind::PhaseWithdraw,
            Self::StoryDeclare { .. } => ChangeKind::StoryDeclare,
            Self::StoryEdit { .. } => ChangeKind::StoryEdit,
            Self::StoryReassign { .. } => ChangeKind::StoryReassign,
            Self::StoryDrop { .. } => ChangeKind::StoryDrop,
            Self::BacklogReorder { .. } => ChangeKind::BacklogReorder,
        }
    }
}

/// A change submission judged whole and accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSubmission {
    /// The change's typed members.
    pub change: ScopeChange,
    value: Value,
    body: Vec<u8>,
    digest: Hash,
    target: String,
}

impl ChangeSubmission {
    /// The change's kind.
    pub fn kind(&self) -> ChangeKind {
        self.change.kind()
    }

    /// The members as given, with `kind` added.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// The canonical JSON bytes the digest is taken over, stored as the
    /// change's body.
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// The SHA-256 of the body and nothing else. The body holds the kind,
    /// so two kinds with the same members never share a digest.
    pub fn digest(&self) -> Hash {
        self.digest
    }

    /// What the change is filed under among drafts of its kind, so only a
    /// newer submission of the same change makes it stale: the phase number
    /// or story id it changes, `order`, `description`, or for a declaration
    /// its own digest, since each declaration is a change of its own.
    pub fn target(&self) -> &str {
        &self.target
    }
}

/// Judges a change submission of `kind` given as plain JSON, its shape
/// only. A `kind` member the caller sent is unknown, since Baley adds it.
pub fn judge_change(kind: ChangeKind, value: &Value) -> Judged<ChangeSubmission> {
    let Value::Object(members) = value else {
        return Err(malformed("submission", "submission"));
    };
    if let Some(unknown) = unknown_member(members, kind.members()) {
        return Err(malformed(unknown, unknown));
    }
    let change = judge_members(kind, members)?;
    let mut body = members.clone();
    body.insert(KIND_MEMBER.into(), Value::String(kind.as_str().into()));
    let value = Value::Object(body);
    // Every leaf of an accepted change is text, a number within u32 or
    // null, so it always has a canonical form.
    let digest = request_digest(&value).expect("an accepted change is canonical");
    let body = canonical_json(&value).expect("an accepted change is canonical");
    let target = match &change {
        ScopeChange::PhaseEdit { phase, .. } | ScopeChange::PhaseWithdraw { phase, .. } => {
            phase.to_string()
        }
        ScopeChange::StoryEdit { story, .. }
        | ScopeChange::StoryReassign { story, .. }
        | ScopeChange::StoryDrop { story } => story.to_string(),
        ScopeChange::PhaseReorder { .. } | ScopeChange::BacklogReorder { .. } => {
            ORDER_TARGET.into()
        }
        ScopeChange::ProjectDescribe(_) => DESCRIPTION_TARGET.into(),
        ScopeChange::PhaseDeclare(_) | ScopeChange::StoryDeclare { .. } => digest.to_hex(),
    };
    Ok(ChangeSubmission {
        change,
        value,
        body,
        digest,
        target,
    })
}

fn judge_members(kind: ChangeKind, members: &Map<String, Value>) -> Judged<ScopeChange> {
    let member = |name: &str| members.get(name);
    Ok(match kind {
        ChangeKind::ProjectDescribe => {
            ScopeChange::ProjectDescribe(description_members(members, None)?)
        }
        ChangeKind::PhaseDeclare => ScopeChange::PhaseDeclare(PhaseDeclaration {
            name: text_member(members, "name")?,
            goal: text_member(members, "goal")?,
            detail: text_member(members, "detail")?,
            depends_on: distinct(member("depends_on"), "depends_on")?,
            stories: distinct(member("stories"), "stories")?,
            position: optional(members, "position", |value| {
                number(value, "position", "position")
            })?,
        }),
        ChangeKind::PhaseEdit => {
            let phase = number(member("phase"), "phase", "phase")?;
            let fields = PhaseFields {
                name: optional(members, "name", |_| text_member(members, "name"))?,
                goal: optional(members, "goal", |_| text_member(members, "goal"))?,
                detail: optional(members, "detail", |_| text_member(members, "detail"))?,
                depends_on: optional(members, "depends_on", |value| distinct(value, "depends_on"))?,
                stories: optional(members, "stories", |value| distinct(value, "stories"))?,
            };
            // With nothing to change it would record the declaration in
            // force again.
            if fields == PhaseFields::default() {
                return Err(malformed("submission", "submission"));
            }
            ScopeChange::PhaseEdit { phase, fields }
        }
        ChangeKind::PhaseReorder => ScopeChange::PhaseReorder {
            order: order(member("order"))?,
        },
        ChangeKind::BacklogReorder => ScopeChange::BacklogReorder {
            order: order(member("order"))?,
        },
        ChangeKind::PhaseWithdraw => ScopeChange::PhaseWithdraw {
            phase: number(member("phase"), "phase", "phase")?,
            decisions: decisions(member("decisions"))?,
        },
        ChangeKind::StoryDeclare => {
            let sentence = text_member(members, "sentence")?;
            let status = match member("status") {
                Some(Value::String(status)) => StoryStatus::parse(status),
                _ => None,
            }
            .ok_or_else(|| malformed("status", "status"))?;
            let phase = number_or_null(member("phase"), "phase")?;
            // Only an active story is promised, so only one may be served.
            if phase.is_some() && status != StoryStatus::Active {
                return Err(malformed("phase", "phase"));
            }
            ScopeChange::StoryDeclare {
                sentence,
                status,
                phase,
            }
        }
        ChangeKind::StoryEdit => ScopeChange::StoryEdit {
            story: number(member("story"), "story", "story")?,
            sentence: text_member(members, "sentence")?,
        },
        ChangeKind::StoryReassign => ScopeChange::StoryReassign {
            story: number(member("story"), "story", "story")?,
            phase: number_or_null(member("phase"), "phase")?,
        },
        ChangeKind::StoryDrop => ScopeChange::StoryDrop {
            story: number(member("story"), "story", "story")?,
        },
    })
}

/// The top-level text member `name`, named by itself.
fn text_member(members: &Map<String, Value>, name: &str) -> Judged<String> {
    text(members.get(name), name, name)
}

/// `judge` of member `name`, or none when the member is absent. A member
/// present as null is judged, and refused.
fn optional<T>(
    members: &Map<String, Value>,
    name: &str,
    judge: impl FnOnce(Option<&Value>) -> Judged<T>,
) -> Judged<Option<T>> {
    match members.get(name) {
        None => Ok(None),
        value => judge(value).map(Some),
    }
}

/// A phase number or story id: a whole number from 1 to `u32::MAX`.
fn number(value: Option<&Value>, section: &str, field: &str) -> Judged<u32> {
    value
        .and_then(Value::as_u64)
        .and_then(|number| u32::try_from(number).ok())
        .filter(|number| *number >= 1)
        .ok_or_else(|| malformed(section, field))
}

/// The top-level member `name` as a phase number, or null for none.
fn number_or_null(value: Option<&Value>, name: &str) -> Judged<Option<u32>> {
    match value {
        Some(Value::Null) => Ok(None),
        value => number(value, name, name).map(Some),
    }
}

/// The top-level list `name` of numbers, possibly none, none repeated.
fn distinct(value: Option<&Value>, name: &str) -> Judged<Vec<u32>> {
    let mut seen = HashSet::new();
    let mut numbers = Vec::new();
    for (at, entry) in list(value, name, name)?.iter().enumerate() {
        let field = format!("{name}[{at}]");
        let number = number(Some(entry), name, &field)?;
        if !seen.insert(number) {
            return Err(malformed(name, &field));
        }
        numbers.push(number);
    }
    Ok(numbers)
}

/// A full order of at least one number. A repeat is left to the judgement
/// against accepted scope, which names what the order lacks.
fn order(value: Option<&Value>) -> Judged<Vec<u32>> {
    const NAME: &str = "order";
    let entries = list(value, NAME, NAME)?;
    if entries.is_empty() {
        return Err(malformed(NAME, NAME));
    }
    entries
        .iter()
        .enumerate()
        .map(|(at, entry)| number(Some(entry), NAME, &format!("{NAME}[{at}]")))
        .collect()
}

/// A withdrawal's decisions, possibly none, no story decided twice.
fn decisions(value: Option<&Value>) -> Judged<Vec<Decision>> {
    const SECTION: &str = "decisions";
    let mut decided = HashSet::new();
    let mut decisions = Vec::new();
    for (at, entry) in list(value, SECTION, SECTION)?.iter().enumerate() {
        let path = format!("{SECTION}[{at}]");
        let entry = object(Some(entry), SECTION, &path, DECISION_MEMBERS)?;
        let story_path = format!("{path}.story");
        let story = number(entry.get("story"), SECTION, &story_path)?;
        if !decided.insert(story) {
            return Err(malformed(SECTION, &story_path));
        }
        let to = match entry.get("to") {
            Some(Value::String(to)) if to == DROP => Destination::Drop,
            to => Destination::Phase(number(to, SECTION, &format!("{path}.to"))?),
        };
        decisions.push(Decision { story, to });
    }
    Ok(decisions)
}

#[cfg(test)]
mod tests {
    //! Expected digests, bodies and targets are the plan's, the digests
    //! computed with `sha256sum` over the canonical bytes written out by
    //! hand.

    use serde_json::json;

    use super::super::submission::SubmissionRefusal;
    use super::*;

    /// D1, a revised description.
    fn d1() -> Value {
        json!({"what": "A ledger for agent work",
            "core_value": "Nothing moves on an unproven claim",
            "constraints": ["One binary", "No server"]})
    }

    /// P, a new phase depending on phase 1.
    fn p() -> Value {
        json!({"name": "Reports", "goal": "The owner reads progress",
            "detail": "Progress from the ledger", "depends_on": [1], "stories": []})
    }

    fn accepted(kind: ChangeKind, value: &Value) -> ChangeSubmission {
        judge_change(kind, value).unwrap_or_else(|refusal| panic!("{value}: {refusal:?}"))
    }

    fn refusal(kind: ChangeKind, value: &Value) -> SubmissionRefusal {
        judge_change(kind, value).expect_err("refused")
    }

    fn malformed(section: &str, field: &str) -> SubmissionRefusal {
        SubmissionRefusal::Malformed {
            section: section.into(),
            field: field.into(),
        }
    }

    #[test]
    fn a_change_digest_without_its_kind_or_over_more_than_its_canonical_body_is_caught() {
        let order = json!({"order": [2, 1]});
        let phases = accepted(ChangeKind::PhaseReorder, &order);
        assert_eq!(
            phases.value(),
            &json!({"kind": "phase-reorder", "order": [2, 1]})
        );
        assert_eq!(phases.body(), br#"{"kind":"phase-reorder","order":[2,1]}"#);
        assert_eq!(phases.body().len(), 38);
        assert_eq!(
            phases.digest().to_hex(),
            "5c4b7650561455ec8d35bfe5be8bdca5b426260fb4ada5934e9a2b0ea0de6619"
        );
        let backlog = accepted(ChangeKind::BacklogReorder, &order);
        assert_eq!(
            backlog.digest().to_hex(),
            "cd1d1badf514e109dbb4cf81d97a87ec909b603aabd5876bba434c560c084159"
        );
    }

    #[test]
    fn a_change_member_missing_blank_unknown_or_off_type_accepted_or_named_wrongly_is_caught() {
        let cases = [
            (
                ChangeKind::PhaseDeclare,
                json!({"name": "n", "detail": "d", "depends_on": [], "stories": []}),
                ("goal", "goal"),
            ),
            (
                ChangeKind::StoryEdit,
                json!({"story": 3, "sentence": "  "}),
                ("sentence", "sentence"),
            ),
            (
                ChangeKind::StoryDeclare,
                json!({"sentence": "s", "status": "active", "phase": null, "capture": "c-1"}),
                ("capture", "capture"),
            ),
            (
                ChangeKind::PhaseWithdraw,
                json!({"phase": 2, "decisions": [{"story": 2, "to": "later"}]}),
                ("decisions", "decisions[0].to"),
            ),
            (
                ChangeKind::PhaseReorder,
                json!({"kind": "phase-reorder", "order": [2, 1]}),
                ("kind", "kind"),
            ),
            (
                ChangeKind::PhaseReorder,
                json!({"order": []}),
                ("order", "order"),
            ),
            (
                ChangeKind::PhaseEdit,
                json!({"phase": 0, "goal": "g"}),
                ("phase", "phase"),
            ),
            (
                ChangeKind::PhaseReorder,
                json!({"order": [1, 4_294_967_296_u64]}),
                ("order", "order[1]"),
            ),
            (
                ChangeKind::StoryDrop,
                json!(["3"]),
                ("submission", "submission"),
            ),
        ];
        for (kind, value, (section, field)) in cases {
            assert_eq!(refusal(kind, &value), malformed(section, field), "{value}");
        }
    }

    #[test]
    fn a_story_declared_deferred_into_a_phase_accepted_is_caught() {
        let declare = |status: &str, phase: Value| {
            judge_change(
                ChangeKind::StoryDeclare,
                &json!({"sentence": "s", "status": status, "phase": phase}),
            )
        };
        assert_eq!(
            declare("deferred", json!(2)),
            Err(malformed("phase", "phase"))
        );
        assert!(declare("active", json!(2)).is_ok());
        assert!(declare("deferred", Value::Null).is_ok());
        assert_eq!(
            declare("excluded", json!(2)),
            Err(malformed("phase", "phase"))
        );
    }

    #[test]
    fn an_edit_with_nothing_to_change_accepted_is_caught() {
        assert_eq!(
            refusal(ChangeKind::PhaseEdit, &json!({"phase": 2})),
            malformed("submission", "submission")
        );
    }

    #[test]
    fn a_repeated_dependency_story_or_decision_accepted_is_caught() {
        let mut depends = p();
        depends["depends_on"] = json!([1, 1]);
        let mut stories = p();
        stories["stories"] = json!([2, 2]);
        let withdraw = json!({"phase": 2,
            "decisions": [{"story": 2, "to": 1}, {"story": 2, "to": "drop"}]});
        assert_eq!(
            refusal(ChangeKind::PhaseDeclare, &depends),
            malformed("depends_on", "depends_on[1]")
        );
        assert_eq!(
            refusal(ChangeKind::PhaseDeclare, &stories),
            malformed("stories", "stories[1]")
        );
        assert_eq!(
            refusal(ChangeKind::PhaseWithdraw, &withdraw),
            malformed("decisions", "decisions[1].story")
        );
    }

    #[test]
    fn a_revised_description_judged_by_another_rule_than_the_starts_is_caught() {
        assert_eq!(
            refusal(
                ChangeKind::ProjectDescribe,
                &json!({"what": " ", "core_value": "v", "constraints": []})
            ),
            malformed("what", "what")
        );
        assert_eq!(
            refusal(
                ChangeKind::ProjectDescribe,
                &json!({"what": "w", "core_value": "v", "constraints": ["One binary", ""]})
            ),
            malformed("constraints", "constraints[1]")
        );
        assert_eq!(
            accepted(ChangeKind::ProjectDescribe, &d1())
                .digest()
                .to_hex(),
            "db6830acb9a086805f20c56b6dbec837c2fa59938cfabcec77d3ed5336ec1213"
        );
    }

    #[test]
    fn a_change_filed_under_another_target_is_caught() {
        let cases = [
            (ChangeKind::PhaseEdit, json!({"phase": 2, "goal": "g"}), "2"),
            (
                ChangeKind::PhaseWithdraw,
                json!({"phase": 2, "decisions": []}),
                "2",
            ),
            (
                ChangeKind::StoryEdit,
                json!({"story": 3, "sentence": "s"}),
                "3",
            ),
            (
                ChangeKind::StoryReassign,
                json!({"story": 3, "phase": null}),
                "3",
            ),
            (ChangeKind::StoryDrop, json!({"story": 3}), "3"),
            (ChangeKind::PhaseReorder, json!({"order": [1]}), "order"),
            (ChangeKind::BacklogReorder, json!({"order": [1]}), "order"),
            (ChangeKind::ProjectDescribe, d1(), "description"),
            (
                ChangeKind::PhaseDeclare,
                p(),
                "8f0a327edbe4d66b07976b54ba35edf61ce327e277bde38142c63777337b7874",
            ),
            (
                ChangeKind::StoryDeclare,
                json!({"sentence": "s", "status": "active", "phase": null}),
                "9566200e6d7007029754491b4f35fe57f4015e9688074490aada9fc742a72296",
            ),
        ];
        for (kind, value, target) in cases {
            assert_eq!(accepted(kind, &value).target(), target, "{value}");
        }
    }
}
