//! The evidence map judged over the phase as it would stand with the plan
//! approved (PLN-R12, PLN-R13, D-06): this plan's items beside the items of
//! the phase's approved plans.

use serde_json::Value;

use super::evidence::EvidenceItem;
use super::judge::{ApprovedItem, CurrentTruth, EvidenceKind, PlanFacts, PlanRefusal};

/// Step (8), item by item in submitted order: the first approved item, in
/// the facts' order, with the item's id and another definition refuses it
/// `evidence-item-conflict`. A definition is the whole item, compared as a
/// JSON value, so an approved item equal to it is the same item. Approved
/// items that differ only from each other were judged at their approval
/// and are not judged again. `submitted` holds the items as submitted, in
/// the order of `items`.
pub(super) fn consistent(
    submitted: &[Value],
    items: &[EvidenceItem],
    approved: &[ApprovedItem],
) -> Result<(), PlanRefusal> {
    for (position, (definition, item)) in submitted.iter().zip(items).enumerate() {
        if let Some(other) = approved
            .iter()
            .find(|other| other.id == item.id && other.definition != *definition)
        {
            return Err(PlanRefusal::EvidenceItemConflict {
                position,
                id: item.id.clone(),
                plan: other.plan,
            });
        }
    }
    Ok(())
}

/// One item of the union: its id, the truth it serves and its kind.
struct Proof<'a> {
    id: &'a str,
    story: u32,
    truth: &'a str,
    version: u32,
    kind: EvidenceKind,
}

/// The phase as it would stand with the plan approved: the truths the plan
/// serves and the union of the approved plans' items with its own.
pub(super) struct Phase<'a> {
    /// The current truths of the plan's stories, in the plan's story order
    /// and then each story's own order, each with its story.
    served: Vec<(u32, &'a CurrentTruth)>,
    /// The approved items in the facts' order, then this plan's items in
    /// submitted order.
    union: Vec<Proof<'a>>,
}

impl<'a> Phase<'a> {
    /// The phase for a plan serving `stories` with `items`, judged after
    /// step (8). An item listed again with an equal definition is the same
    /// item and stands in the union once.
    pub(super) fn new(stories: &[u32], items: &'a [EvidenceItem], facts: &'a PlanFacts) -> Self {
        let served = stories
            .iter()
            .filter_map(|id| facts.stories.iter().find(|story| story.id == *id))
            .flat_map(|story| story.truths.iter().map(|truth| (story.id, truth)))
            .collect();
        let approved = facts.approved.iter().enumerate().filter(|&(n, item)| {
            !facts.approved[..n]
                .iter()
                .any(|earlier| earlier.id == item.id && earlier.definition == item.definition)
        });
        // Step (8) has refused any item whose id an approved item holds with
        // another definition, so an approved id here is the same item.
        let submitted = items
            .iter()
            .filter(|item| !facts.approved.iter().any(|other| other.id == item.id));
        let approved = approved.map(|(_, item)| Proof {
            id: &item.id,
            story: item.story,
            truth: &item.truth,
            version: item.truth_version,
            kind: item.kind,
        });
        let submitted = submitted.map(|item| Proof {
            id: &item.id,
            story: item.story,
            truth: &item.truth,
            version: item.truth_version,
            kind: item.kind(),
        });
        Self {
            served,
            union: approved.chain(submitted).collect(),
        }
    }

    /// The union's items that name `truth` of `story` at its current
    /// version, in union order. An item naming another version proves
    /// nothing here.
    fn naming(&self, story: u32, truth: &CurrentTruth) -> Vec<&Proof<'a>> {
        self.union
            .iter()
            .filter(|proof| {
                proof.story == story && proof.truth == truth.id && proof.version == truth.version
            })
            .collect()
    }

    /// Whether an item naming `truth` of `story` at its current version is
    /// of `kind`.
    fn holds(&self, story: u32, truth: &CurrentTruth, kind: EvidenceKind) -> bool {
        self.naming(story, truth)
            .iter()
            .any(|proof| proof.kind == kind)
    }

    /// Step (10): the first served truth no item names is refused
    /// `uncovered-truth`.
    pub(super) fn covered(&self) -> Result<(), PlanRefusal> {
        for &(story, truth) in &self.served {
            if self.naming(story, truth).is_empty() {
                return Err(PlanRefusal::UncoveredTruth {
                    story,
                    truth: truth.id.clone(),
                    version: truth.version,
                });
            }
        }
        Ok(())
    }

    /// Step (11): the first served truth whose items hold no check is
    /// refused `truth-without-check`.
    pub(super) fn checked(&self) -> Result<(), PlanRefusal> {
        for &(story, truth) in &self.served {
            if !self.holds(story, truth, EvidenceKind::Check) {
                return Err(PlanRefusal::TruthWithoutCheck {
                    story,
                    truth: truth.id.clone(),
                    version: truth.version,
                });
            }
        }
        Ok(())
    }

    /// Step (12): the first served truth a person must prove
    /// (`human_verify`) whose items hold no observation is refused
    /// `truth-without-observation` (D-04). Step (11) has already asked it
    /// for its one check.
    pub(super) fn observed(&self) -> Result<(), PlanRefusal> {
        for &(story, truth) in &self.served {
            if truth.human_verify.is_some() && !self.holds(story, truth, EvidenceKind::Observation)
            {
                return Err(PlanRefusal::TruthWithoutObservation {
                    story,
                    truth: truth.id.clone(),
                    version: truth.version,
                });
            }
        }
        Ok(())
    }

    /// Step (13): the first served truth whose items hold more than one
    /// distinct check id is refused `truth-check-limit`, naming every one
    /// in union order. An id counts once however many plans list it.
    pub(super) fn limited(&self) -> Result<(), PlanRefusal> {
        for &(story, truth) in &self.served {
            let mut checks: Vec<String> = Vec::new();
            for proof in self.naming(story, truth) {
                if proof.kind == EvidenceKind::Check && !checks.iter().any(|id| id == proof.id) {
                    checks.push(proof.id.to_owned());
                }
            }
            if checks.len() > 1 {
                return Err(PlanRefusal::TruthCheckLimit {
                    story,
                    truth: truth.id.clone(),
                    version: truth.version,
                    checks,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, the phase's
    //! decisions and the plan's test data, never from running this code.

    use serde_json::json;

    use crate::plan::PlanRefusal;
    use crate::plan::fixtures::{
        accepted, approved, ar, c1, c2, c3, c5, c6, dropping, f1, f3, o1, p1, p2, plus, refusal,
        replacing, under, with,
    };

    fn conflict(plan: u32) -> PlanRefusal {
        PlanRefusal::EvidenceItemConflict {
            position: 0,
            id: "c1".into(),
            plan,
        }
    }

    #[test]
    fn an_item_id_reused_with_another_definition_accepted_is_caught() {
        let cases = [
            (
                vec![approved(
                    1,
                    with(c1(), "/spec", "level", json!("integration")),
                )],
                conflict(1),
            ),
            (
                vec![
                    approved(2, c1()),
                    approved(3, with(c1(), "", "reason", json!("another reason"))),
                ],
                conflict(3),
            ),
            (
                vec![approved(
                    1,
                    with(
                        with(c1(), "", "story", json!(2)),
                        "",
                        "truth_version",
                        json!(3),
                    ),
                )],
                conflict(1),
            ),
        ];
        for (items, refused) in cases {
            assert_eq!(refusal(&p1(), &under(items)), refused);
        }
        assert_eq!(conflict(1).code(), "evidence-item-conflict");
    }

    fn uncovered(story: u32, truth: &str, version: u32) -> PlanRefusal {
        PlanRefusal::UncoveredTruth {
            story,
            truth: truth.into(),
            version,
        }
    }

    #[test]
    fn a_truth_covered_by_another_storys_item_or_an_old_version_is_caught() {
        assert_eq!(
            refusal(&dropping(p2(), &["c4"]), &f1()),
            uncovered(2, "t1", 3)
        );
        let mut moved = under(vec![approved(1, c1())]);
        moved.stories[0].truths[0].version = 2;
        assert_eq!(
            refusal(&dropping(p1(), &["c1"]), &moved),
            uncovered(1, "t1", 2)
        );
        assert_eq!(
            refusal(&dropping(p2(), &["c4"]), &f3()),
            uncovered(2, "t1", 1)
        );
        assert_eq!(uncovered(2, "t1", 3).code(), "uncovered-truth");
    }

    #[test]
    fn a_check_in_an_approved_plan_ignored_is_caught() {
        accepted(&dropping(p1(), &["c2"]), &under(vec![approved(1, c2())]));
    }

    #[test]
    fn an_artifact_taken_for_a_check_is_caught() {
        let refused = refusal(&replacing(p1(), "c1", ar()), &f1());
        assert_eq!(
            refused,
            PlanRefusal::TruthWithoutCheck {
                story: 1,
                truth: "t1".into(),
                version: 1,
            }
        );
        assert_eq!(refused.code(), "truth-without-check");
    }

    #[test]
    fn an_observation_for_another_truth_or_in_an_approved_plan_judged_wrongly_is_caught() {
        let unobserved = PlanRefusal::TruthWithoutObservation {
            story: 1,
            truth: "t2".into(),
            version: 2,
        };
        let elsewhere = with(
            with(p1(), "/evidence_map/2", "truth", json!("t1")),
            "/evidence_map/2",
            "truth_version",
            json!(1),
        );
        assert_eq!(refusal(&elsewhere, &f1()), unobserved);
        let unlisted = dropping(p1(), &["o1"]);
        accepted(&unlisted, &under(vec![approved(1, o1())]));
        let stale = with(o1(), "", "truth_version", json!(1));
        assert_eq!(
            refusal(&unlisted, &under(vec![approved(1, stale)])),
            unobserved
        );
        assert_eq!(unobserved.code(), "truth-without-observation");
    }

    fn limit(checks: &[&str]) -> PlanRefusal {
        PlanRefusal::TruthCheckLimit {
            story: 1,
            truth: "t1".into(),
            version: 1,
            checks: checks.iter().map(|&id| id.to_owned()).collect(),
        }
    }

    #[test]
    fn a_second_check_in_an_approved_plan_missed_or_a_listed_again_check_counted_twice_is_caught() {
        assert_eq!(
            refusal(&p1(), &under(vec![approved(1, c3())])),
            limit(&["c3", "c1"])
        );
        accepted(&p1(), &under(vec![approved(1, c1()), approved(2, c1())]));
        assert_eq!(
            refusal(&plus(p1(), c6()), &under(vec![approved(1, c3())])),
            limit(&["c3", "c1", "c6"])
        );
        assert_eq!(limit(&["c3", "c1"]).code(), "truth-check-limit");
    }

    #[test]
    fn a_check_of_an_older_version_counted_against_the_current_is_caught() {
        let mut moved = under(vec![approved(1, c3())]);
        moved.stories[0].truths[0].version = 2;
        accepted(
            &with(p1(), "/evidence_map/0", "truth_version", json!(2)),
            &moved,
        );
        accepted(&replacing(p2(), "c4", c5()), &f3());
    }
}
