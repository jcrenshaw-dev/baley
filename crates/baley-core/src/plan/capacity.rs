//! Step (14) of the plan judge, last: the phase's capacity (PLN-R9, D-02).

use super::judge::{PlanFacts, PlanRefusal};

/// Step (14): with a capacity set, a plan whose `tasks` would take the
/// phase's current size above it is refused `over-capacity`, naming every
/// story it serves and the size the phase would reach. Each task counts
/// once whatever number of stories the plan serves; landing exactly on the
/// capacity is accepted, and with no capacity there is no ceiling.
pub(super) fn within(stories: &[u32], tasks: usize, facts: &PlanFacts) -> Result<(), PlanRefusal> {
    let Some(capacity) = facts.capacity else {
        return Ok(());
    };
    // Summed wider than the counter, so a size past u32::MAX is over any
    // capacity instead of wrapping under it.
    let size = u64::from(facts.size) + tasks as u64;
    if size > u64::from(capacity.get()) {
        return Err(PlanRefusal::OverCapacity {
            stories: stories.to_vec(),
            size,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, the phase's
    //! decisions and the plan's test data, never from running this code.

    use crate::plan::PlanRefusal;
    use crate::plan::fixtures::{accepted, at, f1, p1, p2, p4, refusal};

    fn over(stories: &[u32], size: u64) -> PlanRefusal {
        PlanRefusal::OverCapacity {
            stories: stories.to_vec(),
            size,
        }
    }

    #[test]
    fn a_plan_over_capacity_accepted_or_counted_per_story_is_caught() {
        accepted(&p1(), &at(3, 2));
        assert_eq!(refusal(&p1(), &at(3, 3)), over(&[1], 4));
        accepted(&p2(), &at(2, 1));
        assert_eq!(refusal(&p2(), &at(2, 2)), over(&[1, 2], 3));
        let mut unbounded = f1();
        unbounded.size = 4_000_000_000;
        accepted(&p1(), &unbounded);
        accepted(&p4(), &at(4, 2));
        assert_eq!(refusal(&p4(), &at(4, 3)), over(&[1], 5));
        assert_eq!(over(&[1], 4).code(), "over-capacity");
    }

    #[test]
    fn a_size_that_wraps_past_the_counter_is_caught() {
        assert_eq!(
            refusal(&p1(), &at(u32::MAX, u32::MAX)),
            over(&[1], 4_294_967_296)
        );
        assert_eq!(
            refusal(&p4(), &at(u32::MAX, u32::MAX - 1)),
            over(&[1], 4_294_967_296)
        );
    }
}
