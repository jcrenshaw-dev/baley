# 0049: Let a ported text add a rule an accepted design requirement states

| | |
|---|---|
| Status | Accepted |
| Date | 2026-10-23 |
| Deciders | John Crenshaw |
| Design document | [0005: Context, plans and acceptance](../design/0005-context-plans-and-acceptance.md), [0012: Host interface](../design/0012-host-interface.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

[ADR 0048](0048-port-what-cadence-hands-agents.md) makes Cadence's text the source of every Baley text that has a Cadence counterpart. A ported text changes only where it names Cadence's own files, commands or state. Where Cadence contradicts an accepted Baley ADR, the ADR stands and the change is listed beside the text. Where Cadence contradicts only a design document's wording, Cadence's rule stands and the document is corrected.

None of those cases covers a rule Cadence does not have at all. Build 4 T7 ported the analyzer's questions and drafting texts and the `bal-context` front door, and the owner's decisions for that build put rules in them that no Cadence text states and no ADR forces. Design 0005 states them as requirements: the host session submits the analyzer's questions as returned, and drops a draft truth that does not hold against the code or the answers, before the owner sees it (PLN-R6); a truth is one sentence in a fixed form (PLN-R2); the owner attests a truth's observability and fixed oracle (PLN-R3). An analyzer that called `baley_apply` itself, or a session that put every draft truth to the owner unchecked, would contradict those requirements, yet ADR 0048's rule left no room to say so in the text.

The question is whether a ported text may carry such a rule, and how it stays visible and reviewed when it does.

## Decision drivers

- Every change from the proven Cadence source is visible and reviewed (ADR 0048).
- A served text tells its reader what the design requires of it, so a rule the owner decided is not left for the reader to guess.
- A rule's reason is written down where an engineer who reads the text can follow it.
- ADR 0048's body is never edited.

## Considered options

1. Leave the rule out of the ported text and rely on the operation's refusals
2. Record an ADR for each added rule
3. Let a ported text add a rule an accepted owner decision requires, where a design document states it, and list each addition beside the text with that requirement

## Decision

Chosen option: **3**, because it keeps every change from Cadence listed and reviewed, puts the owner's rule in the text its reader follows, and costs no ADR per rule.

A ported text may add a rule that an accepted owner decision requires, where a Baley design document states that decision as a requirement. Each such addition is listed beside the text, in the doc comment of the Rust module that holds the text's constant, naming this record and the design requirement that states the rule. A rule no committed design document states is not added; the decision is first written into its design document.

This adds to ADR 0048's rule and changes nothing else in it. A change that replaces a Cadence file, command or state name with Baley's still needs no listing, a change an accepted ADR forces is still listed with that ADR, and a design document whose wording only contradicts Cadence is still corrected to Cadence's rule.

## Consequences

### Positive

- A ported text states what the design requires of its reader, such as who submits a worker's return.
- Every addition is listed beside the text with the requirement it comes from, so the review against the Cadence source finds no unexplained line.

### Negative

- A ported text drifts further from its Cadence source with each addition, so a later port of a newer Cadence release has more listed lines to carry over.
- The rule's source is a design requirement, which can change in a later pull request; the listing beside the text has to change with it.

### Follow-up

- Build 4 T7 lists its additions beside the analyzer's texts (`crates/baley/src/instruction/analyzer.rs`) and the `bal-context` text (`crates/baley/src/instruction/context.rs`).
- Design 0012's instructions table says each ported text lists its differences with the ADR or design requirement behind each.

## Options in detail

### Option 1: leave the rule out

Keeps the text closest to Cadence. The analyzer would not be told that its return is submitted by the session, and a session would not be told to drop a draft truth that does not hold, so the reader learns the rule only from a refusal or not at all. Fails the second driver.

### Option 2: an ADR per added rule

Visible and reviewed, but each rule is already decided and stated in its design document, so the ADR would repeat it and add a record per sentence. Costly for no gain in review.

### Option 3: add the rule and list it with its design requirement (chosen)

Visible and reviewed beside the text, with the reason one link away in the design document. Costs a listing per addition and the work of keeping it current.
