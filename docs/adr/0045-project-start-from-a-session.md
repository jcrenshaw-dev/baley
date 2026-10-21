# 0045: Start a project from a session, with the brief sent as text

| | |
|---|---|
| Status | Accepted |
| Date | 2026-10-21 |
| Deciders | John Crenshaw |
| Design document | [0004: Starting a project and changing scope](../design/0004-starting-a-project-and-changing-scope.md), [0003: Configuration and routing](../design/0003-configuration-and-routing.md), [0012: Host interface](../design/0012-host-interface.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

`project start` turns an owner's brief into the start planner's work order, and the planner's answer becomes the project's first scope draft. Design 0004 first had the start do everything a new project needs: run `baley init` on a repository with no project, run the settings interview of design 0003 when a settings file was missing, reaching the owner through the host's question mechanism when the start ran in a session, and read the brief from a file named with `--brief <file>`.

Three things made that shape wrong. The start's only output is a work order that a session launches, and [ADR 0044](0044-owner-acts-on-the-command-line.md) makes a step whose only purpose is to issue a work order session-only, so the start is an MCP operation, `project-start` on `baley_apply`. Over MCP every call comes from the model, so running `baley init` or the interview from the start would let a model tie a checkout to the ledger or write the owner's settings, which ADR 0044 keeps at the owner's terminal. And `baley serve` runs outside Claude Code's sandbox: a path argument a model names would be the first file the server opens on a model's word, and any file the owner can read, such as `~/.ssh` or Baley's own home, could be carried into the ledger and read back through the order.

A failed or abandoned start also needs an answer. A start planner's order that no worker answered can be failed and retried, with escalation, so a second start beside it would leave two orders one submission could answer and a retry count a session could step around.

The decision on the start itself was taken on 2026-10-09; the choice to send the brief as text was taken on 2026-10-18.

## Decision drivers

- Only the owner, at the terminal, ties a checkout to the ledger or writes settings.
- The server opens no path a model names.
- A new user learns the setup step from the start's own answer, without the start failing.
- A refused start leaves nothing recorded.
- A project has one record of how it began, and one live start order at a time, so retries and escalation count.

## Considered options

1. The start runs `baley init` and the settings interview itself
2. The start needs the project in the ledger and names the missing settings, with the brief as a path confined to the repository root
3. The same, with the brief as an unconfined path
4. The same, with the brief sent as text, and a new chain at attempt 1 for every start
5. The same, with the brief sent as text, and one live start order at a time

## Decision

Chosen option: **5**, because it is the only one that keeps settings and project identity at the owner's terminal, opens no path on a model's word and keeps one live start order whose retries count.

The start is a session operation. It needs the project already in this machine's ledger: for a project the ledger does not list, preparation answers `failed` `project-not-in-ledger`, naming `baley init` as the owner's step, and the start runs neither `baley init` nor the interview. Once a start is approved it is refused `project-already-started`.

When the host's policy has no global settings file, or the project file sets nothing beyond `[project]`, the answer carries one line for each, naming the missing file, saying the other file's settings and Baley's defaults apply, and naming `baley config interview`, and the start planner's order is still issued, under the host's merged effective policy.

The session reads the brief inside its own sandbox and sends its text as the `brief` argument; the server reads no path for it. The text is stored as a `record` payload and is the order's one input. A blank brief is refused `brief-blank` and one over 65,536 bytes `brief-too-large`, before anything is prepared and with nothing recorded.

One start order is live at a time. A start made while the start planner's order is pending, neither failed nor answered, is refused `start-order-pending`, naming that order, with nothing recorded. The session reads that order, or fails it with `work-order-fail` and retries it with `work-order-retry`.

This amends design 0004's PRJ-R5 (the brief), PRJ-R6 (the project must already be in the ledger) and PRJ-R9 (missing settings are named, not interviewed).

## Consequences

### Positive

- A model cannot create a ledger project or write settings through the start.
- No model-named path reaches the server, so the threat model gains no path argument to confine.
- A new user's first start still issues an order and says which command finishes the setup.
- The retry count and escalation of a start cannot be reset by starting again.

### Negative

- A repository with no project needs `baley init` at the terminal before the first start.
- The session must read the brief itself, so a brief Claude Code's sandbox denies cannot be started from.
- A brief over 65,536 bytes must be shortened.
- A stuck start order must be failed before a new attempt; a fresh start does not replace it.

### Follow-up

- The forge check at start (PRJ-R8) is Build 6's.

## Options in detail

### Option one

The start runs `baley init` on a repository with no project and the settings interview when a file is missing. It gives one front door for a new project, but every call over MCP comes from the model, so a model would create the ledger project and answer the interview. It fails the first driver.

### Option two

The brief is a path the server reads, refused when it resolves outside the repository root. It keeps the old `--brief <file>` input, but it is the first MCP path argument into a server that runs outside Claude Code's sandbox, and confining it needs a new section of the threat model. It fails the second driver.

### Option three

The brief is a path the server reads with no confinement. It is the simplest to build, and a session could have the server read `~/.ssh` or Baley's home into the ledger and read it back through the order. It fails the second driver outright.

### Option four

Every start opens a new chain at attempt 1. It never refuses a start, but a session can step around a failed order's retry count and escalation by starting again, and several pending start orders could each be answered by a `scope-submit`. It fails the last driver.

### Option five

The decision above. It meets every driver; its costs are the negatives listed.
