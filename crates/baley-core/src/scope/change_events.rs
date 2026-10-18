//! The records an approved scope change becomes (design 0004 sections 5
//! and 6).
//!
//! Each change kind appends its own events, in design 0004's order, each
//! on its stream, and every approval ends with `scope.approved` of its
//! kind. Nothing here reads a store or a clock: the binary supplies the
//! accepted scope, any stored detail, the owner and Baley's time.

use baley_store::{NewEvent, PayloadRef};

use super::approval::{NumbersExhausted, event, next_numbers};
use super::change::{ChangeSubmission, Destination, ScopeChange};
use super::event::{
    PHASE_DECLARED, PHASE_DECLARED_VERSION, PHASE_REORDERED, PHASE_REORDERED_VERSION,
    PHASE_WITHDRAWN, PHASE_WITHDRAWN_VERSION, PROJECT_DESCRIBED, PROJECT_DESCRIBED_VERSION,
    PROJECT_STREAM, ROADMAP_STREAM, SCOPE_APPROVED, SCOPE_APPROVED_VERSION, STORY_CORRECTED,
    STORY_CORRECTED_VERSION, STORY_DECLARED, STORY_DECLARED_VERSION, STORY_DROPPED,
    STORY_DROPPED_VERSION, STORY_REASSIGNED, STORY_REASSIGNED_VERSION, STORY_REPRIORITIZED,
    STORY_REPRIORITIZED_VERSION, approved_payload, described_payload, phase_declared_payload,
    phase_reordered_payload, phase_withdrawn_payload, story_corrected_payload,
    story_declared_payload, story_dropped_payload, story_reassigned_payload,
    story_reprioritized_payload,
};
use super::rules::AcceptedScope;
use super::submission::StoryStatus;

/// The events an approved change appends, ending with `scope.approved` of
/// its kind on `project`, given the accepted scope it was judged against,
/// the stored detail a declaration or an edit attaches, and the approval's
/// owner and time.
///
/// A declaration is numbered one past the highest phase number ever
/// declared, withdrawn phases included, so no number is reused (D-14), and
/// goes into the order at its position or last. An edit records a whole
/// new `phase.declared` for its phase, each field it leaves out taken from
/// the declaration in force (D-06), and `detail` is then the edit's new
/// detail or the one in force. A withdrawal records `phase.withdrawn`, then
/// each of the phase's stories moved or dropped in its stories' order, then
/// the order without it.
/// A story declaration takes the id one past the highest ever declared and
/// goes last in priority order; a story given a phase becomes active, and
/// one sent to the backlog or corrected keeps its status.
///
/// When a number would pass `u32::MAX` it is [`NumbersExhausted`] and no
/// events are given.
///
/// # Panics
///
/// When a declaration or an edit has no `detail`, an edit names a phase
/// `scope` does not hold, or a story change names a story it does not hold
/// or one dropped. The scope judgement refuses such a change first.
pub fn change_events(
    change: &ChangeSubmission,
    scope: &AcceptedScope,
    detail: Option<&PayloadRef>,
    owner: &str,
    at: &str,
) -> Result<Vec<NewEvent>, NumbersExhausted> {
    let digest = change.digest();
    let attached = || detail.expect("a declaration or an edit attaches its detail");
    let roadmap = |type_name: &str, version: u32, payload, attachments| {
        event(ROADMAP_STREAM, type_name, version, payload, attachments)
    };
    let mut events = Vec::new();
    match &change.change {
        ScopeChange::PhaseDeclare(declaration) => {
            let number = next_numbers(scope.highest_phase, 1)?[0];
            let detail = attached();
            events.push(roadmap(
                PHASE_DECLARED,
                PHASE_DECLARED_VERSION,
                phase_declared_payload(
                    number,
                    &declaration.name,
                    &declaration.goal,
                    detail,
                    &declaration.depends_on,
                    &declaration.stories,
                    digest,
                ),
                vec![detail.clone()],
            ));
            let mut order = scope.order.clone();
            let at = declaration
                .position
                .and_then(|position| usize::try_from(position).ok())
                .map_or(order.len(), |position| (position - 1).min(order.len()));
            order.insert(at, number);
            events.push(roadmap(
                PHASE_REORDERED,
                PHASE_REORDERED_VERSION,
                phase_reordered_payload(&order, digest),
                Vec::new(),
            ));
        }
        ScopeChange::PhaseEdit { phase, fields } => {
            let held = scope
                .phase(*phase)
                .expect("an approved edit names a phase the scope holds");
            let detail = attached();
            events.push(roadmap(
                PHASE_DECLARED,
                PHASE_DECLARED_VERSION,
                phase_declared_payload(
                    *phase,
                    fields.name.as_ref().unwrap_or(&held.name),
                    fields.goal.as_ref().unwrap_or(&held.goal),
                    detail,
                    fields.depends_on.as_ref().unwrap_or(&held.depends_on),
                    fields.stories.as_ref().unwrap_or(&held.stories),
                    digest,
                ),
                vec![detail.clone()],
            ));
        }
        ScopeChange::PhaseReorder { order } => {
            events.push(roadmap(
                PHASE_REORDERED,
                PHASE_REORDERED_VERSION,
                phase_reordered_payload(order, digest),
                Vec::new(),
            ));
        }
        ScopeChange::PhaseWithdraw { phase, decisions } => {
            events.push(roadmap(
                PHASE_WITHDRAWN,
                PHASE_WITHDRAWN_VERSION,
                phase_withdrawn_payload(*phase, digest),
                Vec::new(),
            ));
            let stories = scope.phase(*phase).map_or(&[][..], |held| &held.stories);
            for story in stories {
                // The scope judgement gives every story of the phase a
                // decision.
                let Some(decision) = decisions.iter().find(|decision| decision.story == *story)
                else {
                    continue;
                };
                events.push(match decision.to {
                    Destination::Phase(to) => roadmap(
                        STORY_REASSIGNED,
                        STORY_REASSIGNED_VERSION,
                        story_reassigned_payload(*story, StoryStatus::Active, Some(to), digest),
                        Vec::new(),
                    ),
                    Destination::Drop => roadmap(
                        STORY_DROPPED,
                        STORY_DROPPED_VERSION,
                        story_dropped_payload(*story, digest),
                        Vec::new(),
                    ),
                });
            }
            let order: Vec<u32> = scope
                .order
                .iter()
                .copied()
                .filter(|number| number != phase)
                .collect();
            events.push(roadmap(
                PHASE_REORDERED,
                PHASE_REORDERED_VERSION,
                phase_reordered_payload(&order, digest),
                Vec::new(),
            ));
        }
        ScopeChange::StoryDeclare {
            sentence,
            status,
            phase,
        } => {
            let id = next_numbers(scope.highest_story, 1)?[0];
            events.push(roadmap(
                STORY_DECLARED,
                STORY_DECLARED_VERSION,
                story_declared_payload(id, sentence, *status, *phase, digest),
                Vec::new(),
            ));
        }
        ScopeChange::StoryEdit { story, sentence } => {
            events.push(roadmap(
                STORY_CORRECTED,
                STORY_CORRECTED_VERSION,
                story_corrected_payload(*story, sentence, held_status(scope, *story), digest),
                Vec::new(),
            ));
        }
        ScopeChange::StoryReassign { story, phase } => {
            // A story given a phase is promised, so a deferred one becomes
            // active (Figure 2); one sent to the backlog keeps its status.
            let status = match phase {
                Some(_) => StoryStatus::Active,
                None => held_status(scope, *story),
            };
            events.push(roadmap(
                STORY_REASSIGNED,
                STORY_REASSIGNED_VERSION,
                story_reassigned_payload(*story, status, *phase, digest),
                Vec::new(),
            ));
        }
        ScopeChange::StoryDrop { story } => {
            events.push(roadmap(
                STORY_DROPPED,
                STORY_DROPPED_VERSION,
                story_dropped_payload(*story, digest),
                Vec::new(),
            ));
        }
        ScopeChange::BacklogReorder { order } => {
            events.push(roadmap(
                STORY_REPRIORITIZED,
                STORY_REPRIORITIZED_VERSION,
                story_reprioritized_payload(order, digest),
                Vec::new(),
            ));
        }
        ScopeChange::ProjectDescribe(description) => {
            events.push(event(
                PROJECT_STREAM,
                PROJECT_DESCRIBED,
                PROJECT_DESCRIBED_VERSION,
                described_payload(description, digest),
                Vec::new(),
            ));
        }
    }
    events.push(event(
        PROJECT_STREAM,
        SCOPE_APPROVED,
        SCOPE_APPROVED_VERSION,
        approved_payload(digest, change.kind().as_str(), owner, at),
        Vec::new(),
    ));
    Ok(events)
}

/// Story `id`'s status in accepted scope.
///
/// # Panics
///
/// When the scope holds no such story or it is dropped. The scope judgement
/// refuses a change to either first.
fn held_status(scope: &AcceptedScope, id: u32) -> StoryStatus {
    scope
        .story(id)
        .and_then(|story| story.status.stated())
        .expect("an approved story change names a story still served")
}

#[cfg(test)]
mod tests {
    //! Expected payloads are written out from design 0004 section 6's field
    //! names and the plan's digests, never from running this code.

    use baley_store::PayloadRef;
    use serde_json::{Value, json};

    use super::*;
    use crate::scope::rules::tests::{a0, a1, change, d1, p, scope, with};
    use crate::scope::{ChangeKind, ChangeSubmission};

    const AT: &str = "2026-10-13T09:00:05Z";

    /// DP, the stored detail `Progress from the ledger`.
    fn dp() -> PayloadRef {
        PayloadRef::from_value(&json!({
            "payload": "40cd320a31a7ab226dc42f8ca21d7c9159fcdbec87ff12f2b1a8fc362ac0642a",
            "bytes": 24, "class": "record"}))
        .expect("a reference")
    }

    /// DB, the stored detail `Approval by digest`.
    fn db() -> PayloadRef {
        PayloadRef::from_value(&json!({
            "payload": "0eae12e355314abb5595d44ce4f4014a294d8d4cf709555975ef5f41648d7339",
            "bytes": 18, "class": "record"}))
        .expect("a reference")
    }

    fn events_of(
        change: &ChangeSubmission,
        document: &Value,
        detail: Option<&PayloadRef>,
    ) -> Vec<NewEvent> {
        change_events(change, &scope(document), detail, "Ada Owner", AT).expect("numbers fit")
    }

    /// Each event's type, stream and payload.
    fn placed(events: &[NewEvent]) -> Vec<(&str, &str, &Value)> {
        events
            .iter()
            .map(|event| {
                (
                    event.type_name.as_str(),
                    event.stream.0.as_str(),
                    &event.payload,
                )
            })
            .collect()
    }

    fn approved(digest: &str, kind: &str) -> Value {
        json!({"submission": digest, "kind": kind, "owner": "Ada Owner", "at": AT})
    }

    #[test]
    fn a_declared_phase_numbered_below_a_withdrawn_one_or_placed_wrongly_is_caught() {
        const P: &str = "8f0a327edbe4d66b07976b54ba35edf61ce327e277bde38142c63777337b7874";
        let events = events_of(&p(&[]), &a1(), Some(&dp()));
        assert_eq!(
            placed(&events),
            [
                (
                    "phase.declared",
                    "roadmap",
                    &json!({"number": 3, "name": "Reports", "goal": "The owner reads progress",
                        "detail": dp().to_value(), "depends_on": [1], "stories": [],
                        "submission": P})
                ),
                (
                    "phase.reordered",
                    "roadmap",
                    &json!({"order": [1, 3], "submission": P})
                ),
                ("scope.approved", "project", &approved(P, "phase-declare")),
            ]
        );
        assert_eq!(events[0].attachments, [dp()]);
        let placed_second = events_of(&p(&[("position", json!(2))]), &a0(), Some(&dp()));
        assert_eq!(placed_second[1].payload["order"], json!([1, 3, 2]));
    }

    #[test]
    fn a_phase_number_wrapped_past_the_largest_is_caught() {
        let full = scope(&with(a0(), &[("/highest_phase", json!(4_294_967_295_u64))]));
        assert_eq!(
            change_events(&p(&[]), &full, Some(&dp()), "Ada Owner", AT),
            Err(NumbersExhausted)
        );
    }

    #[test]
    fn an_edit_recording_only_its_changed_fields_is_caught() {
        const E_B: &str = "c95eaf35543ddb6517a592689362c88bb8d10f6b158924207c90833fb5c4ebd6";
        let e_b = change(
            ChangeKind::PhaseEdit,
            json!({"phase": 2, "goal": "The owner approves every change"}),
        );
        let events = events_of(&e_b, &a0(), Some(&db()));
        assert_eq!(
            placed(&events),
            [
                (
                    "phase.declared",
                    "roadmap",
                    &json!({"number": 2, "name": "Approval",
                        "goal": "The owner approves every change", "detail": db().to_value(),
                        "depends_on": [1], "stories": [2], "submission": E_B})
                ),
                ("scope.approved", "project", &approved(E_B, "phase-edit")),
            ]
        );
        assert_eq!(events[0].attachments, [db()]);
    }

    #[test]
    fn a_withdrawal_or_reorder_appending_its_events_in_another_order_is_caught() {
        const W: &str = "830b6dfddb927b4f83c616d5fc67832b672c1af4c686d8ea578f17e01fcdf0b6";
        let w = |to: Value| {
            change(
                ChangeKind::PhaseWithdraw,
                json!({"phase": 2, "decisions": [{"story": 2, "to": to}]}),
            )
        };
        assert_eq!(
            placed(&events_of(&w(json!(1)), &a0(), None)),
            [
                (
                    "phase.withdrawn",
                    "roadmap",
                    &json!({"number": 2, "submission": W})
                ),
                (
                    "story.reassigned",
                    "roadmap",
                    &json!({"id": 2, "status": "active", "phase": 1, "submission": W})
                ),
                (
                    "phase.reordered",
                    "roadmap",
                    &json!({"order": [1], "submission": W})
                ),
                ("scope.approved", "project", &approved(W, "phase-withdraw")),
            ]
        );
        let dropped = w(json!("drop"));
        let digest = dropped.digest().to_hex();
        let events = events_of(&dropped, &a0(), None);
        let types: Vec<(&str, &str)> = placed(&events)
            .into_iter()
            .map(|(type_name, stream, _)| (type_name, stream))
            .collect();
        assert_eq!(
            types,
            [
                ("phase.withdrawn", "roadmap"),
                ("story.dropped", "roadmap"),
                ("phase.reordered", "roadmap"),
                ("scope.approved", "project"),
            ]
        );
        assert_eq!(
            events[1].payload,
            json!({"id": 2, "status": "dropped", "submission": digest})
        );

        let reorder = change(ChangeKind::PhaseReorder, json!({"order": [1, 2]}));
        let digest = reorder.digest().to_hex();
        assert_eq!(
            placed(&events_of(&reorder, &a0(), None)),
            [
                (
                    "phase.reordered",
                    "roadmap",
                    &json!({"order": [1, 2], "submission": digest})
                ),
                (
                    "scope.approved",
                    "project",
                    &approved(&digest, "phase-reorder")
                ),
            ]
        );
    }

    #[test]
    fn a_declared_story_given_a_reused_id_or_wrapped_is_caught() {
        let declare = change(
            ChangeKind::StoryDeclare,
            json!({"sentence": "Owners read progress", "status": "active", "phase": 1}),
        );
        let digest = declare.digest().to_hex();
        assert_eq!(
            placed(&events_of(&declare, &a0(), None)),
            [
                (
                    "story.declared",
                    "roadmap",
                    &json!({"id": 4, "sentence": "Owners read progress", "status": "active",
                        "phase": 1, "submission": digest})
                ),
                (
                    "scope.approved",
                    "project",
                    &approved(&digest, "story-declare")
                ),
            ]
        );
        let full = scope(&with(a0(), &[("/highest_story", json!(4_294_967_295_u64))]));
        assert_eq!(
            change_events(&declare, &full, None, "Ada Owner", AT),
            Err(NumbersExhausted)
        );
    }

    #[test]
    fn a_reassigned_deferred_story_left_deferred_is_caught() {
        for (value, expected) in [
            (
                json!({"story": 3, "phase": 2}),
                json!({"id": 3, "status": "active", "phase": 2}),
            ),
            (
                json!({"story": 1, "phase": null}),
                json!({"id": 1, "status": "active", "phase": null}),
            ),
            (
                json!({"story": 3, "phase": null}),
                json!({"id": 3, "status": "deferred", "phase": null}),
            ),
        ] {
            let reassign = change(ChangeKind::StoryReassign, value.clone());
            let mut expected = expected;
            expected["submission"] = json!(reassign.digest().to_hex());
            let events = events_of(&reassign, &a0(), None);
            assert_eq!(events[0].type_name, "story.reassigned", "{value}");
            assert_eq!(events[0].payload, expected, "{value}");
        }
    }

    #[test]
    fn a_story_change_event_missing_its_status_order_or_description_is_caught() {
        const D1: &str = "db6830acb9a086805f20c56b6dbec837c2fa59938cfabcec77d3ed5336ec1213";
        for (change, kind, first) in [
            (
                change(
                    ChangeKind::StoryEdit,
                    json!({"story": 3, "sentence": "Teammates read the backlog"}),
                ),
                "story-edit",
                (
                    "story.corrected",
                    "roadmap",
                    json!({"id": 3, "sentence": "Teammates read the backlog",
                        "status": "deferred"}),
                ),
            ),
            (
                change(ChangeKind::StoryDrop, json!({"story": 1})),
                "story-drop",
                (
                    "story.dropped",
                    "roadmap",
                    json!({"id": 1, "status": "dropped"}),
                ),
            ),
            (
                change(ChangeKind::BacklogReorder, json!({"order": [2, 1, 3]})),
                "backlog-reorder",
                (
                    "story.reprioritized",
                    "roadmap",
                    json!({"order": [2, 1, 3]}),
                ),
            ),
            (
                d1(),
                "project-describe",
                (
                    "project.described",
                    "project",
                    json!({"what": "A ledger for agent work",
                        "core_value": "Nothing moves on an unproven claim",
                        "constraints": ["One binary", "No server"]}),
                ),
            ),
        ] {
            let digest = change.digest().to_hex();
            let (type_name, stream, mut payload) = first;
            payload["submission"] = json!(digest);
            assert_eq!(
                placed(&events_of(&change, &a0(), None)),
                [
                    (type_name, stream, &payload),
                    ("scope.approved", "project", &approved(&digest, kind)),
                ],
                "{kind}"
            );
        }
        assert_eq!(d1().digest().to_hex(), D1);
    }
}
