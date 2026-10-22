//! Claude Code's effort table: for each exact concrete Claude model id, the
//! effort levels Claude Code offers for it, or that it has no effort setting
//! (design 0003, CFG-R15).
//!
//! The table predicts what Claude Code will run. It is never an observation:
//! a managed cap can still lower a level, and only a run shows what was used.
//! The levels come from Claude Code's model configuration page
//! (`https://code.claude.com/docs/en/model-config`, "Adjust effort level"),
//! and the ids and which models are still available from the Claude models
//! overview (`https://platform.claude.com/docs/en/about-claude/models/overview`).
//! The owner reviews the rows in the pull request that changes them.
//!
//! Rows are keyed by exact id only. There is no prefix, family or alias row:
//! a family rule would give a level to a model nobody has checked.

use crate::policy::Rung;

/// The day the rows were read from their sources.
pub const EFFORT_TABLE_DATE: &str = "2026-10-18";

/// The page the levels come from.
pub const EFFORT_TABLE_SOURCE: &str =
    "https://code.claude.com/docs/en/model-config#adjust-effort-level";

/// One model's row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffortRow {
    /// The exact concrete model id.
    pub model: &'static str,
    /// The levels Claude Code offers for it, lowest first, or `None` when the
    /// model has no effort setting.
    pub levels: Option<&'static [Rung]>,
}

const ALL_FIVE: &[Rung] = &Rung::ALL;
const NO_XHIGH: &[Rung] = &[Rung::Low, Rung::Medium, Rung::High, Rung::Max];

const fn row(model: &'static str, levels: Option<&'static [Rung]>) -> EffortRow {
    EffortRow { model, levels }
}

/// The rows. From the 4.6 generation on every id is a dateless pinned
/// snapshot; before it the dateless name is an API alias, so only the dated
/// id is a row.
pub const EFFORT_ROWS: &[EffortRow] = &[
    row("claude-fable-5-1", Some(ALL_FIVE)),
    row("claude-fable-5", Some(ALL_FIVE)),
    row("claude-opus-5-5", Some(ALL_FIVE)),
    row("claude-sonnet-5-5", Some(ALL_FIVE)),
    row("claude-haiku-5-5", Some(ALL_FIVE)),
    row("claude-opus-5", Some(ALL_FIVE)),
    row("claude-sonnet-5", Some(ALL_FIVE)),
    row("claude-opus-4-8", Some(ALL_FIVE)),
    row("claude-opus-4-7", Some(ALL_FIVE)),
    row("claude-opus-4-6", Some(NO_XHIGH)),
    row("claude-sonnet-4-6", Some(NO_XHIGH)),
    row("claude-opus-4-5-20251101", None),
    row("claude-haiku-4-5-20251001", None),
];

/// What the table says Claude Code supports for one model at one rung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedEffort {
    /// The highest level the row lists at or below the request.
    Level(Rung),
    /// The model is listed and has no effort setting.
    NotApplicable,
    /// No row names the model exactly.
    Unknown,
}
impl SupportedEffort {
    /// The text records use: the rung's name, `not-applicable` or `unknown`.
    pub fn as_str(self) -> &'static str {
        match self {
            SupportedEffort::Level(rung) => rung.name(),
            SupportedEffort::NotApplicable => "not-applicable",
            SupportedEffort::Unknown => "unknown",
        }
    }
}

/// The table's answer for `model` at `requested`. Only an exact row match
/// counts, so an alias, a provider-prefixed id or a dated snapshot of a
/// dateless id is unknown.
pub fn supported_effort(model: &str, requested: Rung) -> SupportedEffort {
    let Some(row) = EFFORT_ROWS.iter().find(|row| row.model == model) else {
        return SupportedEffort::Unknown;
    };
    let Some(levels) = row.levels else {
        return SupportedEffort::NotApplicable;
    };
    // Every row with levels lists `low`, so this finds one.
    levels
        .iter()
        .copied()
        .filter(|level| *level <= requested)
        .max()
        .map_or(SupportedEffort::Unknown, SupportedEffort::Level)
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from the rows read on 2026-10-18,
    //! never from running this code.

    use super::*;

    #[test]
    fn a_capability_given_to_an_alias_family_or_provider_id_is_caught() {
        for model in [
            "opus",
            "fable",
            "claude-opus",
            "claude-opus-5-5-20261001",
            "claude-sonnet-4-5",
            "claude-haiku-4-5",
            "anthropic.claude-opus-5-5",
        ] {
            let answer = supported_effort(model, Rung::High);
            assert_eq!(answer, SupportedEffort::Unknown, "{model}");
            assert_eq!(answer.as_str(), "unknown", "{model}");
        }
    }

    #[test]
    fn a_level_above_the_request_or_below_the_highest_supported_is_caught() {
        let cases = [
            ("claude-opus-4-6", Rung::Xhigh, Rung::High),
            ("claude-opus-4-6", Rung::Max, Rung::Max),
            ("claude-opus-4-6", Rung::Medium, Rung::Medium),
            ("claude-sonnet-4-6", Rung::Xhigh, Rung::High),
            ("claude-opus-5-5", Rung::Xhigh, Rung::Xhigh),
            ("claude-opus-5-5", Rung::Low, Rung::Low),
            ("claude-fable-5-1", Rung::Max, Rung::Max),
        ];
        for (model, requested, expected) in cases {
            assert_eq!(
                supported_effort(model, requested),
                SupportedEffort::Level(expected),
                "{model} at {}",
                requested.name()
            );
        }
    }

    #[test]
    fn a_listed_model_without_effort_answered_unknown_or_given_a_level_is_caught() {
        for model in ["claude-opus-4-5-20251101", "claude-haiku-4-5-20251001"] {
            for requested in [Rung::Low, Rung::Max] {
                let answer = supported_effort(model, requested);
                assert_eq!(answer, SupportedEffort::NotApplicable, "{model}");
                assert_eq!(answer.as_str(), "not-applicable", "{model}");
            }
        }
    }

    #[test]
    fn a_table_without_its_date_and_source_or_off_its_listed_rows_is_caught() {
        use Rung::{High, Low, Max, Medium, Xhigh};
        assert_eq!(EFFORT_TABLE_DATE, "2026-10-18");
        assert_eq!(
            EFFORT_TABLE_SOURCE,
            "https://code.claude.com/docs/en/model-config#adjust-effort-level"
        );
        let five: &[Rung] = &[Low, Medium, High, Xhigh, Max];
        let four: &[Rung] = &[Low, Medium, High, Max];
        let expected: [(&str, Option<&[Rung]>); 13] = [
            ("claude-fable-5-1", Some(five)),
            ("claude-fable-5", Some(five)),
            ("claude-opus-5-5", Some(five)),
            ("claude-sonnet-5-5", Some(five)),
            ("claude-haiku-5-5", Some(five)),
            ("claude-opus-5", Some(five)),
            ("claude-sonnet-5", Some(five)),
            ("claude-opus-4-8", Some(five)),
            ("claude-opus-4-7", Some(five)),
            ("claude-opus-4-6", Some(four)),
            ("claude-sonnet-4-6", Some(four)),
            ("claude-opus-4-5-20251101", None),
            ("claude-haiku-4-5-20251001", None),
        ];
        assert_eq!(EFFORT_ROWS.len(), expected.len());
        for (model, levels) in expected {
            let rows: Vec<&EffortRow> = EFFORT_ROWS.iter().filter(|r| r.model == model).collect();
            assert_eq!(rows.len(), 1, "{model} rows");
            assert_eq!(rows[0].levels, levels, "{model} levels");
        }
    }
}
