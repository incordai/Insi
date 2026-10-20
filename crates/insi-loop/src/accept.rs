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
}
