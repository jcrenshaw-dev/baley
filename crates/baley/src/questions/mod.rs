//! Question sets over the storage port (design 0005 sections 5 and 6, ADR
//! 0030): a set opened and abandoned inside its caller's transaction, its
//! round read from the ledger, and an answer call checked and recorded all
//! or none. The pure half, the set judge, the records, the view, the round
//! and the answer judge, is `baley_core::questions`.

/// Opens and abandons a set inside the caller's open transaction.
pub mod record;

/// Reads one set's round from its `questions` view document.
pub mod read;

/// Checks and records one answer call, for both surfaces.
pub mod answer;
