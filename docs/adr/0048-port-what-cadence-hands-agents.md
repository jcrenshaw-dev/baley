# 0048: Port what Cadence hands its agents into the binary

| | |
|---|---|
| Status | Accepted |
| Date | 2026-10-22 |
| Deciders | John Crenshaw |
| Design document | [0012: Host interface](../design/0012-host-interface.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

Baley grew out of Cadence, a planning and gate system for Claude Code. Cadence runs Baley's own builds, so what it hands its agents is proven in use: a role contract per agent, the dispatch prompt its workflow files compose, the templates the agent fills in, and the checks its `.mjs` code runs on what the agent returns.

[ADR 0009](0009-served-instructions.md) decided where Baley's instructions live: compiled into the binary and served by identity. It did not decide where their text comes from. The [PRD](../design/prd/baley.md) said everything underneath the working loop is designed afresh and that Cadence's code is not carried over, so each build's plan describes the rules a text must carry and the text is written new. Five texts exist, none with a Cadence counterpart. None of the six role texts (analyzer, planner, checker, reviewer, executor, verifier) is written yet; the first is Build 4 T7.

Writing a role text new from design prose loses rules Cadence learned by running. The question is what Baley's parts that have a Cadence counterpart are built from.

## Decision drivers

- Behaviour proven in use is kept.
- Every instruction still has one source, compiled into the binary (ADR 0009).
- Every change from the proven source is visible and reviewed.
- Nothing from Cadence is needed at run time: no Node, no Cadence install.

## Considered options

1. Write each text and check new from Baley's design documents
2. Port Cadence's texts and checks, adapting only what names Cadence's own files and commands
3. Ship Cadence's files beside the binary and serve them as they are

## Decision

Chosen option: **2**, because it keeps the proven behaviour and keeps one compiled source.

For every Baley part with a Cadence counterpart, the counterpart is the source:

- **A text** is Cadence's text compiled into the binary under ADR 0009. It changes only where it names Cadence's own files, commands or state, such as `.planning/`, `PLAN.md`, the STATE cursor or a `planning.mjs` call, which become the Baley operations and records that do the same job.
- **A check** is Cadence's `.mjs` logic ported to Rust with the same rules.

Parts with no Cadence counterpart are written for Baley: the start planner and start survey, the scope front doors, capture, the read contract and the rounds relay.

Where a Cadence text or check contradicts an accepted Baley ADR, the ADR stands, and the owning task lists each such change beside the ported text. Where it contradicts only a design document's wording, Cadence's rule stands and the design document is corrected in the same pull request.

The source is the Cadence release current when the owning task is planned, named by its tag in the task's plan. A later Cadence change reaches Baley only through a task that ports it.

Each ported text names its source, the Cadence tag and file, in the doc comment of its Rust constant. The owning task's pull request shows the served text beside its source, so every change from Cadence is reviewed.

The sources, by owning task:

| Baley part | Owner | Cadence source |
|---|---|---|
| Analyzer text | Build 4 T7 | `skills/cad-assumptions-analyzer-contract/SKILL.md`, `cadence-core/workflows/context.md` |
| Planner text | Build 4 T8 | `skills/cad-planner-contract/SKILL.md`, `cadence-core/workflows/plan.md`, `cadence-core/templates/PLAN.md` |
| Checker text | Build 4 T10 | `skills/cad-plan-checker-contract/SKILL.md` |
| Reviewer text | Build 4 T13 | `skills/cad-reviewer-contract/SKILL.md`, `cadence-core/references/reviewer-brief.md`, `cadence-core/references/review-triggers.md` |
| Adjudication and owner rulings | Build 4 T14 | `planning.mjs adjudication`, `cadence-core/references/triage-gate.md`, `cadence-core/references/review-record.md` |
| Lease risk and effective gate | Build 4 T11 | `planning.mjs risk-check`, `cadence-core/references/risk-surface.md` |
| Executor text | Build 5 | `skills/cad-executor-contract/SKILL.md`, `cadence-core/workflows/execute.md` |
| Verifier text | Build 5 | `skills/cad-verifier-contract/SKILL.md`, `cadence-core/workflows/verify.md` |
| Tests and checks run by Baley ([ADR 0014](0014-baley-runs-tests.md)) | Build 5 | `planning.mjs test-checks`, `planning.mjs test-evidence` |
| Lease enforced at close ([ADR 0018](0018-lease-enforced-at-close.md)) | Build 5 | `planning.mjs lease-check` |
| `bal-context`, `bal-plan`, `bal-review`, `bal-plan-review`, `bal-decision-review`, `bal-minimalism-review` | Build 4 | `/cad-context`, `/cad-plan`, `fire()` in `review-triggers.md`, `/cad-plan-review`, `/cad-decision-review`, `/cad-minimalism-review` |
| `bal-execute`, `bal-verify`, `bal-coverage`, `bal-audit` | Build 5 | `/cad-execute`, `/cad-verify`, `/cad-coverage`, `/cad-audit` |
| `bal-land`, `bal-milestone`, `bal-undo` | Build 6 | `/cad-land`, `/cad-milestone`, `/cad-undo` |
| `bal-progress`, `bal-suggest` | Build 7 | `/cad-progress`, `/cad-suggest` |
| `bal-task`, `bal-debug`, `bal-spike`, `bal-why` | Build 8 | `/cad-task`, `/cad-debug`, `/cad-spike`, `/cad-why` |

## Consequences

### Positive

- Rules Cadence learned by running reach Baley's agents unchanged.
- Each owning task starts from working text and logic instead of a blank page.
- Every difference from Cadence is a reviewed change with a stated reason.

### Negative

- Cadence's text is long: its six role contracts are about 60 KB together, and its workflow files add more. A served text is read in parts (HST-R6), so length costs reads, not correctness.
- Adapting a text means mapping each Cadence file and command to the Baley operation that does the same job, which is work in every owning task.
- Baley's text follows Cadence only up to the release it was ported from.

### Follow-up

- The PRD's two passages on Cadence say that what Cadence hands its agents carries over.
- Each owning task's entry in the Build 4 plan and in the roadmap's Builds 5 to 8 names its Cadence source from the table above.
- Design 0012's list of related ADRs names this record.
- Design 0015 section 6 classifies the Cadence names this record, the roadmap and each ported text's doc comment carry as ported sources, never aliases (UPK-R4, UPK-Q1).
- Whether `/cad-pause`, `/cad-health`, `/cad-report`, `/cad-docs-verify` and `/cad-config` get Baley parts is not decided here.

## Options in detail

### Option 1: write each text and check new

What the builds were doing. One source and full control of the wording, but every rule has to be rediscovered from design prose, and a rule Cadence learned in use is lost unless someone remembers it. Fails the first driver.

### Option 2: port Cadence's texts and checks

Keeps the proven behaviour and the single compiled source. Costs the mapping work in each owning task and a reviewed diff against the source.

### Option 3: ship Cadence's files beside the binary

Serves exactly what Cadence serves, but the text would name Cadence's files and commands that Baley does not have, the files on disk would drift from the binary that enforces them, and the checks would need Node at run time. Fails the second and fourth drivers, and ADR 0009.
