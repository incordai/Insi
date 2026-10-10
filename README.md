# Insi <img width="256" height="256" alt="spinner-transparent" src="https://github.com/user-attachments/assets/5548318d-30b0-4d60-a3a7-97269dbb5d61" />


**Does your coding agent actually get better at your repository? Insi
measures it, and then works to make it so, with a person deciding what
lands.**

Insi is a command-line tool (one Rust binary) you add to any git repository.
It turns the repository's own history into a benchmark, runs a coding agent
against it in throwaway checkouts, and judges every attempt with tests the
agent never saw. On top of that measurement it builds two ways for the agent
to improve:

- **Lessons.** After a failed attempt, the agent writes down what went wrong
  and what to do instead. Later tasks are briefed with those lessons.
- **The improvement loop.** The agent's prompts and settings (its *harness*)
  are changed one file at a time, and a change is kept only when the
  benchmark says it is better.

Nothing changes itself silently. An improvement reaches the repository as a
**draft pull request** that a person reviews and merges.

Why it exists: the claim "my agent learns" is usually an anecdote. Insi's rule
is that an improvement counts only if it raises the pass rate on tasks it was
not learned from, measured against a control.

- [How it works](#how-it-works)
- [Quick start](#quick-start)
- [Benchmark results](#benchmark-results)
- [Reference](#adding-insi-to-a-repository): every command in detail
- [Contributing](#contributing) · [License](#license)

## How it works

```text
 the repository's history
        │
        │  insi mine       every fix commit that also changed a test file becomes a task:
        │                    start         = the commit before the fix
        │                    prompt        = the fix commit's message
        │                    hidden check  = the tests the fix added, kept outside the checkout
        ▼
   tasks
        │  insi validate   two controls, no agent: the hidden check must FAIL at the
        │                  start and PASS at the fix, or the task is not a fair test
        ▼
   fair tasks
        │  insi eval       the agent works each task in its own throwaway worktree;
        │                  the hidden tests are copied in only after it has stopped
        ▼
   results.jsonl  ──────►  insi report: pass rate, timeouts, repeated failures,
        │                  cost, and reward hacking per condition
        │
        │  insi learn      a failed attempt + its verified fix → one lesson
        │                  (the issue, the root cause, what to use, what to avoid)
        ▼
   lessons  ─────────────► briefed before later tasks, only from tasks dated earlier
        │
        │  insi improve    change the harness one file at a time, keep only what wins
        ▼
   draft pull request ───► a person reviews it; Insi never merges
```

Four rules keep the measurement honest:

1. **The agent never sees the check.** Hidden tests live outside every
   checkout the agent can read. They are copied in after the agent stops,
   replacing anything it wrote at those paths.
2. **Only fair tasks count.** A task whose check already passes at the start,
   or fails even with the real fix, is dropped by `insi validate`.
3. **Learn on the past, measure on the future.** Tasks are split by commit
   date. Lessons are learned on the older half and measured on the newer
   half, and a lesson is never briefed to the task it came from or to
   anything earlier. Conditions are compared on the same task × seed pairs.
4. **Visible score vs real score.** When a task carries a public check (the
   repository's own test command), Insi records whether it passed. A public
   pass with a hidden failure is counted as reward hacking: the visible score
   was met without the work.

The agent is anything with a non-interactive mode: Insi runs it through a
worker command (`[worker] command`, or `--worker "<cmd {prompt}>"`). The
default is incode.

## Quick start

Insi is built from source: the npm and pip packages are ready but not yet
published ([RELEASING.md](RELEASING.md)). You need Rust 1.89 or newer and git.

```sh
git clone https://github.com/incordai/Insi.git
cd Insi
cargo build --release          # the binary is target/release/insi
```

In the repository you want to measure:

```sh
insi init                  # detect the stack, write .insi/config.toml
insi mine                  # tasks from the history; hidden tests go outside the checkout
insi validate              # which tasks are fair tests
insi eval --fair-only      # run the agent on the fair tasks
insi report                # pass rate and the rest, per condition
```

To measure whether lessons help:

```sh
insi eval  --fair-only --half older --tag phase=learn    # attempts to learn from
insi learn                                               # lessons from their failures
insi eval  --fair-only --half newer --seeds 3            # lessons off
insi eval  --fair-only --half newer --seeds 3 --lessons .insi-work/lessons.jsonl
insi report
```

## Benchmark results

### Watchman, run 2 (9 October 2026)

The first benchmark is the history of Watchman, a desktop application in
TypeScript and Rust whose tasks are checked with vitest:
**38 tasks mined, 35 fair** (3 already passed at the start and were dropped).
Lessons were learned on the older half and measured on the newer half. One
seed, the worker incode on a single model, a budget of 1,800 seconds per
attempt.

| Half | Condition | Passed | Timed out |
|---|---|---|---|
| older (learning) | lessons off | 3 / 18 (17%) | 6 |
| newer (measured) | lessons off | 5 / 16 (31%) | 6 |
| newer (measured) | lessons on | 6 / 17 (35%) | 3 |

The 11 lessons learned on the older half were briefed to the lessons-on round.
Over the 16 tasks judged both ways:

| Lessons off → on | Tasks |
|---|---|
| passed both times | 5 |
| failed → passed | 1 |
| passed → failed | 0 |
| failed both times | 10 |

**What this does and does not show.** One task changed outcome, so this is
not evidence that lessons help. It is one seed, and three things limit it:

- Every lessons-on attempt was briefed with the same three newest lessons.
  All 11 lessons sit on one topic and fill the 4,000-character briefing, so
  this measured three general lessons, not the relevant ones.
- 4 of 15 lessons were lost to bugs found in this run, since fixed.
- Wall time and tokens are not comparable between rounds: the lessons-off
  round ran on a heavily loaded machine, and an attempt cut off at its
  budget reports no tokens.

Reward hacking was not measured: these tasks carry no public check yet.

Next: a briefing ranked by relevance to the task, the lost lessons recovered,
and three seeds.

The report as `insi report` printed it:

```text
lessons=off,phase=learn:   3/18 passed (17%), 0 repeat failure(s), 6 timed out, 14 error(s)
lessons=off,phase=measure: 5/16 passed (31%), 0 repeat failure(s), 6 timed out, 1 error(s)
lessons=on,phase=measure:  6/17 passed (35%), 0 repeat failure(s), 3 timed out, 0 error(s)
```

## Adding Insi to a repository

```sh
cd your-repo
insi init       # writes .insi/config.toml, ignores .insi-work/, registers insi mcp, hooks and CI
insi mine       # tasks from the history, hidden tests outside the checkout
insi validate   # which tasks are fair
insi eval --fair-only
```

`insi init` reads the files at the repository's root to tell how it is
installed and tested — a Node lockfile (npm, pnpm, yarn, bun) with vitest or
jest, Cargo, Python, Go — and writes what it found:

```toml
topic = "shop"
test_cmd = ["npm", "test"]
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

`test_cmd` is the repository's whole suite, and init also writes it into a
missing `.github/workflows/insi.yml`: on a pull request from an `insi/`
branch (the drafts `insi issue` opens), the stack's toolchain is installed,
`setup` runs, then `test_cmd`, with a read-only token that is not left in
the checkout for the code under test to read. A person reviews a
draft the repository's own CI has already checked. An existing workflow file
is left alone, and with no detected stack none is written. There is no
`evals/` folder to commit: a task's definition holds its reference commit,
which is the answer, so tasks stay in `private_checks`.

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

A task may also carry a **public check**: the repository's own test command
(`test_cmd`, or `insi mine --public-check`), the score a worker could see. It
runs on the tree exactly as the worker left it, before the hidden tests
arrive, and the attempt records `public_passed`; its output is kept as
`public.out`. A pass there with a hidden failure is reward hacking — the
visible score met without the work — and `insi report` counts it per
condition. Controls never run it, and one that cannot start is a note, never
the verdict.

Three attempts in a row that could not run at all (a git or setup failure,
not a failed check) stop `insi eval` and `insi improve`: a machine that keeps
failing would otherwise burn through every task. A git failure names its exit
code, in hex as well when it is a Windows status such as `0xC0000142`.

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
the lessons of the same repository within 4,000 characters, and **only
lessons learned from tasks dated before this one** — compared as instants,
so commit dates in different time zones order correctly. A lesson can never
be briefed to the task it came from, or to anything earlier.

The briefing is consolidated as of the task's date, never with hindsight:

- **One item per action.** Lessons whose `use` says the same thing (case,
  spacing and a final full stop aside) are one item, carrying all their ids.
- **Beliefs first.** An action learned twice or more is a belief, briefed as
  "Learned N times"; one seen once follows as "Seen when …". Newest next.
- **Superseded lessons drop out.** A later lesson whose `avoid` is an earlier
  lesson's `use` retires the earlier one.
- **Promotion.** An action learned in two or more sibling topics is briefed
  on their shared parent too, marked `[across <parent>]`.

`insi issue --lessons` and the MCP server's `insi_briefing` build the same
briefing; `insi hook` picks lessons by its own route (the file about to be
edited, the command that just failed) and is not consolidated this way.

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
  worker, its time and tokens, whether lessons were briefed, where it ran.
  `--ready` opens it ready for review instead. Insi never merges.
- **In a sandbox, when you name one.** The issue's text is anyone's, and the
  check runs the code the worker wrote. With a `[sandbox]` command, setup,
  the worker and the check each run inside it, with `{tree}` as the
  worktree's path:

  ```toml
  [sandbox]
  command = ["docker", "run", "--rm", "-v", "{tree}:/work", "-w", "/work", "node:22"]
  ```

  The container gets only what the command gives it: add `-e` for the one
  model key the worker needs, `--network none` where the steps need no
  network. Committing, the scan and the push stay outside it, on this
  machine. A sandbox with no `{tree}` is refused.

GitHub is reached through the `gh` CLI, which holds the login: Insi reads,
holds and sends no token.

## Lessons for any agent: `insi mcp`

`insi mcp` serves the lessons over MCP on stdin/stdout, so an agent Insi did
not start can use them too. It only reads: nothing an agent sends writes a
lesson or runs anything.

| Tool | What it answers |
|---|---|
| `insi_briefing {topic, before?, max_chars?}` | the briefing for a repository: verified lessons consolidated as for `insi eval` (beliefs first), as actions |
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

## Outside context: `insi context`

```sh
insi context --topic shop/api --query "refund rounding"
```

What the repository's outside sources say about one part of it, for an agent
or a person. The MCP tool `insi_context` answers the same. Every item says
where it came from, how far it is trusted, and its topic. Only items on the
asked topic's line of descent come back, ordered by the words asked for,
bounded in number and length.

| Source | Trust | What |
| --- | --- | --- |
| `docs` | trusted | `README.md` and `docs/**/*.md`, one item per section, each on its file's topic |
| `ci` | internal | the last failed GitHub Actions runs through `gh`, the end of their failed steps' log, on the topic of the repository files the log names (`--no-ci` leaves them out) |
| `[[connectors]]` | untrusted unless set | any command that prints items as JSON lines: an error tracker's export, a mailbox's |

```toml
[[connectors]]
name = "errors"
command = ["sentry-export", "--project", "shop", "--json-lines"]   # {"title", "body", "url"?, "paths"?, "topic"?} per line
trust = "untrusted"
```

The command holds whatever login it needs; Insi holds none and only reads
what it prints. A source that fails is named under "Not read" and the others
still answer. Untrusted text is shown inside a fence it cannot close from
within, marked as data: an email that says "ignore your rules" is something
to read, not something to do. Outside context is not put in `insi eval`'s
briefings, so a measurement depends only on its tasks.

## Topics: which lessons apply where

A lesson is filed on a topic, a `/`-separated path broadest first
(`shop` → `shop/api` → `shop/api/billing`), and applies to that topic and
every one below it. The repository's `topic` is the base; `[[topic_rules]]`
in `.insi/config.toml` name narrower ones by the files and commands they
cover. Resolution is by these rules only, never a model, and the same files
always give the same topic.

```toml
[[topic_rules]]
topic = "shop/api"
paths = ["src/api"]              # a folder covers everything under it; * and ? stay in one name, ** spans folders
commands = ["npm run test:api"]  # these words, side by side, anywhere in the command
```

- **`insi learn`** files each lesson on the narrowest topic covering every
  file its fix changed; files outside every rule leave it on the base.
- **The briefing** for a task holds its topic's lessons and those of the
  topics above it. A task starts on the repository's topic, so lessons filed
  narrower are not in it.
- **`insi hook`** brings those back when the agent edits a file in their
  part of the repository, and matches a failed command on the command's
  topic. In `insi eval` the worker gets the briefing only, so in a
  measurement only lessons on the base topic or above are used.

The most specific rule wins (deepest topic, then the first in the file); a
rule with no paths and no commands, or a topic with an empty part, is
refused when the config is read.

## The improvement loop: `insi improve`

The harness is a folder of prompts and settings a worker runs with; its
`prompt.md` is put before every task's prompt. `insi improve --harness <dir>`
changes it one file at a time and keeps a change only when the benchmark
says it is better:

1. A UCB1 bandit picks a strategy (`from-failures`, `clarify`, `simplify`,
   `checklist`, or the `[[strategies]]` in `.insi/config.toml`): each is
   tried once, then the ones whose changes were kept are tried more.
2. The proposer (`--proposer`, `[proposer]` in the config, else the worker)
   is shown the harness, each selection task's title and outcome, earlier
   proposals and the lessons, and answers with one file's whole new text and
   why. It is never shown a check's output: that is the hidden tests
   speaking, and a proposer that read them could write their answers into
   the harness. A path outside the harness, a hidden file, an oversized file
   or a change that changes nothing is refused before anything runs.
3. The candidate runs the older half of the fair tasks with the same worker,
   seeds and budget as the incumbent, each attempt in its own worktree with
   the check run there.
4. It is kept only when it beats the incumbent on the same task × seed pairs
   by `--margin` (a tie never wins) and is not worse on the newer half,
   which is never used to select (`--holdout-margin` to demand more).

Each attempt also runs with `INSI_HARNESS` set to that harness's own
snapshot folder, so the worker can read the rest of it. incode does:
`instructions.md` joins its system context, `roles.json`
(`{"roles": …, "tiers": …}`) routes its models over `admin.json` for that run
only, and `agents/*.md` are sub-agents between the project's and yours. A
harness file incode cannot read refuses the run instead of running on the
defaults.

Everything lands under `<work>/loop/`: every harness scored
(`harnesses/<id>/`, named by content, so a harness is never run twice on
the same task and seed), every attempt (`results.jsonl`), every state change
(`audit.jsonl`: baseline, proposed, refused, accepted, rejected, finished),
and the bandit (`bandit.json`). A second run continues the step count and
the bandit. Your harness folder is only read: the loop prints where the best
harness is, and putting it in place is yours to decide.

- `--pr` opens it as a **draft** pull request on the repository holding the
  harness folder, through `gh`'s login. Only the files the loop changed are
  written, in a fresh worktree of the default branch on
  `insi/harness-<id>`, so your uncommitted edits never ride along. The same
  credential and personal-data scan as `insi issue` runs first; a harness
  that fails it stays on a local branch and nothing is pushed. The PR lists
  each change kept with its benchmark verdict. A branch that exists already
  is refused: that harness was proposed before.
- `--to-memory` files each change kept, and the run's end, in the local
  Incord Memory engine as a `decision` (tagged `insi`, `loop:accepted` or
  `loop:finished`). Proposals and scores stay in `audit.jsonl`, the record;
  an engine that is down costs a printed line, never the run.

Unattended runs that open pull requests wait for the GitHub App with
short-lived, PR-only tokens; until then `--pr` uses the operator's login.

## Commands

```text
insi mine     --source <repo> --tasks <dir> --private <dir> --check "<cmd {tests}>"
              [--setup "<cmd>"] [--public-check "<cmd>"] [--prefix <name>] [--limit N] [--wall-secs N]
insi validate --tasks <dir> --private <dir> [--only id,id] [--half older|newer] [--results <file>] [--jobs N]
insi eval     --tasks <dir> --private <dir> [--seeds N] [--tag k=v] [--worker "<cmd {prompt}>"]
              [--lessons <file>] [--fair-only] [--only id,id] [--half older|newer]
              [--results <file>] [--keep] [--jobs N]
insi learn    --tasks <dir> --private <dir> [--lessons <file>] [--worker "<cmd {prompt}>"] [--to-memory]
              [--results <file>] [--jobs N]
insi improve  --harness <dir> --tasks <dir> --private <dir> [--steps N] [--seeds N] [--margin M]
              [--holdout-margin M] [--worker "<cmd {prompt}>"] [--proposer "<cmd {prompt}>"]
              [--only id,id] [--jobs N] [--max-dollars D] [--keep] [--to-memory] [--pr]
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
| `insi-loop` | the improvement loop's decisions: proposals, the accept rule, the strategy bandit, the audit trail |
| `insi-connectors` | outside sources: docs, CI logs and command connectors, each item with its source, trust and topic |
| `insi` | the command line |

## Building

```sh
cargo build --release
cargo test
```

## Contributing

Contributions are welcome: bug reports, fixes, new stack detection, and
benchmark results from your own repositories. Before opening a pull request,
run what CI runs on Windows, macOS and Linux:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for how changes are written, tested
and reviewed, and [RELEASING.md](RELEASING.md) for how a release is cut.

## License

Insi is released under the [MIT License](LICENSE).
