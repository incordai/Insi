# Insi

Insi measures whether a coding agent gets better at a repository over time,
and is built to be added to any repository as a dependency.

Phase 1 is the measurement: a benchmark mined from the repository's own
history, run by any agent through a worker command, with controls that say
whether each task is a fair test. Lessons (an agent writing down what went
wrong and being briefed with it next time) come next, and are judged by the
same benchmark: `lessons=off` against `lessons=on`.

## Adding Insi to a repository

```sh
cd your-repo
insi init       # writes .insi/config.toml, ignores .insi-work/, registers insi mcp
insi mine       # tasks from the history, hidden tests outside the checkout
insi validate   # which tasks are fair
insi eval --fair-only
```

`insi init` reads the files at the repository's root to tell how it is
installed and tested — a Node lockfile (npm, pnpm, yarn, bun) with vitest or
jest, Cargo, Python, Go — and writes what it found:

```toml
topic = "shop"
check = ["npx", "vitest", "run", "{tests}"]
setup = ["npm", "ci"]
private_checks = "../insi-private/shop"

[setup_cache]
key = ["package-lock.json"]
keep = ["node_modules"]
```

`[setup_cache]` lets attempts reuse what setup made instead of making it
again: after an attempt, `node_modules` is moved (renamed, not copied) into
`<work>/deps/<key>/`, and the next attempt whose setup command and lockfile
match moves it back in and skips setup. A worker that changed the lockfile
keeps its folder out of the cache. On Watchman, `npm ci` was 4 to 5.5
minutes of every attempt. Written for a Node project with a lockfile; a
`--setup` given by hand gets none, since it may make something else.

Every key can be edited; `--check` and `--setup` replace what was detected,
and an existing config is never overwritten without `--force`. With a
config, `mine` needs no flags, and every command finds its tasks in
`<private_checks>/tasks` and the hidden tests in `<private_checks>/hidden`,
outside the checkout a worker works in.

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

The worker's prompt is the fix commit's message, plus two things a real
issue would say:

- **The interface the check needs.** Hidden tests often import a module the
  fix created or a function it exported — names the commit message never
  mentions, and no worker can guess. The prompt lists the module paths and
  exported names the hidden tests import that do not exist at the base;
  never a test, an assertion or an expected value. On the Watchman
  benchmark 20 of 38 tasks needed this; without it every pilot attempt
  failed on a missing name. `insi brief` prints what each task's worker is
  told, read from the base with `git show`.
- **That nobody will answer.** A worker that asked a question waited out
  its whole budget for a reply; the prompt now says to decide, make the
  change and stop.

The full prompt is kept with the attempt as `prompt.txt`.

## Lessons

Each worker attempt keeps what it left behind under `<work>/attempts/<name>/`:
the worker's diff, its output, the check's output.

`insi learn` takes every failed attempt on a fair task — whose fix is
therefore verified: the reference commit passed the check — and asks the
worker for one lesson from the attempt, the failure and the fix: an issue, a
root cause, an action to **use** and one to **avoid**. A lesson is refused if
a field is empty, longer than a few sentences, or holds a code block. Lessons
are kept in `<work>/lessons.jsonl` (or `--lessons <file>`), one per attempt.

With `--to-memory`, each lesson is also written to the local Incord Memory
engine (`INCORD_MEMORY_ADDR`, default `127.0.0.1:9180`; nothing off this
machine) as an `issue` item filed under the repository it came from, tagged
`insi` and `verified:check`, so every agent on the machine is briefed with it.
Leave it off for a benchmark: an experiment's lessons belong in its own file,
where they can be reset.

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

## An issue, worked into a draft pull request: `insi issue`

```sh
insi issue --number 42            # from the repository's folder
```

- **Only a labelled issue is worked.** It must be open and carry the label
  named by `trusted_issue_label` in `.insi/config.toml`. Anyone can write an
  issue; only a maintainer can label one, so the label is the decision that
  an agent may take it. An unlabelled issue is refused before its text is
  given to anything.
- **Its own branch.** `insi/issue-<n>`, from the default branch on `origin`,
  in a worktree under the work folder. A branch that already exists is
  refused: the issue was worked before, and that branch is a person's to read
  first.
- **Nothing leaves without the check.** The worker's change is committed, then
  the repository's own `test_cmd` runs. A failing check keeps the commit on
  the local branch and pushes nothing. So does anything credential-shaped in
  the lines the branch adds (a prefixed key, a cloud key id, a JWT, a
  private-key block, a bearer token, or a file such as `.env` or `id_rsa`).
- **A draft, with its evidence.** `Closes #<n>`, the check that passed, the
  worker, its time and tokens, whether lessons were briefed. `--ready` opens
  it ready for review instead. Insi never merges.

GitHub is reached through the `gh` CLI, which holds the login: Insi reads,
holds and sends no token.

## Lessons for any agent: `insi mcp`

`insi mcp` serves the lessons over MCP on stdin/stdout, so an agent Insi did
not start can use them too. It only reads: nothing an agent sends writes a
lesson or runs anything.

| Tool | What it answers |
|---|---|
| `insi_briefing {topic, before?, max_chars?}` | the briefing for a repository: verified lessons, newest first, as actions |
| `insi_lessons_for_error {error, topic?}` | lessons whose failure looked like this one: the error is normalized (paths, numbers, colour codes gone) and matched line by line |

`insi init` registers it for the repository: a missing `.mcp.json` is written
with the entry `claude mcp add --scope project insi -- insi mcp` would add,
which reads `.insi-work/lessons.jsonl`. An existing `.mcp.json` is never
rewritten; init prints that command instead. Anywhere else, register it like
any stdio MCP server, e.g.
`{"command": "insi", "args": ["mcp", "--lessons", "/path/to/lessons.jsonl"]}`.

## Lessons when they apply: `insi hook`

The briefing comes once, before a task. `insi hook` is a Claude Code hook
that brings a lesson back at the step it is about:

| Event | When | What comes back |
|---|---|---|
| `PreToolUse` (`Edit`, `Write`, `MultiEdit`, `NotebookEdit`) | before a file is changed | lessons that name that file, by its path in the repository or its file name |
| `PostToolUseFailure` (`Bash`) | after a command fails | lessons whose failure signature matches the output, as `insi_lessons_for_error` |

A failed command raises `PostToolUseFailure`, not `PostToolUse` (measured on
Claude Code 2.1.288). The answer is `additionalContext`, so the hook never
blocks, allows or changes a call; at most two lessons at a time, and each one
once per session (the ids shown are kept in `.insi-work/hook-shown/`).
Anything it cannot read is no answer and exit 0, with the reason on stderr.

`insi init` writes it into a missing `.claude/settings.json`; an existing one
is left alone and init prints the block to merge in.

## Commands

```text
insi mine     --source <repo> --tasks <dir> --private <dir> --check "<cmd {tests}>"
              [--setup "<cmd>"] [--prefix <name>] [--limit N] [--wall-secs N]
insi validate --tasks <dir> --private <dir> [--only id,id] [--half older|newer] [--results <file>] [--jobs N]
insi eval     --tasks <dir> --private <dir> [--seeds N] [--tag k=v] [--worker "<cmd {prompt}>"]
              [--lessons <file>] [--fair-only] [--only id,id] [--half older|newer]
              [--results <file>] [--keep] [--jobs N]
insi learn    --tasks <dir> --private <dir> [--lessons <file>] [--worker "<cmd {prompt}>"] [--to-memory]
              [--results <file>] [--jobs N]
insi report   [--results <file>]
```

Every command takes `--work <dir>` (default `.insi-work`): Insi's clones,
worktrees and the results file (`results.jsonl`, one line per attempt,
appended). `--jobs N` runs N attempts at once. `--setup-jobs N` (default 1)
lets only N pieces of bulk disk work run at once — a setup, or deleting a
finished worktree. Both are bound by the disk: on Watchman, whole attempts
run one after another took 145–205s, while with three installs at once a
single setup took about 1,060s. Workers and checks still overlap.

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

# US dollars per million tokens, by the model name the worker reports.
# A model on a subscription is an entry with no keys (all zero).
[prices."claude-sonnet-5-5"]
input = 3.0
output = 15.0
cache_write = 3.75
cache_read = 0.3
```

Any agent with a non-interactive mode can be the worker. The model and token
counts are read from incode's `stream-json` result; another worker's attempts
are timed but not counted in tokens.

Each attempt records `cost_usd` from `[prices]`, and `insi report` sums it
per condition. `insi eval --max-dollars D` starts no new attempt once the
run has spent D; an attempt already running is bounded by its wall time, so
the run can end above D by at most the attempts in flight. An attempt whose
cost is not known — no usage reported, or no price for its model — stops a
capped run rather than counting as free. (The prices above are an example
of the format; set the ones you pay.)

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
