# Security policy

## Reporting a vulnerability

Report security problems privately through GitHub: on this repository, open the **Security** tab and choose **Report a vulnerability**. Do not open a public issue.

Include what you found, how to reproduce it, and what an attacker could do with it. You will get an answer, and credit in the fix if you want it.

## Scope

Baley is under active redesign and has no released version. Reports about the current `main` branch are in scope. Of most interest:

- anything that lets an agent bypass the owner's approval, the recorded evidence, or the branch policy the hook enforces;
- secrets reaching logs, records or the material sent to review providers;
- tampering with records that goes undetected.

## How this repository is audited

Baley is audited with [sentinel](https://github.com/jcrenshaw-dev/sentinel), pinned as the `.sentinel` submodule, so the checks that run here are the ones of the kit version this repository was audited with. The full audit runs on the maintainer's machine, never in CI, so findings stay private until they are fixed. Once an audit has run, its record is `SENTINEL.md` at the repository root: the date, the commit it covered, what ran and how it went, and the open findings as counts by severity, with no finding's title or location. The audit only finds and records. It fixes nothing, writes no tests and commits nothing.

## What runs on every pull request

Two workflows run on every pull request and every push to `main`. All six of their checks are required by the `main` ruleset, so a pull request cannot merge until each one passes.

- `test.yml` runs `cargo-test`, `clippy`, `cargo-deny` and `fmt`.
- `fast.yml` is sentinel's workflow, copied from `.sentinel/ci/fast.yml`. Its `mechanical` job runs the kit's `check.sh`, which fails on cargo deny, clippy, semgrep and any audit output tracked in git, and reports the rest. Its `security-review` job runs on pull requests from this repository and has Claude review the diff against [THREAT-MODEL.md](THREAT-MODEL.md) with read-only tools. This repository is public, so the review is encrypted to the maintainer's key and the pull request comment shows only the number of findings by severity. The job fails only when the review did not run; findings do not fail it. `mechanical` also runs on `main` every Monday, so a new advisory against a dependency already in use shows up without a pull request.

The full list of checks, which ones gate and which only report, is in the kit's README at `.sentinel/README.md`, at the version this repository is pinned to.

## Fixed findings

Every finding that is fixed gets a regression test. The test goes into the test suite of the crate the finding is in, it fails before the fix and passes after, and it stays there so the same bug cannot come back unnoticed.
