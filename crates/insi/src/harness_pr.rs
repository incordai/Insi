//! `insi improve --pr`: the harness the loop found best, as a draft pull
//! request on the repository that holds the harness folder.
//!
//! Reached from `improve_cmd` (crates/insi/src/main.rs) once the loop
//! (crates/insi/src/improve.rs) has kept at least one change. Its sibling is
//! `insi issue` (crates/insi/src/issue.rs): the same [`Hub`] (`gh`'s login,
//! or a stand-in), the same credential and personal-data scan
//! (`insi_core::secrets::scan_diff`) before anything is pushed, and the same
//! rule that a pull request is a DRAFT a person decides on.
//!
//! Only the files the loop changed are written, over the harness folder in a
//! fresh worktree of the default branch: a person's uncommitted edits to the
//! folder never ride along, and the person's own checkout is not touched.

use std::path::{Path, PathBuf};

use insi_eval::git::git;
use insi_loop::harness::Harness;

use crate::issue::{Hub, NewPr, Outcome};

pub struct HarnessPr<'a> {
    /// The person's harness folder, inside the repository the PR is for.
    pub harness: &'a Path,
    /// The harness the loop started from, and the best it found.
    pub start: &'a Harness,
    pub best: &'a Harness,
    /// One line per change kept, with the benchmark's verdict on it.
    pub kept: &'a [String],
    /// The loop's audit trail, named in the PR.
    pub audit: &'a Path,
    /// Where the PR's worktree is made.
    pub work_root: &'a Path,
}

/// The branch a harness is proposed on: one per harness content.
pub fn branch_for(best: &Harness) -> String {
    format!("insi/harness-{}", &best.id()[..12])
}

/// Propose `pr.best` in place of `pr.start`. Refused, with nothing done, when
/// the two are the same, a changed path could leave the folder, the folder
/// is not in a git repository, or the branch already exists (this harness was
/// proposed before; that branch is a person's to look at first).
pub fn open(pr: &HarnessPr, hub: &dyn Hub) -> Result<Outcome, String> {
    let changed: Vec<(&String, &String)> = pr
        .best
        .files
        .iter()
        .filter(|(file, text)| pr.start.files.get(*file) != Some(text))
        .collect();
    if changed.is_empty() {
        return Err("the best harness is the one the loop started from: nothing to propose".into());
    }
    if let Some((file, _)) = changed
        .iter()
        .find(|(file, _)| !insi_core::task::is_safe_relative(file))
    {
        return Err(format!(
            "harness file \"{file}\" is not a plain relative path"
        ));
    }
    let repo = PathBuf::from(
        git(pr.harness, &["rev-parse", "--show-toplevel"])
            .map_err(|e| format!("{} is not in a git repository: {e}", pr.harness.display()))?,
    );
    // The folder's path inside the repository, `/`-separated, empty at the root.
    let folder = git(pr.harness, &["rev-parse", "--show-prefix"])?;
    let branch = branch_for(pr.best);
    if git(
        &repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok()
    {
        return Err(format!(
            "branch {branch} already exists: this harness was proposed before; look at that branch first"
        ));
    }

    let base = hub.default_branch(&repo)?;
    git(&repo, &["fetch", "--quiet", "origin", &base])?;
    let root = pr.work_root.join("harness-prs");
    std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    let tree = root.join(&branch["insi/".len()..]);
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            &tree.display().to_string(),
            &format!("origin/{base}"),
        ],
    )?;
    let at = Place {
        repo: &repo,
        tree: &tree,
        folder: &folder,
        branch: &branch,
        base: &base,
    };
    let outcome = propose_in(pr, hub, &at, &changed);
    // The branch keeps the work either way; the checkout goes.
    let _ = git(
        &repo,
        &["worktree", "remove", "--force", &tree.display().to_string()],
    );
    outcome
}

/// Where a proposal is made: the repository, the branch's worktree, the
/// harness folder's path in it, and the branch and its base.
#[derive(Clone, Copy)]
struct Place<'a> {
    repo: &'a Path,
    tree: &'a Path,
    folder: &'a str,
    branch: &'a str,
    base: &'a str,
}

fn propose_in(
    pr: &HarnessPr,
    hub: &dyn Hub,
    at: &Place,
    changed: &[(&String, &String)],
) -> Result<Outcome, String> {
    let Place {
        repo,
        tree,
        folder,
        branch,
        base,
    } = *at;
    let dir = tree.join(folder);
    for (file, text) in changed {
        let path = dir.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    git(tree, &["add", "--all"])?;
    if git(tree, &["diff", "--cached", "--quiet"]).is_ok() {
        return Err(format!(
            "{base} already holds this harness: nothing to propose"
        ));
    }
    let id = pr.best.id();
    let title = format!(
        "Harness {}: {} change(s) the benchmark kept",
        &id[..12],
        pr.kept.len()
    );
    git(tree, &["commit", "--quiet", "-m", &title])?;
    let diff = git(
        tree,
        &["diff", "--no-color", &format!("origin/{base}..HEAD")],
    )?;
    let found = insi_core::secrets::scan_diff(&diff);
    if !found.is_empty() {
        let listed: Vec<String> = found.iter().take(10).map(|f| f.to_string()).collect();
        return Ok(Outcome::Kept {
            branch: branch.to_string(),
            why: format!(
                "the harness adds what looks like a credential or personal data, so nothing \
                 was pushed: {}",
                listed.join("; ")
            ),
        });
    }
    git(
        tree,
        &["push", "--quiet", "--set-upstream", "origin", branch],
    )?;
    let body = format!(
        "Insi's improvement loop found this harness better than `{start}` on the benchmark.\n\
         **Review it before merging — nothing here is merged automatically.**\n\n\
         Changes kept, each only when it beat the incumbent on the same tasks and seeds and \
         was not worse on the held-out tasks:\n\n{kept}\n\n\
         - **Files changed:** {files}.\n\
         - **Audit trail:** `{audit}` on the machine that ran the loop.\n\
         - **Credential and personal-data scan:** nothing credential- or PII-shaped in the \
         lines this branch adds.\n",
        start = pr.start.id(),
        kept = pr
            .kept
            .iter()
            .map(|k| format!("- {k}"))
            .collect::<Vec<_>>()
            .join("\n"),
        files = changed
            .iter()
            .map(|(f, _)| format!("`{folder}{f}`"))
            .collect::<Vec<_>>()
            .join(", "),
        audit = pr.audit.display(),
    );
    let url = hub.create_pr(
        repo,
        &NewPr {
            head: branch.to_string(),
            base: base.to_string(),
            title,
            body,
            draft: true,
        },
    )?;
    Ok(Outcome::Opened {
        url,
        branch: branch.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::Issue;
    use std::cell::RefCell;

    struct FakeHub {
        created: RefCell<Vec<NewPr>>,
    }

    impl Hub for FakeHub {
        fn issue(&self, _: &Path, _: u64) -> Result<Issue, String> {
            Err("no issues here".into())
        }
        fn default_branch(&self, _: &Path) -> Result<String, String> {
            Ok("main".to_string())
        }
        fn create_pr(&self, _: &Path, pr: &NewPr) -> Result<String, String> {
            self.created.borrow_mut().push(pr.clone());
            Ok("https://github.com/o/r/pull/3".to_string())
        }
    }

    fn temp(label: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "insi-harness-pr-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        dir
    }

    /// A repository with an `origin` (a local bare one) on `main`, holding a
    /// harness in `agent/harness/`.
    fn repo() -> (PathBuf, PathBuf) {
        let base = temp("repo");
        let origin = base.join("origin.git");
        let work = base.join("work");
        git(
            &base,
            &["init", "--quiet", "--bare", "-b", "main", "origin.git"],
        )
        .expect("bare");
        git(&base, &["init", "--quiet", "-b", "main", "work"]).expect("init");
        for (k, v) in [
            ("user.email", "t@example.invalid"),
            ("user.name", "t"),
            ("core.autocrlf", "false"),
        ] {
            git(&work, &["config", k, v]).expect("config");
        }
        let harness = work.join("agent/harness");
        std::fs::create_dir_all(&harness).expect("harness");
        std::fs::write(harness.join("prompt.md"), "Work carefully.\n").expect("w");
        std::fs::write(harness.join("roles.json"), "{}\n").expect("w");
        git(&work, &["add", "."]).expect("add");
        git(&work, &["commit", "--quiet", "-m", "first"]).expect("commit");
        git(
            &work,
            &["remote", "add", "origin", &origin.display().to_string()],
        )
        .expect("remote");
        git(&work, &["push", "--quiet", "origin", "main"]).expect("push");
        (work, origin)
    }

    /// **THE BEST HARNESS BECOMES A DRAFT PR CHANGING ONLY WHAT THE LOOP
    /// CHANGED.** The branch reaches origin with the new `prompt.md` in the
    /// harness folder and `roles.json` untouched; the person's checkout and
    /// its uncommitted edit are left alone; the draft carries the evidence;
    /// and the same harness is not proposed twice.
    #[test]
    fn the_best_harness_is_proposed_as_a_draft() {
        let (work, origin) = repo();
        let folder = work.join("agent/harness");
        // An edit of the person's own, not committed, made before the loop
        // read the folder: it is in the start AND the best harness, and must
        // still not ride along — the loop did not make it.
        std::fs::write(folder.join("roles.json"), "{\"mine\": true}\n").expect("w");
        let start = Harness::read(&folder).expect("start");
        let best = start.with("prompt.md", "Name the markers the checks look for.\n");
        let hub = FakeHub {
            created: RefCell::new(Vec::new()),
        };
        let kept = ["step 1: prompt.md: name the markers (2/2 against 0/2)".to_string()];
        let pr = HarnessPr {
            harness: &folder,
            start: &start,
            best: &best,
            kept: &kept,
            audit: Path::new("loop/audit.jsonl"),
            work_root: &temp("root"),
        };
        let outcome = open(&pr, &hub).expect("opened");
        let branch = branch_for(&best);
        assert_eq!(
            outcome,
            Outcome::Opened {
                url: "https://github.com/o/r/pull/3".into(),
                branch: branch.clone()
            }
        );
        let created = hub.created.borrow();
        assert!(created[0].draft);
        assert_eq!(created[0].base, "main");
        assert!(created[0].body.contains(&kept[0]), "{}", created[0].body);
        assert!(
            created[0].body.contains("`agent/harness/prompt.md`"),
            "{}",
            created[0].body
        );
        assert_eq!(
            git(
                &origin,
                &["show", &format!("{branch}:agent/harness/prompt.md")]
            )
            .expect("pushed"),
            "Name the markers the checks look for."
        );
        assert_eq!(
            git(
                &origin,
                &["show", &format!("{branch}:agent/harness/roles.json")]
            )
            .expect("pushed"),
            "{}",
            "only the files the loop changed"
        );
        assert_eq!(
            std::fs::read_to_string(folder.join("prompt.md")).expect("theirs"),
            "Work carefully.\n",
            "the person's checkout is not touched"
        );
        drop(created);
        let why = open(&pr, &hub).expect_err("twice");
        assert!(why.contains("already exists"), "{why}");
        let same = HarnessPr { best: &start, ..pr };
        assert!(open(&same, &hub)
            .expect_err("unchanged")
            .contains("nothing to propose"));
    }

    /// **A HARNESS WITH A CREDENTIAL IN IT IS NEVER PUSHED.**
    #[test]
    fn a_credential_in_the_harness_stays_local() {
        let (work, origin) = repo();
        let folder = work.join("agent/harness");
        let start = Harness::read(&folder).expect("start");
        // Built here, so no credential-shaped text sits in the source.
        let token = format!("ghp_{}", "a1".repeat(18));
        let best = start.with("prompt.md", &format!("Use token = \"{token}\".\n"));
        let hub = FakeHub {
            created: RefCell::new(Vec::new()),
        };
        let outcome = open(
            &HarnessPr {
                harness: &folder,
                start: &start,
                best: &best,
                kept: &[],
                audit: Path::new("audit.jsonl"),
                work_root: &temp("root"),
            },
            &hub,
        )
        .expect("ran");
        assert!(
            matches!(&outcome, Outcome::Kept { why, .. } if why.contains("credential")),
            "{outcome:?}"
        );
        assert!(hub.created.borrow().is_empty());
        assert!(git(
            &origin,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{}", branch_for(&best))
            ]
        )
        .is_err());
    }
}
