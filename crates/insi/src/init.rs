//! `insi init`: make a repository ready for Insi.
//!
//! Looks at the files at the repository's root to tell how it is installed
//! and tested, writes `.insi/config.toml` with what it found, keeps Insi's
//! work folder out of git, and registers `insi mcp` in `.mcp.json` so an
//! agent working in the repository can ask for lessons. Every value it writes can be changed in the
//! file; nothing it detects is used without being written down first, so the
//! next command reads the same settings a person can see.

use std::path::Path;

use insi_core::config::{DIR, FILE};
use insi_core::Config;

/// How a repository is installed and checked, as `init` read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    /// What told it: `node (npm, vitest)`, `rust`, `python`, `go`.
    pub stack: String,
    pub setup: Vec<String>,
    pub check: Vec<String>,
}

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

/// Tell how `repo` is installed and tested from the files at its root.
pub fn detect(repo: &Path) -> Option<Detected> {
    let has = |name: &str| repo.join(name).is_file();
    if has("package.json") {
        let (manager, setup) = if has("pnpm-lock.yaml") {
            ("pnpm", argv(&["pnpm", "install", "--frozen-lockfile"]))
        } else if has("yarn.lock") {
            ("yarn", argv(&["yarn", "install", "--frozen-lockfile"]))
        } else if has("bun.lockb") || has("bun.lock") {
            ("bun", argv(&["bun", "install", "--frozen-lockfile"]))
        } else if has("package-lock.json") {
            ("npm", argv(&["npm", "ci"]))
        } else {
            ("npm", argv(&["npm", "install"]))
        };
        let manifest = std::fs::read_to_string(repo.join("package.json")).unwrap_or_default();
        let (runner, check) = if manifest.contains("\"vitest\"") {
            ("vitest", argv(&["npx", "vitest", "run", "{tests}"]))
        } else if manifest.contains("\"jest\"") {
            ("jest", argv(&["npx", "jest", "{tests}"]))
        } else {
            ("npm test", argv(&[manager, "test", "--", "{tests}"]))
        };
        return Some(Detected {
            stack: format!("node ({manager}, {runner})"),
            setup,
            check,
        });
    }
    if has("Cargo.toml") {
        // A test file of its own in `tests/` is its own test binary; the
        // whole suite runs it along with everything else.
        return Some(Detected {
            stack: "rust".to_string(),
            setup: Vec::new(),
            check: argv(&["cargo", "test"]),
        });
    }
    if has("pyproject.toml") || has("requirements.txt") || has("setup.py") {
        let setup = if has("requirements.txt") {
            argv(&["python", "-m", "pip", "install", "-r", "requirements.txt"])
        } else {
            argv(&["python", "-m", "pip", "install", "-e", "."])
        };
        return Some(Detected {
            stack: "python".to_string(),
            setup,
            check: argv(&["python", "-m", "pytest", "{tests}"]),
        });
    }
    if has("go.mod") {
        return Some(Detected {
            stack: "go".to_string(),
            setup: argv(&["go", "mod", "download"]),
            check: argv(&["go", "test", "./..."]),
        });
    }
    None
}

/// What `init` did, for the person who ran it.
#[derive(Debug)]
pub struct Initialised {
    pub detected: Option<Detected>,
    pub config_path: std::path::PathBuf,
    pub gitignore_changed: bool,
    pub config: Config,
    pub mcp: McpRegistration,
}

/// The project MCP file Claude Code reads at a repository's root.
pub const MCP_FILE: &str = ".mcp.json";

/// The line that registers `insi mcp` by hand, in Claude Code's own words.
pub const MCP_ADD: &str = "claude mcp add --scope project insi -- insi mcp";

/// What `init` did about registering `insi mcp` for agents in the repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpRegistration {
    /// [`MCP_FILE`] was written with `insi` as its only server.
    Written,
    /// [`MCP_FILE`] already registers a server named `insi`.
    AlreadyThere,
    /// [`MCP_FILE`] exists without `insi`, and is a person's file: it is not
    /// rewritten. Register with [`MCP_ADD`].
    LeftAlone,
}

/// Register `insi mcp` in `repo`'s [`MCP_FILE`], in the shape `claude mcp
/// add --scope project` writes. Only a missing file is written; an existing
/// one is read and never changed.
fn register_mcp(repo: &Path) -> Result<McpRegistration, String> {
    let path = repo.join(MCP_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let has_insi = serde_json::from_str::<serde_json::Value>(&text)
                .map(|v| v["mcpServers"].get("insi").is_some())
                .unwrap_or(false);
            Ok(if has_insi {
                McpRegistration::AlreadyThere
            } else {
                McpRegistration::LeftAlone
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let file = serde_json::json!({
                "mcpServers": {
                    "insi": { "type": "stdio", "command": "insi", "args": ["mcp"], "env": {} }
                }
            });
            let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
            std::fs::write(&path, format!("{text}\n"))
                .map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(McpRegistration::Written)
        }
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// The repository's name: its folder's.
fn repo_name(repo: &Path) -> String {
    let resolved = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    resolved
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "repo".to_string())
}

/// Write `.insi/config.toml` in `repo`. `check` and `setup`, when given,
/// replace what was detected. Refused when a config is already there and
/// `force` is not set: a person's edits are not overwritten by a guess.
pub fn init(
    repo: &Path,
    check: Vec<String>,
    setup: Option<Vec<String>>,
    force: bool,
) -> Result<Initialised, String> {
    let dir = repo.join(DIR);
    let config_path = dir.join(FILE);
    if config_path.exists() && !force {
        return Err(format!(
            "{} already exists; edit it, or run again with --force to replace it",
            config_path.display()
        ));
    }
    let detected = detect(repo);
    let check = if check.is_empty() {
        detected.as_ref().map(|d| d.check.clone()).ok_or_else(|| {
            "could not tell how this repository is tested; pass --check \"<cmd {tests}>\""
                .to_string()
        })?
    } else {
        check
    };
    let setup = setup.unwrap_or_else(|| {
        detected
            .as_ref()
            .map(|d| d.setup.clone())
            .unwrap_or_default()
    });
    let name = repo_name(repo);
    let config = Config {
        topic: Some(name.clone()),
        check,
        setup,
        // Outside the checkout: a worker in this repository must not be able
        // to read the hidden tests.
        private_checks: Some(format!("../insi-private/{name}")),
        ..Config::default()
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(
        &config_path,
        format!(
            "# Written by `insi init`. Every key may be edited; see the Insi README.\n{}",
            config.to_toml()?
        ),
    )
    .map_err(|e| format!("{}: {e}", config_path.display()))?;
    let gitignore_changed = ignore_work_folder(repo)?;
    let mcp = register_mcp(repo)?;
    Ok(Initialised {
        detected,
        config_path,
        gitignore_changed,
        config,
        mcp,
    })
}

/// The line that keeps Insi's work folder out of git.
pub const WORK_IGNORE: &str = "/.insi-work/";

/// Add [`WORK_IGNORE`] to `repo/.gitignore` unless it is there; whether the
/// file changed.
fn ignore_work_folder(repo: &Path) -> Result<bool, String> {
    let path = repo.join(".gitignore");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if text.lines().any(|l| l.trim() == WORK_IGNORE) {
        return Ok(false);
    }
    let mut out = text;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("# Insi's clones, worktrees and results (insi init).\n");
    out.push_str(WORK_IGNORE);
    out.push('\n');
    std::fs::write(&path, out).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_with(files: &[(&str, &str)]) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir()
            .join(format!("insi-init-{}-{nanos}", std::process::id()))
            .join("shop");
        std::fs::create_dir_all(&dir).expect("dir");
        for (name, text) in files {
            std::fs::write(dir.join(name), text).expect("write");
        }
        dir
    }

    #[test]
    fn each_stack_is_told_by_its_files() {
        let node = repo_with(&[
            ("package.json", r#"{"devDependencies":{"vitest":"^3"}}"#),
            ("package-lock.json", "{}"),
        ]);
        let d = detect(&node).expect("node");
        assert_eq!(d.setup, ["npm", "ci"]);
        assert_eq!(d.check, ["npx", "vitest", "run", "{tests}"]);

        let pnpm_jest = repo_with(&[
            ("package.json", r#"{"devDependencies":{"jest":"29"}}"#),
            ("pnpm-lock.yaml", ""),
        ]);
        let d = detect(&pnpm_jest).expect("pnpm");
        assert_eq!(d.setup, ["pnpm", "install", "--frozen-lockfile"]);
        assert_eq!(d.check, ["npx", "jest", "{tests}"]);

        assert_eq!(
            detect(&repo_with(&[("Cargo.toml", "")]))
                .expect("rust")
                .check,
            ["cargo", "test"]
        );
        assert_eq!(
            detect(&repo_with(&[("requirements.txt", "")]))
                .expect("python")
                .check,
            ["python", "-m", "pytest", "{tests}"]
        );
        assert_eq!(
            detect(&repo_with(&[("go.mod", "")])).expect("go").stack,
            "go"
        );
        assert_eq!(detect(&repo_with(&[("notes.txt", "")])), None);
    }

    /// **INIT WRITES WHAT IT FOUND, AND NEVER OVER A PERSON'S EDITS.** The
    /// config it writes reads back with the detected commands, a private
    /// folder outside the checkout and the repo's name as the topic; the
    /// work folder is ignored once; a second init is refused without
    /// --force; a repo it cannot read asks for --check.
    #[test]
    fn init_writes_a_config_that_reads_back_and_keeps_edits() {
        let repo = repo_with(&[
            ("package.json", r#"{"devDependencies":{"vitest":"^3"}}"#),
            ("package-lock.json", "{}"),
            (".gitignore", "node_modules"),
        ]);
        let done = init(&repo, vec![], None, false).expect("init");
        assert!(done.gitignore_changed);
        let config = Config::load(&repo).expect("reads back");
        assert_eq!(config, done.config);
        assert_eq!(config.topic.as_deref(), Some("shop"));
        assert_eq!(config.check, ["npx", "vitest", "run", "{tests}"]);
        assert_eq!(
            config.private_checks.as_deref(),
            Some("../insi-private/shop")
        );
        let (tasks, hidden) = config.private_dirs(&repo).expect("dirs");
        assert!(
            tasks.ends_with("insi-private/shop/tasks"),
            "{}",
            tasks.display()
        );
        assert!(hidden.ends_with("insi-private/shop/hidden"));
        let ignore = std::fs::read_to_string(repo.join(".gitignore")).expect("ignore");
        assert!(ignore.starts_with("node_modules\n") && ignore.contains(WORK_IGNORE));

        assert!(init(&repo, vec![], None, false).is_err(), "would overwrite");
        let again = init(&repo, argv(&["make", "check"]), Some(vec![]), true).expect("forced");
        assert!(!again.gitignore_changed, "the line is not added twice");
        assert_eq!(Config::load(&repo).expect("reads").check, ["make", "check"]);

        let unknown = repo_with(&[("notes.txt", "")]);
        assert!(init(&unknown, vec![], None, false)
            .expect_err("no stack")
            .contains("--check"));
    }

    /// **`insi mcp` IS REGISTERED, AND A PERSON'S `.mcp.json` IS NOT
    /// REWRITTEN.** A repo with none gets the file `claude mcp add --scope
    /// project` writes; one that already names `insi` is left as it is; one
    /// with other servers only is left byte for byte, and init says how to
    /// add Insi to it.
    #[test]
    fn insi_mcp_is_registered_without_touching_an_existing_file() {
        let fresh = repo_with(&[("Cargo.toml", "")]);
        let done = init(&fresh, vec![], None, false).expect("init");
        assert_eq!(done.mcp, McpRegistration::Written);
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(fresh.join(MCP_FILE)).expect("file"))
                .expect("json");
        assert_eq!(
            written,
            serde_json::json!({"mcpServers": {"insi": {
                "type": "stdio", "command": "insi", "args": ["mcp"], "env": {}
            }}})
        );
        let again = init(&fresh, vec![], None, true).expect("forced");
        assert_eq!(again.mcp, McpRegistration::AlreadyThere);

        let theirs = r#"{"mcpServers":{"memory":{"command":"mem"}}}"#;
        let other = repo_with(&[("Cargo.toml", ""), (MCP_FILE, theirs)]);
        let done = init(&other, vec![], None, false).expect("init");
        assert_eq!(done.mcp, McpRegistration::LeftAlone);
        assert_eq!(
            std::fs::read_to_string(other.join(MCP_FILE)).expect("file"),
            theirs
        );
    }
}
