//! `insi improve`: the improvement loop over a harness.
//!
//! The harness is a folder of prompts and settings a worker runs with; its
//! `prompt.md` is put before every task's prompt. Each step:
//!
//! 1. The bandit picks a strategy ([`insi_loop::bandit`]).
//! 2. The proposer is shown the harness, how it scored, earlier proposals
//!    and the lessons, and answers with ONE file replaced
//!    ([`insi_loop::proposal`]). It is never shown a check's output: that is
//!    the hidden tests speaking, and a proposer that read them could write
//!    their answers into the harness.
//! 3. The candidate runs the selection tasks under the same budget, seeds
//!    and worker as the incumbent, each attempt in its own worktree with the
//!    check run there.
//! 4. It is kept only when it beats the incumbent on the same task × seed
//!    pairs by the margin, and is not worse on the held-out tasks
//!    ([`insi_loop::accept`]).
//!
//! The person's harness folder is only read. Every harness the loop scores
//! is kept under `<work>/loop/harnesses/<id>/`, every attempt in
//! `<work>/loop/results.jsonl` (a harness scored before is not run again),
//! every state change in `<work>/loop/audit.jsonl`, the bandit in
//! `<work>/loop/bandit.json`. The best harness found is reported; putting it
//! in place is a person's decision.

use std::path::{Path, PathBuf};
use std::time::Duration;

use insi_core::config::Strategy;
use insi_core::{EvalResult, Lesson, Task, Worker};
use insi_eval::run::{self, Attempt, Kind, Work};
use insi_eval::store::{self, Store};
use insi_eval::{pool, process};
use insi_loop::accept::{self, Score};
use insi_loop::audit::{self, Event};
use insi_loop::bandit::Bandit;
use insi_loop::harness::Harness;
use insi_loop::proposal;
use insi_memory::writer;

/// How long the proposer may take for one proposal.
pub const PROPOSE_SECS: u64 = 10 * 60;
/// The most of the harness shown to the proposer.
const HARNESS_CHARS: usize = 24_000;
/// How many earlier proposals and lessons the proposer is shown.
const HISTORY: usize = 10;

pub struct Improve<'a> {
    pub work: &'a Work,
    pub store: &'a Store,
    /// The tasks a candidate is selected on.
    pub select: &'a [Task],
    /// The tasks it must hold up on, never used to select.
    pub holdout: &'a [Task],
    pub worker: &'a Worker,
    pub proposer: &'a Worker,
    /// The person's harness folder: the starting incumbent, only read.
    pub harness: &'a Path,
    pub strategies: &'a [Strategy],
    pub lessons: &'a [Lesson],
    pub steps: u32,
    pub seeds: u32,
    pub margin: f64,
    pub holdout_margin: f64,
    pub jobs: usize,
    pub keep: bool,
}

/// Where the loop keeps its state.
pub struct LoopDir {
    pub dir: PathBuf,
}

impl LoopDir {
    pub fn new(work: &Work) -> LoopDir {
        LoopDir {
            dir: work.root.join("loop"),
        }
    }
    pub fn results(&self) -> PathBuf {
        self.dir.join("results.jsonl")
    }
    pub fn audit(&self) -> PathBuf {
        self.dir.join("audit.jsonl")
    }
    fn bandit(&self) -> PathBuf {
        self.dir.join("bandit.json")
    }
    pub fn harness(&self, id: &str) -> PathBuf {
        self.dir.join("harnesses").join(id)
    }
}

/// What a run of the loop ended with.
#[derive(Debug)]
pub struct Outcome {
    pub start: String,
    pub best: String,
    pub best_dir: PathBuf,
    pub accepted: u32,
    pub steps: u32,
}

/// Run the loop. `on_result` sees every attempt before it is saved (to
/// price it, and to stop the run at a spending cap by returning an error).
pub fn improve(
    l: &Improve,
    on_result: &mut dyn FnMut(&mut EvalResult) -> Result<(), String>,
) -> Result<Outcome, String> {
    if l.select.is_empty() || l.holdout.is_empty() {
        return Err("the loop needs selection tasks and held-out tasks".to_string());
    }
    let ld = LoopDir::new(l.work);
    let names: Vec<String> = l.strategies.iter().map(|s| s.name.clone()).collect();
    let mut bandit = read_bandit(&ld.bandit())?.with_arms(&names);
    let first_step = audit::read(&ld.audit())?
        .iter()
        .map(|e| e.step + 1)
        .max()
        .unwrap_or(1);

    let mut incumbent = Harness::read(l.harness)?;
    let start = incumbent.id();
    keep_snapshot(&ld, &incumbent)?;
    let mut inc_select = score(l, &ld, &incumbent, l.select, "select", on_result)?;
    let base = own_score(&inc_select);
    audit::append(
        &ld.audit(),
        &Event::new(
            first_step - 1,
            "baseline",
            format!("{}/{} on the selection tasks", base.passed, base.judged),
        )
        .harness(&start),
    )?;
    println!(
        "incumbent {start}: {}/{} on the selection tasks",
        base.passed, base.judged
    );

    let mut accepted = 0;
    for step in first_step..first_step + l.steps {
        let Some(arm) = bandit.pick() else {
            return Err("no strategies to propose with".to_string());
        };
        let strategy = &l.strategies[arm];
        let history = audit::read(&ld.audit())?;
        let prompt = proposer_prompt(
            strategy,
            &incumbent,
            l.select,
            &inc_select,
            &history,
            l.lessons,
        );
        let candidate = match propose(l, &ld, step, &prompt)
            .and_then(|answer| proposal::parse(&answer, &incumbent))
        {
            Ok(p) => {
                let cand = incumbent.with(&p.file, &p.content);
                keep_snapshot(&ld, &cand)?;
                audit::append(
                    &ld.audit(),
                    &Event::new(step, "proposed", format!("{}: {}", p.file, p.why))
                        .harness(&cand.id())
                        .strategy(&strategy.name),
                )?;
                println!("step {step} [{}]: {} — {}", strategy.name, p.file, p.why);
                cand
            }
            Err(why) => {
                audit::append(
                    &ld.audit(),
                    &Event::new(step, "refused", &why).strategy(&strategy.name),
                )?;
                println!("step {step} [{}]: no usable proposal: {why}", strategy.name);
                bandit.record(arm, 0.0);
                write_bandit(&ld.bandit(), &bandit)?;
                continue;
            }
        };

        let cand_select = score(l, &ld, &candidate, l.select, "select", on_result)?;
        let (cs, is) = accept::paired(&cand_select, &inc_select);
        let mut verdict = accept::beats(cs, is, l.margin);
        if verdict.accepted {
            let inc_hold = score(l, &ld, &incumbent, l.holdout, "holdout", on_result)?;
            let cand_hold = score(l, &ld, &candidate, l.holdout, "holdout", on_result)?;
            let (ch, ih) = accept::paired(&cand_hold, &inc_hold);
            let held = accept::holds(ch, ih, l.holdout_margin);
            verdict = accept::Verdict {
                accepted: held.accepted,
                why: format!("{}; {}", verdict.why, held.why),
            };
        }
        let event = if verdict.accepted {
            "accepted"
        } else {
            "rejected"
        };
        audit::append(
            &ld.audit(),
            &Event::new(step, event, &verdict.why)
                .harness(&candidate.id())
                .strategy(&strategy.name),
        )?;
        println!("step {step}: {event} ({})", verdict.why);
        bandit.record(arm, if verdict.accepted { 1.0 } else { 0.0 });
        write_bandit(&ld.bandit(), &bandit)?;
        if verdict.accepted {
            incumbent = candidate;
            inc_select = cand_select;
            accepted += 1;
        }
    }

    let best = incumbent.id();
    audit::append(
        &ld.audit(),
        &Event::new(
            first_step + l.steps - 1,
            "finished",
            format!(
                "{accepted} accepted of {} step(s); started at {start}",
                l.steps
            ),
        )
        .harness(&best),
    )?;
    Ok(Outcome {
        start,
        best_dir: ld.harness(&best),
        best,
        accepted,
        steps: l.steps,
    })
}

/// `harness`'s attempts on `tasks` × seeds, running only the pairs it has
/// not been judged on yet.
fn score(
    l: &Improve,
    ld: &LoopDir,
    harness: &Harness,
    tasks: &[Task],
    split: &str,
    on_result: &mut dyn FnMut(&mut EvalResult) -> Result<(), String>,
) -> Result<Vec<EvalResult>, String> {
    let tag = format!("harness={}", harness.id());
    let mine = |all: Vec<EvalResult>| -> Vec<EvalResult> {
        all.into_iter()
            .filter(|r| r.kind == "worker" && r.error.is_none() && r.tags.contains(&tag))
            .filter(|r| r.seed < l.seeds && tasks.iter().any(|t| t.id == r.task))
            .collect()
    };
    let have = mine(existing(&ld.results())?);
    let missing: Vec<(u32, &Task)> = (0..l.seeds)
        .flat_map(|seed| tasks.iter().map(move |t| (seed, t)))
        .filter(|(seed, t)| !have.iter().any(|r| r.task == t.id && r.seed == *seed))
        .collect();
    let preface = harness.prompt().to_string();
    let tags = vec![tag.clone(), format!("split={split}"), "loop".to_string()];
    pool::each(
        &missing,
        l.jobs,
        |(seed, task)| {
            run::attempt(
                l.work,
                l.store,
                &Attempt {
                    task,
                    kind: Kind::Worker,
                    seed: *seed,
                    worker: l.worker,
                    preface: &preface,
                    briefed: vec![],
                    tags: tags.clone(),
                    keep: l.keep,
                },
            )
        },
        |(seed, task), mut r| {
            on_result(&mut r)?;
            store::append_result(&ld.results(), &r)?;
            println!(
                "  {} {split} {} s{seed}: {}",
                &harness.id()[..8],
                task.id,
                match (&r.error, r.passed) {
                    (Some(e), _) => format!("ERROR {}", e.lines().next().unwrap_or_default()),
                    (None, true) => "pass".to_string(),
                    (None, false) => "fail".to_string(),
                }
            );
            Ok(())
        },
    )?;
    Ok(mine(existing(&ld.results())?))
}

/// One harness's own score, over every attempt it has.
fn own_score(results: &[EvalResult]) -> Score {
    Score {
        passed: results.iter().filter(|r| r.passed).count(),
        judged: results.len(),
    }
}

fn existing(file: &Path) -> Result<Vec<EvalResult>, String> {
    if file.exists() {
        store::read_results(file)
    } else {
        Ok(Vec::new())
    }
}

fn keep_snapshot(ld: &LoopDir, h: &Harness) -> Result<(), String> {
    let dir = ld.harness(&h.id());
    if dir.exists() {
        return Ok(());
    }
    h.write(&dir)
}

/// Ask the proposer; its final answer. It runs in a folder of its own: it
/// needs no checkout, and must not change one.
fn propose(l: &Improve, ld: &LoopDir, step: u32, prompt: &str) -> Result<String, String> {
    let room = ld.dir.join("propose").join(step.to_string());
    std::fs::create_dir_all(&room).map_err(|e| format!("{}: {e}", room.display()))?;
    std::fs::write(room.join("prompt.txt"), prompt)
        .map_err(|e| format!("{}: {e}", room.display()))?;
    let ran = process::run(
        &l.proposer.render(prompt)?,
        &room,
        Duration::from_secs(PROPOSE_SECS),
    )?;
    if !ran.success() {
        return Err(format!(
            "the proposer {}",
            if ran.timed_out {
                "timed out".to_string()
            } else {
                format!("exited {:?}", ran.code)
            }
        ));
    }
    writer::final_answer(&ran.stdout).ok_or_else(|| "the proposer answered nothing".to_string())
}

/// What the proposer is told. Task titles and outcomes, never a check's
/// output; the harness in full up to [`HARNESS_CHARS`].
pub fn proposer_prompt(
    strategy: &Strategy,
    harness: &Harness,
    tasks: &[Task],
    results: &[EvalResult],
    history: &[Event],
    lessons: &[Lesson],
) -> String {
    let mut out = String::from(
        "You improve the harness a coding agent works with. Its files are below; \
         `prompt.md` is put before every task the agent is given. Propose ONE change: \
         the whole new text of one file, existing or new.\n\n",
    );
    out.push_str(&format!(
        "Strategy for this change ({}): {}\n\n",
        strategy.name, strategy.prompt
    ));

    out.push_str(&format!("The harness now ({}):\n", harness.id()));
    if harness.files.is_empty() {
        out.push_str("(no files yet)\n");
    }
    let mut shown = 0;
    for (file, text) in &harness.files {
        if shown + text.len() > HARNESS_CHARS {
            out.push_str(&format!(
                "=== {file} === ({} bytes, not shown)\n",
                text.len()
            ));
            continue;
        }
        shown += text.len();
        out.push_str(&format!("=== {file} ===\n{text}\n"));
    }

    let s = own_score(results);
    out.push_str(&format!(
        "\nHow it did on the selection tasks: {}/{} attempts passed.\n",
        s.passed, s.judged
    ));
    for task in tasks {
        let mine: Vec<&EvalResult> = results.iter().filter(|r| r.task == task.id).collect();
        let passed = mine.iter().filter(|r| r.passed).count();
        let timed_out = mine.iter().filter(|r| r.timed_out).count();
        out.push_str(&format!(
            "- {}: {passed}/{} passed{} — {}\n",
            task.id,
            mine.len(),
            if timed_out > 0 {
                format!(", {timed_out} ran out of time")
            } else {
                String::new()
            },
            task.prompt.lines().next().unwrap_or_default()
        ));
    }

    let earlier: Vec<&Event> = history
        .iter()
        .filter(|e| {
            matches!(
                e.event.as_str(),
                "proposed" | "accepted" | "rejected" | "refused"
            )
        })
        .collect();
    if !earlier.is_empty() {
        out.push_str("\nEarlier steps, oldest first:\n");
        for e in earlier
            .iter()
            .skip(earlier.len().saturating_sub(HISTORY * 2))
        {
            out.push_str(&format!(
                "- step {} {}{}: {}\n",
                e.step,
                e.event,
                e.strategy
                    .as_deref()
                    .map(|s| format!(" [{s}]"))
                    .unwrap_or_default(),
                e.detail
            ));
        }
    }

    if !lessons.is_empty() {
        out.push_str("\nLessons from verified fixes in this repository:\n");
        for l in lessons.iter().rev().take(HISTORY) {
            out.push_str(&format!("- {}\n", l.use_.trim()));
        }
    }

    out.push_str(
        "\nAnswer with one JSON object and nothing else:\n\
         {\"file\": \"<path inside the harness, e.g. prompt.md>\", \
         \"content\": \"<the whole new text of that file>\", \
         \"why\": \"<one sentence: what this changes and why it should help>\"}\n",
    );
    out
}

fn read_bandit(file: &Path) -> Result<Bandit, String> {
    match std::fs::read_to_string(file) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Bandit::default()),
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

fn write_bandit(file: &Path, b: &Bandit) -> Result<(), String> {
    let text = serde_json::to_string_pretty(b).map_err(|e| e.to_string())?;
    std::fs::write(file, text).map_err(|e| format!("{}: {e}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **THE PROPOSER IS NEVER SHOWN WHAT THE HIDDEN TESTS SAID.** A failed
    /// attempt's signature (the check's output) stays out of the prompt; the
    /// task's title, its outcome and the harness go in.
    #[test]
    fn the_proposer_sees_outcomes_not_check_output() {
        let task = Task {
            id: "shop-1".into(),
            source: "s".into(),
            base: "a".into(),
            reference: "b".into(),
            date: None,
            prompt: "fix: refunds round down\n\nlonger body".into(),
            hidden_tests: vec!["t.test.ts".into()],
            check: vec!["x".into()],
            setup: vec![],
            setup_cache: None,
            budget: insi_core::Budget {
                wall_secs: 1,
                tokens: None,
            },
        };
        let failed = EvalResult {
            task: "shop-1".into(),
            seed: 0,
            kind: "worker".into(),
            passed: false,
            timed_out: true,
            wall_secs: 1.0,
            tokens: None,
            model: None,
            usage: None,
            cost_usd: None,
            signature: Some("Expected refund(5) to be 4.99 SECRET-EXPECTATION".into()),
            error: None,
            tags: vec![],
            artifacts: None,
            briefed: vec![],
            phases: Default::default(),
        };
        let strategy = &Strategy::defaults()[0];
        let h = Harness::default().with("prompt.md", "Be careful.");
        let text = proposer_prompt(strategy, &h, &[task], &[failed], &[], &[]);
        assert!(!text.contains("SECRET-EXPECTATION"), "{text}");
        assert!(text.contains("shop-1: 0/1 passed, 1 ran out of time — fix: refunds round down"));
        assert!(text.contains("=== prompt.md ===\nBe careful."));
        assert!(text.contains(&strategy.prompt));
    }
}
