## Index

- purpose: what Insi is for and who uses it
- users: who runs it
- requirements: what it must do
- non-goals: what it deliberately does not do
- must never: lines it never crosses

## purpose

Insi is one Rust command-line binary added to a git repository. It turns the
repository's history into a benchmark, runs a coding agent against it in
throwaway checkouts, judges attempts with hidden tests, and proposes
improvements as draft pull requests that a person reviews (README.md).

## users

Maintainers of a repository who want evidence that their coding agent improves
on their own code, and who decide what is merged.

## requirements

- Mine tasks from fix commits that changed a test; validate each task with two
  agent-free controls before use (README.md "How it works").
- Keep an improvement only if it beats a control on tasks it was not learned
  from.
- Review pull requests evidence-first: tools come only from the operator's plan
  (`crates/insi/src/review.rs` header; `docs/REVIEW.md`).
- Code suggestions are gated by a project benchmark suite (`docs/SUGGESTIONS.md`,
  `docs/BENCHMARKS.md`).

## non-goals

- No automatic approval or merge of any pull request (the review comment says
  "No automatic approval or merge", `review.rs` `review_payload`).
- No HTTP client, async runtime or argument parser in phase 1 (`Cargo.toml`
  dependency comment).
- Not a hosted service; it runs where the operator runs it.

## must never

- Never change the repository silently: improvements arrive as draft PRs.
- Never publish text containing a credential or personal data
  (`insi_core::secrets`, applied before publishing in `review.rs`).
- Never run a pull request's own workflow or configuration with a write token
  (`.github/workflows/insi-review.yml` builds only trusted base code).
- Never let the agent that proposes a change set the bar that change must clear.
