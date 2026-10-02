//! `insi learn`: lessons from failed attempts whose fix is verified.
//!
//! A failed worker attempt on a FAIR task (its reference commit passed the
//! hidden check) has a verified fix. The worker is shown the attempt, the
//! failure and the fix, and asked for one lesson; the lesson is stored with
//! the task as its evidence and the check as what verified it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use insi_core::{EvalResult, Lesson, Task, Verification, Worker};
use insi_eval::run::{self, Work, CHECK_OUTPUT, WORKER_DIFF};
use insi_eval::{pool, process, report, store};
use insi_memory::store::{FileLessons, LessonStore};
use insi_memory::writer;

/// How long the lesson writer may take for one lesson.
pub const WRITE_SECS: u64 = 10 * 60;

pub struct Learn<'a> {
    pub work: &'a Work,
    pub tasks: &'a [Task],
    pub results: &'a Path,
    pub lessons: &'a FileLessons,
    pub worker: &'a Worker,
    pub topic: &'a (dyn Fn(&Task) -> String + Sync),
    pub jobs: usize,
    /// Also write each lesson to the Incord Memory engine at this address,
    /// filed under the repository it came from. `None` keeps lessons in the
    /// file only, as a benchmark must.
    pub to_memory: Option<std::net::SocketAddr>,
}

/// The attempts a lesson can be learned from: failed, judged, on a fair
/// task, with what they left behind, and not learned from yet.
fn candidates<'t>(
    l: &Learn<'t>,
    all: &[EvalResult],
) -> Result<Vec<(&'t Task, EvalResult)>, String> {
    let fair = report::fair_tasks(all);
    let by_id: BTreeMap<&str, &Task> = l.tasks.iter().map(|t| (t.id.as_str(), t)).collect();
    let known: Vec<String> = l.lessons.all()?.into_iter().map(|x| x.id).collect();
    Ok(all
        .iter()
        .filter(|r| r.kind == "worker" && !r.passed && r.error.is_none())
        .filter(|r| fair.get(&r.task) == Some(&Some(true)))
        .filter_map(|r| {
            let task = by_id.get(r.task.as_str())?;
            let dir = r.artifacts.as_deref()?;
            let id = lesson_id(dir);
            (!known.contains(&id) && Path::new(dir).join(WORKER_DIFF).is_file())
                .then(|| (*task, r.clone()))
        })
        .collect())
}

/// One lesson per attempt: named after the attempt's artifacts folder.
fn lesson_id(artifacts: &str) -> String {
    Path::new(artifacts)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| artifacts.to_string())
}

pub fn learn(l: &Learn) -> Result<(), String> {
    let all = store::read_results(l.results)?;
    let todo = candidates(l, &all)?;
    if todo.is_empty() {
        println!("no failed attempt on a fair task is waiting for a lesson");
        return Ok(());
    }
    let mut written = 0usize;
    let mut published = 0usize;
    pool::each(
        &todo,
        l.jobs,
        |(task, r)| write_one(l, task, r),
        |(task, r), outcome| {
            match outcome {
                Ok(lesson) => {
                    // The file first: it is the record, and a memory engine
                    // that is down must not cost the lesson.
                    l.lessons.add(&lesson)?;
                    written += 1;
                    println!("{}  {}", lesson.id, lesson.use_);
                    if let Some(addr) = l.to_memory {
                        match insi_memory::incord::publish(addr, &lesson, &task.source) {
                            Ok(()) => published += 1,
                            Err(why) => println!("{}  not written to memory: {why}", lesson.id),
                        }
                    }
                }
                Err(why) => println!(
                    "{} s{}  no lesson: {}",
                    task.id,
                    r.seed,
                    why.lines().next().unwrap_or_default()
                ),
            }
            Ok(())
        },
    )?;
    println!(
        "{written} lesson(s) written to {}",
        l.lessons.path.display()
    );
    if l.to_memory.is_some() {
        println!("{published} of them written to Incord Memory");
    }
    Ok(())
}

fn write_one(l: &Learn, task: &Task, r: &EvalResult) -> Result<Lesson, String> {
    let dir = PathBuf::from(r.artifacts.as_deref().unwrap_or_default());
    let read = |file: &str| {
        std::fs::read_to_string(dir.join(file))
            .map_err(|e| format!("{}: {e}", dir.join(file).display()))
    };
    let prompt = writer::prompt(
        &task.prompt,
        &read(WORKER_DIFF)?,
        &read(CHECK_OUTPUT)?,
        &run::fix_diff(l.work, task)?,
    );
    // The writer runs in a folder of its own: it needs no checkout, and must
    // not change one.
    let id = lesson_id(&dir.display().to_string());
    let room = l.work.root.join("learn").join(&id);
    std::fs::create_dir_all(&room).map_err(|e| format!("{}: {e}", room.display()))?;
    let ran = process::run(
        &l.worker.render(&prompt)?,
        &room,
        Duration::from_secs(WRITE_SECS),
    )?;
    if !ran.success() {
        return Err(format!(
            "the lesson writer {}",
            if ran.timed_out {
                "timed out".to_string()
            } else {
                format!("exited {:?}", ran.code)
            }
        ));
    }
    let answer = writer::final_answer(&ran.stdout)
        .ok_or_else(|| "the lesson writer answered nothing".to_string())?;
    let draft = writer::parse(&answer)?;
    Ok(Lesson {
        id,
        topic: (l.topic)(task),
        issue: draft.issue,
        root_cause: draft.root_cause,
        use_: draft.use_,
        avoid: draft.avoid,
        signature: r.signature.clone().unwrap_or_default(),
        evidence: task.id.clone(),
        learned_at: task.date.clone(),
        verified_by: Verification::Check,
    })
}
