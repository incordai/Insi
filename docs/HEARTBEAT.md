## Index

- scheduling: run a development cycle on a timer
- operation: state, recovery and limits
- research: Insi chooses questions, Watchman gathers evidence
- verification: tested paths and evidence limits

## scheduling

```sh
insi heartbeat --repo . --interval-secs 3600
insi heartbeat --repo . --status
insi heartbeat --repo . --queue
```

The first check is immediate, then each check happens one interval after the
previous cycle finishes. Default interval: 3600 seconds; allowed: 1..604800.
`--max-cycles 2` exits after two attempted cycles; the default is unlimited.
Ctrl-C stops the foreground process. `--once` checks the saved deadline,
runs at most one due cycle and exits, suitable for an external OS scheduler.
This command does not install a service or wake a sleeping computer.

Each cycle observes configured tests, project documents and connectors, recent
GitHub CI results and open issues. Remote data is untrusted advice; unavailable
remote sources are recorded explicitly. `--offline` skips GitHub reads and shared research (configured
connectors retain their own behavior). Commands run in an isolated observation
checkout under the configured sandbox. Failed observations are retained as input
for planning rather than silently discarded.

When the backlog has no pending task for the current baseline, the configured
proposer researches the code and creates up to five bounded proposals. Each has
a stable id, priority 0..100, justification and frozen success checks. The highest
priority pending task is selected, with id as a stable tie breaker. Existing ids
and titles on the same baseline are deduplicated, including terminal tasks.

Each selected task uses [suggest](SUGGESTIONS.md): frozen benchmarks or behavior
acceptance checks, project fit and feasibility justification, isolated
implementation, regression checks, security checks and measured acceptance. Suggestion
options including `--worker`, `--benchmarks`, `--suggestion` and `--to-memory`
apply. Existing workers and sandbox settings are reused, with no new paid service.
Direct `suggest --audit-finding` execution is a separate engine workflow;
heartbeat rejects that option rather than silently bypassing its approval path.
Results stay local unless `--publish` allows qualified draft PRs. No automatic
merge occurs. Explicit `--local` still takes precedence over `--publish`.

## operation

Autonomous operation requires a clean committed repository, configured `test_cmd`
and worker, plus a trusted `.insi/heartbeat.json` (or `--plan <path>`). For example:

```json
{
  "security_checks": [["python3", "scripts/security/run_checks.py"]],
  "observation_checks": [],
  "max_tasks": 5
}
```

Choose real security commands appropriate to the project; the command above is
only appropriate for a repository providing that script. At least one security
check is required and must pass before delivery. Only the operator's plan sets
these commands; issue text and the planner cannot replace them. `--suggestion`
can supply one proposal instead of invoking the planner, but uses the same queue
and gates. A batch is bounded to 1..10 proposals by `max_tasks`.

Performance tasks use the existing statistical benchmark gate. Behavior tasks
declare `goal: "behavior"`, `expected_behavior`, `acceptance` argv arrays and new
test sources in `files` under `.insi-bench/`. Every frozen acceptance check must
exit exactly 1 on the baseline and 0 on the candidate. Timeouts or infrastructure
exit codes cannot count as evidence of missing behavior. Existing regressions
may fail on a behavior baseline, but must all pass on the candidate. Benchmark
and test sources are frozen before implementation and checked for tampering.
These are task-specific controls, not proof that model-generated tests cover
all requirements. Review their quality alongside the implementation.

State and an OS lock live under `insi-heartbeat/` in the Git common directory.
Linked worktrees share the lock regardless of `--work`. Process exit releases
the lock. State is replaced atomically and corrupt state fails closed.
Default experiment evidence lives in `<repo>/.insi-work/suggestions/`.
Status records attempts, next deadline, consecutive errors and the latest error.
Suggestion reports contain detailed outcomes, including rejected improvements.
The persistent `queue.json` retains task priority, baseline, frozen proposal,
status and result references. Each tick also saves observations, the planning
decision, selected proposal and final result under `<work>/heartbeat/`.
Terminal tasks are not automatically retried; pending tasks on an old baseline
are superseded. Interrupted tasks are marked for inspection and are not replayed,
which prevents automatically repeating a partially completed publication.
The next tick selects the next pending task, including after a rejected task.

Three consecutive errors or interrupted attempts pause the scheduler. Inspect
`--status` and evidence, fix the cause, then use `--reset-failures` to restart.
A completed evaluation resets errors even if the proposal did not qualify.
Restarts respect the deadline and start fresh experiments rather than resuming
interrupted workers. Configuration is reloaded each cycle. The checkout must be
clean and committed, with `test_cmd` and a worker configured.

Worker and check deadlines follow the existing workflow; there is no additional
whole-cycle deadline. The cycle handles bounded repairs, features and performance
work. It does not automatically retrain the harness or merge results. New modules
must fit project requirements and pass independent frozen acceptance, regressions
and security checks; passing the existing suite alone does not qualify them.

## research

Insi owns research decisions. Watchman provides shared research tools, rather
than a separate agenda or task queue. To enable the connection, add this optional
field to the trusted heartbeat plan:

```json
"research": {
  "sources": ["github", "hackernews"],
  "max_queries": 2,
  "days": 30,
  "limit": 3
}
```

Before planning fresh development tasks, Insi's configured proposer inspects the
project and observations, then selects questions, sources and project-specific
purposes. It may decide no research is needed. Every question must cite exact
excerpts from committed source or documentation. Insi validates the entire batch
before any search runs, then calls Watchman's existing `inbrowse.social_search`
through the local Incord engine. Only GitHub and Hacker News searches are enabled
by this adapter; the other paper/model/package APIs are not connected here.
The operator permits sources; Insi chooses among them. Limits are 1..3 questions,
1..5 results per question, and a 1..365 day window. No credentials or paid API
keys are supplied by Insi. Search terms must use public technical vocabulary,
not private code, secrets or logs.

The existing local engine and Inbrowse plugin must be running and enabled.
`INCORD_MEMORY_ADDR` selects a loopback engine address (default
`127.0.0.1:9180`). Each shared tool call uses the existing bounded memory adapter
deadline. No extra research service or model dependency is installed; question
selection uses the configured proposer and its existing access limits.

Research runs only when there is no pending task for the current baseline.
`--offline` and operator-supplied `--suggestion` skip it. Questions, planner
output and findings are saved as `research-prompt.txt`, `research-planner.json`
and `research.json` beside `observations.json`. The development planner receives
the findings, citations and explicit unavailable statuses as untrusted evidence.
Invalid question batches stop planning; unavailable searches are recorded and
allow planning from local evidence. Research cannot change trusted commands,
create queue entries directly, or bypass code justification and frozen tests.

## verification

`cargo test -p insi --test autonomous --test heartbeat` exercises compiled CLI
observation, ranking, successive task delivery, deduplication, interrupted tasks,
baseline/candidate acceptance, security rejection, scheduling and locking.
Remote observation uses a deterministic `gh` adapter in these tests. Draft PR
delivery is tested against a local bare Git remote with a hosting adapter.
Research tests exercise the compiled CLI with a local RPC server: Insi selects
questions, findings reach task planning, engine errors stay visible, offline
mode skips research, and invented evidence or disallowed sources stop planning.

Failure tests deliberately inject worker crashes, timeouts, incorrect behavior,
regressions, frozen-test edits, failed security checks and invalid security
policy. They verify that failed work never qualifies, a no-op retains the
baseline, and a later tick can select the next task while preserving the failed
task's recorded outcome. These cases use deterministic worker adapters.

A real Codex planning-and-implementation cycle completed on 2026-11-15 in a
disposable arithmetic repository using the existing native sandbox policy.
The frozen behavioral test exited 1 on baseline and 0 on candidate; regressions
and the configured AST security check passed, and delivery was `improved-local`.
The acceptance source matched byte-for-byte across both checkouts. This trial
does not establish broad feature quality, comprehensive security or a published
GitHub PR. Its local artifacts are under
`.incord/temp/heartbeat-live-20261115-v5/`.
