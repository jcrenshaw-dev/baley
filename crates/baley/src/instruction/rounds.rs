//! The rounds relay's served text, pinned in source: how a host session
//! submits the analyzer's questions as returned, reads a question set's
//! round, puts it to the owner and relays the owner's answers and deferrals
//! (design 0005 section 10, PLN-R6, PLN-R24 to PLN-R26, HST-R9). It is no
//! help row, so it gets no skill stub: `bal-context`, the round read and
//! the `questions-answer` receipt name it.

/// The registry identity of the rounds relay's text.
pub const IDENTITY: &str = "bal-rounds";

/// Pinned by hand: raise it by one whenever [`TEXT`] changes.
pub const VERSION: &str = "2";

/// Lowercase hex SHA-256 of [`TEXT`]'s exact bytes, computed outside the
/// code so a changed text without a new pin fails the registry's pin test.
pub const HASH: &str = "420c5ece69f5f29e60337a0ab905db6a2c00439f77b7422290062a0fd358bb66";

/// What a session reads before it submits a story's questions or puts a
/// set's rounds to the owner.
pub const TEXT: &str = r#"Baley's rounds relay puts a question set to the owner one round at a time and records each of the owner's decisions exactly as the owner gave it.

When the analyzer returns its questions for a story, send them first, with one `baley_apply` call: `{"operation":"story-questions-submit","request_id":"<a fresh lowercase UUID>","story":<the story's id>,"work_order":"<the questions order's id>","questions":[...]}`. Send the `questions` exactly as returned: never edit, drop, merge, reword or reorder a question, and answer none of them yourself. The receipt gives the set id in `set` and its first round in `now`. An empty list's receipt is already `closed` and names the analyzer's drafting order in `drafting`.

When a receipt shows a story's set `closed`, its `drafting` names the analyzer's drafting order: return to the instruction that sent you here, which launches it. If you get a closed receipt with no drafting, another analyzer order of the story is still pending, so no drafting order was issued: return to the instruction that sent you here, which calls `story-refine` again. That call is refused `analyzer-order-pending` naming the pending order, which you read with `document`, or fail with `work-order-fail` and retry with `work-order-retry`, as that instruction says. A refused submission is also returned to that instruction, never fixed.

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
