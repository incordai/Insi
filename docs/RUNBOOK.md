# Insi local checks

## checks

- `python3 scripts/ci/test_all.py`

## Operation

Incord Memory reads the checks section after edits and runs the repository's
commands. The runner prints a summary and returns failure if any check fails.
Use Python 3, Node and Rust with rustfmt and Clippy installed. Coverage is
generated separately by the CI coverage job; no service credentials are needed.
