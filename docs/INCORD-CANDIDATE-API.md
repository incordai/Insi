# Delegating an improvement candidate to Incord

```sh
insi improve --code --repo . --suggestion improvement.json --audit-finding <id> --benchmarks benchmarks.json --to-memory --local
```

Omit `--local` only when draft-PR publication is intended. The finding must be an
approved **improvement** for this project. The optional existing proposer can
generate its benchmark; the actual implementation is delegated to the engine's
audit coder, not Insi's configured implementation worker. Supplying a suggestion
avoids proposer execution. No coder/model was invoked during development tests.

## Service source and rollout

The sibling memory-service checkout contains the implementation in:

- `src/api/insi_api.rs`: candidate scope, existing owner/proxy approval, status,
  execution and audit-note handlers.
- `src/audit.rs`: candidate adapter reusing the configured audit coder and the
  backup → unique edit → test → keep/revert mechanics.
- `src/api/mod.rs` and `src/router.rs`: owner-console feature routing.

These source changes are local to that service checkout. Build/test that service
with its default features, including `owner_console`; a no-default-feature check
hits unrelated embedding-backend compilation errors in that checkout. The active
service has not been restarted or upgraded by this work. An older engine refuses
the new routes; Insi fails closed and does not silently run a direct worker.
Insi-only CI cannot establish that the deployed engine supports this protocol.

The engine must have its audit CLI source enabled, a configured test command and
original-tree `auto_apply` disabled for this mode. Paused files are refused. Only
the existing user-selected CLI coder is used; no service/API model fallback is
introduced. The current adapter proposes one unique find/replace in the finding's
single file; it is not a general multi-file refactoring executor.
An owner console must be connected or a device registered, as required by the
existing owner-enqueue API. If nobody can answer, prepare refuses immediately
instead of leaving an approval card that cannot be answered.

## Protocol

Read `GET /v1/memory/audit/findings/{id}`. All other routes below are POST:

| Route suffix | Effect |
|---|---|
| `/candidate/prepare` | Validate scope and enqueue a candidate-specific decision through existing owner/proxy machinery; return its token |
| `/candidate/status` | Read pending/approved/denied-or-revised/expired state without invoking a coder |
| `/candidate/execute` | Require matching approval, claim the candidate, ask the existing coder for its edit, and run the configured gate |
| `/candidate/report` | Append the final Insi outcome/evidence reference to that finding's conversation without resolving it |

Request fields for the first three routes are `project`, `candidate`, `baseline`,
`finding_updated_at`, and `approval_token` when available. The project is the
original absolute checkout. Candidate clones must be under that project's
`.insi-work/` or `.incord/temp/`; use `--work` accordingly when `--repo` differs
from the current directory. Git SHA-1 baselines are supported by this initial
service adapter. The original HEAD must still match. The candidate may contain
only the frozen `.insi-bench/` commit changes, no implementation edits or dirty
files before execution. Its target must match the original source and cannot
escape the candidate through a symlink.

Approval is read from the engine's existing decision store. A caller's boolean,
a memory summary or a proxy recommendation cannot substitute for a consumed,
approved, unexpired record matching the request and coder settings. Existing
proxy/owner policy decides the card; Insi does not implement a second policy
engine or reinterpret recalled allow rules as fresh grants. If policy cannot
decide, it follows the existing owner path. Scope changes require a new decision.

Insi polls the existing decision via the status endpoint within its configured
wall budget (1–3,600 seconds for this adapter), then waits for the bounded execute
response. Denial, revision, expiry, timeout, stale revision or service outage
stops execution/publication. No merge permission is requested or inferred.

## Edits, evidence and recovery

The service stores a candidate claim in `.git/insi-candidate.lock` before invoking
the coder. Claims survive errors and process restarts; automatic re-execution
after an ambiguous timeout is deliberately refused. Inspect a retained candidate
and its claim before deciding whether to start another run. This is not a fully
automatic execution-resume protocol.

Backups are created exclusively under the candidate's `.git/`, so original source
is preserved without adding backups to a proposed patch. A failed candidate gate
reverts that candidate edit. The original file is checked for unexpected changes
during execution; a change withholds the result and is not overwritten by a
rollback. The configured agent CLI still defines host access; clones and checks
are not a substitute for OS sandboxing.

After execution, the original finding is parked in `needs_human` awaiting review;
it is not resolved or fed into original-tree auto-apply. Insi independently checks
the frozen benchmark/configuration/history, project regressions and suite before
qualifying a draft PR. It does not treat the service's passing gate as enough.

Local artifacts include `engine-candidate.json` (contains a private approval
token), `engine-execution.json`, normal benchmark evidence and `report.json`.
Do not publish the private request/token file. The final audit write-back is a
concise outcome/evidence reference, not raw logs or a claim that the original tree
has been fixed. If that delivery fails:

```sh
insi suggest --retry-audit <run-directory>
```

This retries only the audit note. It never invokes a coder, reruns measurements,
opens another PR or marks the finding resolved. Old approval records may age out
of the owner's queue; such a retry fails rather than forging a new authorization.

## Verification boundary

Development tests use existing engine test stores, real isolated Git/filesystem
fixtures and controlled commands. They cover scoped approval, stale or dirty
candidate refusal, registered HTTP routes, and gated keep/revert/backup mechanics.
The Insi denial test verifies that the execute endpoint is never called after a
denied/revised decision. Model quality and a live approved CLI-coder round trip
remain unverified because the standing instruction prohibits invoking a worker.
