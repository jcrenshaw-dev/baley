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

/// The registry identity of the scope front door.
pub const SCOPE_IDENTITY: &str = "bal-scope";

/// Pinned by hand: raise it by one whenever [`SCOPE_TEXT`] changes.
pub const SCOPE_VERSION: &str = "1";

/// Lowercase hex SHA-256 of [`SCOPE_TEXT`]'s exact bytes, computed outside
/// the code so a changed text without a new pin fails the registry's pin test.
pub const SCOPE_HASH: &str = "8ca51542feb46f946b787f5609c6a896ee5cd7954655c9ba7397a709390e74c8";

/// What a session is told when it follows the scope command.
pub const SCOPE_TEXT: &str = r#"Baley scope reads a scope draft by its digest, with the accepted roadmap and its stories beside it, and shows the owner how to approve the draft at the terminal. It changes nothing.

To read a draft, call `baley_query` with `{"operation":"document","identity":{"kind":"draft","digest":"<the draft's digest>"}}`. To read the accepted roadmap, its description, phases and stories, call it with `{"operation":"document","identity":{"kind":"roadmap"}}`. To read one accepted story, call it with `{"operation":"document","identity":{"kind":"story","id":<the story's id>}}`. An answer too long for one part names its `next` part. Ask for it by adding `"part":<that number>` beside `identity`, and join the bodies in order.

These reads take no request id. Send a read again unchanged only to retry it.

On a `refused` answer, show the owner its `code`, `slot` and `reason`. On a `failed` answer marked `retryable`, send the same call again unchanged. On any other `failed` answer, show the owner its `code` and `reason`.

Show the owner what the draft changes and give them its digest. The owner previews it with `baley scope preview <digest>` and approves it with `baley scope approve <digest>` at the terminal. Nothing is approved over MCP, so approve nothing yourself.

Write no file and make no commit. A draft lives only in the ledger.
"#;

/// The registry identity of the phase front door.
pub const PHASE_IDENTITY: &str = "bal-phase";

/// Pinned by hand: raise it by one whenever [`PHASE_TEXT`] changes.
pub const PHASE_VERSION: &str = "1";

/// Lowercase hex SHA-256 of [`PHASE_TEXT`]'s exact bytes, computed outside
/// the code so a changed text without a new pin fails the registry's pin test.
pub const PHASE_HASH: &str = "8232b5d90eea73bef1d92f09919ce23ec8dae6dec60bd76b75dca6c65c09ba3b";

/// What a session is told when it follows the phase command.
pub const PHASE_TEXT: &str = r#"Baley phase declares, edits, reorders or withdraws a phase of the accepted roadmap. Each change is a draft the owner approves at the terminal.

Read the accepted roadmap first with `baley_query` `{"operation":"document","identity":{"kind":"roadmap"}}`. Phases are named by number and stories by id.

To declare a phase, call `baley_apply` with `{"operation":"phase-declare","request_id":"<a fresh lowercase UUID>","submission":{"name":"<its name>","goal":"<its goal>","detail":"<its detail>","depends_on":[<the numbers of the phases it depends on>],"stories":[<the ids of the stories it serves>],"position":<its place in the phase order, counted from 1>}}`. Leave out `position` to place it last.

To edit a phase, call `baley_apply` with `{"operation":"phase-edit","request_id":"<a fresh lowercase UUID>","submission":{"phase":<its number>,"goal":"<its new goal>"}}`, sending only the fields that change, at least one of `name`, `goal` and `detail` as text, `depends_on` as phase numbers and `stories` as story ids.

To reorder the phases, call `baley_apply` with `{"operation":"phase-reorder","request_id":"<a fresh lowercase UUID>","submission":{"order":[<the number of every phase, each once, in the new order>]}}`.

To withdraw a phase, call `baley_apply` with `{"operation":"phase-withdraw","request_id":"<a fresh lowercase UUID>","submission":{"phase":<its number>,"decisions":[{"story":<a story id it serves>,"to":<the number of the phase that serves it instead, or "drop">}]}}`, with one decision for each story it serves.

Send no `instruction` on these calls, since their shapes take none.

Use a fresh request id for each call. Send a request id again only to retry the same call, with every other argument unchanged.

On a `refused` answer, show the owner its `code`, `slot` and `reason`. On a `failed` answer marked `retryable`, send the same call again unchanged. On any other `failed` answer, show the owner its `code` and `reason`.

The receipt names the draft's `digest`. Give it to the owner, who previews it with `baley scope preview <digest>` and approves it with `baley scope approve <digest>` at the terminal. Nothing is approved over MCP, so approve nothing yourself.

Write no file and make no commit. A draft lives only in the ledger.
"#;

/// The registry identity of the story front door.
pub const STORY_IDENTITY: &str = "bal-story";

/// Pinned by hand: raise it by one whenever [`STORY_TEXT`] changes.
pub const STORY_VERSION: &str = "1";

/// Lowercase hex SHA-256 of [`STORY_TEXT`]'s exact bytes, computed outside
/// the code so a changed text without a new pin fails the registry's pin test.
pub const STORY_HASH: &str = "7688ddacbf34f6c93a4eb82b8eb60d69089eb2fee0ab5176097a4dc560877693";

/// What a session is told when it follows the story command.
pub const STORY_TEXT: &str = r#"Baley story declares, edits, reassigns or drops a story of the accepted roadmap. Each change is a draft the owner approves at the terminal.

Read the accepted roadmap first with `baley_query` `{"operation":"document","identity":{"kind":"roadmap"}}`. Phases are named by number and stories by id.

To declare a story, call `baley_apply` with `{"operation":"story-declare","request_id":"<a fresh lowercase UUID>","submission":{"sentence":"<the story>","status":"active","phase":<the number of the phase that serves it, or null for the backlog>}}`. The status is `active`, `deferred` or `excluded`, and only an active story is served by a phase.

To correct a story's sentence, call `baley_apply` with `{"operation":"story-edit","request_id":"<a fresh lowercase UUID>","submission":{"story":<its id>,"sentence":"<its corrected text>"}}`.

To move a story, call `baley_apply` with `{"operation":"story-reassign","request_id":"<a fresh lowercase UUID>","submission":{"story":<its id>,"phase":<the number of the phase that serves it now, or null for the backlog>}}`.

To drop a story, call `baley_apply` with `{"operation":"story-drop","request_id":"<a fresh lowercase UUID>","submission":{"story":<its id>}}`.

Send no `instruction` on these calls, since their shapes take none.

Use a fresh request id for each call. Send a request id again only to retry the same call, with every other argument unchanged.

On a `refused` answer, show the owner its `code`, `slot` and `reason`. On a `failed` answer marked `retryable`, send the same call again unchanged. On any other `failed` answer, show the owner its `code` and `reason`.

The receipt names the draft's `digest`. Give it to the owner, who previews it with `baley scope preview <digest>` and approves it with `baley scope approve <digest>` at the terminal. Nothing is approved over MCP, so approve nothing yourself.

Write no file and make no commit. A draft lives only in the ledger.
"#;

/// The registry identity of the backlog front door.
pub const BACKLOG_IDENTITY: &str = "bal-backlog";

/// Pinned by hand: raise it by one whenever [`BACKLOG_TEXT`] changes.
pub const BACKLOG_VERSION: &str = "1";

/// Lowercase hex SHA-256 of [`BACKLOG_TEXT`]'s exact bytes, computed outside
/// the code so a changed text without a new pin fails the registry's pin test.
pub const BACKLOG_HASH: &str = "4dceb6272fd0d130c5d74c904919128e51a06eab7330fb7bc02b932aa3322351";

/// What a session is told when it follows the backlog command.
pub const BACKLOG_TEXT: &str = r#"Baley backlog reorders the backlog of the accepted roadmap. The new order is a draft the owner approves at the terminal.

Read the accepted roadmap first with `baley_query` `{"operation":"document","identity":{"kind":"roadmap"}}`. Stories are named by id.

To reorder the backlog, call `baley_apply` with `{"operation":"backlog-reorder","request_id":"<a fresh lowercase UUID>","submission":{"order":[<the id of every active and deferred story, each once, in the new priority order>]}}`.

Send no `instruction` on this call, since its shape takes none.

Use a fresh request id for each call. Send a request id again only to retry the same call, with every other argument unchanged.

On a `refused` answer, show the owner its `code`, `slot` and `reason`. On a `failed` answer marked `retryable`, send the same call again unchanged. On any other `failed` answer, show the owner its `code` and `reason`.

The receipt names the draft's `digest`. Give it to the owner, who previews it with `baley scope preview <digest>` and approves it with `baley scope approve <digest>` at the terminal. Nothing is approved over MCP, so approve nothing yourself.

Write no file and make no commit. A draft lives only in the ledger.
"#;
