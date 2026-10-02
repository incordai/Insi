# Insi

Insi measures whether a coding agent gets better at a repository over time,
and is built to be added to any repository as a dependency.

Phase 1 is the measurement: a benchmark mined from the repository's own
history, run by any agent through a worker command, with controls that say
whether each task is a fair test. Lessons (an agent writing down what went
wrong and being briefed with it next time) come next, and are judged by the
same benchmark: `lessons=off` against `lessons=on`.

## How a task is made

`insi mine` reads a repository's history — only reads it — and turns every
fix commit that changed source **and** a test file of its own into a task:

| Part of the task | Where it comes from |
|---|---|
| start (`base`) | the fix commit's parent |
| prompt | the fix commit's message |
| hidden check | the test files the fix added or changed |
| reference | the fix commit itself |

The task's public part goes to `--tasks`; its hidden test files go to
`--private`, which should be outside every checkout an agent can read. A test
written inline beside the code (Rust's `#[cfg(test)]`) cannot be held back
without handing over the fix, so only separate test files count.

## Fair tasks

`insi validate` runs two controls per task, with no agent:

- **base** — the hidden check at the starting commit. It must **fail**, or the
  task asks for nothing.
- **reference** — the hidden check at the fix. It must **pass**, or the task
  cannot be solved as set.

Only tasks that do both are fair; `insi eval --fair-only` runs just those.

## An attempt

Each attempt gets its own worktree from Insi's own clone of the source, so the
source repository is never written to. Setup runs, then the worker with the
task's prompt and wall-clock budget, then the hidden tests are copied in —
after the worker has stopped, replacing anything it wrote at those paths —
and the check runs. A failure's output is reduced to a signature (paths,
hashes and numbers normalised), so the same mistake made twice is counted as
a repeat.

## Lessons

Each worker attempt keeps what it left behind under `<work>/attempts/<name>/`:
the worker's diff, its output, the check's output.

`insi learn` takes every failed attempt on a fair task — whose fix is
therefore verified: the reference commit passed the check — and asks the
worker for one lesson from the attempt, the failure and the fix: an issue, a
root cause, an action to **use** and one to **avoid**. A lesson is refused if
a field is empty, longer than a few sentences, or holds a code block. Lessons
are kept in `<work>/lessons.jsonl` (or `--lessons <file>`), one per attempt.

`insi eval --lessons <file>` puts a briefing in front of each task's prompt:
the lessons of the same repository, newest first, within 4,000 characters,
and **only lessons learned from tasks dated before this one** — compared as
instants, so commit dates in different time zones order correctly. A lesson
can never be briefed to the task it came from, or to anything earlier.

Every worker attempt is tagged `lessons=on` or `lessons=off`, and records
which lessons it was briefed with.

The measurement, end to end:

```sh
insi validate ... --jobs 2                                 # which tasks are fair
insi eval  ... --fair-only --half older --tag phase=learn  # attempts to learn from
insi learn ...                                              # lessons from their failures
insi eval  ... --fair-only --half newer --seeds 3          # lessons=off
insi eval  ... --fair-only --half newer --seeds 3 --lessons <work>/lessons.jsonl
insi report
```

`--half older|newer` splits the selected tasks by commit date, so lessons are
learned on one half and measured on the other. The report groups attempts by
their tags, so the learning run carries a tag of its own and is never
counted with the measured `lessons=off` runs.

## Commands

```text
insi mine     --source <repo> --tasks <dir> --private <dir> --check "<cmd {tests}>"
              [--setup "<cmd>"] [--prefix <name>] [--limit N] [--wall-secs N]
insi validate --tasks <dir> --private <dir> [--only id,id] [--half older|newer] [--results <file>] [--jobs N]
insi eval     --tasks <dir> --private <dir> [--seeds N] [--tag k=v] [--worker "<cmd {prompt}>"]
              [--lessons <file>] [--fair-only] [--only id,id] [--half older|newer]
              [--results <file>] [--keep] [--jobs N]
insi learn    --tasks <dir> --private <dir> [--lessons <file>] [--worker "<cmd {prompt}>"]
              [--results <file>] [--jobs N]
insi report   [--results <file>]
```

Every command takes `--work <dir>` (default `.insi-work`): Insi's clones,
worktrees and the results file (`results.jsonl`, one line per attempt,
appended). `--jobs N` runs N attempts at once. `--setup-jobs N` (default 1)
lets only N of them run setup at once: installing dependencies is bound by
the disk, and on Watchman three `npm ci` at once took over seventeen minutes
each against under two alone. Workers and checks still overlap.

Each result records seconds per phase — `prepare`, `queue` (waiting for a
setup slot), `setup`, `worker`, `check` — so a slow run says where its time
went. Deleting a finished worktree happens in the background and is in none
of them. A worker declares the files its harness writes for itself
(`[worker] own_files`, git glob pathspecs; incode's state, spec documents and
backups by default), and they are left out of the worker's recorded diff.

Example, a TypeScript project tested with vitest:

```sh
insi mine --source ../app --tasks ../insi-private/app/tasks --private ../insi-private/app/hidden \
  --check "npx vitest run {tests}" --setup "npm ci" --prefix app --limit 40
insi validate --tasks ../insi-private/app/tasks --private ../insi-private/app/hidden --work ../insi-work
insi eval --tasks ../insi-private/app/tasks --private ../insi-private/app/hidden --work ../insi-work \
  --fair-only --seeds 3 --tag lessons=off
insi report --work ../insi-work
```

## Configuration

`.insi/config.toml` in the repository being worked on; every key is optional,
and an unknown key is refused.

```toml
topic = "app"
test_cmd = ["npm", "test"]
protected = ["src/auth/**"]
eval_tasks = "evals/tasks"
private_checks = "../insi-private/app"
trusted_issue_label = "insi"

[worker]
# {prompt} is replaced by the task's prompt. The default is incode.
command = ["incode", "-t", "{prompt}", "--output-format", "stream-json"]
```

Any agent with a non-interactive mode can be the worker. Token counts are read
from incode's `stream-json` result; another worker's attempts are timed but
not counted in tokens.

## Layout

| Crate | What |
|---|---|
| `insi-core` | config, the task format, failure signatures |
| `insi-eval` | running commands with deadlines, mining, the store, attempts, reports |
| `insi-memory` | lessons: the writer's prompt and parsing, the store, the briefing |
| `insi` | the command line |

## Building

```sh
cargo build --release
cargo test
```
