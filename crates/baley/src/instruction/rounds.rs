//! The rounds relay's served text, pinned in source: how a host session
//! reads a question set's round, puts it to the owner and relays the
//! owner's answers and deferrals (design 0005 section 10, PLN-R24 to
//! PLN-R26, HST-R9). It is no help row, so it gets no skill stub: the round
//! read and the `questions-answer` receipt name it.
//!
//! This version starts from a set already open. The Rounds row's first
//! step, submitting the analyzer's questions as returned, arrives with T7's
//! `story questions submit` as version 2 of this text.

/// The registry identity of the rounds relay's text.
pub const IDENTITY: &str = "bal-rounds";

/// Pinned by hand: raise it by one whenever [`TEXT`] changes.
pub const VERSION: &str = "1";

/// Lowercase hex SHA-256 of [`TEXT`]'s exact bytes, computed outside the
/// code so a changed text without a new pin fails the registry's pin test.
pub const HASH: &str = "8bab77d68c4253a45e63cf5f16397e58e058c21f94781674c0cf08c09bfa3e6d";

/// What a session reads before it puts a set's rounds to the owner.
pub const TEXT: &str = r#"Baley's rounds relay puts a question set to the owner one round at a time and records each of the owner's decisions exactly as the owner gave it.

Read the set's round with one `baley_query` call: `{"operation":"document","identity":{"kind":"questions","set":"<the set id>"}}`. The answer gives the set's state, `open`, `closed` or `abandoned`, the round number, every question of the current round with its `recommended` answer, every waiting question with what it waits on, marked open or `deferred`, and the questions already answered or deferred.

Put every question of the current round to the owner with its recommended answer. Show each waiting question and what it waits on.

Relay the owner's decisions with one `baley_apply` call: `{"operation":"questions-answer","request_id":"<a fresh lowercase UUID>","set":"<the set id>","items":[...]}`, with one item for each question the owner settles:

- `{"question":"<id>","act":"take"}` when the owner takes the recommended answer.
- `{"question":"<id>","act":"answer","answer":"<the owner's words>"}` when the owner gives an answer of their own.
- `{"question":"<id>","act":"defer","reason":"<the owner's reason>"}` when the owner leaves the question undecided.

The owner may settle part of a round, and may defer a waiting question. Relay each act unchanged: never answer, drop, merge, reword or reorder a question or an answer yourself, and decide nothing for the owner.

After each call, read the round again and put the next round to the owner, until the set is `closed`. A question waiting on a deferred one never enters a round: it closes only when the owner defers it too.

On a `refused` answer, put what the refusal names back to the owner and change nothing yourself. On a `failed` answer marked `retryable`, send the same call again unchanged. An `abandoned` set takes no answer.

The owner can also read and answer a set at the terminal, with `baley questions round <set>` and `baley questions answer <set>`.
"#;
