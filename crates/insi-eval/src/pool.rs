//! At most N attempts at once.
//!
//! An attempt spends most of its time waiting on a child process (installing
//! dependencies, the worker, the test run), so a few at once finish a
//! benchmark in a fraction of the time. Results come back to the CALLING
//! thread as each attempt ends, so writing them — to one results file, in
//! one order of lines — needs no lock.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

/// Run `work` on every item, at most `jobs` at a time, and hand each result
/// to `done` on this thread as soon as it is ready. `done` returning an error
/// stops new items from starting; the ones already running finish, and the
/// first error is returned.
pub fn each<T, R>(
    items: &[T],
    jobs: usize,
    work: impl Fn(&T) -> R + Sync,
    mut done: impl FnMut(&T, R) -> Result<(), String>,
) -> Result<(), String>
where
    T: Sync,
    R: Send,
{
    let next = AtomicUsize::new(0);
    let stop = std::sync::atomic::AtomicBool::new(false);
    let mut first_error = None;
    std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel::<(usize, R)>();
        for _ in 0..jobs.clamp(1, items.len().max(1)) {
            let tx = tx.clone();
            let (next, stop, work) = (&next, &stop, &work);
            scope.spawn(move || loop {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some(item) = items.get(i) else { break };
                if tx.send((i, work(item))).is_err() {
                    break;
                }
            });
        }
        // Only the workers hold senders now: the loop below ends when the
        // last of them has finished.
        drop(tx);
        for (i, result) in rx {
            if first_error.is_some() {
                continue;
            }
            if let Err(e) = done(&items[i], result) {
                stop.store(true, Ordering::SeqCst);
                first_error = Some(e);
            }
        }
    });
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// **N AT ONCE, EVERY ONE ONCE.** Eight items that each wait 200 ms take
    /// about two waits with four jobs, not eight, and each is handled once.
    #[test]
    fn items_run_in_parallel_and_each_result_arrives_once() {
        let items: Vec<u32> = (0..8).collect();
        let started = Instant::now();
        let mut seen = Vec::new();
        each(
            &items,
            4,
            |n| {
                std::thread::sleep(Duration::from_millis(200));
                n * 10
            },
            |n, r| {
                assert_eq!(r, n * 10);
                seen.push(*n);
                Ok(())
            },
        )
        .expect("all done");
        seen.sort();
        assert_eq!(seen, items);
        assert!(
            started.elapsed() < Duration::from_millis(1200),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn an_error_stops_new_items_and_is_returned() {
        let items: Vec<u32> = (0..50).collect();
        let started = AtomicUsize::new(0);
        let err = each(
            &items,
            2,
            |_| {
                started.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(20));
            },
            |n, ()| {
                if *n == 1 {
                    Err("disk full".into())
                } else {
                    Ok(())
                }
            },
        )
        .expect_err("stops");
        assert_eq!(err, "disk full");
        assert!(started.load(Ordering::SeqCst) < 50);
    }

    #[test]
    fn no_items_and_zero_jobs_are_fine() {
        each(&[] as &[u32], 0, |_| (), |_, ()| Ok(())).expect("nothing to do");
        let mut count = 0;
        each(
            &[1, 2, 3],
            0,
            |n| *n,
            |_, _| {
                count += 1;
                Ok(())
            },
        )
        .expect("one job at least");
        assert_eq!(count, 3);
    }
}
