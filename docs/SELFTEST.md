# Insi testing its issue pipeline

## Insi-on-Insi smoke test, 14 November 2026

Tested commit `3925dbc` in a throwaway local clone using a fresh release build.
Artifacts are under `.incord/temp/insi-on-insi-20261114/`; no test branch, PR or
candidate fix was published. The owner's working files and watch checks were
not changed by the scratch init/review runs.

| Path | Observed result |
| --- | --- |
| `doctor`, `onboard`, `benchmark --detect` | Ran successfully; Rust workspace detected; onboarding emitted a plan and correctly retained `needs_configuration` status |
| `init --no-mcp --no-hooks --no-ci`, `watch-init` | Wrote configuration in the scratch clone; existing watch catalog was preserved on repeated initialization |
| Standard `review --base HEAD~1` | Complete; formatting, strict offline Clippy and workspace tests passed; about 159 seconds; this plan had no analyzer tools |
| Security-tool review | Complete; Semgrep, Gitleaks and cargo-audit completed, security checks passed, zero findings from this configured scan; about 32 seconds |
| `mine`, corrected Python `validate` | A history task from `60b390f` failed its base control and passed its reference control; 1/1 fair |
| `eval --fair-only` with a deliberate no-op subprocess | 0/1 solved, zero infrastructure errors; the unresolved task was scored as a failure |
| `report` | Reported the failed attempt, unknown cost and unsandboxed isolation accurately |

The security plan used local Semgrep rules and excluded `scripts/security` from
that scan because it contains deliberately vulnerable controls. Its zero findings
do not establish exhaustive code/security coverage. The initial Gitleaks command
used `/dev/stdout` and failed under process pipes; Insi marked review incomplete.
The supported `--report-path -` completed successfully. The documentation and
installed-scanner regression now cover that form.

An initial validation command accidentally passed literal quotes around the
unittest filename pattern, selected no tests and was rejected. Re-mining with
`python3 -m unittest discover -s scripts/security` produced the passing-reference,
failing-base controls above. The no-op worker command was
`python3 -c print(0) {prompt}`; this tests orchestration, not coding-agent quality.

Remaining environment limitations: the owner's checkout has no `.insi/config.toml`,
so doctor selects the absent default `incode` worker. Codex is authenticated with
ChatGPT, but the installed `sbx` is a different CLI and rejects the documented
Incode sandbox preset's `-q` flag. The follow-up model runs below used Codex's
built-in workspace sandbox instead; no automatic improvement loop was tested.
The deterministic no-op attempt was
unsandboxed and its report explicitly says the answer could have been read from
disk; it is not evidence for a fair isolated model comparison.

Evidence: `review/report.json`, `security-review-v2/report.json`,
`python-controls-v2.jsonl`, `doctor.md` and the corresponding private tool/check
logs in that scratch directory. The review heads are pinned to `3925dbc`; the
source history task uses its separately recorded base/reference commits.

### Follow-up Codex worker attempts

Ran ChatGPT-authenticated `codex exec` with explicit model `gpt-6.1-sol`,
`--sandbox workspace-write`, `--ignore-user-config`, `--ignore-rules`,
`--ephemeral` and `--json` as Insi's worker. This records the CLI model used;
it does not establish that it is the model serving the surrounding chat.

| Task | Observed result |
| --- | --- |
| Security feature from `60b390f`, 120-second budget | Timed out; hidden checks failed; 0/1 solved |
| Same feature, fresh attempt with 600-second budget | Finished in 237 seconds; hidden checks failed; 0/1 solved; zero timeouts or infrastructure errors |
| Unicode pathname bug from `c72b9a2`, 600-second budget | Base regression failed with the expected Git-quoted pathname error; reference passed; Codex's candidate passed in 95 seconds; 1/1 solved; zero timeouts or infrastructure errors |

The focused repair was judged with
`cargo test -p insi --test pipeline --locked --offline mine_preserves_git_quoted_test_paths -- --exact`.
Insi restored that hidden regression after the worker finished. The candidate
changed mining to consume NUL-delimited Git paths. The broad feature attempt
left required scanner and memory-fixture files missing; its failure was correctly
retained rather than changing the hidden tests or declaring success.
An independent `cargo test --workspace --locked --offline` in the retained
Unicode candidate checkout also passed, including the complete pipeline suite.

These first three runs used disposable base checkouts and did not publish a
candidate fix. Codex's workspace sandbox is not Insi's outer benchmark isolation:
Insi reports them as unsandboxed because no `[worker] sandbox` was configured.
The source repository and reference answers were not proven inaccessible. This
is evidence of worker orchestration and one passing repair, not a leakage-free
model comparison. Cost was unknown because no prices were configured.

Evidence under the scratch root: `python-controls-v2.jsonl` (120-second attempt),
`codex-600-results.jsonl`, `repair-results.jsonl`, and the corresponding
`codex-work/attempts`, `codex-600-work/attempts` and `repair-work/attempts` folders.
Each attempt retains its prompt, worker transcript, patch and hidden-check log.

### Resolving the follow-up failures

The failed feature candidate was continued with visibility of its restored
tests. It now passes all 19 Python security tests and
`cargo test --workspace --locked --offline` when independently run outside
Codex's socket restrictions. All ten original hidden test files were compared
byte-for-byte with the stored copies and remained unchanged. This assisted
completion is separate from the failed blind attempt and does not change its
recorded score. Neither candidate was merged into the current checkout, which
already contains the security feature and passes its 36 security tests.

The broad task exposed a current miner defect: substring matching accepted
`feat: add security scanner adapters and memory regression controls` as a fix.
The classifier regression failed on that exact subject before the correction.
Mining now respects explicit non-repair conventional commit types and matches
repair words rather than substrings inside words such as `debug` or `fixtures`.
Unit coverage and a source-plus-test Git-history fixture cover the exclusion.
A fresh CLI mining run against the same history wrote 11 repair tasks instead
of 12 and excluded `60b390f`, while retaining the Unicode repair. The main
checkout's complete `python3 scripts/ci/test_all.py` run passed 8/8 groups,
including formatting, strict Clippy, Rust tests, Python suites and npm checks.

An additional fresh Unicode repair ran inside a native macOS outer sandbox,
configured through `[worker] sandbox`. The OS probes allowed base-checkout
reads and path canonicalization while rejecting reads of the source checkout,
the original project README and the hidden test, and rejecting writes outside
the allowed worker locations. The final attempt passed the hidden regression
in 100 seconds with zero infrastructure errors or timeouts. Insi recorded
`isolation: {history: "base-only", sandboxed: true}`.

Initial sandbox configurations failed: nested Codex sandbox application was
denied, and a later profile denied required ancestor metadata and Codex state
writes. Their failed/error results remain in `repair-results.jsonl`. The final
profile permits traversal metadata while denying project data outside the
worker tree. A fresh Codex home inside the tree links authentication without
copying credential bytes or loading old sessions. Codex uses the external OS
sandbox directly; its `danger-full-access` option is inside that enforced
wrapper, not an unsandboxed invocation. Codex bookkeeping is excluded from
the recorded candidate patch. This is a tested local reference-file boundary,
not an independent certification of every network or sandbox escape route.

The final condition is `selftest=external-native-traversal` in
`repair-results.jsonl`. Its transcript, patch and hidden-check output are in
`native-work/attempts/insi-repair-c72b9a20-worker-s0-lessons-off-selftest-external-native-traversal/`.
The machine-specific wrapper, policy and config are retained under
`native-sandbox/`; they are scratch experiment artifacts, not a portable preset.
The assisted feature completion transcript is `security-followup.jsonl`.

## Model-driven run

A subsequent run used ChatGPT-authenticated `codex exec` for independent discovery
and as Insi's actual repair worker. The read-only discovery stage found
[issue #42](https://github.com/incordai/Insi/issues/42), a Unicode pathname bug
in mining, without being supplied a target defect. Insi fetched the labelled
issue and launched Codex, checked its candidate with `test_all.py`, committed
and pushed it, and opened [draft PR #43](https://github.com/incordai/Insi/pull/43)
at `c72b9a2`. The worker ran for 370 seconds.

Independent verification applied only Codex's new CLI regression to the old
code: it failed with Git exit 128 for the quoted café test pathname. Applying
the model's production patch made that same test pass. The patch uses raw
NUL-delimited Git paths and tests Unicode, rename/deletion and Unix quoting
edge cases without weakening unsafe-path rejection.

This demonstrates one model-driven discovery and repair cycle; it does not
prove exhaustive bug detection. It used existing ChatGPT Codex authentication,
not an API-key worker. The PR targets the tested integration branch; a small
local gh adapter selected that base while real issue/PR operations used GitHub.
Insi's own sandbox was not configured; Codex ran with workspace-write permissions.

CI additionally exposed Windows installer reader contention, filed as
[issue #44](https://github.com/incordai/Insi/issues/44) for another Codex/Insi
repair run. PR #43 should not be described as all-green while that job fails.
The second real Codex worker produced commit `e098aa8` and Insi opened
[draft PR #45](https://github.com/incordai/Insi/pull/45). Its Windows Python
3.9 and 3.12 packaging jobs passed with the new race regression and original
eight-process test. The fix refuses to replace a previously published cache
binary. It was integrated into PR #43's candidate branch for combined CI;
neither candidate was merged into main by this experiment.

## Earlier deterministic run

On 2026-11-06, a new regression against Insi's own code found
[issue #41](https://github.com/incordai/Insi/issues/41): a worker could change a
file, exit unsuccessfully, and still have its branch pushed when repository
checks passed.

The test uses real Git repositories, a real subprocess worker and a local bare
remote. A fake GitHub Hub records PR requests without publishing test candidates
to GitHub. Before the fix it failed with `Outcome::Opened`; after the fix it
expects a local candidate, no remote branch and no PR request. Successful worker
publication and failed repository-check rejection are covered by adjacent tests.

Reproduce with:

```sh
cargo test -p insi --bin insi issue::tests --locked
python3 scripts/ci/test_all.py
```

This verifies orchestration with deterministic workers, not autonomous model
reasoning. No local model runner was installed during this run and no paid API
was used. Insi works labelled issues; it does not independently discover every
bug or invent fixes without an external worker. The new defect was identified
during code review, then reproduced and verified by automation.
