# Insi local checks

## checks

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --locked`
- `python3 -m unittest discover -s packaging/pip -p 'test_*.py'`
- `python3 -m unittest discover -s scripts/checks -p 'test_*.py'`
- `python3 -m unittest discover -s scripts/ci -p 'test_*.py'`
- `node packaging/npm/test.js`

## Operation

Incord Memory reads the checks section after edits and runs the repository's
commands. The runner prints a summary and returns failure if any check fails.
Use Python 3, Node and Rust with rustfmt and Clippy installed. Coverage is
generated separately by the CI coverage job; no service credentials are needed.
