//! Steps (5) and (6) of the plan judge: the phase's committed stories
//! (PLN-R13). Neither reads any other fact.

use super::judge::{PlanFacts, PlanRefusal};

/// Step (5): a phase none of whose committed stories holds a current truth,
/// including a phase with no committed story, is refused `no-truths`. A
/// story whose approved truth list is empty holds none.
pub(super) fn holds_truths(facts: &PlanFacts) -> Result<(), PlanRefusal> {
    if facts.stories.iter().all(|story| story.truths.is_empty()) {
        return Err(PlanRefusal::NoTruths);
    }
    Ok(())
}

/// Step (6): the first story the plan serves, in its order, that is not
/// committed to the phase is refused `story-not-in-phase`, naming it.
pub(super) fn committed(stories: &[u32], facts: &PlanFacts) -> Result<(), PlanRefusal> {
    match stories
        .iter()
        .find(|id| !facts.stories.iter().any(|story| story.id == **id))
    {
        Some(&story) => Err(PlanRefusal::StoryNotInPhase { story }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, the phase's
    //! decisions and the plan's test data, never from running this code.

    use serde_json::json;

    use crate::plan::PlanRefusal;
    use crate::plan::fixtures::{accepted, f0, f1, f2, p1, p3, refusal, with};

    #[test]
    fn a_plan_for_a_phase_without_truths_accepted_or_one_with_truths_refused_is_caught() {
        assert_eq!(refusal(&p1(), &f0()), PlanRefusal::NoTruths);
        let mut uncommitted = f1();
        uncommitted.stories.clear();
        assert_eq!(refusal(&p1(), &uncommitted), PlanRefusal::NoTruths);
        accepted(&p3(), &f2());
    }

    #[test]
    fn a_story_not_committed_named_wrongly_is_caught() {
        let facts = f1();
        for stories in [json!([4, 5]), json!([2, 4])] {
            assert_eq!(
                refusal(&with(p1(), "", "stories", stories), &facts),
                PlanRefusal::StoryNotInPhase { story: 4 }
            );
        }
    }
}
