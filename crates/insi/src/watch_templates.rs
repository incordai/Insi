// Copyright (c) 2026-2027 Incord Ai.
// https://incord.ai
//
// SPDX-License-Identifier: BUSL-1.1

//! Shared Inmem watch starter: Insi's catalog, plus the checks Insi enables by itself. No scheduler (Inmem runs every
//! check in `.incord/watch.toml`) and no tool installation: a check is enabled only when it fits the project AND its
//! program answers here (the owner, 2026-11-25: "only checks that fit"; Watchman ships gitleaks and trivy beside Insi
//! and the engine, so every user has them). Everything else stays a commented template.
//!
//! Callers: main.rs `watch_init_cmd` (`insi watch-init`, which the engine runs for each project: Inmem
//! src/cli/project_layout.rs `add_insi_checks`) and init.rs (`insi init`). Sibling: the engine's own templates
//! (Inmem src/cli/layout_templates.json, written by project_layout.rs `write_watch_file`) schedule the build and test
//! checks; a check here is skipped when an active check already runs the same program, so nothing is scheduled twice.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const CATALOG: &str = include_str!("watch_templates.toml");
const BEGIN: &str = "# BEGIN INSI WATCH MODULE";
const END: &str = "# END INSI WATCH MODULE";
/// The comment line inside Insi's module naming every check Insi has enabled there, so a check the owner or an agent
/// removed is never enabled again.
const LEDGER: &str = "# INSI ENABLED: ";

/// One check Insi enables by itself.
struct Auto {
    name: &'static str,
    command: &'static str,
    every_minutes: u64,
    timeout_secs: u64,
    /// What an existing active check's command holds when it already does this job (no second schedule).
    runs: &'static str,
    /// The program the command starts, and the arguments that make it answer without touching the project.
    program: &'static str,
    probe: &'static [&'static str],
    /// Other programs the command starts, each of which must answer `--version` too.
    needs: &'static [&'static str],
    /// Whether the project has what this check checks.
    fits: fn(&Path) -> bool,
}

/// Secrets: the committed history (`gitleaks dir` would read every build folder: 1.6 GB and 107 s on a Rust project,
/// measured 2026-11-25; the history read 2,653 commits in 40 s). A secret that ever reached history is leaked, so it
/// counts. Known false positives go in the project's `.gitleaksignore`, which gitleaks reads by default.
const SECRETS: &str = "gitleaks git --redact --exit-code 1 .";
/// Dependencies and configuration. Secrets are left to gitleaks; build, package and agent folders are skipped (the
/// lockfiles name the dependencies), which took a scan that ran past 5 minutes down to 8 s (measured 2026-11-25).
const DEPENDENCIES: &str = "trivy fs --scanners vuln,misconfig --severity HIGH,CRITICAL --exit-code 1 --quiet --timeout 15m --skip-dirs \"**/target\" --skip-dirs \"**/node_modules\" --skip-dirs .incord --skip-dirs .claude .";

const AUTO: &[Auto] = &[
    Auto {
        name: "insi: security secrets",
        command: SECRETS,
        every_minutes: 1440,
        timeout_secs: 600,
        runs: "gitleaks",
        program: "gitleaks",
        probe: &["version"],
        // `gitleaks git` reads the history through git.
        needs: &["git"],
        fits: is_git_repository,
    },
    Auto {
        name: "insi: security dependencies and configuration",
        command: DEPENDENCIES,
        every_minutes: 1440,
        timeout_secs: 1200,
        runs: "trivy",
        program: "trivy",
        probe: &["--version"],
        needs: &[],
        fits: has_dependency_or_config_files,
    },
    Auto {
        name: "insi: rust strict lint",
        command: "cargo clippy --workspace --all-targets --locked -- -D warnings",
        every_minutes: 1440,
        timeout_secs: 600,
        runs: "cargo clippy",
        program: "cargo",
        probe: &["clippy", "--version"],
        needs: &[],
        fits: is_locked_rust_project,
    },
    Auto {
        name: "insi: security source rules",
        command: "semgrep scan --metrics=off --error --config .insi/security-rules.yml .",
        every_minutes: 1440,
        timeout_secs: 600,
        runs: "semgrep",
        program: "semgrep",
        probe: &["--version"],
        needs: &[],
        fits: has_source_rules,
    },
    Auto {
        name: "insi: security application boundaries",
        command: "python3 scripts/security/check_application.py",
        every_minutes: 1440,
        timeout_secs: 300,
        runs: "scripts/security/check_application.py",
        program: "python3",
        probe: &["--version"],
        needs: &[],
        fits: has_application_script,
    },
    Auto {
        name: "insi: runtime errors",
        command: "insi errors check --repo .",
        every_minutes: 5,
        timeout_secs: 20,
        runs: "insi errors check",
        program: "insi",
        probe: &["--version"],
        needs: &[],
        fits: has_insi_project,
    },
];

fn is_git_repository(repo: &Path) -> bool {
    repo.join(".git").exists()
}

fn is_locked_rust_project(repo: &Path) -> bool {
    repo.join("Cargo.toml").is_file() && repo.join("Cargo.lock").is_file()
}

fn has_source_rules(repo: &Path) -> bool {
    repo.join(".insi/security-rules.yml").is_file()
}

fn has_application_script(repo: &Path) -> bool {
    repo.join("scripts/security/check_application.py").is_file()
}

fn has_insi_project(repo: &Path) -> bool {
    repo.join(".insi").is_dir()
}

/// The files trivy reads dependencies or configuration from.
const DEPENDENCY_AND_CONFIG_FILES: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "go.sum",
    "requirements.txt",
    "poetry.lock",
    "Pipfile.lock",
    "uv.lock",
    "composer.lock",
    "Gemfile.lock",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "packages.lock.json",
    "compose.yaml",
    "compose.yml",
    "docker-compose.yml",
    "docker-compose.yaml",
    "Chart.yaml",
];
/// Folders the fit walk and the trivy scan both leave out.
const SKIPPED_DIRS: &[&str] = &["target", "node_modules", ".git", ".incord", ".claude"];

/// A dependency lockfile or a container/infrastructure file in the project's top three folder levels.
fn has_dependency_or_config_files(repo: &Path) -> bool {
    fn walk(dir: &Path, depth: usize) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        let mut folders = Vec::new();
        for entry in entries.flatten().take(5000) {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if depth < 2 && !SKIPPED_DIRS.contains(&name.as_str()) {
                    folders.push(entry.path());
                }
            } else if kind.is_file()
                && (DEPENDENCY_AND_CONFIG_FILES.contains(&name.as_str())
                    || name.starts_with("Dockerfile")
                    || name.ends_with(".Dockerfile")
                    || name.ends_with(".tf"))
            {
                return true;
            }
        }
        folders.iter().any(|folder| walk(folder, depth + 1))
    }
    walk(repo, 0)
}

/// Where a check's program is found: beside this Insi first (Watchman's program folder, which the engine also puts
/// first on a check's PATH: Inmem src/guard/watch_checks.rs `path_with_bundled_tools`), then the PATH.
fn find_program(program: &str) -> Option<PathBuf> {
    let file = format!("{program}{}", std::env::consts::EXE_SUFFIX);
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    beside
        .into_iter()
        .chain(path)
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
}

/// The check's program is here and answers its probe within 30 s (a store alias or a broken install does not), and so
/// does every other program it needs.
fn program_answers(check: &Auto) -> bool {
    answers(check.program, check.probe)
        && check
            .needs
            .iter()
            .all(|program| answers(program, &["--version"]))
}

fn answers(program: &str, probe: &[&str]) -> bool {
    let Some(path) = find_program(program) else {
        return false;
    };
    let Ok(mut child) = std::process::Command::new(path)
        .args(probe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Add the catalog once and enable the checks that fit; existing checks are preserved and nothing is scheduled twice.
pub fn install(repo: &Path, test_command: &[String]) -> Result<bool, String> {
    install_with(repo, test_command, &program_answers)
}

fn install_with(
    repo: &Path,
    test_command: &[String],
    answers: &dyn Fn(&Auto) -> bool,
) -> Result<bool, String> {
    let path = repo.join(".incord/watch.toml");
    // Refuse redirecting generated writes through a symlinked config file or directory.
    for checked in [repo.join(".incord"), path.clone()] {
        if std::fs::symlink_metadata(&checked).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("watch template path must not be a symlink".into());
        }
    }
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let parsed = toml::from_str::<toml::Value>(&existing)
            .map_err(|e| format!("existing watch file is invalid: {e}"))?;
        // A partial section is not safe to repair automatically; preserve all agent and owner content.
        if existing.contains(BEGIN) != existing.contains(END) {
            return Err(
                "incomplete Insi watch module; preserve the file and repair its markers".into(),
            );
        }
        let has_module = existing.contains(BEGIN);
        // A catalog without markers leaves Insi no segment of its own to write in.
        if !has_module && existing.contains("# INSI CHECK CATALOG v1") {
            return Ok(false);
        }
        let active = active_checks(&parsed);
        let enabled = ledger(&existing);
        let adds: Vec<&Auto> = AUTO
            .iter()
            .filter(|c| !enabled.iter().any(|name| name == c.name))
            .filter(|c| {
                !active
                    .iter()
                    .any(|(name, command)| name == c.name || command.contains(c.runs))
            })
            .filter(|c| (c.fits)(repo) && answers(c))
            .collect();
        let nl = if existing.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let text = if has_module {
            if adds.is_empty() {
                return Ok(false);
            }
            insert_in_module(&existing, &enabled_block(&adds, &enabled, nl))
        } else {
            // The owner's file keeps every byte; Insi's module goes after it.
            format!("{existing}{nl}{nl}{}", module(&adds, &enabled, &[], nl)?)
        };
        written_as_intended(&text, active.len() + adds.len())?;
        replace_file(&path, &text)?;
        return Ok(true);
    }
    std::fs::create_dir_all(repo.join(".incord")).map_err(|e| e.to_string())?;
    let adds: Vec<&Auto> = AUTO
        .iter()
        .filter(|c| (c.fits)(repo) && answers(c))
        .collect();
    let text = module(&adds, &[], test_command, "\n")?;
    written_as_intended(&text, adds.len() + usize::from(!test_command.is_empty()))?;
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e.to_string()),
    };
    file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Every active check's name and command.
fn active_checks(parsed: &toml::Value) -> Vec<(String, String)> {
    let field = |check: &toml::Value, key: &str| {
        check
            .get(key)
            .and_then(toml::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    parsed
        .get("check")
        .and_then(toml::Value::as_array)
        .map(|checks| {
            checks
                .iter()
                .map(|c| (field(c, "name"), field(c, "command")))
                .collect()
        })
        .unwrap_or_default()
}

/// The names on the ledger line(s) of Insi's module.
fn ledger(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.strip_prefix(LEDGER))
        .flat_map(|names| names.split(" | "))
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

/// The enabled checks as TOML (each command serialized as a TOML string, never interpolated) and the ledger line that
/// names them with every check enabled before. Empty when nothing is added.
fn enabled_block(adds: &[&Auto], enabled_before: &[String], nl: &str) -> String {
    if adds.is_empty() {
        return String::new();
    }
    let mut text = format!("{nl}# Enabled by Insi: each fits this project and its program answered on this machine.{nl}");
    for check in adds {
        let command = toml::Value::String(check.command.to_string()).to_string();
        text.push_str(&format!(
            "[[check]]{nl}name = \"{}\"{nl}command = {command}{nl}every_minutes = {}{nl}timeout_secs = {}{nl}{nl}",
            check.name, check.every_minutes, check.timeout_secs
        ));
    }
    let names: Vec<&str> = enabled_before
        .iter()
        .map(String::as_str)
        .chain(adds.iter().map(|c| c.name))
        .collect();
    text.push_str(&format!("{LEDGER}{}{nl}", names.join(" | ")));
    text
}

/// Insi's whole module: the catalog, the enabled checks and, in a new file, the project's detected full suite.
fn module(
    adds: &[&Auto],
    enabled_before: &[String],
    test_command: &[String],
    nl: &str,
) -> Result<String, String> {
    // The catalog may be checked out with either line ending; the module takes the file's own.
    let mut text = CATALOG
        .trim_end()
        .strip_suffix(END)
        .ok_or("watch catalog end marker missing")?
        .replace("\r\n", "\n")
        .replace('\n', nl);
    text.push_str(&enabled_block(adds, enabled_before, nl));
    if !test_command.is_empty() {
        // Serialize TOML strings rather than interpolating paths or shell punctuation into TOML.
        let command = test_command
            .iter()
            .map(|arg| crate::init::shell_quoted(arg))
            .collect::<Vec<_>>()
            .join(" ");
        let value = toml::Value::String(command).to_string();
        text.push_str(&format!("{nl}# Detected full suite; enabled for this repository.{nl}[[check]]{nl}name = \"insi: project full test suite\"{nl}command = {value}{nl}every_minutes = 1440{nl}timeout_secs = 600{nl}paths = []{nl}"));
    }
    text.push_str(&format!("{END}{nl}"));
    Ok(text)
}

/// `block` put at the end of Insi's existing module, whose old ledger line it replaces. Bytes before the module and
/// from its end marker on are unchanged.
fn insert_in_module(existing: &str, block: &str) -> String {
    let start = existing.find(BEGIN).unwrap_or(0);
    let end = existing[start..]
        .find(END)
        .map_or(existing.len(), |at| start + at);
    let inside: String = existing[start..end]
        .split_inclusive('\n')
        .filter(|line| !line.starts_with(LEDGER))
        .collect();
    format!(
        "{}{inside}{}{}",
        &existing[..start],
        block.trim_start_matches(['\r', '\n']),
        &existing[end..]
    )
}

/// The text Insi is about to write parses and holds exactly the checks it should; otherwise nothing is written.
fn written_as_intended(text: &str, checks: usize) -> Result<(), String> {
    let parsed = toml::from_str::<toml::Value>(text).map_err(|e| {
        format!("the watch file Insi built is not valid TOML; nothing written: {e}")
    })?;
    let got = active_checks(&parsed).len();
    if got != checks {
        return Err(format!(
            "the watch file Insi built holds {got} checks, not {checks}; nothing written"
        ));
    }
    Ok(())
}

/// Write `text` beside `path` and move it over `path`, so a reader never sees half a file.
fn replace_file(path: &Path, text: &str) -> Result<(), String> {
    let staged = path.with_extension("toml.insi-new");
    let _ = std::fs::remove_file(&staged);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)
        .map_err(|e| e.to_string())?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(&staged, path).map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "watch_templates_tests.rs"]
mod tests;
