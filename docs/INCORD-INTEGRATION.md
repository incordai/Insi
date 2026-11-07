# Connecting Insi to Incord Memory

Incord owns durable project knowledge, retrieval, code relationships, lessons,
mistakes and beliefs. Insi owns controlled experiments, frozen benchmarks,
regression checks and the decision to propose a draft PR. Connect them through
the existing local memory adapter; do not build another knowledge graph.

`insi suggest --to-memory` / `insi improve --code --to-memory` opt into the
connection. Without the flag, the workflow remains offline from memory.

`--to-memory` is a data connection. The separate opt-in `--audit-finding <id>`
mode delegates candidate edits to the engine's auditor/coder and existing
owner/proxy approval path. It requires the new service-side
[candidate API](INCORD-CANDIDATE-API.md) to be deployed; this work changed service
source but did not restart the running engine. A stored memory decision grants
no approval. Standalone mode still uses Insi's configured worker.

## Auditor, fixer and proxy: candidate adapter

The service already implements these capabilities. Do not replace them with a
second production fixer or approval engine in Insi:

| Existing service component | Verified behavior | Current Insi connection |
|---|---|---|
| `audit.rs`: improvement finding loop | Reviews improvement findings through auditor/coder rounds. | Reads an approved improvement; source must match its project/revision. |
| `audit.rs`: coder and gated edit mechanics | Requests a find/replace, backs up, edits and keeps/reverts after tests. | New `apply_candidate` adapter reuses these mechanics on the isolated candidate. |
| Findings endpoints | Read findings and retain conversations/status. | Candidate endpoints append outcomes without claiming the original is fixed. |
| Owner/proxy console | Decides existing queued cards. | Candidate prepare uses that queue; execute requires a matching approved record. |
| Proxy active/settings | Selects the applicable policy for a project. | Existing console/proxy machinery handles the card; client does not create another policy evaluator. |
| `proxy_coder.rs` | Existing proxy-to-coder exchange. | Unchanged; candidate edits reuse the audit coder rather than creating a third model setting. |

The external-fixer endpoints inspected are:

- `GET /v1/memory/audit/findings/{id}`
- `POST /v1/memory/audit/findings/{id}/apply`
- `POST /v1/memory/audit/findings/{id}/resolve`

`apply` is explicitly authoritative approval to edit the finding's project file.
It is not an isolated-candidate executor. `resolve` with `fixed: true` marks the
finding resolved, while `fixed: false` withdraws it. Neither is an appropriate
way to record an unmerged improvement PR or an inconclusive benchmark.
Do not call either just because a memory decision was stored.

### Execution contract

The intended production flow is: **existing auditor improvement finding →
existing approval/proxy policy → existing coder/fixer operating on an isolated
candidate → Insi's frozen benchmark and regressions → draft PR → owner merge**.
Insi contributes measurement and PR qualification; Incord owns audit conversation,
policy and the production edit executor.

The original apply path does not provide that isolated execution contract.
The new candidate API adds separate service-side support, rather than treating
`lesson_write` as an execution request:

1. Bind one reviewed finding and its revision to a run, original project,
   baseline commit and isolated candidate. The finding remains about the original
   project, while permitted edits target only the registered candidate scope.
2. Delegate policy decisions through the existing owner/proxy path. Preserve
   deny/revise/escalate and standing allow rules. Memory or proxy unavailability
   must not silently authorize edits or publication. This differs from optional
   memory storage, where an outage need not fail the experiment.
3. Give the existing coder/fixer the frozen candidate scope and regression
   contract. It must not mutate the original checkout or frozen benchmark. Do
   not point the current apply endpoint at the original file to simulate this.
4. Return structured execution/check evidence to Insi. Passing a build alone
   cannot replace the paired benchmark gate.
5. Attach run/evidence/PR references as audit conversation updates without
   prematurely resolving the finding. Qualifying, proposed and merged are
   different outcomes; resolution must follow verified application to the
   intended project revision. Failed or inconclusive experiments do not prove
   the finding wrong.
6. Test denial, stale approvals/findings, candidate scope escape, benchmark
   modification, failed checks and retries before enabling automatic execution.

The candidate bridge source is implemented on both sides; scoped approval,
refusal and gated-edit mechanics have development tests. A live approved coder
round trip is unverified: no AI worker was invoked and the running service was
not upgraded. See [rollout and recovery](INCORD-CANDIDATE-API.md). Direct-worker
mode remains standalone and does not inherit engine approval merely by enabling
memory storage.

## What is connected today

| Path | Current behavior | Source |
|---|---|---|
| `insi learn --to-memory` | Writes verified lessons as `issue` items, after saving the local experiment record. Publication errors are reported. | [learn.rs](../crates/insi/src/learn.rs), [incord.rs](../crates/insi-memory/src/incord.rs) |
| Harness `insi improve --to-memory` | Writes accepted changes and the run's end as `decision` items. Detailed proposals and scores remain in the local audit log. | [improve.rs](../crates/insi/src/improve.rs), `remember` |
| Checks after edits | Incord reads `## checks` from this repository's RUNBOOK. Execution remains subject to the engine's command guard. | [RUNBOOK.md](RUNBOOK.md) |
| Code suggestions with `--to-memory` | Captures bounded context before discovery and publishes a terminal decision with a saved request and receipt. Raw evidence stays local. | [suggest.rs](../crates/insi/src/suggest.rs), [suggest_memory.rs](../crates/insi/src/suggest_memory.rs) |
| Recorded experiment assessment | Reads the supplied plan/results and produces an assessment. Does not retrieve or publish memory. | [experiment.rs](../crates/insi/src/experiment.rs) |

The existing adapter uses JSON-RPC `tools/call` at
`http://127.0.0.1:9180/v1/memory/mcp`. `INCORD_MEMORY_ADDR` overrides the
socket address; the adapter rejects non-loopback addresses. Its current transport
is a bounded plain HTTP socket, not a remote or authenticated cloud client.
Do not expose this endpoint publicly or add credentials to run artifacts.

Existing CLI configuration, for an already prepared task set:

```sh
export INCORD_MEMORY_ADDR=127.0.0.1:9180
insi learn --tasks <tasks-dir> --private <private-dir> --to-memory
```

This command is an example, not a check performed for this document: learning
can invoke the configured worker. No worker was invoked during this audit.
Code suggestions use the same opt-in flag. Setting the address alone does not
connect a workflow.

```sh
insi improve --code --repo <clean-repo> --to-memory --local
insi suggest --retry-memory <run-directory>
```

The first command can invoke the configured proposer/worker. Retry only delivers
the saved decision; it does not rerun a worker, measurements or PR publication.

## What overlaps and what to retain

| Capability | Insi implementation | Incord implementation | Recommendation |
|---|---|---|---|
| Durable lesson storage and deduplication | `insi-memory/src/store.rs`: `FileLessons` and lesson IDs | `project_stores/lesson.rs`, `memory.rs`, `dedupe.rs` | Use Incord for production knowledge. Keep the file store as a fixed, inspectable experiment input/output. |
| Topic retrieval and prompt briefing | `insi-memory/src/briefing.rs` | `context` tool and project/code index | Reuse Incord for production context. Keep deterministic briefing for controlled comparisons. |
| Repeated evidence becoming a belief; contradiction handling | `briefing.rs`: belief threshold, topic promotion and supersession | Existing memory graph/belief facilities | Avoid adding another production belief engine. Benchmark these behaviors against fixed snapshots when needed. |
| Lesson extraction from attempts | `insi-memory/src/writer.rs` and `learn.rs` | Memory service's durable lesson/mistake records | Insi may produce a narrowly evidenced lesson; Incord stores and retrieves it. Do not independently maintain the same production lesson in two mutable stores. |
| Automatic lint/test scheduling | Insi's test runner and documented commands | `auto_checks.rs`, `watch_checks.rs` | Keep project commands in Insi; use Incord to schedule/report them. Do not copy the engine scheduler. |
| Command execution and timeout handling | Insi process/sandbox runners | Engine check runner | Shared concern, different execution contracts. Retain Insi's isolated experimental execution; an unrelated background check cannot replace candidate validation. |
| Execution logs and measurements | Run JSON, JSONL, benchmark samples and `.insi-bench/evidence.json` | Graph references and summaries | Retain raw artifacts in Insi. Save concise claims and artifact references in Incord. These files are evidence, not a second knowledge graph. |
| Suggestion qualification and draft PR | Frozen baseline/candidate comparison, regression and publication guards | Automatic checks and findings do not establish this paired comparison | Keep in Insi. Passing an engine check alone must never qualify an improvement PR. |

Overlap does not mean these modules can be deleted immediately. The benchmark
uses time/task-filtered lesson sets to prevent future information leaking into
earlier tasks. Replacing those inputs with live memory would change the
experiment. Preserve that contract and its tests before removing anything.

## Adapter flow and boundaries

1. Resolve the original repository to its Incord project. For this repository
   the project ID is `insi-vsgesh`. Use the original registered checkout for
   `cwd`, never an ephemeral candidate clone. If both project and cwd are sent,
   they must identify the same project.
2. Before discovery, request bounded `context` for the improvement question.
   Supply `cwd`, `query` and `budget_tokens`; reuse returned code references and
   relevant lessons. Do not crawl/export the whole graph. For a controlled
   component experiment, freeze the retrieved context and its provenance before
   comparing candidates; live changing context is another experimental variable.
3. Let Insi perform the existing benchmark and regression workflow. Retrieved
   advice is a suggestion; it cannot override frozen checks, execution policy,
   publication guards or the owner's review.
4. After a terminal outcome, publish one concise `decision` through
   `lesson_write`. Record no-gain and failed runs too. Export the outcome and last
   execution phase; keep detailed failure text local to avoid leaking output.
   Report a failure category as a hypothesis unless execution evidence establishes
   its cause. A positive benchmark result supports only its measured workload.
5. Keep raw samples and logs in the run directory. Put their durable location,
   baseline/candidate SHAs, metric, unit, direction, sample count, margin,
   interval, sign-test result, regression result and PR URL in the decision's
   explanation. Include a Git blob digest of the local report. Local
   absolute paths are useful only while those artifacts remain on that machine;
   prefer the committed PR evidence for shared references.
6. Return the existing Insi result even if memory is unavailable. Retain a local
   pending publication record and provide explicit retry. A memory receipt
   confirms storage, not benchmark validity or permission to publish a PR.

The connection reuses `insi_memory::incord` for context and stored-item replies.
A small pending-publication file is delivery state, not a graph.

## Write contract

The service source accepts `lesson_write` with `kind`, `record`, project identity,
optional `agent_id`, and an optional `write_key` of at most 128 characters. The
key is scoped to the agent: the same key and content returns the original item;
different content with the same key is refused. Code suggestions supply these
keys and retain receipts. Older lesson/harness publishing retains its existing
delivery behavior.

Illustrative JSON-RPC request for the exporter (placeholders are not
measurements):

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "lesson_write",
    "arguments": {
      "project": "insi-vsgesh",
      "agent_id": "insi",
      "write_key": "suggest:<run-id>:terminal-v1",
      "kind": "decision",
      "record": {
        "decision": "Keep suggestion local: benchmark did not qualify",
        "why": "Run <id>; baseline <sha>; candidate <sha>; measured workload <name>; artifact <reference-and-digest>; actual checks and measurements go here.",
        "tags": ["insi", "suggestion", "outcome:no-gain"]
      }
    }
  }
}
```

Keep the retried payload identical. A later PR event needs its own event key;
do not change a previous event under its old key. Use `locations` containing
`file` and `symbol` when linking code, rather than copying source into memory.
Resolve locations in the original checkout; a candidate-only symbol may not
exist there until merge, so retain its commit/artifact reference instead.
Do not upload private tests, secrets or unfiltered command output.

## Automation ownership

Incord's `auto_checks.rs` reads RUNBOOK commands after edits; `watch_checks.rs`
runs named scheduled checks from `.incord/watch.toml` and records/resolves audit
findings. Insi's [test runner](../scripts/ci/test_all.py) remains a convenient
manual/CI entry point. Avoid registering both that complete runner and all of
its individual commands for the same engine trigger.

Do not schedule `insi improve --code` as an ordinary health check: it can invoke
a worker and publish a branch/PR. Automatic checks should remain validation
commands. Improvement execution needs the owner's configured scope and policy.

## Delivery artifacts and verification

- `memory-context.json`: bounded context snapshot and retrieval status, captured
  before discovery. Unavailable memory produces empty context. A supplied
  suggestion is already fixed, so retrieved context does not alter it.
- `memory-pending.json`: exact terminal decision request, saved before sending.
- `memory-receipt.json`: stored-item acknowledgement. A receipt skips subsequent
  delivery. A lost receipt can replay the same request and write key.

Requests and receipts use a synced temporary file followed by rename. Delivery
errors do not change the Insi outcome. Sensitive-data matches withhold export;
blind retry cannot repair these. Raw logs and failure text are never exported.
The summary includes metric metadata where available, paired values, statistics,
required margin, commit references, report digest and PR URL when present.
Candidate commit metadata is captured at export time; committed benchmark
evidence remains authoritative for the comparison. Detailed regression outcomes
remain in the local evidence.

Five regressions exercise bounded context and original-checkout routing, refusal
and exact replay, receipt reuse, unacknowledged responses, raw-error exclusion,
and a complete suggestion run where hostile advice cannot bypass the no-change
gate and offline delivery leaves the original result intact. They use a fake
engine and controlled shell commands, not an AI worker.

Live round trips and workload generalization must be reported separately;
adapter tests alone establish neither. Recorded experiment assessment still has
no automatic memory export. Keep its inputs fixed for controlled comparisons.

A worker-free smoke run used the compiled CLI against the running local service.
The first delivery returned a stored-item receipt. Removing the local receipt
and replaying the identical request returned the same item ID and
`duplicate: true`. Evidence is in
`.incord/temp/live-memory-1794060201128740000/result.json` and that directory's
two receipts. No code improvement, AI discovery or performance gain is claimed
by this delivery test.

## Source inspection and limits

Inspected the Insi adapter, learning call sites, harness memory decision helper,
RUNBOOK and suggestion/assessment memory references. Inspected the sibling
service checkout at `../chat/incord-memory-service/src/`, particularly
`mcp.rs`, `project_stores/lesson.rs`, `project_stores/tools.rs`,
`project_stores/tools_tests.rs`, `auto_checks.rs` and `watch_checks.rs`.
The write-key tests were read, not executed during this audit. Local service
source is not proof that the running binary has the same version.

The Codecov project page was attempted during the audit but was inaccessible
through the web tool. No Codecov verification is claimed; this documentation
change does not alter the free local/CI coverage workflow.
