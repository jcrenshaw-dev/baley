# Security policy

## Reporting a vulnerability

Report security problems privately through GitHub: on this repository, open the **Security** tab and choose **Report a vulnerability**. Do not open a public issue.

Include what you found, how to reproduce it, and what an attacker could do with it. You will get an answer, and credit in the fix if you want it.

## Scope

Baley is under active redesign and has no released version. Reports about the current `main` branch are in scope. Of most interest:

- anything that lets an agent bypass the owner's approval, the recorded evidence, or the branch policy the hook enforces;
- secrets reaching logs, records or the material sent to review providers;
- tampering with records that goes undetected.

## What runs on every pull request

Two workflows run on every pull request.

- `test.yml` holds the required checks: `cargo-test`, `clippy`, `cargo-deny` and `fmt`. A pull request cannot merge into `main` until all four pass.
- `fast.yml` runs the fast checks from [sentinel](https://github.com/jcrenshaw-dev/sentinel), in two jobs. Neither is a required check, so neither blocks a merge, but a failed job shows on the pull request.
  - `mechanical` runs sentinel's `check.sh`. `cargo audit`, `cargo deny`, `cargo clippy` and `semgrep` are gates, and the job fails when one of them fails. `cargo geiger`, a build of the fuzz targets in `crates/baley-core/fuzz` and `crates/baley/fuzz`, and Miri on the unit tests (the code has `unsafe` blocks) are reported only.
  - `security-review` has Claude review the pull request's diff against [THREAT-MODEL.md](THREAT-MODEL.md) with read-only tools. This repository is public, so the review is encrypted to the maintainer's key, and the pull request comment shows only the number of findings by severity. The job fails only when the review did not run or its output is malformed. Findings do not fail it. Pull requests opened by Dependabot skip it, because GitHub gives them no secrets.

## The full audit

The full audit runs on the maintainer's machine through sentinel's `/snt:audit`, never in CI, so findings stay private until they are fixed. It runs in three steps and stops after each one until the maintainer says to go on.

1. **Mechanical checks:** the same `check.sh`, with its output saved to `audit/check.txt`.
2. **Claude Security scan:** one codebase scan of the whole repository.
3. **Findings log:** one row per step: which mechanical checks failed, and how many findings the scan reported at each severity.

The audit finds and records. It fixes no code, writes no tests and commits nothing. What happens to each finding is the maintainer's decision.

## Fixed findings

Every finding that is fixed gets a regression test. The test goes into the test suite of the crate the finding is in, it fails before the fix and passes after, and it stays there so the same bug cannot come back unnoticed.
