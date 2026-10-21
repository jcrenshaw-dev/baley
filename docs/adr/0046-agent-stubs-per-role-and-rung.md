# 0046: Carry a route's effort in one agent stub per role and rung

| | |
|---|---|
| Status | Accepted |
| Date | 2026-10-21 |
| Deciders | John Crenshaw |
| Design document | [0012: Host interface](../design/0012-host-interface.md), [0003: Configuration and routing](../design/0003-configuration-and-routing.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

Every work order carries a route: the role, the model to run, if any, and the effort rung, raised one rung on a retry when escalation is on ([0003](../design/0003-configuration-and-routing.md)). The worker is a Claude Code subagent the session starts with the Agent tool, so the route has to reach that subagent through something Claude Code reads. A subagent is defined by a file under `agents/` in the Claude folder, whose frontmatter may set `name`, `description`, `model`, `effort` and `mcpServers`; only `name` and `description` are required, `model` takes `inherit`, `effort` takes `low` to `max`, and an `mcpServers` entry may be the bare name of a server the session already has. The Agent tool also takes a `model` parameter per call and, from Claude Code v2.1.292, an `effort` parameter.

Nothing Baley can see tells it which model and effort a subagent actually ran at. An alias such as `opus` is resolved by Claude Code; no route sends a model when the owner set none; a concrete id may never have been observed in a run; and a managed `maxEffortLevel` or a per-role cap may lower any level. A dated table of the levels each concrete model supports can predict an answer, but it is a reading of documentation, not an observation.

The question is how the route's model and effort reach the subagent, and what the record claims about what ran. It was decided on 2026-10-09.

## Decision drivers

- The effort Baley requests for a worker is fixed by what Baley installs, not by a choice the session makes on each call.
- `baley doctor` can check that what Claude Code reads is what Baley wrote.
- The owner's per-role model setting reaches the worker without reinstalling.
- The record never claims a model or effort was observed when it was not.
- No dependence on a recent Claude Code feature where an older mechanism works.

## Considered options

1. One stub per role and rung, carrying the rung's `effort` and `model: inherit`, with the model passed per call
2. One stub per role, with the effort passed per call through the Agent tool's `effort` parameter
3. One stub per role with a fixed model
4. One stub per role and rung, with the effective model and effort filled in from the dated table

## Decision

Chosen option: **1**, because it fixes the effort Baley sends in a file the doctor can compare, lets the model follow the owner's settings and claims nothing that was not observed.

Install places thirty agent stubs, one for each of the six roles (`planner`, `analyzer`, `checker`, `executor`, `verifier`, `reviewer`) at each of the five rungs (`low`, `medium`, `high`, `xhigh`, `max`), named `bal-<role>-<rung>` and placed at `agents/<name>.md` in the Claude folder. Each stub's frontmatter has its `name`, a `description` saying to use it only when a Baley work order names it, `model: inherit`, its rung's `effort` and the session's `baley` server by name, with no inline server and no fixed model. Its body is one line telling the worker to read its order by id through `baley_query` `document` with kind `work_order`. The bytes depend only on compiled role, rung and name tables, so the doctor compares each placed file with what install wrote, and the guard protects each placed file by its own path, never the `agents/` folder, so the owner's own agent files stay editable.

The work order names the stub for its role and rung. When the route has a model, the order tells the session to pass it as the Agent tool's `model` parameter; with none, nothing is passed and `model: inherit` applies.

The route record keeps the effective model and effort apart from what was asked and sent, and records both as `unknown`, with the reasons: `no-model`, `alias` or `unobserved-id` for the model, then `possible-host-cap`. The dated effort table's answer is recorded beside them as `supported_effort`, a prediction for the requested concrete model, and never stands in for an effective value.

## Consequences

### Positive

- The effort Baley sends for a worker is held in the stub install wrote and the doctor checks, so no session can forget to send it.
- Escalation is a change of stub name, which needs no new host mechanism.
- A role's `model` setting reaches the worker as soon as the next order is issued.
- No record says a model or effort ran when nothing observed it.

### Negative

- Thirty files are placed in the Claude folder instead of six.
- Whether Claude Code honours the Agent tool's `model` over `model: inherit`, and how a managed cap meets a stub's `effort`, is not yet observed.
- Every effective value stays `unknown` until a run is observed.

### Follow-up

- Build 4 T15's live check observes whether Claude Code accepts the stub frontmatter, whether the per-call `model` overrides `model: inherit`, and what a capped effort does.

## Options in detail

### Option one

The decision above. It meets every driver; its costs are the negatives listed.

### Option two

Six stubs, one per role, and the session passes the rung as the Agent tool's `effort` parameter on each call. It places fewer files, but the session must choose to pass the parameter, a missed parameter runs silently at the model's default (`medium` on Opus 5.5, Sonnet 5.5 and Haiku 5.5), the doctor cannot check a per-call value, and it needs Claude Code v2.1.292 or later. It fails the first, second and last drivers.

### Option three

One stub per role with the model written into its frontmatter. The model would be fixed when install ran, so a role's `model` setting changed later would not reach the worker until install ran again, and the effort would still need another carrier. It fails the third driver.

### Option four

The stubs of option one, with the effective model and effort filled in from the dated table or the stub. It makes the record look complete, but it invents effort for an alias and for a concrete id under a cap, and states as observed what was only predicted. It fails the fourth driver.
