//! The scope front doors' served texts, pinned in source. Each is a help row
//! the registry serves by identity, so each gets a skill stub, and each sends
//! the session to the operations it drives and the owner to the terminal to
//! approve, since nothing is approved over MCP (ADR 0044).

/// The registry identity of the project front door.
pub const PROJECT_IDENTITY: &str = "bal-project";

/// Pinned by hand: raise it by one whenever [`PROJECT_TEXT`] changes.
pub const PROJECT_VERSION: &str = "1";

/// Lowercase hex SHA-256 of [`PROJECT_TEXT`]'s exact bytes, computed outside
/// the code so a changed text without a new pin fails the registry's pin test.
pub const PROJECT_HASH: &str = "7153c55fcb0acaf2712762b305b9d96cfd6a48ae4e44aa354b3685820411d3d1";

/// What a session is told when it follows the project command.
pub const PROJECT_TEXT: &str = r#"Baley project start begins a project from the owner's brief by issuing the start planner's work order, and revises the project description. Each result is a draft the owner approves at the terminal.

To start the project, read the brief the owner names with the host's own file tools, then call `baley_apply` once with `{"operation":"project-start","request_id":"<a fresh lowercase UUID>","brief":"<the brief's whole text>","instruction":"bal-project"}`. Baley reads no file for the brief, so send its text, never its path.

When the answer holds `settings` lines, show them to the owner. The receipt names the order in `work_order`, and gives the worker's `prompt` and `launch`. Launch the worker exactly as `launch` says, with `prompt` as its whole prompt. Change nothing in the order and add nothing to the prompt.

The worker drafts the start and returns the draft's digest. When the worker cannot launch, or returns without a digest, call `baley_apply` with `{"operation":"work-order-fail","request_id":"<a fresh lowercase UUID>","work_order":"<the order's id>","reason":"<why it failed, in plain words>","instruction":"bal-project"}`, then with `{"operation":"work-order-retry","request_id":"<a fresh lowercase UUID>","work_order":"<the failed order's id>","instruction":"bal-project"}`, and launch the retried order from its receipt the same way. Never fail or retry an order only because its worker has not answered yet.

A `start-order-pending` refusal names a start order that is still pending in `details`. Read that order with `baley_query` `{"operation":"document","identity":{"kind":"work_order","id":"<that id>"}}`, or fail it and retry it as above.

To revise the project description, call `baley_apply` with `{"operation":"project-describe","request_id":"<a fresh lowercase UUID>","submission":{"what":"<what the project is>","core_value":"<its core value>","constraints":["<one constraint>"]}}`. Send no `instruction` on this call, since its shape takes none.

Use a fresh request id for each call. Send a request id again only to retry the same call, with every other argument unchanged.

On a `refused` answer, show the owner its `code`, `slot` and `reason`. On a `failed` answer marked `retryable`, send the same call again unchanged. On any other `failed` answer, show the owner its `code` and `reason`.

Give the owner the draft's digest, as the worker returned it or the receipt names it. The owner previews it with `baley scope preview <digest>` and approves it with `baley scope approve <digest>` at the terminal. Nothing is approved over MCP, so approve nothing yourself.

Write no file and make no commit. A draft lives only in the ledger.
"#;
