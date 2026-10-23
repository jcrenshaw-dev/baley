//! Step (9) of the plan judge: a link's value named by its truth (PLN-R12,
//! D-13). The rule matches the parked `names` in
//! `crates/baley/src/plan/limits.rs`, written anew here; the parked one
//! goes with the rest of that engine in Build 9.

use super::evidence::{EvidenceItem, Spec};
use super::judge::{CurrentTruth, PlanRefusal};

/// Step (9), each link in submitted order: its value, trimmed at both
/// ends, must be named by its bound truth's trigger, observer or outcome,
/// each slot on its own. The first link no slot names is refused
/// `link-value-not-named`. `truths` holds each item's bound truth, in the
/// plan's item order.
pub(super) fn named(items: &[EvidenceItem], truths: &[&CurrentTruth]) -> Result<(), PlanRefusal> {
    for (position, (item, truth)) in items.iter().zip(truths).enumerate() {
        let Spec::Link(link) = &item.spec else {
            continue;
        };
        let value = link.value.trim();
        if ![&truth.trigger, &truth.observer, &truth.outcome]
            .iter()
            .any(|slot| names(slot, value))
        {
            return Err(PlanRefusal::LinkValueNotNamed {
                position,
                id: item.id.clone(),
            });
        }
    }
    Ok(())
}

/// Whether `slot` holds `value` as a delimited token: where the value
/// begins with a word character (a letter, a digit or `_`), none precedes
/// it, and where it ends with one, none follows it. Every occurrence is
/// tried, so a word holding the value does not hide a later delimited
/// one. An empty value is named nowhere.
fn names(slot: &str, value: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let (Some(first), Some(last)) = (value.chars().next(), value.chars().next_back()) else {
        return false;
    };
    slot.char_indices().any(|(at, _)| {
        let end = at + value.len();
        slot[at..].starts_with(value)
            && !(word(first) && slot[..at].chars().next_back().is_some_and(word))
            && !(word(last) && slot[end..].chars().next().is_some_and(word))
    })
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, the phase's
    //! decisions and the plan's test data, never from running this code.

    use serde_json::json;

    use crate::plan::PlanRefusal;
    use crate::plan::fixtures::{accepted, f1, l1, p1, plus, refusal, with};

    fn linked(value: &str) -> serde_json::Value {
        plus(p1(), with(l1(), "/spec", "value", json!(value)))
    }

    fn unnamed() -> PlanRefusal {
        PlanRefusal::LinkValueNotNamed {
            position: 3,
            id: "l1".into(),
        }
    }

    #[test]
    fn a_link_value_inside_a_word_across_slots_or_blank_accepted_is_caught() {
        let facts = f1();
        for value in ["lea", "session lease", "  ", "plans"] {
            assert_eq!(refusal(&linked(value), &facts), unnamed(), "{value:?}");
        }
        for value in [
            "plan",
            " lease ",
            "a valid refusal",
            "refusal",
            "the session",
        ] {
            accepted(&linked(value), &facts);
        }
        let mut suffixed = f1();
        suffixed.stories[0].truths[0].outcome = "the id_suffix".into();
        assert_eq!(refusal(&linked("id"), &suffixed), unnamed());
        assert_eq!(unnamed().code(), "link-value-not-named");
    }

    #[test]
    fn a_later_delimited_occurrence_hidden_by_an_embedded_one_is_caught() {
        let mut facts = f1();
        facts.stories[0].truths[0].outcome = "a valid id".into();
        accepted(&linked("id"), &facts);
    }
}
