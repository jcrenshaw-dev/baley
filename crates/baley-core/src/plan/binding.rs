//! Step (7) of the plan judge: each evidence item bound to its own story's
//! current truth (PLN-R13, D-01). A truth id is unique only within its
//! story, so the truth is looked up inside the item's story alone.

use super::evidence::EvidenceItem;
use super::judge::{CurrentTruth, PlanFacts, PlanRefusal};

/// Step (7), item by item in submitted order: an item whose story is not
/// one the plan serves, or whose truth id is not a current truth of that
/// story, is refused `evidence-item-truth`; one naming its truth at another
/// version, older or newer, is refused `truth-version-mismatch` with the
/// current version. Answers each item's truth, in the plan's item order.
pub(super) fn bind<'a>(
    stories: &[u32],
    items: &[EvidenceItem],
    facts: &'a PlanFacts,
) -> Result<Vec<&'a CurrentTruth>, PlanRefusal> {
    items
        .iter()
        .enumerate()
        .map(|(position, item)| {
            let truth = stories
                .contains(&item.story)
                .then(|| facts.stories.iter().find(|story| story.id == item.story))
                .flatten()
                .and_then(|story| story.truths.iter().find(|truth| truth.id == item.truth));
            match truth {
                None => Err(PlanRefusal::EvidenceItemTruth {
                    position,
                    id: item.id.clone(),
                }),
                Some(truth) if truth.version != item.truth_version => {
                    Err(PlanRefusal::TruthVersionMismatch {
                        position,
                        id: item.id.clone(),
                        current: truth.version,
                    })
                }
                Some(truth) => Ok(truth),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, the phase's
    //! decisions and the plan's test data, never from running this code.

    use serde_json::json;

    use crate::plan::PlanRefusal;
    use crate::plan::fixtures::{ar, f1, p1, p2, plus, refusal, with};

    fn untrue(position: usize, id: &str) -> PlanRefusal {
        PlanRefusal::EvidenceItemTruth {
            position,
            id: id.into(),
        }
    }

    fn mismatch(position: usize, id: &str, current: u32) -> PlanRefusal {
        PlanRefusal::TruthVersionMismatch {
            position,
            id: id.into(),
            current,
        }
    }

    #[test]
    fn an_item_naming_no_served_truth_or_an_old_version_accepted_is_caught() {
        let facts = f1();
        let cases = [
            (
                with(p1(), "/evidence_map/0", "story", json!(3)),
                untrue(0, "c1"),
            ),
            (
                with(p1(), "/evidence_map/0", "truth", json!("t3")),
                untrue(0, "c1"),
            ),
            (
                plus(
                    p1(),
                    with(
                        with(ar(), "", "truth", json!("t2")),
                        "",
                        "truth_version",
                        json!(1),
                    ),
                ),
                mismatch(3, "a1", 2),
            ),
        ];
        for (plan, refused) in cases {
            assert_eq!(refusal(&plan, &facts), refused, "{plan}");
        }
        assert_eq!(untrue(0, "c1").code(), "evidence-item-truth");
        assert_eq!(mismatch(3, "a1", 2).code(), "truth-version-mismatch");
    }

    #[test]
    fn an_item_bound_by_truth_id_alone_is_caught() {
        let facts = f1();
        let cases = [
            (
                with(p2(), "/evidence_map/3", "truth_version", json!(1)),
                mismatch(3, "c4", 3),
            ),
            (
                with(p2(), "/evidence_map/3", "story", json!(1)),
                mismatch(3, "c4", 1),
            ),
        ];
        for (plan, refused) in cases {
            assert_eq!(refusal(&plan, &facts), refused, "{plan}");
        }
    }

    #[test]
    fn a_newer_version_than_the_current_accepted_is_caught() {
        assert_eq!(
            refusal(
                &with(p1(), "/evidence_map/1", "truth_version", json!(3)),
                &f1()
            ),
            mismatch(1, "c2", 2)
        );
    }
}
