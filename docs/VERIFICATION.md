## Index

- local: commands to run before pushing
- ci: what CI runs
- evidence: what counts as tested

## local

From `docs/RUNBOOK.md`:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --locked`
- `python3 -m unittest discover -s packaging/pip -p 'test_*.py'`
- `python3 -m unittest discover -s scripts/checks -p 'test_*.py'`
- `python3 -m unittest discover -s scripts/ci -p 'test_*.py'`
- `node packaging/npm/test.js`

## ci

On a PR: tests on ubuntu, macOS and Windows; MSRV (Rust 1.89); coverage;
documentation; spelling; dependency policy; workflow security; API
compatibility per crate; Python packaging; artifact round trip.

## evidence

A green suite shows the build works, not that a change works. A fix is verified
by a test that fails without it and passes with it.
