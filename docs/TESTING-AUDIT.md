# Testing audit — 2026-11-06

## Integration update

[PR #40](https://github.com/incordai/Insi/pull/40) integrates PR #38's fixes
with the Codecov removal and doctor fixture correction. The sections below
record the earlier audit; fixes are now combined on `fix/free-test-automation`,
pending review and main-branch integration. RUNBOOK lists individual standard
commands because the engine held the custom Python wrapper for guard review.
The sibling engine has no Git remote in its current checkout, so its tested
Python fallback change remains local and is excluded from the Insi PR.

## Status

GitHub issues are status reports, not proof of passing tests. Local checkout:
`daef1c1`. [PR #38](https://github.com/incordai/Insi/pull/38) remains open,
with passing CI, dependency and semver workflows. Its changes are not yet in
this checkout. Rebase this work after that PR merges.

Closed fixes include #5 (answer isolation), #24 (process output and descendants),
#25 (pristine setup cache), #27 (portable long prompt regression), and #28
(Python extraction support). Engineering work #8–22 adds MSRV, locked builds,
dependency policy, workflow security, parser properties/fuzz targets, linting,
API compatibility, docs, spelling, contribution forms and diagnostics.
Closure alone does not establish cross-platform validation.

## Remaining work

Follow-up review: `docs/RUNBOOK.md` now connects Insi to engine checks.
Doctor fixtures collided under parallel execution because clock nanoseconds
were their only discriminator; an atomic sequence now separates them. The
lock-release regression also reproduced; explicit probe and owner unlocks
correct it, matching PR #38. All seven local check groups now pass.
Actionlint and the whitespace check also pass.
The doctor tests also pass 20 repeated parallel runs. The PR checkout's eight
Python packaging tests and seven CI configuration tests pass locally.

Review of PR #38 confirms checked seed conversion, explicit guard unlock,
Windows junction regressions, synchronized installer concurrency tests and
publication-failure cleanup tests. Its three-platform CI passes, but the PR
remains unmerged. The sibling engine now tries Python 3 before Python for lone
scripts and has a regression test. That source fix needs a rebuild/restart;
the running engine has not been replaced.
The engine compiled successfully and its new Python fallback regression passed
(one selected test; the full engine suite was not run).
The isolated PR #38 Rust workspace suite also passed with its locked dependencies.
The parallel doctor fixture collision found here is an additional correction
outside that PR. No GitHub issues were closed or PRs merged in this review.

The [open issues](https://github.com/incordai/Insi/issues) are #26 (hidden-test
symlink traversal), #29 (concurrent Python installation), #36 (seed count
validation), #37 (Python CI and dependency audit scope), and #39 (lock release).
PR #38 proposes fixes and regressions for all five; finish review and merge,
then verify the resulting main-branch checks before closing the audit.

Other gaps: fuzz targets exist but continuous fuzz execution is not established
by the CI workflow; release compatibility needs published baselines; sandbox
protection requires explicit worker configuration. README worker examples can
use paid models. Insi does not supply a free model: configure your own local
noninteractive worker to avoid model charges. Tests use fixtures and processes.

## Coverage without a service

Codecov was an optional CI uploader and badge, not a runtime dependency or test
runner. This change removes both upload actions, their OIDC permission, badge
and service configuration. Existing free tools run tests: Cargo, nextest and
[cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov).
CI now puts a coverage summary in the job summary and preserves HTML, LCOV and
JUnit as artifacts. No coverage account, token or paid API is required.
Historical Codecov patch comparisons, per-crate thresholds and analytics are
not replicated by these reports. Configure branch protection to remove required
Codecov checks if present; repository settings were not changed in this audit.

## Reproduce

Local validation: all seven groups in `test_all.py` passed, including formatting,
Clippy, Rust workspace tests and the Python/Node suites. `actionlint` and
`git diff --check` passed. An earlier Rust run failed in
`doctor::tests::executable_worker_is_detected_without_running_it` with a missing
file; the full rerun passed. Investigate this intermittent fixture failure.
Coverage generation was not run locally because cargo-llvm-cov is not installed;
the revised coverage workflow still needs a GitHub run.

Run `python3 scripts/ci/test_all.py` for formatting, linting, Rust tests,
Python tests and npm wrapper tests. Python 3.9+, Node and stable Rust with
rustfmt/clippy are prerequisites. No Python packages are installed.

For coverage install the open-source cargo-llvm-cov and cargo-nextest tools,
add `llvm-tools-preview` through rustup, then run:

```sh
cargo llvm-cov nextest --workspace --all-features --locked --profile ci --lcov --output-path lcov.info
cargo llvm-cov report --summary-only
cargo llvm-cov report --html
```

GitHub-hosted CI remains subject to GitHub runner/storage allowances; local
execution requires no hosted service. Coverage reports do not enforce a numeric
gate. No release or merge is performed by this work.
