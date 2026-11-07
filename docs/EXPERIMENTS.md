# Component experiment assessment

`insi experiment --plan experiment.json` assesses existing evaluation records.
It prints a JSON report to stdout. It runs no worker, installs nothing, and
needs no paid API or service. Failure evidence can point to existing Incord
graph nodes; this command creates no memory store.

This is an assessment layer alongside `eval` and `improve`. It does not
automatically collect rollouts, change the improvement loop's acceptance
policy, or independently authenticate imported execution history.

For automatic collection and benchmark-gated code improvement PRs, use
[`insi improve --code` / `insi suggest`](SUGGESTIONS.md). That workflow
measures a proposed code change's specific workload; this assessment command
handles recorded component experiments and unseen-repository claims.

## Plan

Paths are relative to the plan file. Each run's JSONL file uses the existing
`EvalResult` format. Keep exactly one worker record per task/seed in each
file; put reruns in separate files. Different conditions must be separate
runs. Include failed attempts, rather than silently dropping them.

```json
{
  "version": 1,
  "selection_repositories": ["github.com/example/training"],
  "tasks": {
    "task-a": {"repository": "github.com/example/unseen", "commit": "immutable-commit-id"},
    "task-b": {"repository": "github.com/example/unseen", "commit": "immutable-commit-id"}
  },
  "runs": {
    "baseline": {
      "results": "baseline.jsonl",
      "model": "fixed-model-version",
      "worker_version": "worker-commit-id",
      "environment": "container-image-digest",
      "tool_versions": {"git": "2.49.0"},
      "budget_secs": 1800,
      "components": {"prompt": "baseline-content-id", "tools": "unchanged-content-id"},
      "model_seed_supported": false
    },
    "prompt-change": {
      "results": "prompt-change.jsonl",
      "model": "fixed-model-version",
      "worker_version": "worker-commit-id",
      "environment": "container-image-digest",
      "tool_versions": {"git": "2.49.0"},
      "budget_secs": 1800,
      "components": {"prompt": "candidate-content-id", "tools": "unchanged-content-id"},
      "model_seed_supported": false,
      "labels": [
        {"task": "task-a", "seed": 0, "category": "tooling", "evidence": "incord:existing-evidence-node"}
      ]
    }
  },
  "comparisons": [
    {"baseline": "baseline", "candidate": "prompt-change", "component": "prompt"}
  ],
  "planned_comparisons": 1,
  "min_tasks": 10,
  "min_seeds": 3,
  "margin": 0.05,
  "final_set_untouched": true
}
```

Expand this illustrative two-task plan to at least ten tasks. The sample
label is valid only when that recorded attempt failed. Choose sample sizes
before collecting outcomes; ten tasks and three seeds are starting points,
not a guarantee of statistical power. Seed labels do not imply the model
provider supports deterministic seeds.

## What is checked

- Failure classification: optional, multiple evidence-linked hypotheses per
  failed attempt. Categories are `reasoning`, `retrieval`, `tooling`,
  `environment`, `orchestration`, and `unknown`. Errors, timeouts and artifact
  locations are reported separately as observations. Missing labels remain
  unknown; a failed check is not automatically a reasoning failure.
- Attribution: each contrast must change exactly one declared component.
  Model, worker version, tools, environment and budget must match. Add
  reversed contrasts to test removal and separate contrasts under different
  component configurations to investigate interactions. Include all planned
  contrasts in `planned_comparisons`, including those assessed elsewhere.
- Generalization: final repositories must be disjoint from selection
  repositories. Use canonical names consistently. Pin commits. Freeze the
  candidate before final evaluation, attest `final_set_untouched`, and never
  use final outcomes to select the next candidate. Repeatedly consulted
  holdouts are selection sets. Cross-repository transfer requires multiple
  final repositories; results from one repository support only that setting.
  An empty `selection_repositories` list permits a diagnostic report but
  always blocks `supported: true`; unknown history is never invented.
- Statistics: exact task/seed alignment, minimum paired task and seed counts,
  existing task-level bootstrap comparison and regression checks, plus a
  one-sided exact sign test over task-level changes with Bonferroni control
  across the predeclared comparison family. Tied tasks are excluded from the
  sign test. Its conclusion concerns the direction of task effects, alongside
  the existing mean-effect gate. Bootstrap intervals remain descriptive.
- Reproducibility: model, worker, environment, tool versions, budget, task
  commits and component identifiers appear in the report. These are
  producer declarations, not independently verified snapshots. Archive the
  plan and immutable artifacts together. Missing historical metadata must
  not be reconstructed as if observed.
- Reliability and efficiency: overall completion includes infrastructure
  errors; judged pass rate excludes them. Reports also include errors,
  timeouts, elapsed attempt time, successes per summed attempt-hour, tokens
  and token coverage, public/hidden check disagreements, and task regressions.
  Summed attempt time differs from elapsed batch time under concurrency.
- Evaluator integrity: each task needs at least two successful reference
  controls and two failing base controls per run. Missing, errored or unstable
  controls block support. Workers need recorded sandbox isolation and
  base-only history. Stable controls reduce concern about flakiness; they
  cannot prove its absence or independently prove isolation.

`supported: false` is a valid report, not a command error. Invalid input or
confounded comparisons return a nonzero exit status. Assessment does not
deploy or promote a harness. Existing Incord tooling can store the report
and link it to the corresponding edits and evidence.

## Validation

Regression tests cover controlled improvement, repeated-comparison penalties,
untouched-set declarations, component and environment confounding, duplicate
records, failure evidence, infrastructure denominators and unstable controls.
No AI worker is required for these tests. Codecov's project page was checked
on 2026-11-06 but was inaccessible through the browsing tool; no Codecov
verification is claimed.
