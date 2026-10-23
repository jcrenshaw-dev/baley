//! The `bal-context` front door's served text, pinned in source: how a host
//! session refines one story with its owner, from the analyzer's questions in
//! rounds to the truth draft the owner approves at the terminal (design 0005
//! sections 5 to 7 and 10, PLN-R1 to PLN-R6). It is a help row, so it
//! gets a skill stub.
//!
//! The text is ported from Cadence (ADR 0048). Cadence's own files,
//! commands and state are replaced by Baley's: the skill's front matter
//! description and the workflow's purpose become the opening summary line;
//! `resolve_phase` becomes resolving the story through the roadmap read;
//! `check_existing` and `load_priors` become the story's refinement read,
//! current truths meaning a revision, with the analyzer's priors bound into
//! its order; `analyze` becomes `story-refine` and the order's `launch`, and
//! the dispatch bracket becomes `work-order-fail` and `work-order-retry`;
//! `close_gray_areas` and `confirm_decisions` become the rounds through
//! `bal-rounds`; `acceptance_criteria` becomes the drafted truths and
//! `context-submit`, its tool-availability tag the truth's `human_verify`
//! field. Where Cadence names a command, the text names the Baley operation
//! with how its request ids, receipts, refusals, settings lines and parts
//! are read. These changes from Cadence's wording are forced by accepted
//! Baley ADRs:
//!
//! - It takes a story id, not a phase, since truths belong to stories
//!   (ADR 0017), and says owner where Cadence says user, the glossary's
//!   term (ADR 0031).
//! - The spend question is dropped: Baley dispatches the analyzer twice,
//!   once for questions and once to draft truths (ADR 0030). With it goes
//!   the plain pass that derives gray areas without the analyzer, on a
//!   skip or a failed dispatch: a failed order is retried instead.
//! - Decisions are the owner's answers and deferrals, recorded round by
//!   round, with no decision list to confirm, no durability filter and no
//!   flagged assumptions (ADR 0030). The session sees only questions, so
//!   its question budget says a question the code settles is never asked
//!   where Cadence's says `Confident` items cost none, and its last
//!   guardrail speaks of what a truth promises where Cadence's speaks of
//!   what was decided.
//! - No size question and no count of 3 to 7 criteria (ADR 0017).
//! - No CONTEXT.md, no requirement row correction, no cursor update and no
//!   commit of planning files: the ledger records every step (ADR 0006).
//! - The owner approves at the terminal, with no lock-or-edit question put
//!   by the session (ADR 0044).
//! - Cadence's recalled memory is dropped: `recall` is Build 8's,
//!   `retired("recall", 8)` in `crate::mcp::operations`.
//!
//! These rules are added under ADR 0049, which lets a ported text carry a
//! rule a design requirement states, each stated by a requirement of design
//! 0005:
//!
//! - "Check each against the code and the answers, and drop a truth that
//!   does not hold before the owner sees it": PLN-R6 has the host session
//!   drop a draft truth that does not hold against the code or the answers.
//! - The truth's sentence in its two forms (PLN-R2).
//! - The two attestations, `observable` and `fixed_oracle`, set only as the
//!   owner says, and a truth attested false refused `truth-internal` or
//!   `truth-prose-oracle` (PLN-R3).
//! - A truth put to the owner keeps its id (PLN-R4).
//! - The `analyzer-order-pending` and `question-open` refusals of
//!   `story-refine`, and the owner alone ending a set early by deferring its
//!   open questions with a reason (section 5, the `story refine` refusals).

/// The registry identity of the context front door.
pub const IDENTITY: &str = "bal-context";

/// Pinned by hand: raise it by one whenever [`TEXT`] changes.
pub const VERSION: &str = "1";

/// Lowercase hex SHA-256 of [`TEXT`]'s exact bytes, computed outside the
/// code so a changed text without a new pin fails the registry's pin test.
pub const HASH: &str = "d318a66c27dae8d904b1bf54790a526656da09ed38e5b4b2a126b5a5b2392b4c";

/// What a session is told when it follows the context command. Ported from
/// Cadence tag `v3.9.0`: `skills/cad-context/SKILL.md` and
/// `cadence-core/workflows/context.md`, step by step.
pub const TEXT: &str = r#"Baley context refines one story with its owner before it is planned: the analyzer's questions put to the owner in rounds, then the truths the analyzer drafts from the answers, which the owner approves at the terminal. The flow is codebase-first: the analyzer reads the relevant source off the main context, and questioning covers only what the analyzer could not resolve.

<step name="resolve_story">
Take the story id the owner names. If none is named, read the accepted roadmap with `baley_query` `{"operation":"document","identity":{"kind":"roadmap"}}` and ask the owner which story to refine, by its id. If the roadmap holds no such story, stop and say so: a story is declared through `bal-story` first.
</step>

<step name="read_refinement">
Read what already constrains this story - never re-ask a settled question - with `baley_query` `{"operation":"document","identity":{"kind":"refinement","story":<the story's id>}}`, or `{"operation":"context-intake","story":<the story's id>}`, which answers the same members. The answer gives the approved project description, the story with the phase it is committed to, and its current truths with every earlier version. A story with current truths is being revised: a truth the analyzer restates keeps its id. An answer too long for one part names its `next` part. Ask for it by adding `"part":<that number>`, and join the bodies in order.
</step>

<step name="analyze">
Before refining, settle any owner-only foundational fork the analyzer cannot resolve from code - where new code lives, the target platform, whether a referenced repository is even in scope. Ask the owner first, and do not bake an unverified premise into the story: a wrong premise wastes the whole pass.

Ask for the analyzer's order with one `baley_apply` call: `{"operation":"story-refine","request_id":"<a fresh lowercase UUID>","story":<the story's id>,"instruction":"bal-context"}`. The receipt names the order in `work_order`, says in `analyzer` whether it asks for `questions` or for `drafting`, and gives the worker's `prompt` and `launch`. When the answer holds `settings` lines, show them to the owner. Launch the worker exactly as `launch` says, with `prompt` as its whole prompt. Change nothing in the order and add nothing to the prompt. The order binds the story, the approved description, the roadmap and the story's current truths as they stood when it was issued, so the worker reads them from the order.

When the worker cannot launch, or returns without its result, call `baley_apply` with `{"operation":"work-order-fail","request_id":"<a fresh lowercase UUID>","work_order":"<the order's id>","reason":"<why it failed, in plain words>","instruction":"bal-context"}`, then with `{"operation":"work-order-retry","request_id":"<a fresh lowercase UUID>","work_order":"<the failed order's id>","instruction":"bal-context"}`, and launch the retried order from its receipt the same way. Never fail or retry an order only because its worker has not answered yet.

A story has one live analyzer order. An `analyzer-order-pending` refusal names the pending order in `details`: read it with `baley_query` `{"operation":"document","identity":{"kind":"work_order","id":"<that id>"}}` and launch it, or fail it and retry it as above.

A `story-refine` call while a question of the story's latest set is open is refused `question-open`, naming the set and an open question in `details`. Put that set's rounds to the owner as `bal-rounds` says, then call `story-refine` again. The owner ends a set early only by deferring its open questions with a reason; never defer one for the owner.
</step>

<step name="close_gray_areas">
Adaptive questioning - ask only what the analyzer could not resolve. The questions order's worker returns typed questions, each with its `recommended` answer. Read the rounds relay with `baley_query` `{"operation":"instruction","identity":"bal-rounds"}`, submit the returned questions exactly as it says, and put each round to the owner through it.

Universal rules while questioning:
- If an answer references a doc, spec, or ADR, read it now.
- If an answer drifts outside the story, say so and redirect - scope grows only by the owner's explicit choice, through `bal-story`.
- Never invent an answer the owner was supposed to give.
</step>

<step name="drafting">
The receipt that closes the set names the analyzer's drafting order in `drafting`, and so does the receipt of an empty set's submit: launch that order exactly as its `launch` says, with its `prompt` as the whole prompt. A set the owner closed at the terminal gives no such receipt: call `story-refine` again, and its receipt is the drafting order. If you get a closed receipt with no drafting, another analyzer order of the story was still pending, so no drafting order was issued: call `story-refine` again, which is refused `analyzer-order-pending` naming that order to read, fail or retry as above.
</step>

<step name="acceptance_criteria">
The drafting order's worker returns draft truths. Each truth is one sentence: `When <one trigger>, <one observer> sees / gets / is refused <one outcome>.`, or `For any <input>, <observer> gets <outcome>.`. Check each against the code and the answers, and drop a truth that does not hold before the owner sees it. Rules:

- Pass/fail only - a stranger could check each truth without judgment calls.
- Observed behavior, not implementation: "running X shows Y", not "X is implemented" or "X works well".
- No subjective words: "clean", "reasonable", "robust" are banned.
- A truth whose proof needs an external tool or service (docker, a cloud CLI, a browser driver, a live endpoint) names it in `human_verify`, so verification routes it to a human check from the start.

Put each truth to the owner for edits, keeping its id. Then ask the owner for its two attestations: `observable`, that its outcome is seen from outside, and `fixed_oracle`, that its expected value is fixed in advance and never comes from a model call. The attestations are set only as the owner says: never attest a truth yourself. A truth attested false is refused, `truth-internal` or `truth-prose-oracle`, so take it back to the owner.

Submit the truths with one `baley_apply` call: `{"operation":"context-submit","request_id":"<a fresh lowercase UUID>","work_order":"<the drafting order's id>","story":<the story's id>,"set":"<the question set's id>","truths":[{"id":"t1","form":"when","trigger":"<one trigger>","observer":"<one observer>","verb":"sees","outcome":"<one outcome>","kind":"literal","observable":true,"fixed_oracle":true}]}`, adding `"human_verify":"<the tool or live service>"` to a truth that names one. Send no `instruction` on this call, since its shape takes none. The receipt names the draft's `digest` and marks each truth `unchanged`, `changed` or `added` against the story's current truths, with the ids it leaves out in `removed`.
</step>

<step name="done">
Give the owner the draft's digest. The owner previews it with `baley scope preview <digest>` and approves it with `baley story truths approve <digest>` at the terminal, which gives each truth its version. Nothing is approved over MCP, so approve nothing yourself.
</step>

Use a fresh request id for each call. Send a request id again only to retry the same call, with every other argument unchanged.

On a `refused` answer, show the owner its `code`, `slot` and `reason`. On a `failed` answer marked `retryable`, send the same call again unchanged. On any other `failed` answer, show the owner its `code` and `reason`.

<guardrails>
- Never modify source code. Write no file and make no commit: the questions, the answers and the draft live only in the ledger.
- Question budget is earned by the analyzer: a question the code settles is never asked. Never interview area by area through things the codebase already answers.
- Truths record WHAT the story promises, not HOW to build it - implementation detail belongs to planning.
</guardrails>
"#;
