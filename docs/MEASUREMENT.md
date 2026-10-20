# How Insi measures

Insi's claim is narrow: a harness change is better only when the benchmark
says so, on work it was not chosen on. These rules keep that claim true. A
change to Insi that breaks one of them needs to say why in its pull request.

## The rules

1. **The agent never sees the answer.**
   - A worker attempt's repository holds the base commit and nothing after
     it, so no git command shows the fix.
   - The hidden tests arrive only after the worker stops.
   - The source checkout and the hidden tests on disk are kept from the
     worker by `[worker] sandbox` (for example `sbx --deny-read {source}
     --deny-read {private}`).
   - `insi report` counts attempts that ran without one, because their passes
     aren't proof of the work.

2. **Only fair tasks count.** A task's hidden check must fail at the base
   and pass at the reference (`insi validate`). A check that can't fail
   measures nothing.

3. **Learn on the past, measure on the future.** Lessons and harness changes
   are chosen on the older half of the tasks and checked on the newer half,
   which never takes part in choosing.

4. **Compare the same work.** Two conditions are compared only on the
   (task, seed) pairs both were judged on. An attempt that errored judges
   nothing.

5. **A win clears its interval.** `insi improve` accepts a change only when
   both of these hold:
   - the whole 95% bootstrap interval of the per-task difference (tasks
     resampled 2,000 times, seeds pooled per task) is above zero;
   - the mean difference is at least `--margin`.

   One lucky task in a small set can't do that. The interval is seeded by
   the step, so the same results always give the same decision.

6. **No trading away a sure thing.**
   - A change is rejected if any task the incumbent passed on every seed now
     fails, however many new wins it brings, on selection or held out.
   - It's rejected if reward hacking (public check passed, hidden failed)
     rises.

7. **As good and cheaper ships.** A tie (the interval contains zero, no task
   got worse) is accepted when the candidate used at least 10% fewer tokens
   on the same pairs.

8. **Errors are not a verdict.** When more than 5% of either side's worker
   attempts errored, the comparison measured the infrastructure.
   - `insi improve` records the step as `not-measured` and teaches the
     strategy bandit nothing from it.
   - `insi eval` exits **2**, apart from 1 (failed) and 0, so a script or CI
     never reads infrastructure trouble as a result.

9. **One run decides nothing.** A result names its seeds. A published number
   needs at least three seeds, with the interval beside it.

10. **Every number comes from a run.** It isn't written by hand in code,
    docs or a PR. Every attempt records its tokens, wall time, phases, cost
    (with `[prices]`) and isolation, so a number can be traced to the
    attempts behind it.

11. **Show it can fail.** A measurement is trusted only after a deliberate
    break turns it red. The pipeline test (`crates/insi/tests/pipeline.rs`)
    runs the whole loop with model-free workers on every change:
    - a careless worker that must fail;
    - a diligent one that passes only when a lesson reaches it;
    - a better harness the loop must accept, and a worse one it must reject.

## Exit codes

| code | meaning |
|---|---|
| 0 | ran and measured |
| 1 | failed (a bad flag, a missing file, a refused config) |
| 2 | not measured: too many attempts errored to say anything about quality |
