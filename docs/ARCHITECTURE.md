## Index

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
