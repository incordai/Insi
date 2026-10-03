//! UCB1 over named proposal strategies.
//!
//! Each step the loop asks the proposer for a change in the manner of one
//! strategy ("read the failures", "simplify"). UCB1 picks the strategy with
//! the highest `mean reward + sqrt(2 ln N / n)`: one that has paid off is
//! tried again, one tried little is still tried. Every strategy is tried
//! once before any is tried twice; ties go to the first named. No
//! randomness, so a run can be repeated.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Arm {
    pub name: String,
    pub pulls: u32,
    /// The sum of rewards, each in `0..=1`.
    pub reward: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Bandit {
    pub arms: Vec<Arm>,
}

impl Bandit {
    pub fn new(names: &[String]) -> Self {
        Bandit {
            arms: names
                .iter()
                .map(|n| Arm {
                    name: n.clone(),
                    pulls: 0,
                    reward: 0.0,
                })
                .collect(),
        }
    }

    /// Keep what was learned about strategies still named, add new ones,
    /// drop ones no longer named.
    pub fn with_arms(&self, names: &[String]) -> Self {
        Bandit {
            arms: names
                .iter()
                .map(|n| {
                    self.arms
                        .iter()
                        .find(|a| &a.name == n)
                        .cloned()
                        .unwrap_or(Arm {
                            name: n.clone(),
                            pulls: 0,
                            reward: 0.0,
                        })
                })
                .collect(),
        }
    }

    /// The arm to pull next; `None` with no arms.
    pub fn pick(&self) -> Option<usize> {
        if let Some(fresh) = self.arms.iter().position(|a| a.pulls == 0) {
            return Some(fresh);
        }
        let total: f64 = self.arms.iter().map(|a| f64::from(a.pulls)).sum();
        let mut best: Option<(usize, f64)> = None;
        for (i, a) in self.arms.iter().enumerate() {
            let n = f64::from(a.pulls);
            let value = a.reward / n + (2.0 * total.ln() / n).sqrt();
            if best.is_none_or(|(_, b)| value > b) {
                best = Some((i, value));
            }
        }
        best.map(|(i, _)| i)
    }

    /// Record a pull of arm `i` that earned `reward`, clamped to `0..=1`.
    pub fn record(&mut self, i: usize, reward: f64) {
        if let Some(a) = self.arms.get_mut(i) {
            a.pulls += 1;
            a.reward += reward.clamp(0.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// **EACH STRATEGY ONCE, THEN THE ONE THAT PAYS — WITHOUT STARVING THE
    /// REST.**
    #[test]
    fn ucb1_tries_each_then_favours_what_pays() {
        let mut b = Bandit::new(&names(&["a", "b", "c"]));
        for expect in [0, 1, 2] {
            assert_eq!(b.pick(), Some(expect));
            b.record(expect, if expect == 1 { 1.0 } else { 0.0 });
        }
        assert_eq!(b.pick(), Some(1), "b paid");
        let mut picks = Vec::new();
        for _ in 0..30 {
            let i = b.pick().expect("arm");
            picks.push(i);
            b.record(i, if i == 1 { 1.0 } else { 0.0 });
        }
        let of = |arm| picks.iter().filter(|&&p| p == arm).count();
        assert!(of(1) > of(0) + of(2), "{picks:?}");
        assert!(
            of(0) > 0 || of(2) > 0,
            "exploration still happens: {picks:?}"
        );
        assert_eq!(Bandit::default().pick(), None);
    }

    #[test]
    fn renaming_strategies_keeps_what_was_learned_about_the_rest() {
        let mut b = Bandit::new(&names(&["a", "b"]));
        b.record(0, 1.0);
        let next = b.with_arms(&names(&["b", "a", "new"]));
        assert_eq!(next.arms[1].pulls, 1);
        assert_eq!(next.arms[2].pulls, 0);
        assert_eq!(next.pick(), Some(0), "b was never tried");
        b.record(9, 1.0);
        assert_eq!(b.arms.len(), 2, "an unknown arm is ignored");
    }
}
