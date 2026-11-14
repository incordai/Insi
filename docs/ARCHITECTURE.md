## Index
- layout: the folder tree, one line per folder (generated from .incord/layout.toml)

- workspace: the crates and what each owns
- review-flow: how a PR review runs and publishes
- boundaries: trust boundaries

## workspace

Cargo workspace (`Cargo.toml`), resolver 2, Rust 1.89:

- `insi-core`: shared types: tasks, budgets, results, lessons, per-repo config,
  secret scanning (`secrets.rs`).
- `insi-eval`: benchmark: tasks mined from history, throwaway worktrees, hidden
  checks, process control.
- `insi-memory`: lessons written from verified fixes, stored, briefed; Incord
  adapter (`incord.rs`).
- `insi-loop`: improvement loop: proposals, accept rule, strategy bandit, audit
  trail.
- `insi-connectors`: outside sources: docs, CI logs, command connectors.
- `insi`: the binary: subcommands in `main.rs`; review (`review.rs`), suggestions
  (`suggest.rs`, `benchmark.rs`), issues (`issue.rs`), init (`init.rs`).

## review-flow

`.github/workflows/insi-review.yml` has two jobs. `review` (read-only token)
checks out the BASE sha, builds `insi`, and runs `insi review` against the PR
head in a pinned image, uploading `report.json`. `publish` (write token) runs
`insi review-publish` on that report; it runs no PR code. Both jobs run only
when repository variable `INSI_AUTO_REVIEW` is `'true'`.

## boundaries

- The review plan (`.insi/review.json`) is read from the trusted base.
- `report.json` is produced after PR code ran, so it is untrusted input to
  `review::publish` (`review.rs`).
- Everything published passes `insi_core::secrets` first.

## Layout
Generated from `.incord/layout.toml` by `inmem layout render --write`. Do not edit it by hand: change the layout file, get it approved, and render again.

```text
(root)                   root files allowed: Cargo.toml, Cargo.lock, README.md, LICENSE, LICENSE-MIT, LICENSE-APACHE, rust-toolchain.toml, .gitignore
.github/                 Trusted CI workflows and repository automation.
.incord/                 Local engine configuration and ignored scratch artifacts.
.insi/                   Trusted review and security benchmark example plans.
crates/insi/             CLI commands, review scanner adapters, security policy and orchestration. (may use: crates/insi-core, crates/insi-connectors, crates/insi-eval, crates/insi-loop, crates/insi-memory)
crates/insi-connectors/  External context sources and command connectors.
crates/insi-core/        Shared task, config, usage, trust and secret-scanning types. [core: names no provider]
crates/insi-eval/        History mining, isolated attempts, process control and benchmark execution.
crates/insi-loop/        Harness proposals, acceptance, strategy selection and audit trail. (may use: crates/insi-core, crates/insi-eval)
crates/insi-memory/      Lesson persistence, briefing, extraction and Incord integration. (may use: crates/insi-core)
docs/                    The project documents: PRD, ROADMAP, ARCHITECTURE and the rest.
fuzz/                    Parser fuzz targets and corpus metadata.
packaging/               Python and npm distribution wrappers and their tests.
scripts/                 CI checks, scanner wrappers, review scripts and reproducible security labs.
```
