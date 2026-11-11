# Evidence-first review and improvement execution

Insi reviews a specific revision, runs configured local checks in a disposable
clone, and records what was actually observed. Incord remains the source of
project knowledge, audit findings, owner/proxy decisions and the production
coder. No new model service or memory database is introduced.

## Seven capabilities

| Capability | Entry point | Boundary |
|---|---|---|
| Auditor → policy → isolated coder → benchmark → draft PR | `improve --code --audit-finding <id>` | Requires the new candidate API in the local engine; no direct-worker fallback |
| PR reviews, summaries and inline findings | `review --pr N`, optionally `--publish` | Exact base/head checks, author-scoped duplicate suppression; COMMENT only |
| Reproduction evidence | `reproductions` in the review plan | Identical frozen test on base/head; observed failures need a declared assertion marker |
| Test-impact selection | `tests`, revision-pinned `graph`, `--with-memory` | Missing/stale/incomplete mappings fall back to full checks; graph completeness is not proven |
| Unified tool findings | `tools` in the review plan | Normalized JSON, a SARIF subset and ESLint JSON; invalid output makes review incomplete |
| Feedback and review-quality evaluation | `review-feedback`, `review-quality` | Durable judgments go to Incord; corpus labels and repository identities are producer supplied |
| Setup inspection and verification | `onboard`, existing `init`/`doctor` | Writes a new setup directory, optionally verifies commands; does not install tools or overwrite configuration |

These are configurable execution and evidence features, not a claim that every
language/security analyzer or an autonomous semantic reviewer is built in. An
empty tools list runs checks only. Local tools or a separately configured review
adapter must supply findings; tests alone do not establish that code is bug-free.

## Local review

```sh
insi onboard --repo . --output .incord/temp/setup
insi onboard --repo . --output .incord/temp/verified-setup --verify
insi review --repo . --base HEAD~1 --plan .insi/review.json --output .incord/temp/review
insi review --repo . --pr 47 --plan .insi/review.json --output .incord/temp/pr-review --with-memory
```

### Model review with the local `claude -p` login

```sh
python3 scripts/review/review_pr.py 47           # print findings, post nothing
python3 scripts/review/review_pr.py 47 --post    # also post one COMMENT-only review
python3 scripts/review/review_pr.py --all --post # every open, non-draft PR
```

This runs on a maintainer's machine with its own `gh` and `claude` logins; no key
is stored in the repo. It builds a throwaway checkout of the PR head, sends only
the diff (split by file) to `claude -p` with no tools, and publishes through
`insi review-publish`, so the review is COMMENT-only and a head already reviewed
is skipped. PR code is never executed: the generated plan's only check is
`git rev-parse HEAD`. Needs `target/debug/insi` (`cargo build -p insi`) or
`INSI_BIN`. The model sees limited context around each change, so treat
findings as leads to verify, not verdicts. It cannot approve or merge; a
branch-protection review is still a human step.

Plans are explicit operator inputs. PR descriptions, comments and modified PR
configuration do not select executable commands. The review clone contains only
committed files; the owner's uncommitted edits remain in the original checkout.
Tools/checks must leave tracked files and history unchanged. Generated build
outputs should be ignored by the project. Output must name a fresh directory.

PR tool execution requires a configured sandbox. Alternatively, set
`INSI_REVIEW_IMAGE` to an immutable `image@sha256:<64 hex digits>` containing the
project's tools and offline dependencies. Insi then uses Docker with networking
disabled, dropped capabilities, no-new-privileges, resource limits and only the
review checkout mounted. Host credentials and Docker's socket are not mounted.
The configured sandbox is a trust boundary; a pass-through command is not OS
isolation. The controlled tests use a pass-through fixture only for trusted test
commands and do not certify container isolation.

## Review plan

```json
{
  "version": 1,
  "tools": [
    {
      "name": "local-analyzer",
      "command": ["my-local-analyzer", "--format", "json"],
      "format": "json",
      "accepted_exits": [0, 1],
      "timeout_secs": 300
    }
  ],
  "full_checks": [["cargo", "test", "--workspace", "--locked", "--offline"]],
  "block_on": "error",
  "tests": [],
  "graph": [],
  "reproductions": []
}
```

Supply an installed analyzer; the example name is a placeholder. Normalized JSON
is `{"findings":[{"file":"src/lib.rs","line":12,"rule":"bounds","message":"Index is unchecked","severity":"error"}]}`.
Every finding needs a checkout-relative path, positive line and nonempty message.
Absolute ESLint paths within the reviewed checkout are converted to relative
paths. Outside paths are refused. SARIF requires `runs[].results[]` with
`ruleId`, `message.text` and a first physical location containing `uri` and
`region.startLine`. Other SARIF features need a project adapter; unsupported data
is not silently treated as a successful empty scan. Output is bounded by the
existing process runner, so truncated/non-JSON output makes the tool incomplete.

`accepted_exits` defaults to `[0]`; add `1` only when the tool uses that exit to
report findings. Timeouts, unavailable commands, unexpected exits and malformed
output make the review incomplete. Raw logs remain local. The exported report
withholds strings matching the existing sensitive-data scanner and marks that
review incomplete. Do not upload raw command logs as public artifacts.

Findings carry stable fingerprints independent of line movement, source-tool
references and explicit verification labels. Imported labels cannot turn a
tool's allegation into a reproduction. Duplicate claims from several tools are
combined when file, rule and normalized message match; semantic equivalence is
not guessed. `block_on` is optional (`note`, `warning`, `error`). Without it the
review is advisory; with it, findings at or above the threshold fail the command.
Neither mode forces a correction, approves code or merges anything.

## Identical-test reproduction

Add a reproduction contract to the plan:

```json
{
  "id": "parser-contract",
  "file": "src/parser.py",
  "line": 12,
  "rule": "parser-contract",
  "message": "Parser no longer preserves the required output",
  "command": ["python3", ".insi-review/parser_contract.py"],
  "files": {
    ".insi-review/parser_contract.py": "# Complete project-specific test source goes here\n"
  },
  "assertion_marker": "ASSERT_PARSER_CONTRACT",
  "failure_exit": 1
}
```

The source is a placeholder, not a runnable test. Insi installs identical files
under `.insi-review/` in independent baseline/head clones. Files cannot overwrite
existing paths or escape through symlink parents. A reproduction is classified:

- Base passes; head fails with the expected exit and marker: reproduced regression.
- Base fails with the expected exit and marker; head passes: verified fix for that test.
- Both pass: not reproduced by this test.
- Other outcomes, including environment failures: inconclusive; review incomplete.

Only exits 1 and 101 are supported assertion-failure codes. The marker is evidence
of the intended assertion, not a proof of root cause or full behavior coverage.
Tracked-source/history changes invalidate publication. No worker is needed to run
a supplied reproduction. Supply fixtures and dependencies that work on both
revisions; a missing baseline environment must not become a claimed regression.

## Impact selection and Incord context

`--with-memory` asks the original registered project's `git_blast_radius` tool
about the committed diff, saves its response and retains recommended impacted
tests/coverage limits in the evidence. Failure or truncation does not establish
an empty impact set. This does not register a second project graph for the clone.

An optional frozen graph snapshot can narrow checks:

```json
{
  "graph_commit": "exact-baseline-sha",
  "graph": [
    {"file":"src/parser.py","callers":["tests/test_parser.py"]},
    {"file":"tests/test_parser.py","callers":[]}
  ],
  "tests": [{"paths":["src/parser.py","tests"],"command":["python3","-m","pytest","tests/test_parser.py"]}]
}
```

This is an experiment input exported from the existing graph, not another
database. Every affected file must have a graph entry and a test mapping. Stale
revision, unknown caller, configuration/dependency changes or no changed files
use `full_checks`. Static call graphs cannot prove completeness, especially for
dynamic dispatch/macros. Live memory recommendations alone do not bypass the full
fallback. Keep full merge checks where stronger assurance is required.

## Publishing and automation

```sh
insi review --repo . --pr 47 --plan trusted-review.json --output .incord/temp/review-47 --publish
insi review-publish --repo . --pr 47 --report report.json
```

`gh` supplies authentication. Before posting, Insi checks both current PR SHAs
against the report and associates the review with that exact head. Only added
lines receive RIGHT-side inline comments; other findings appear in the summary.
At most 100 finding details are posted. It checks its authenticated author's
prior markers to suppress repeated findings and identical reviews. Changed
results at the same head get a different review marker. Cross-machine concurrent
publication can still race; use the workflow's per-PR concurrency coordination.
Old review threads are not automatically resolved merely because a new scan is
empty. All posted reviews use COMMENT, never APPROVE or an automatic merge.

For GitHub App/action tokens that cannot call `/user`, set `INSI_REVIEW_AUTHOR`
to the actual posting identity. Incorrect identity prevents reliable duplicate
suppression; it is not an authorization credential.

The opt-in [workflow](../.github/workflows/insi-review.yml) triggers on PR opens,
updates/reopens and manual dispatch. After these features are in the trusted base,
set repository variables `INSI_AUTO_REVIEW=true` and `INSI_REVIEW_IMAGE` to the
pinned, preprovisioned image. Review `.insi/review.json` on the trusted base.
The workflow builds only trusted base code, runs PR tools in the sandbox with a
read token, uploads only the redacted report, and publishes in a separate job
that does not execute PR code. Fork PRs retain read-only artifacts; automatic
publication is limited to same-repository PRs or explicit manual dispatch.
This workflow is not enabled merely by adding its file. Missing image, plan,
toolchain or offline dependencies fails setup; do not report that as a clean
review. Live GitHub publication/deployment was not performed during development.

## Feedback and quality

```sh
insi review-feedback --report report.json --finding <id> --verdict false-positive --note 'Reason for this judgment'
insi review-quality --plan review-corpus.json
```

Feedback accepts `accepted`, `false-positive`, `fixed` or `regression` and writes
one idempotent Incord decision scoped to the report's original project. A verdict
is a human judgment, not independent test proof; it never silently suppresses a
rule or changes a production belief. Changed content under the same delivery key
is refused by memory rather than overwriting earlier evidence.

Corpus format:

```json
{
  "version": 1,
  "selection_repositories": ["repository-used-for-tuning"],
  "tasks": [
    {
      "id": "heldout-change-1",
      "repository": "different-heldout-repository",
      "report": "heldout-report.json",
      "truth": [{"file":"src/parser.py","line":12,"rule":"parser-contract"}]
    }
  ]
}
```

The evaluator computes precision, recall, misses, false positives, completion rate
and review time. Incomplete runs stay in the denominator. Duplicate tasks/reports
and overlap with declared selection repositories are rejected. Labels use exact
file/line/rule matching and must be independently adjudicated by the operator;
repository identities must be canonical and selection history honest. The command
does not itself establish model superiority or generalization. Use
[experiment assessment](EXPERIMENTS.md) for matched-seed/family-controlled claims.

## Incord execution rollout

See [candidate API](INCORD-CANDIDATE-API.md). The client is opt-in; an older engine
returns an error and no direct-worker edit is substituted. The sibling service's
source changes must be built and deployed separately. No live worker/model was
invoked and the active memory service was not restarted during development.

## Development verification

All seven local project check groups passed, including 74 binary tests. After a
portable path-normalization adjustment, Clippy and the eight review/evidence/
quality tests passed again. The workflow passed `actionlint` and documentation
relative links were checked. Engine checks used default features; scoped approval,
real Git candidate boundaries, unknown/unauthorized route refusal and gated
keep/revert/backup tests passed. This is targeted engine validation, not a claim
that its complete test suite or deployed runtime was verified.

The CLI trial at `.incord/temp/review-cli-1794064422908105000/result.json` used real
Git clones and a fake hosting executable: a paired reproduction detected a
regression, exactly one inline comment was prepared, retry left one review, and
the original checkout stayed unchanged. No real GitHub mutation was made.

The checks-only trial at `.incord/temp/real-insi-review-1794065040562588000/result.json`
ran the actual Insi signature tests, exercised onboarding and retrieved a live
Incord blast-radius response. It does not establish semantic review quality,
live model behavior, superiority over another reviewer or production deployment.
