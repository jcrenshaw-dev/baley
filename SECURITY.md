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

`test.yml` runs on every pull request and every push to `main`: `cargo-test`, `clippy`, `cargo-deny` and `fmt`. All four checks are required by the `main` ruleset, so a pull request cannot merge until each one passes.

## Fixed findings

Every finding that is fixed gets a regression test. The test goes into the test suite of the crate the finding is in, it fails before the fix and passes after, and it stays there so the same bug cannot come back unnoticed.
