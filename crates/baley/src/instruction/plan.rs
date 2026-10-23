//! The `bal-plan` front door's text, compiled but not served: how a host
//! session turns one phase into its typed plan with its owner, from the
//! planner's order to the plan the owner approves at the terminal (design
//! 0005 sections 5, 8 and 10, PLN-R7, PLN-R9, PLN-R10, PLN-R15, PLN-R24,
//! PLN-R26, PLN-R27). The registry entry, help row and stub of `bal-plan`
//! stay unavailable at build 4 until Build 4 T9 serves this text with
//! `phase plan`, `plan submit` and `plan read`, and pins its version and
//! hash then, so this module holds neither.
//!
//! The text is ported from Cadence (ADR 0048). Cadence's own files,
//! commands and state are replaced by Baley's: the skill's objective
//! becomes the opening summary line; `parse` and `load_phase` become
//! resolving the phase, the owner naming it or the session reading the
//! roadmap with `document` and asking; `spawn_planner` becomes
//! `phase-plan` and the order's `launch`, and the dispatch bracket becomes
//! `work-order-fail` and `work-order-retry`; `handle_return` becomes
//! `plan-submit`, with Cadence's "PHASE TOO BIG" return and its `too_big`
//! step becoming a proposed split put to the owner; `done` becomes the
//! owner's preview and approval at the terminal. Where Cadence names a
//! command, the text names the Baley operation with how its request ids,
//! receipts, refusals, settings lines and parts are read, as `bal-context`
//! does. The operation shapes follow design 0005 section 5, and T9 aligns
//! them with what it builds when it serves the text.
//!
//! The text carries only the steps T8 and T9 own (D-17). These steps of
//! Cadence's are dropped, each with the ADR or the build that owns it:
//!
//! - The plan-checker gate and its `--skip-check` flag: Build 4 T10.
//! - The `plan` review fire: Build 4 T13 and T14.
//! - `--gaps` and its gap plans: Build 5.
//! - `--inline` and the `route` step: every role runs from a work order
//!   (ADR 0046).
//! - `plan-size`, `check_size`, the task ceiling and the byte ceiling: no
//!   number is handed to the planner (ADR 0017, PLN-R17); `over-capacity`
//!   at submit takes their place.
//! - `check_census`: Cadence's census of its own counts; the lease check
//!   at close is Build 5's (ADR 0018).
//! - `count_planned`, `count_committed` and the recalled memory: Build 8,
//!   `retired("recall", 8)` in `crate::mcp::operations`.
//! - `commit`, `seed-reqs`, the cursor and every planning-file commit: the
//!   ledger records each step (ADR 0006).
//! - The dispatch bracket's trace lines, which are Cadence's own
//!   `planning.mjs` calls.
//! - The test switches and the `<test_method>` block they gate: Baley
//!   always requires the evidence map (PLN-R12, ADR 0014), which the
//!   planner's own text carries.
//! - The ask to replan from scratch or abort when plans exist: no plan is
//!   overwritten, and a further plan is numbered at approval (ADR 0006,
//!   D-20).
//! - The consult offer of `too_big`, which ADR 0048's table ports to no
//!   Baley part.
//! - The closing suggestion of Cadence's execute command: Build 5's
//!   `bal-execute`.
//!
//! These changes from Cadence's wording are forced by accepted Baley ADRs:
//!
//! - The owner commits stories to the phase in order when it is planned,
//!   since a phase delivers stories (ADR 0017), and says owner where
//!   Cadence says user (ADR 0031).
//! - The owner approves at the terminal, with no lock-or-edit question put
//!   by the session (ADR 0044).
//!
//! These rules are added under ADR 0049, each stated by a requirement of
//! design 0005:
//!
//! - The draft's question set put to the owner through `bal-rounds`, and
//!   the revision order for a set closed with a rejected recommendation,
//!   its plan submitted as a new draft (PLN-R24, PLN-R27).
//! - `question-open` refusing the approval while a question is open
//!   (PLN-R26).
//! - The proposed split, which the owner decides on (PLN-R10), with
//!   `over-capacity` (PLN-R9).

/// What a session is told when it follows the plan command. Ported from
/// Cadence tag `v3.9.0`: `skills/cad-plan/SKILL.md` and
/// `cadence-core/workflows/plan.md`, step by step.
pub const TEXT: &str = r#"Baley plan turns one phase into a typed plan: atomic tasks, each with files, an action and a falsifiable verification, and the evidence map that binds every truth the phase serves to its proof. The planner writes it in a fresh context from its work order, and the owner answers its questions and approves it at the terminal.

<step name="resolve_phase">
Take the phase the owner names. If none is named, read the accepted roadmap with `baley_query` `{"operation":"document","identity":{"kind":"roadmap"}}` and ask the owner which phase to plan, by its number. If the roadmap holds no such phase, stop and say so: there is no phase to plan until the roadmap holds it, and a phase is declared through `bal-phase` first. An answer too long for one part names its `next` part. Ask for it by adding `"part":<that number>`, and join the bodies in order.
</step>

<step name="plan_phase">
Ask the owner which stories the phase commits, in order: by default the next backlog stories the owner picks. Ask for the planner's order with one `baley_apply` call: `{"operation":"phase-plan","request_id":"<a fresh lowercase UUID>","phase":<the phase's number>,"stories":[<each story's id, in order>],"instruction":"bal-plan"}`.

Show the owner a refusal of `phase-plan` with its `code`, `slot` and `reason`: `phase-active` while another phase is active, `no-such-phase` and `no-such-story` for a phase or story the roadmap does not hold, `story-unrefined` for a story with no approved truths, which is refined through `bal-context` first, and `story-in-phase` for a story committed to another open phase.

When the stories change what the phase commits, the answer is a draft of that change: give the owner its digest to preview with `baley scope preview <digest>` and approve with `baley scope approve <digest>` at the terminal, then call `phase-plan` again.

The receipt names the order in `work_order` and gives the worker's `prompt` and `launch`. When the answer holds `settings` lines, show them to the owner. Launch the worker exactly as `launch` says, with `prompt` as its whole prompt. Change nothing in the order and add nothing to the prompt. The order binds the phase, its committed stories with their approved truths and the project's description as they stood when it was issued, so the worker reads them from the order. Then wait - do not read, edit, or plan anything else while the worker runs.

When the worker cannot launch, or returns without its result, call `baley_apply` with `{"operation":"work-order-fail","request_id":"<a fresh lowercase UUID>","work_order":"<the order's id>","reason":"<why it failed, in plain words>","instruction":"bal-plan"}`, then with `{"operation":"work-order-retry","request_id":"<a fresh lowercase UUID>","work_order":"<the failed order's id>","instruction":"bal-plan"}`, and launch the retried order from its receipt the same way. Never fail or retry an order only because its worker has not answered yet.
</step>

<step name="submit_plan">
The planner returns one typed plan, or a proposed split beginning `Proposed split:`.

Submit a plan with one `baley_apply` call: `{"operation":"plan-submit","request_id":"<a fresh lowercase UUID>","work_order":"<the planner order's id>","plan":<the plan exactly as returned>}`. Send the plan exactly as returned: never edit it. On a refusal, show the owner its `code`, `slot` and `reason`. The receipt gives the draft's `digest`, its preview, the plan's size and the phase's size against its capacity, and the draft's question set.

A returned `Proposed split:`, or a plan refused `over-capacity`, goes to the owner. State the planner's reason, or the size the phase would reach and its capacity, in one line. The owner decides which story waits, or splits a story or the phase through `bal-story` or `bal-phase`; then plan again from the top. A split counts only when the owner approves it.
</step>

<step name="rounds">
Put the draft's question set to the owner. Read the rounds relay with `baley_query` `{"operation":"instruction","identity":"bal-rounds"}`, and put each round to the owner as it says. A draft with no questions has an empty set, closed at once.

When the set closes with an answer that rejects a recommended answer, the draft is not approved as written: Baley issues the planner a revision order carrying the draft and every answer and deferral on its set. Launch it exactly as its `launch` says, with its `prompt` as the whole prompt, and submit its plan with `plan-submit` as a new draft, as above. A deferral leaves the plan on that question's recommended answer.
</step>

<step name="done">
Give the owner the draft's digest. The owner previews it with `baley scope preview <digest>` and approves it with `baley plan approve <digest>` at the terminal, which records the plan with its number. While a question of the draft's set is open, the approval is refused `question-open`. Nothing is approved over MCP, so approve nothing yourself.

A phase that needs another plan gets it from another planner order, submitted on its own and numbered when it is approved.
</step>

Use a fresh request id for each call. Send a request id again only to retry the same call, with every other argument unchanged.

On a `refused` answer, show the owner its `code`, `slot` and `reason`. On a `failed` answer marked `retryable`, send the same call again unchanged. On any other `failed` answer, show the owner its `code` and `reason`.

<guardrails>
- The planner runs in a fresh context from its work order. Never plan in the main context.
- Never modify source code. Write no file and make no commit: the plan, its questions and their answers live only in the ledger.
</guardrails>
"#;
