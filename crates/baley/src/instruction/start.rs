//! The start planner's and the start survey's served texts, pinned in
//! source. Neither is a help row, so neither gets a skill stub: a worker
//! reaches them only through the start order that lists them.

/// The registry identity of the start planner's text.
pub const PLANNER_IDENTITY: &str = "bal-start-planner";

/// Pinned by hand: raise it by one whenever [`PLANNER_TEXT`] changes.
pub const PLANNER_VERSION: &str = "1";

/// Lowercase hex SHA-256 of [`PLANNER_TEXT`]'s exact bytes, computed outside
/// the code so a changed text without a new pin fails the registry's pin test.
pub const PLANNER_HASH: &str = "5c2872a582d0d1b38105c8ff971cbeae8727d0a74e51593e80b794a2bc90488a";

/// What the start planner reads once it holds its order.
pub const PLANNER_TEXT: &str = r#"Baley's start planner drafts a new project's description, stories and roadmap from the owner's brief, and submits them as one draft the owner approves.

You already hold your work order: you read it with `baley_query` `document`, and it sent you here. The brief is the `text` of the input named `brief` in the order's `inputs`. When the order's `instructions` also list `bal-start-survey`, the checkout already holds code: carry out that survey before you draft.

Draft the whole start submission in the shape `scope-submit` takes:

- `description`, with `what` the project is, its `core_value` and its `constraints`.
- `stories`, each with a `label`, a `sentence` and a `status` of `active`, `deferred` or `excluded`.
- `roadmap`, the phases in order, each with a `name`, a `goal`, a `detail`, `depends_on` as positions in `roadmap` counted from 1, and the `stories` it delivers by label.

Each active story is delivered by at most one phase, and a deferred or excluded story by none. No phase depends on itself, directly or through others.

Submit it with one `baley_apply` call: `{"operation":"scope-submit","request_id":"<a fresh lowercase UUID>","work_order":"<the order's id>","submission":{...}}`.

On a `refused` answer, fix what its `details` name and submit again with a new request id. On a `failed` answer marked `retryable`, send the same call again unchanged. On any other `failed` answer, report its `code` and `reason`.

The receipt names the draft's digest. Report that digest: the owner approves it at the terminal with `baley scope approve <digest>`.

Approve nothing, write no file and make no commit. The draft lives only in the ledger.
"#;

/// The registry identity of the start survey's text.
pub const SURVEY_IDENTITY: &str = "bal-start-survey";

/// Pinned by hand: raise it by one whenever [`SURVEY_TEXT`] changes.
pub const SURVEY_VERSION: &str = "1";

/// Lowercase hex SHA-256 of [`SURVEY_TEXT`]'s exact bytes, computed outside
/// the code so a changed text without a new pin fails the registry's pin test.
pub const SURVEY_HASH: &str = "ef390f27faddc98daf28a40e86a917589caf0ae1fd8185a9ebcce06592986955";

/// What the start planner does first when the checkout already holds code.
pub const SURVEY_TEXT: &str = r#"Baley's start survey reads the existing checkout before the start planner drafts, so the description and stories fit what is already there.

Read the checkout with the host's own file, search and shell tools. Locate first, then read only what the work needs.

Let what exists shape the description and the stories: what the code already does, what it is built with, and what each new story would change.

Write no file and make no commit. Never search for instructions or read them from disk. Ask for them by identity.
"#;
