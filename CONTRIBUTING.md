# Contributing to Insi

Thank you for helping. Insi measures coding agents, so its own bar is that a
change is shown to work, not assumed to.

## Ways to help

- **Report a bug.** Open an issue with the command you ran, what you
  expected, what happened, and the relevant lines of `results.jsonl` or the
  attempt's folder under `.insi-work/attempts/`. Leave out task definitions
  and hidden tests: they hold the answers.
- **Fix something.** Small pull requests that do one thing are reviewed
  fastest.
- **Teach `insi init` a stack.** Detection lives in `crates/insi/src/init.rs`; a new
  stack needs a test with a repository of that shape.
- **Share a benchmark result.** Insi's claims rest on measurements. A run on
  your own repository, with `insi report`'s output and the conditions (seeds,
  worker, budget), is a welcome addition to the README's results.

## Building and checking

You need Rust 1.89 or newer and git. CI runs these three commands on Windows,
macOS and Linux, and a pull request is merged only when all three pass:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests drive real git repositories and real processes (deadlines, process
trees), so git needs a name and email configured for the tests to commit with.

## How changes are written

- **A change in behaviour comes with a test of that behaviour.** The test
  runs the changed code, asserts what it did, and fails when the change is
  taken back out. Check that last part before opening the pull request.
- **No silenced checks.** Do not add `#[allow(...)]`, skip a test, or loosen
  a type to make CI pass; fix the cause, or say in the pull request why it
  cannot be fixed yet.
- **Root causes, not patches.** If the wrong value comes from another module,
  fix that module rather than correcting the value where it arrives.
- **Stay in scope.** Unrelated renames, reformatting and refactors belong in
  their own pull request.
- **Explain the wiring.** A non-trivial function's doc comment says who calls
  it and what it calls, including other code that does the same job by a
  different route.
- **No secrets and no private data.** Never commit keys, tokens, `.env`
  files, task definitions or hidden tests. `.insi-work/` and private checks
  stay out of the repository.
- **Measurements are not edited by hand.** A benchmark number in the README
  comes from `insi report` and is quoted with its conditions.

## Pull requests

1. Fork the repository and branch from `main`.
2. Make the change, with its test.
3. Run the three commands above.
4. Open the pull request: say what changed, why, and how you verified it.

Branches named `insi/...` are reserved for the pull requests Insi itself opens
(`insi issue`, `insi improve --pr`).

## Releases

Releases are cut by maintainers; see [RELEASING.md](RELEASING.md).

## License

By contributing, you agree that your contributions are licensed under the
[Apache License, Version 2.0](LICENSE), the license of the project (section 5
of the license).
