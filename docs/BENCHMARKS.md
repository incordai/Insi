# Project benchmark suites

Insi supplies measurement methods. Each project supplies representative inputs,
commands and correctness checks. An API, compiler and frontend cannot share one
meaningful workload. Any project that can run a command or report a numeric
measurement can use this interface; meaningful coverage still requires project
configuration and review.

## Commands

```sh
insi benchmark --detect --repo .
insi benchmark --plan benchmarks.json --repo . --output .incord/temp/baseline-run
insi benchmark --plan benchmarks.json --repo ../baseline --candidate ../candidate --output .incord/temp/comparison-run
insi improve --code --benchmarks benchmarks.json --local
```

Detection reads root markers for Rust, JavaScript, Python, Go, Maven, Gradle and
CMake projects. It prints suggested test/build commands and
`needs_configuration`. It does not run commands, inspect every monorepo package,
install tools or certify that suggested scripts exist. Pass a package directory
to `--repo` for a monorepo component. Other stacks use the same explicit plans.

The standalone command runs the commands you configured in the supplied
checkout(s), with the baseline's configured sandbox when present. It does not
clone, edit source through an agent, open PRs or require a worker. Build/test
commands can create or modify files. Output must be a new directory whose parent
already exists; evidence is never silently overwritten. A nonqualifying
comparison or command failure returns a nonzero exit status and retains evidence.
A baseline-only report makes no improvement claim.
Check-only or secondary-only plans can run diagnostics without declaring a
primary metric. Comparisons of such plans always return `qualified: false`;
they cannot qualify a code-improvement PR.

`improve --code --benchmarks` adds the suite to the existing suggestion workflow.
Insi reads it before discovery, freezes a copy in `.insi-bench/` alongside the
benchmark source, and checks the baseline before implementation. After candidate
regressions, it runs the required suite on both clones. Every suite workload and
the existing suggestion benchmark must qualify before a draft PR can be created.
Tampering or failed measurements blocks publication. A qualifying PR includes
the frozen plan and suite evidence under unique run-specific names; earlier
benchmark files remain intact. Auditor/fixer delegation is a separate opt-in
[candidate execution mode](INCORD-CANDIDATE-API.md), not a permission granted by
passing a suite.
Discovery can return the same plan as an optional `suite` field in its suggestion
JSON. Generated suites must pass the same validation and gate. An explicit
`--benchmarks` plan takes precedence over a generated suite.

## Measurement methods

| `kind` | Insi measures | Required configuration |
|---|---|---|
| `wall_time` | Successful process wall time in seconds, including process/sandbox overhead but excluding prepare/verify and log writing | Command, verification; `direction: lower` |
| `throughput` | Declared number of completed operations divided by successful process wall time | Command that performs exactly `operations`, verification; `direction: higher` |
| `artifact_bytes` | Total bytes of listed regular files/directories after the optional command | Artifact paths, verification; `direction: lower` |
| `json` | Numeric `value` in the last nonblank stdout line | Project measurement command and verification; either direction and explicit unit |
| `check` | Successful command exit; failed command blocks the suite | Command; cannot be the primary improvement target |

Built-in units are enforced: `seconds`, `operations/second`, `bytes` and `pass`,
respectively. JSON adapters declare the units of their own measured values.

The time measurement reuses Insi's process runner. Aggregate enough operations
to make process startup, polling resolution and timer noise small compared with
the work being measured. Throughput is not a concurrency/load generator: the
project command must perform the declared workload and verification must check
completion. For long-running services, use a bounded load-test command that exits
and reports its measurements, rather than timing an indefinitely running server.

Artifact paths must stay relative to the checkout. Missing artifacts, symlinks,
Windows reparse points and overlapping paths are refused. Size is an uncompressed
byte count; use a JSON adapter for compressed transfer size or another definition.
Custom metrics must be finite and nonnegative. Relative comparisons require
positive baseline values; a zero baseline is an error, never a passing result.

## Workload catalogue

These are configuration patterns, not claims that Insi implements each domain's
measurement tool. Use existing local tools; Insi adds no paid service dependency.

| Project concern | Method and workload |
|---|---|
| Build, startup, compiler, CLI or batch speed | `wall_time`, with fixed input and declared cache preparation |
| Parser, serialization, compression, compute kernel | Repeated operation command; time/throughput plus output equivalence checks |
| API, networking, database or queue | JSON adapter reporting latency percentile, requests/s, queries/s or lag against a fixed dataset and load |
| Memory, CPU, disk I/O, allocations, energy | JSON adapter around a locally available profiler; explicit units and process scope |
| Frontend delivery | `artifact_bytes` for bundles; JSON adapter for browser timing, layout shift or compressed size |
| Reliability and correctness | `check` for unit/integration/property/compatibility/accessibility tests; JSON adapter for a defined rate or score |
| Coverage, complexity, dependency counts | JSON adapter for the project's existing tool; retain behavior checks and review against metric gaming |
| Binary/model/package size | Artifact sizes or a project-defined JSON metric |
| New language or domain | Command plus verification and a JSON metric; no new Insi language plugin required |

No generic numerical metric proves readability, maintainability, security or
correctness. These still need appropriate tests and review. Do not invent a
performance claim for a qualitative change. A suite with only pass/fail checks
cannot qualify an improvement PR under this measured-improvement policy.

## Plan format

This example expects the project's two Python workload scripts to exist. It is
not runnable until the project supplies those scripts and its representative
inputs. For generated suggestions, put measurement/verification drivers in the
suggestion's frozen `.insi-bench/` files; avoid mutable benchmark drivers mixed
into the implementation under test.

```json
{
  "version": 1,
  "samples": 10,
  "warmup": 1,
  "workloads": [
    {
      "id": "api-p95",
      "category": "latency",
      "kind": "json",
      "command": ["python3", ".insi-bench/api_latency.py"],
      "verify": ["python3", ".insi-bench/api_correctness.py"],
      "unit": "milliseconds",
      "direction": "lower",
      "primary": true,
      "min_gain": 0.05,
      "timeout_secs": 300
    },
    {
      "id": "bundle-size",
      "category": "frontend",
      "kind": "artifact_bytes",
      "command": ["npm", "run", "build"],
      "verify": ["npm", "test"],
      "artifacts": ["dist"],
      "unit": "bytes",
      "direction": "lower",
      "max_regression": 0.01
    }
  ]
}
```

JSON adapters end stdout with, for example, `{"value": 12.5}`. `category` is a
descriptive label, not a request to launch a built-in database/browser profiler.
Commands are argv arrays. Optional `prepare` runs before each measured invocation
and is excluded from time. Use it to reset fixtures/cache when the workload needs
that contract; no implicit cold-cache guarantee is made. `verify` runs after each
successful measurement and is excluded from time. Every numeric workload needs
verification. The user must ensure that it tests equivalent behavior and that
baseline/candidate use equivalent data, dependencies and environment.

Limits: version 1, at most one declared numeric primary target (required for
improvement qualification), 1–32 unique workloads,
5–1,000 sample pairs, 0–20 warmups per side, 1–3,600 seconds per command. A suite
can still take substantial time: each workload repeats prepare, measurement and
verification. `min_gain` and `max_regression` are finite fractions in `[0,1)`;
their defaults are zero. Project plans should declare meaningful thresholds.

## Qualification and evidence

Baseline/candidate order alternates per pair. The primary requires positive mean
gain meeting `min_gain`, a positive descriptive bootstrap interval, and the
existing one-sided sign-test threshold of 0.05. Declaring one primary prevents
selecting a winning metric after seeing results. Secondary numeric guardrails
require the bootstrap lower gain bound to be at least `-max_regression`; checks
must succeed throughout. No configured workload is silently skipped. A missing
profiler, unsupported command, correctness failure, timeout or invalid metric
fails the run.

Evidence includes the plan, command logs, per-pair sample files, workload progress,
results and host OS/architecture. Environment equality and representative data
remain the operator's responsibility; OS/architecture metadata cannot establish
them. Bootstrap intervals are descriptive and are not proof of equivalence.
Repeated samples measure one workload, not unseen repositories. Repeatedly tuning
against the same suite introduces selection bias; use the separate
[experiment assessment](EXPERIMENTS.md) for declared comparison families and
held-out task/repository claims.

## Validation

Nine added regressions cover plan validation, stack hints, artifact boundaries,
measurement methods, diagnostic-only suites, primary/secondary qualification,
generated suites, baseline-before-edit ordering and frozen-suite tampering.
The CLI was also run on Insi itself: detection identified Rust, and a baseline
workload ran the four existing signature tests plus a separate verification run.
Evidence: `.incord/temp/real-project-benchmark-1794061396199136000/`.
This was a baseline check, not a performance-improvement claim. No AI worker was
invoked. Git/process integration tests use controlled command workers on Unix;
portable validation and artifact tests also compile for Windows.
