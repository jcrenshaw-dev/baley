# 0047: Record an owner's act a session relays as Baley's, naming the session

| | |
|---|---|
| Status | Accepted |
| Date | 2026-10-22 |
| Deciders | John Crenshaw |
| Design document | [0005: Context, plans and acceptance](../design/0005-context-plans-and-acceptance.md), [0012: Host interface](../design/0012-host-interface.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

Baley puts the decisions a story or a plan needs to the owner as question sets, answered in rounds ([ADR 0030](0030-question-rounds.md)). The host session reads each round, puts it to the owner and relays the owner's answers and deferrals back unchanged (design 0005, PLN-R24). HST-R9 records every answer with `owner.name` and Baley's clock (design 0012). The review rulings Build 4 adds later reach Baley the same way: the owner rules, and the session relays the ruling.

Over MCP every call comes from the model. Nothing in a call proves that a person gave the answer it carries, which is why approvals are served only on the command line and never over MCP ([ADR 0044](0044-owner-acts-on-the-command-line.md)). The threat model holds that an agent must never get an act recorded as the owner's (`THREAT-MODEL.md`, owner acts). A relayed answer is still the owner's decision as far as the session reports it, and the next reader of the record needs to know whose decision it is said to be.

The question is what the record says about who gave an answer or a deferral the session relays, and how it tells such an act apart from one the owner gives at the terminal. It was decided on 2026-10-22.

## Decision drivers

- No MCP call gets an act recorded as the owner's.
- Every answer and deferral still carries the owner's name and Baley's time (HST-R9).
- A reader can tell from the record which acts the owner gave at the terminal and which a session relayed, and which session relayed them.
- The owner can answer a round in a session without leaving it for the terminal each time.
- One rule serves answers and deferrals now and relayed review rulings later.

## Considered options

1. Record a relayed act with actor `owner`
2. Record a relayed act with actor `baley`, the session as the caller, and `owner.name` and Baley's clock in the payload
3. Refuse relayed acts, and take answers and deferrals only at the terminal

## Decision

Chosen option: **2**, because it keeps every answer's owner and time on the record without letting a model's call claim to be the owner's, and it keeps answering in a session.

An answer or deferral a session relays through `questions-answer` is recorded under a command with actor `baley` and the session's server caller, the same caller checkout admission and the policy step record, so the record names the session that relayed it. Each event's payload carries `owner`, read from `owner.name` with git `user.name` as the fallback, and `at`, the command's time by Baley's clock, as an approval's does. An act given at the terminal with `baley questions answer` goes through the same judgement and recording, after checkout admission and the policy step, and is recorded with actor `owner` and no caller, so the record shows which acts the owner gave there. Both surfaces call the same library functions and differ only in the command's actor and caller.

## Consequences

### Positive

- The record never claims a person gave an act that only a session relayed, and the threat model's rule stands as written.
- Every relayed answer and deferral still carries the owner's name and Baley's time.
- A reader tells relayed acts from terminal acts by actor and caller, and finds the relaying session in the caller.
- The owner answers rounds in the session or at the terminal, whichever is at hand.

### Negative

- A relayed act's owner name is Baley's statement of whose act the session relayed, not proof that the owner gave it.
- A reader who wants only the owner's own acts has to read the actor as well as the `owner` field.

### Follow-up

- The review rulings a session relays follow the same rule when they are built: actor `baley`, the session as the caller, and the owner's name and Baley's time in the payload, with actor `owner` only for a ruling given at the terminal.

## Options in detail

### Option one

Record a relayed answer with actor `owner`, as an approval at the terminal is. Answers would read the same whichever surface gave them, but any model could produce an act recorded as the owner's by calling the operation, which is what the threat model forbids and what ADR 0044 avoided for approvals. It fails the first and third drivers.

### Option two

The decision above. It meets every driver; its costs are the negatives listed.

### Option three

Serve no relayed answer at all and take every answer and deferral at the terminal, as approvals are taken. Every recorded act would be the owner's, but each round would send the owner from the session to the terminal and back, though a round is a question to answer, not an approval of work, and the session already holds the conversation the questions came from. It fails the fourth driver, and relayed review rulings would need the same detour.
