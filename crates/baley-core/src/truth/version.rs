//! The version each truth of a revision gets when it is approved (design
//! 0005 PLN-R4, D-02, D-03).
//!
//! A truth keeps its version while its text is unchanged and gets the next
//! one when its text changes. Versions are given at approval, never at
//! submit, and a submit receipt and a preview mark truths with the same
//! function. Nothing here records anything.

use std::collections::BTreeMap;

use super::submission::Truth;

/// How a submitted truth stands against the story's current truths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Its id is current and its text is byte-equal to the current one's.
    Unchanged,
    /// Its id is current and its text differs.
    Changed,
    /// Its id is not current.
    Added,
}

/// One submitted truth's mark and the version approval gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Versioned {
    /// The truth's id.
    pub id: String,
    /// How it stands against the current truths.
    pub mark: Mark,
    /// Its version once approved.
    pub version: u32,
}

/// A revision's versions: each submitted truth in submitted order, and the
/// ids of the current truths it leaves out, in current order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Versions {
    /// The submitted truths' marks and versions.
    pub truths: Vec<Versioned>,
    /// The current truths the revision removes.
    pub removed: Vec<String>,
}

/// A truth whose next version would pass `u32::MAX`. No versions are given,
/// so no number is ever reused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionsExhausted {
    /// The truth's id.
    pub id: String,
}

/// Gives each of `submitted` its version against the story's `current`
/// truths, each with its version, and `kept`, every version each id ever
/// had.
pub fn versions(
    submitted: &[Truth],
    current: &[(Truth, u32)],
    kept: &BTreeMap<String, Vec<u32>>,
) -> Result<Versions, VersionsExhausted> {
    let truths = submitted
        .iter()
        .map(|truth| {
            let held = current.iter().find(|(held, _)| held.id == truth.id);
            let (mark, version) = match held {
                Some((held, version)) if same_text(held, truth) => (Mark::Unchanged, *version),
                Some((_, version)) => (Mark::Changed, next(&truth.id, Some(*version), kept)?),
                None => (Mark::Added, next(&truth.id, None, kept)?),
            };
            Ok(Versioned {
                id: truth.id.clone(),
                mark,
                version,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let removed = current
        .iter()
        .filter(|(held, _)| !submitted.iter().any(|truth| truth.id == held.id))
        .map(|(held, _)| held.id.clone())
        .collect();
    Ok(Versions { truths, removed })
}

/// Whether two truths' six text members are byte-equal. An attestation or
/// `human_verify` alone is not text (D-02).
fn same_text(a: &Truth, b: &Truth) -> bool {
    (
        &a.form,
        &a.trigger,
        &a.observer,
        &a.verb,
        &a.outcome,
        &a.kind,
    ) == (
        &b.form,
        &b.trigger,
        &b.observer,
        &b.verb,
        &b.outcome,
        &b.kind,
    )
}

/// One more than the highest version `id` ever had, so a changed or re-added
/// truth never reuses a number evidence may be bound to (D-03), or 1 for an
/// id never kept.
fn next(
    id: &str,
    current: Option<u32>,
    kept: &BTreeMap<String, Vec<u32>>,
) -> Result<u32, VersionsExhausted> {
    let highest = kept
        .get(id)
        .into_iter()
        .flatten()
        .copied()
        .chain(current)
        .max()
        .unwrap_or(0);
    highest
        .checked_add(1)
        .ok_or_else(|| VersionsExhausted { id: id.to_owned() })
}

#[cfg(test)]
mod tests {
    //! Expected versions are written out from PLN-R4, D-02 and D-03 and the
    //! plan's test data, never from running this code.

    use super::*;

    /// T1's sentence parts under `id`.
    fn t1(id: &str) -> Truth {
        Truth {
            id: id.into(),
            form: "when".into(),
            trigger: "the owner approves a draft by a stale digest".into(),
            observer: "the owner".into(),
            verb: "is refused".into(),
            outcome: "stale-draft naming the first differing part".into(),
            kind: "literal".into(),
            human_verify: None,
        }
    }

    /// T2's sentence parts under `id`.
    fn t2(id: &str) -> Truth {
        Truth {
            id: id.into(),
            form: "for-any".into(),
            trigger: "order the answers of a set are recorded in".into(),
            observer: "a ledger reader".into(),
            verb: "gets".into(),
            outcome: "the same round".into(),
            kind: "property".into(),
            human_verify: None,
        }
    }

    fn kept(entries: &[(&str, &[u32])]) -> BTreeMap<String, Vec<u32>> {
        entries
            .iter()
            .map(|(id, versions)| ((*id).to_owned(), versions.to_vec()))
            .collect()
    }

    fn versioned(id: &str, mark: Mark, version: u32) -> Versioned {
        Versioned {
            id: id.into(),
            mark,
            version,
        }
    }

    #[test]
    fn a_version_bumped_for_unchanged_text_or_kept_for_changed_text_is_caught() {
        let current = [(t1("t1"), 1), (t2("t2"), 1), (t1("t3"), 2), (t2("t6"), 1)];
        let kept = kept(&[
            ("t1", &[1]),
            ("t2", &[1]),
            ("t3", &[1, 2]),
            ("t4", &[1, 2, 3]),
            ("t6", &[1]),
        ]);
        let mut changed = t2("t2");
        changed.outcome = "the same round and the same waiting list".into();
        let mut verified = t1("t3");
        verified.human_verify = Some("a live endpoint".into());
        let submitted = [t1("t1"), changed, verified, t1("t4"), t2("t5")];
        assert_eq!(
            versions(&submitted, &current, &kept),
            Ok(Versions {
                truths: vec![
                    versioned("t1", Mark::Unchanged, 1),
                    versioned("t2", Mark::Changed, 2),
                    versioned("t3", Mark::Unchanged, 2),
                    versioned("t4", Mark::Added, 4),
                    versioned("t5", Mark::Added, 1),
                ],
                removed: vec!["t6".into()],
            })
        );
    }

    #[test]
    fn a_change_numbered_from_the_current_version_not_the_highest_is_caught() {
        let current = [(t1("t3"), 2)];
        let kept = kept(&[("t3", &[1, 2, 3])]);
        let mut changed = t1("t3");
        changed.trigger = "the owner approves a draft by an older digest".into();
        assert_eq!(
            versions(&[changed], &current, &kept),
            Ok(Versions {
                truths: vec![versioned("t3", Mark::Changed, 4)],
                removed: Vec::new(),
            })
        );
    }

    #[test]
    fn a_version_wrapped_past_the_counter_is_caught() {
        let current = [(t1("t1"), u32::MAX)];
        let kept = kept(&[("t1", &[u32::MAX])]);
        let mut changed = t1("t1");
        changed.outcome = "unknown-draft".into();
        assert_eq!(
            versions(&[changed], &current, &kept),
            Err(VersionsExhausted { id: "t1".into() })
        );
    }
}
