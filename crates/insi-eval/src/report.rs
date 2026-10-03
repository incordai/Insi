//! Attempts summed per task and per condition.

use std::collections::BTreeMap;

use insi_core::EvalResult;

/// One condition's totals (a condition is the set of an attempt's tags, e.g.
/// `lessons=off`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Totals {
    pub attempts: usize,
    pub passed: usize,
    /// Attempts that could not be judged (setup or git failed): left out of
    /// the pass rate, and counted so they are not hidden.
    pub errors: usize,
    pub timed_out: usize,
    pub tokens: u64,
    pub wall_secs: f64,
    /// US dollars, summed over the attempts that have a cost.
    pub cost_usd: f64,
    /// Attempts whose worker ran but whose cost is not known (no usage
    /// reported, or no price for its model): the cost above is a floor.
    pub uncosted: usize,
    /// Failures whose signature an earlier failure in the same condition
    /// already had: the repeat mistakes lessons exist to prevent.
    pub repeat_failures: usize,
}

impl Totals {
    pub fn pass_rate(&self) -> f64 {
        let judged = self.attempts - self.errors;
        if judged == 0 {
            0.0
        } else {
            self.passed as f64 / judged as f64
        }
    }
}

/// Worker attempts summed by condition. Control runs (`base`, `reference`)
/// are not attempts at the task and are left out.
pub fn by_condition(results: &[EvalResult]) -> BTreeMap<String, Totals> {
    let mut out: BTreeMap<String, Totals> = BTreeMap::new();
    let mut seen: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in results.iter().filter(|r| r.kind == "worker") {
        let condition = r.condition();
        let t = out.entry(condition.clone()).or_default();
        t.attempts += 1;
        t.tokens += r.tokens.unwrap_or(0);
        t.wall_secs += r.wall_secs;
        match r.cost_usd {
            Some(c) => t.cost_usd += c,
            None if r.phases.contains_key("worker") => t.uncosted += 1,
            None => {}
        }
        if r.error.is_some() {
            t.errors += 1;
            continue;
        }
        if r.timed_out {
            t.timed_out += 1;
        }
        if r.passed {
            t.passed += 1;
        } else if let Some(sig) = &r.signature {
            let earlier = seen.entry(condition).or_default();
            if earlier.contains(sig) {
                t.repeat_failures += 1;
            } else {
                earlier.push(sig.clone());
            }
        }
    }
    out
}

/// Whether each task is a fair test, from its control runs: the check fails
/// at the base and passes at the reference. `None` while a control is
/// missing.
pub fn fair_tasks(results: &[EvalResult]) -> BTreeMap<String, Option<bool>> {
    let mut base: BTreeMap<&str, bool> = BTreeMap::new();
    let mut reference: BTreeMap<&str, bool> = BTreeMap::new();
    for r in results {
        let ok = r.error.is_none();
        match r.kind.as_str() {
            "base" => {
                base.insert(&r.task, ok && !r.passed);
            }
            "reference" => {
                reference.insert(&r.task, ok && r.passed);
            }
            _ => {}
        }
    }
    let mut out = BTreeMap::new();
    for task in base.keys().chain(reference.keys()) {
        let verdict = match (base.get(task), reference.get(task)) {
            (Some(b), Some(r)) => Some(*b && *r),
            _ => None,
        };
        out.insert(task.to_string(), verdict);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(task: &str, kind: &str, passed: bool, sig: Option<&str>, tags: &[&str]) -> EvalResult {
        EvalResult {
            task: task.into(),
            seed: 0,
            kind: kind.into(),
            passed,
            timed_out: false,
            wall_secs: 1.0,
            tokens: Some(10),
            model: None,
            usage: None,
            cost_usd: Some(0.5),
            signature: sig.map(String::from),
            error: None,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            artifacts: None,
            briefed: vec![],
            phases: Default::default(),
            notes: vec![],
        }
    }

    #[test]
    fn conditions_are_compared_and_repeats_counted() {
        let results = vec![
            r("a", "worker", false, Some("E1"), &["lessons=off"]),
            r("b", "worker", false, Some("E1"), &["lessons=off"]),
            r("c", "worker", true, None, &["lessons=off"]),
            r("a", "worker", true, None, &["lessons=on"]),
            r("b", "worker", false, Some("E1"), &["lessons=on"]),
            r("c", "worker", true, None, &["lessons=on"]),
            r("a", "base", false, Some("x"), &[]),
        ];
        let by = by_condition(&results);
        let off = &by["lessons=off"];
        assert_eq!((off.attempts, off.passed, off.repeat_failures), (3, 1, 1));
        assert!((off.cost_usd - 1.5).abs() < 1e-9, "{}", off.cost_usd);
        assert_eq!(off.uncosted, 0);
        let on = &by["lessons=on"];
        assert_eq!((on.attempts, on.passed, on.repeat_failures), (3, 2, 0));
        assert!(on.pass_rate() > off.pass_rate());
        assert!(!by.contains_key("untagged"), "controls are not attempts");
    }

    #[test]
    fn a_task_is_fair_only_when_base_fails_and_reference_passes() {
        let results = vec![
            r("good", "base", false, None, &[]),
            r("good", "reference", true, None, &[]),
            r("trivial", "base", true, None, &[]),
            r("trivial", "reference", true, None, &[]),
            r("half", "base", false, None, &[]),
        ];
        let fair = fair_tasks(&results);
        assert_eq!(fair["good"], Some(true));
        assert_eq!(fair["trivial"], Some(false));
        assert_eq!(fair["half"], None);
    }
}
