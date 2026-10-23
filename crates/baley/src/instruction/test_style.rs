//! The default test style: the guidance on how to write tests that Baley
//! hands over when the project has set no test style of its own (design 0005
//! section 10, the Default test style row, PLN-R19). The planner receives it
//! inside `bal-phase-planner`'s text, and the executor receives it through
//! design 0006 when the project has set nothing. It is guidance only: Baley
//! never refuses a plan on style.
//!
//! No Cadence tag `v3.9.0` text carries a default test style: none of
//! `skills/`, `cadence-core/workflows/` or `cadence-core/templates/` holds
//! one. So under ADR 0048 it is written for Baley from design 0005's row,
//! and the planner text carries it as an ADR 0049 addition (PLN-R19). It
//! holds no digit and no count word, since the planner text carries it whole.

/// The default test style's text: design 0005 section 10's Default test
/// style row, one sentence per rule, for a project that has set none.
pub const TEXT: &str = "This is the default test style, for a project that has set no test style of its own. It is guidance: Baley never refuses a plan on style.

- Test a unit through what it exposes, never through its internals.
- Fake only the outside world, never the project's own code.
- Never start a program from a test.
- Skip trivial code: it needs no test of its own.
- Write the expected value by hand, never from the code under test.
";
