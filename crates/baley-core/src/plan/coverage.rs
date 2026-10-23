//! The evidence map judged over the phase as it would stand with the plan
//! approved (PLN-R12, PLN-R13, D-06): this plan's items beside the items of
//! the phase's approved plans.

use serde_json::Value;

use super::evidence::EvidenceItem;
use super::judge::{ApprovedItem, PlanRefusal};

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

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, the phase's
    //! decisions and the plan's test data, never from running this code.

    use serde_json::json;

    use crate::plan::PlanRefusal;
    use crate::plan::fixtures::{approved, c1, p1, refusal, under, with};

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
}
