# Contributing to Insi

Thank you for helping. Insi measures coding agents, so its own bar is that a
change is shown to work, not assumed to.

## Ways to help

Questions, ideas and examples belong in
[Discussions](https://github.com/incordai/Insi/discussions). Use the issue forms
for reproducible bugs and scoped changes; report vulnerabilities privately.

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
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

The tests drive real git repositories and real processes (deadlines, process
trees), so git needs a name and email configured for the tests to commit with.

CI also checks all targets on Rust 1.89.0, the declared minimum version:

```sh
cargo +1.89.0 check --workspace --all-targets --locked
```

Dependency policy is in `deny.toml`. After installing `cargo-deny`, run
`cargo deny --all-features --locked check`. CI runs it on pull requests,
main-branch pushes, and weekly so newly published advisories are checked.
New dependencies must satisfy its license and source policy; advisory
exceptions require an explicit reason and review.

Maintainers: keep Dependabot security alerts enabled. After these files land,
configure `main` to require CODEOWNERS approval, `test (windows-latest)`,
`test (macos-latest)`, `test (ubuntu-latest)`, `msrv (Rust 1.89)`,
`packaging (Python 3.9)`, `packaging (Python 3.12)`, and `dependency policy`.
Repository settings are separate from the files and must be verified on GitHub.

## How changes are written

CI builds public documentation with warnings denied and runs doctests:

```sh
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo test --doc --workspace --locked
```

Additional checks are `typos --hidden`, `zizmor --offline .github/workflows`,
and `actionlint`. Intentional project names are in `_typos.toml`. Actions
are pinned to full commit hashes with version comments for Dependabot.

The local monitor in `.incord/watch.toml` checks optional evaluation records,
the audit schedule configuration, and disk space. No persistent Insi server
or deployed host is expected. Run its tests with
`python3 -m unittest discover -s scripts/checks -p 'test_*.py'`.

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

Parser properties run with the regular Rust tests; see [fuzz/README.md](fuzz/README.md)
for bounded sanitizer runs. Production `unwrap`, `expect` and explicit panics
are Clippy warnings, denied in CI. Test fixtures have a documented test-only
exception so setup failures remain clear.

The coverage job emits LCOV and nextest JUnit reports and preserves both as
workflow artifacts even when tests fail. `codecov.yml` tracks each crate;
core/loop targets are 95%/93%, set below their initial local line-coverage
measurements (97.0%/95.5% over 141 passing tests). Confirm these targets against
the first Linux CI baseline. Other crates use their measured baseline.
To activate Codecov reporting, install/authorize the Codecov GitHub App for this
repository and set repository variable `CODECOV_ENABLED=true`. Uploads use
GitHub OIDC, so no long-lived `CODECOV_TOKEN` secret is required. Keep uploads
disabled until the account is connected; report generation still runs.

Releases are cut by maintainers; see [RELEASING.md](RELEASING.md).

## License

By contributing, you agree that your contributions are licensed under the
[Apache License, Version 2.0](LICENSE), the license of the project (section 5
of the license).
