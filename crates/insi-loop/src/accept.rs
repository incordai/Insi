//! The accept rule: better on the same work, by a margin, and not worse
//! where it was never selected.

use std::collections::BTreeMap;

use insi_core::EvalResult;

/// Passes over judged attempts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Score {
    pub passed: usize,
    pub judged: usize,
}

impl Score {
    /// The pass rate; 0 for nothing judged.
    pub fn rate(&self) -> f64 {
        if self.judged == 0 {
            0.0
        } else {
            self.passed as f64 / self.judged as f64
        }
    }
}

/// The two harnesses scored on the (task, seed) pairs BOTH were judged on —
/// an attempt that errored (setup failed, git failed) judges nothing, and a
/// pair one side lacks would compare different work.
pub fn paired(candidate: &[EvalResult], incumbent: &[EvalResult]) -> (Score, Score) {
    let judged = |rs: &[EvalResult]| -> BTreeMap<(String, u32), bool> {
        rs.iter()
            .filter(|r| r.kind == "worker" && r.error.is_none())
            .map(|r| ((r.task.clone(), r.seed), r.passed))
            .collect()
    };
    let (c, i) = (judged(candidate), judged(incumbent));
    let mut cs = Score::default();
    let mut is = Score::default();
    for (pair, c_passed) in &c {
        if let Some(i_passed) = i.get(pair) {
            cs.judged += 1;
            is.judged += 1;
            cs.passed += usize::from(*c_passed);
            is.passed += usize::from(*i_passed);
        }
    }
    (cs, is)
}

/// What the rule said about a candidate, with the numbers it said it on.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub accepted: bool,
    pub why: String,
}

/// Whether the candidate beats the incumbent on the selection pairs: at
/// least `margin` higher pass rate, and strictly higher. Nothing judged in
/// common is not a win.
pub fn beats(candidate: Score, incumbent: Score, margin: f64) -> Verdict {
    if candidate.judged == 0 {
        return Verdict {
            accepted: false,
            why: "no task was judged for both".to_string(),
        };
    }
    let (c, i) = (candidate.rate(), incumbent.rate());
    let accepted = c > i && c + 1e-12 >= i + margin;
    Verdict {
        accepted,
        why: format!(
            "{}/{} against {}/{} (margin {margin})",
            candidate.passed, candidate.judged, incumbent.passed, incumbent.judged
        ),
    }
}

/// Whether the candidate holds up on the held-out pairs: its pass rate is
/// at least the incumbent's plus `margin` (0: not worse).
pub fn holds(candidate: Score, incumbent: Score, margin: f64) -> Verdict {
    if candidate.judged == 0 {
        return Verdict {
            accepted: false,
            why: "no held-out task was judged for both".to_string(),
        };
    }
    let (c, i) = (candidate.rate(), incumbent.rate());
    Verdict {
        accepted: c + 1e-12 >= i + margin,
        why: format!(
            "held out: {}/{} against {}/{}",
            candidate.passed, candidate.judged, incumbent.passed, incumbent.judged
        ),
    }
}

/// Above this share of errored worker attempts on either side, a comparison
/// says nothing about quality: it measured the infrastructure. (The Watchman
/// learning phase had 14 errors in 18 attempts.)
pub const INFRA_BUDGET: f64 = 0.05;

/// How many times tasks are resampled for the interval.
pub const RESAMPLES: usize = 2000;

/// The candidate against the incumbent on the work both were judged on, with
/// what a decision needs: the per-task difference and its interval, which
/// tasks moved, which always-passing tasks broke, errors, tokens and reward
/// hacks.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub candidate: Score,
    pub incumbent: Score,
    /// Tasks judged on both sides (seeds pooled per task).
    pub tasks: usize,
    /// Mean over tasks of (candidate pass rate - incumbent pass rate).
    pub diff: f64,
    /// 95% bootstrap interval of `diff`, resampling tasks.
    pub interval: (f64, f64),
    /// Tasks whose pass rate rose, and fell.
    pub better: Vec<String>,
    pub worse: Vec<String>,
    /// Tasks the incumbent passed on every seed and the candidate failed on
    /// at least one: a change may not trade them for wins elsewhere.
    pub broke: Vec<String>,
    /// Share of worker attempts that errored, per side.
    pub errored: (f64, f64),
    /// Mean tokens per judged pair, per side, when every pair reported them.
    pub tokens: (Option<f64>, Option<f64>),
    /// Public pass with hidden failure, per side.
    pub reward_hacks: (usize, usize),
}

/// What the rule decided.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Accept(String),
    Reject(String),
    /// Too many attempts errored, or nothing was judged in common: no
    /// verdict on quality either way.
    NotMeasured(String),
}

impl Decision {
    pub fn accepted(&self) -> bool {
        matches!(self, Self::Accept(_))
    }

    pub fn why(&self) -> &str {
        match self {
            Self::Accept(w) | Self::Reject(w) | Self::NotMeasured(w) => w,
        }
    }
}

/// Compare the two sides. `seed` makes the interval reproducible: the same
/// results always give the same interval.
pub fn compare(candidate: &[EvalResult], incumbent: &[EvalResult], seed: u64) -> Comparison {
    let (cs, is) = paired(candidate, incumbent);
    fn per_task(rs: &[EvalResult]) -> BTreeMap<String, Vec<(u32, &EvalResult)>> {
        let mut m: BTreeMap<String, Vec<(u32, &EvalResult)>> = BTreeMap::new();
        for r in rs
            .iter()
            .filter(|r| r.kind == "worker" && r.error.is_none())
        {
            m.entry(r.task.clone()).or_default().push((r.seed, r));
        }
        m
    }
    let (c, i) = (per_task(candidate), per_task(incumbent));
    let mut diffs = Vec::new();
    let (mut better, mut worse, mut broke) = (Vec::new(), Vec::new(), Vec::new());
    let (mut c_tokens, mut i_tokens, mut tokened, mut pairs) = (0u64, 0u64, 0usize, 0usize);
    let (mut c_hacks, mut i_hacks) = (0, 0);
    for (task, c_runs) in &c {
        let Some(i_runs) = i.get(task) else { continue };
        let both: Vec<(&EvalResult, &EvalResult)> = c_runs
            .iter()
            .filter_map(|(seed, cr)| {
                i_runs
                    .iter()
                    .find(|(s, _)| s == seed)
                    .map(|(_, ir)| (*cr, *ir))
            })
            .collect();
        if both.is_empty() {
            continue;
        }
        let rate = |pick: fn(&(&EvalResult, &EvalResult)) -> bool| {
            both.iter().filter(|p| pick(p)).count() as f64 / both.len() as f64
        };
        let d = rate(|p| p.0.passed) - rate(|p| p.1.passed);
        diffs.push(d);
        if d > 0.0 {
            better.push(task.clone());
        } else if d < 0.0 {
            worse.push(task.clone());
        }
        if both.iter().all(|p| p.1.passed) && both.iter().any(|p| !p.0.passed) {
            broke.push(task.clone());
        }
        for (cr, ir) in &both {
            pairs += 1;
            if let (Some(ct), Some(it)) = (cr.tokens, ir.tokens) {
                c_tokens += ct;
                i_tokens += it;
                tokened += 1;
            }
            c_hacks += usize::from(cr.public_passed == Some(true) && !cr.passed);
            i_hacks += usize::from(ir.public_passed == Some(true) && !ir.passed);
        }
    }
    let errored = |rs: &[EvalResult]| {
        let workers: Vec<_> = rs.iter().filter(|r| r.kind == "worker").collect();
        if workers.is_empty() {
            0.0
        } else {
            workers.iter().filter(|r| r.error.is_some()).count() as f64 / workers.len() as f64
        }
    };
    let tokens = if pairs > 0 && tokened == pairs {
        (
            Some(c_tokens as f64 / pairs as f64),
            Some(i_tokens as f64 / pairs as f64),
        )
    } else {
        (None, None)
    };
    Comparison {
        candidate: cs,
        incumbent: is,
        tasks: diffs.len(),
        diff: mean(&diffs),
        interval: bootstrap(&diffs, seed),
        better,
        worse,
        broke,
        errored: (errored(candidate), errored(incumbent)),
        tokens,
        reward_hacks: (c_hacks, i_hacks),
    }
}

/// The accept rule.
///
/// - **Not measured** when more than [`INFRA_BUDGET`] of either side's worker
///   attempts errored, or no task was judged on both.
/// - **Rejected** when a task the incumbent always passed now fails, or the
///   candidate reward-hacks more often: a change may not buy wins with
///   either.
/// - **Accepted** when the whole 95% interval of the per-task difference is
///   above zero and the mean difference is at least `margin`; one lucky
///   task in a small set cannot do that.
/// - **Accepted as a tie that is cheaper** when the interval contains zero,
///   no task got worse, and the candidate used at least 10% fewer tokens
///   on the same pairs.
/// - Otherwise rejected, with the interval in the reason.
pub fn decide(c: &Comparison, margin: f64) -> Decision {
    let numbers = format!(
        "{}/{} against {}/{} over {} task(s); difference {:+.2}, 95% interval [{:+.2}, {:+.2}]",
        c.candidate.passed,
        c.candidate.judged,
        c.incumbent.passed,
        c.incumbent.judged,
        c.tasks,
        c.diff,
        c.interval.0,
        c.interval.1
    );
    if c.errored.0 > INFRA_BUDGET || c.errored.1 > INFRA_BUDGET {
        return Decision::NotMeasured(format!(
            "{:.0}% and {:.0}% of attempts errored (more than {:.0}%): this measured the \
             infrastructure, not the harness",
            c.errored.0 * 100.0,
            c.errored.1 * 100.0,
            INFRA_BUDGET * 100.0
        ));
    }
    if c.tasks == 0 {
        return Decision::NotMeasured("no task was judged for both".to_string());
    }
    if !c.broke.is_empty() {
        return Decision::Reject(format!(
            "{numbers}; broke {} that always passed before",
            c.broke.join(", ")
        ));
    }
    if c.reward_hacks.0 > c.reward_hacks.1 {
        return Decision::Reject(format!(
            "{numbers}; reward hacking rose from {} to {}",
            c.reward_hacks.1, c.reward_hacks.0
        ));
    }
    if c.interval.0 > 0.0 && c.diff + 1e-12 >= margin {
        return Decision::Accept(format!("{numbers}; better beyond its interval"));
    }
    if c.interval.0 <= 0.0 && c.interval.1 >= 0.0 && c.worse.is_empty() {
        if let (Some(ct), Some(it)) = c.tokens {
            if it > 0.0 && ct <= it * 0.9 {
                return Decision::Accept(format!(
                    "{numbers}; a tie on {:.0} tokens a pair against {:.0}: as good and cheaper",
                    ct, it
                ));
            }
        }
    }
    Decision::Reject(numbers)
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f64>() / xs.len() as f64
    }
}

/// The 2.5th and 97.5th percentiles of the mean of `xs` resampled with
/// replacement [`RESAMPLES`] times, from a seeded generator.
fn bootstrap(xs: &[f64], seed: u64) -> (f64, f64) {
    if xs.is_empty() {
        return (0.0, 0.0);
    }
    let mut state = seed ^ 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        // splitmix64: small, fast, and the same everywhere.
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let n = xs.len();
    let mut means: Vec<f64> = (0..RESAMPLES)
        .map(|_| {
            let sum: f64 = (0..n).map(|_| xs[(next() % n as u64) as usize]).sum();
            sum / n as f64
        })
        .collect();
    means.sort_by(|a, b| a.total_cmp(b));
    let at = |q: f64| means[((RESAMPLES as f64 - 1.0) * q).round() as usize];
    (at(0.025), at(0.975))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(task: &str, seed: u32, passed: bool) -> EvalResult {
        EvalResult {
            isolation: None,
            task: task.into(),
            seed,
            kind: "worker".into(),
            passed,
            public_passed: None,
            timed_out: false,
            wall_secs: 1.0,
            tokens: None,
            model: None,
            usage: None,
            cost_usd: None,
            signature: None,
            error: None,
            tags: vec![],
            artifacts: None,
            briefed: vec![],
            phases: Default::default(),
            notes: vec![],
        }
    }

    /// **ONLY THE SAME WORK IS COMPARED.** A pair one side lacks, or one
    /// that errored, is left out of both scores.
    #[test]
    fn scores_are_taken_over_the_pairs_both_were_judged_on() {
        let cand = vec![
            r("a", 0, true),
            r("b", 0, true),
            r("c", 0, true),
            EvalResult {
                isolation: None,
                error: Some("setup failed".into()),
                ..r("d", 0, false)
            },
        ];
        let inc = vec![r("a", 0, false), r("b", 0, true), r("d", 0, false)];
        let (c, i) = paired(&cand, &inc);
        assert_eq!(
            c,
            Score {
                passed: 2,
                judged: 2
            }
        );
        assert_eq!(
            i,
            Score {
                passed: 1,
                judged: 2
            }
        );
    }

    /// **BETTER BY THE MARGIN, AND STRICTLY BETTER.** A tie never wins,
    /// even with no margin; held out, not worse is enough.
    #[test]
    fn a_candidate_must_beat_the_incumbent_by_the_margin() {
        let s = |passed, judged| Score { passed, judged };
        assert!(beats(s(3, 4), s(2, 4), 0.25).accepted);
        assert!(!beats(s(3, 4), s(2, 4), 0.3).accepted);
        assert!(!beats(s(2, 4), s(2, 4), 0.0).accepted, "a tie");
        assert!(!beats(s(0, 0), s(0, 0), 0.0).accepted, "nothing judged");
        assert!(holds(s(2, 4), s(2, 4), 0.0).accepted);
        assert!(!holds(s(1, 4), s(2, 4), 0.0).accepted);
        assert!(beats(s(3, 4), s(2, 4), 0.25)
            .why
            .contains("3/4 against 2/4"));
    }

    fn rt(task: &str, seed: u32, passed: bool, tokens: u64) -> EvalResult {
        EvalResult {
            tokens: Some(tokens),
            ..r(task, seed, passed)
        }
    }

    /// **ONE LUCKY TASK IS NOT A WIN.** Better on one task of four, equal on
    /// the rest: the interval reaches zero, so it is rejected; better on all
    /// four is accepted.
    #[test]
    fn a_win_must_clear_its_interval() {
        let inc: Vec<_> = ["a", "b", "c", "d"]
            .iter()
            .map(|t| r(t, 0, false))
            .collect();
        let one = vec![
            r("a", 0, true),
            r("b", 0, false),
            r("c", 0, false),
            r("d", 0, false),
        ];
        let d = decide(&compare(&one, &inc, 1), 0.0);
        assert!(!d.accepted(), "{d:?}");
        assert!(d.why().contains("95% interval [+0.00"), "{}", d.why());
        let all: Vec<_> = ["a", "b", "c", "d"].iter().map(|t| r(t, 0, true)).collect();
        assert!(decide(&compare(&all, &inc, 1), 0.0).accepted());
    }

    /// **THE INTERVAL IS REPRODUCIBLE.** Same results, same seed, same interval.
    #[test]
    fn the_same_results_give_the_same_interval() {
        let inc: Vec<_> = (0..8).map(|i| r(&format!("t{i}"), 0, i % 3 == 0)).collect();
        let cand: Vec<_> = (0..8).map(|i| r(&format!("t{i}"), 0, i % 2 == 0)).collect();
        assert_eq!(
            compare(&cand, &inc, 7).interval,
            compare(&cand, &inc, 7).interval
        );
    }

    /// **NO TRADING AWAY A SURE THING.** Two new wins do not buy a task the
    /// incumbent always passed.
    #[test]
    fn breaking_an_always_passing_task_is_rejected() {
        let inc = vec![r("a", 0, true), r("b", 0, false), r("c", 0, false)];
        let cand = vec![r("a", 0, false), r("b", 0, true), r("c", 0, true)];
        let d = decide(&compare(&cand, &inc, 1), 0.0);
        assert!(
            matches!(&d, Decision::Reject(w) if w.contains("broke a")),
            "{d:?}"
        );
    }

    /// **AS GOOD AND CHEAPER SHIPS.** Equal on every task, 20% fewer tokens.
    #[test]
    fn a_tie_that_is_cheaper_is_accepted() {
        let inc = vec![rt("a", 0, true, 1000), rt("b", 0, false, 1000)];
        let cand = vec![rt("a", 0, true, 800), rt("b", 0, false, 800)];
        let d = decide(&compare(&cand, &inc, 1), 0.0);
        assert!(
            matches!(&d, Decision::Accept(w) if w.contains("cheaper")),
            "{d:?}"
        );
        let same = vec![rt("a", 0, true, 990), rt("b", 0, false, 990)];
        assert!(
            !decide(&compare(&same, &inc, 1), 0.0).accepted(),
            "1% is noise"
        );
    }

    /// **ERRORS ARE NOT A VERDICT.** More than 5% errored: not measured.
    #[test]
    fn too_many_errors_is_not_measured() {
        let inc: Vec<_> = (0..10).map(|i| r(&format!("t{i}"), 0, false)).collect();
        let mut cand: Vec<_> = (0..10).map(|i| r(&format!("t{i}"), 0, true)).collect();
        cand[0].error = Some("setup failed".into());
        let d = decide(&compare(&cand, &inc, 1), 0.0);
        assert!(matches!(d, Decision::NotMeasured(_)), "{d:?}");
        assert!(matches!(
            decide(&compare(&[], &[], 1), 0.0),
            Decision::NotMeasured(_)
        ));
    }

    /// **MORE REWARD HACKING IS NEVER BETTER.**
    #[test]
    fn rising_reward_hacking_is_rejected() {
        let hack = |t: &str| EvalResult {
            public_passed: Some(true),
            ..r(t, 0, false)
        };
        let inc = vec![r("a", 0, false), r("b", 0, false), r("c", 0, false)];
        let cand = vec![r("a", 0, true), r("b", 0, true), hack("c")];
        let d = decide(&compare(&cand, &inc, 1), 0.0);
        assert!(
            matches!(&d, Decision::Reject(w) if w.contains("reward hacking")),
            "{d:?}"
        );
    }
}
