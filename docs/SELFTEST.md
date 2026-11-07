# Insi testing its issue pipeline

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
