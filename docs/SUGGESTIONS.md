# Improvements to working code

```sh
insi improve --code --repo . --samples 10 --margin 0.05
# Equivalent:
insi suggest --repo . --samples 10 --margin 0.05
```

Insi suggests an optional improvement, creates its benchmark, measures the
unchanged code, implements in isolation, and opens a **draft** PR only for a
measured gain with passing regression checks. It never merges or replaces the
operator's checkout. The existing `improve --harness` mode still optimizes an
agent harness; `improve --code` improves application code.

The repository needs a clean committed checkout and `test_cmd` configured in
`.insi/config.toml`. The current branch becomes the PR base. Use your existing
local or subscription-authenticated worker; no paid API or service is added.
All execution uses the configured sandbox when present. Clones isolate Git
checkouts, but do not restrict host access without a sandbox.

## Execution

1. Create independent baseline and candidate clones under
   `<work>/suggestions/<run>/` at the current branch's commit.
2. Ask the proposer (default: configured worker) to inspect working code and
   return one suggestion plus benchmark source. Discovery must leave tracked
   source and history unchanged.
3. Install new benchmark files in both clones and commit them **before**
   requesting implementation. Existing benchmark files cannot be overwritten.
4. Run setup and baseline regressions. Verify the frozen benchmark works on
   the unchanged code and save its initial result.
5. Ask the worker to implement the suggestion, preserve behavior and existing
   tests, and add useful regression tests. Failed/timed-out workers, benchmark
   edits, history edits or regression configuration changes block publication.
6. Run candidate regressions, then collect matched baseline/candidate samples
   in alternating order. Default ten pairs; minimum five, maximum 1,000.
7. Require positive mean relative gain at least `--margin` (default 5%), a
   descriptive paired-bootstrap 95% interval wholly above zero, and a
   one-sided paired sign-test p-value at most 0.05. Ties are excluded from
   sign-test wins. Invalid or failed measurements prevent publication.
8. Include frozen benchmark source, raw samples, commits and regression
   command in the candidate. Scan the entire diff for credential/personal-data
   patterns. If the operator's source branch moved, keep the result local.
   Before publishing, also require the remote PR base to still match the
   measured baseline commit; otherwise update the checkout and rerun.
9. Push only a qualifying candidate to a new `insi/suggest-*` branch. Open a
   draft PR through existing `gh` authentication with the suggestion, measured
   benefit, workload, units, uncertainty and regression evidence. You merge.

No gain means no push and no PR. Passing tests alone is insufficient. Failed
runs retain local evidence. If PR creation fails after a successful push, the
qualified remote branch remains; no automatic retry or merge is attempted.

## Options and proposal format

```sh
insi suggest --repo . --suggestion suggestion.json --local
insi suggest --repo . --worker 'my-worker {prompt}' --proposer 'my-proposer {prompt}'
```

`--local` performs the full evaluation but never pushes or requests a PR.
Qualifying results are `improved-local`. Without it, qualifying results request
a draft PR automatically. Worker overrides follow existing argv parsing; use
configuration argv arrays for arguments containing spaces.

`--benchmarks <suite-json>` adds required project workloads and regression
guardrails to the improvement gate. The suite is frozen before implementation;
a faster primary metric cannot override a failing secondary check. Discovery
can also return an optional `suite` object using the same format; an explicit
operator suite takes precedence. See [project benchmark suites](BENCHMARKS.md)
for detection, measurement methods, plan format and evidence limits.

`--to-memory` retrieves bounded Incord context before discovery and sends a
concise outcome with measurement references to the existing local engine.
Unavailable memory does not change the experiment or publication gate. Pending
delivery can be retried with `insi suggest --retry-memory <run-directory>`;
that command invokes no worker and publishes no PR. See the
[integration guide](INCORD-INTEGRATION.md) for delivery artifacts and ownership.

`--suggestion` skips discovery and loads this format:

```json
{
  "title": "Reduce allocations in the parser",
  "rationale": "Reuse the existing buffer while preserving parsed output.",
  "metric": "parser latency on representative inputs",
  "unit": "seconds",
  "direction": "lower",
  "benchmark": ["python3", ".insi-bench/parser-v2.py"],
  "files": {
    ".insi-bench/parser-v2.py": "# Supply complete runnable benchmark source here\n"
  }
}
```

The example source is a placeholder. Supply a real benchmark that checks
equivalent behavior and ends stdout with JSON, such as `{"value": 0.042}`.
Values must be positive and finite. Use `higher` for throughput or similar
metrics. Benchmark argv must reference a supplied file. Files must be new
relative `.insi-bench/` paths, cannot traverse symlinks, and total at most
1 MiB. `.insi-bench/evidence.json` is reserved for the run's evidence.

## Evidence and interpretation

Each run retains `suggestion.json`, initial baseline value, raw samples,
command logs, `benchmark.json`, `state.json`, `report.json`, and both clones.
The PR contains `.insi-bench/evidence.json`. Existing Incord tools can link
these artifacts to code and findings. There is no additional memory store.

The gate measures this workload. It cannot prove that a generated benchmark
is representative or honest, or that regression tests exhaustively establish
equivalence. Review the benchmark and code diff. Samples are repetitions of
one workload, not unseen tasks or repositories. Their uncertainty estimates
assume suitable independence. Repeatedly selecting suggestions against the
same benchmark introduces selection bias. Use [experiment assessment](EXPERIMENTS.md)
for predeclared comparison families and unseen-repository evaluation.

## Validation

Tests use real isolated Git repositories and a bare test remote, controlled
command workers, and a test hosting interface. They cover discovery, qualifying
draft publication with evidence, unchanged source trees, no gain, regressions,
failed workers, tampered benchmarks and invalid measurements. Git/process
integration tests run on Unix; proposal and numeric checks also run on Windows.

An end-to-end run on actual Insi `signature.rs` used a scripted empty-input
fast path, preserved existing tests, and measured about 84% lower latency on
three million empty-input calls over five pairs. It used `--local`; no live
PR was created. This verifies orchestration and that workload, not autonomous
discovery or general application performance.

Memory delivery was also verified against the running local Incord engine using
a worker-free saved decision. Replaying the same request after losing its local
receipt returned the same item ID with `duplicate: true`. This verifies delivery
and retry handling, not autonomous suggestion quality.
