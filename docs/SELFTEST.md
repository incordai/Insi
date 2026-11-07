# Insi testing its issue pipeline

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
