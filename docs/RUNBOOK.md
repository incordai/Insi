## Index

- checks: commands Incord Memory runs after edits
- Operation: how the runner behaves and what it needs

# Insi local checks

## checks

- `python3 scripts/ci/workflow_security.py`
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
generated separately by the CI coverage job without a coverage-service credential.

`python3 scripts/ci/test_all.py` also runs workflow security before the Rust,
Python and npm checks. Install the free scanner in the project scratch environment:

```sh
python3 -m venv .incord/temp/workflow-security-venv
.incord/temp/workflow-security-venv/bin/python -m pip install -r scripts/ci/requirements.txt
```

On Windows, use the virtual environment's `Scripts/python.exe`. The wrapper
requires the same Zizmor version and `.github` scope as CI, including online
audits. It uses `GH_TOKEN`, `GITHUB_TOKEN`, `ZIZMOR_GITHUB_TOKEN` or the existing
`gh` login without printing or storing the token. Missing tools, credentials,
network failures and disabled online audits cannot count as a pass. These audits
need network access and GitHub authentication; they add no paid API or service.
Local checks do not reproduce every OS matrix, release job or hosted CI environment.

See [Incord integration and capability overlap](INCORD-INTEGRATION.md) for the
existing memory adapter, ownership boundaries and the proposed connection for
code suggestion results.
