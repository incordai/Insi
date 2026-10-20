//! `.insi/config.toml` — how one repository is checked and worked on.
//!
//! Every key is optional except where a command needs it; an unknown key is
//! an error, so a misspelt setting is never silently left at its default.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The folder Insi keeps in a repository.
pub const DIR: &str = ".insi";
/// The config file inside it.
pub const FILE: &str = "config.toml";
/// What a worker command must contain: where the task's prompt goes.
pub const PROMPT: &str = "{prompt}";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The topic lessons for this repo are filed under (`rust/incord/...`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// Narrower topics by the files and commands they cover; see
    /// [`crate::topic`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topic_rules: Vec<crate::topic::TopicRule>,
    /// The public check: the repo's own test command, as argv.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub test_cmd: Vec<String>,
    /// The hidden check `insi mine` gives each task, as argv; `{tests}`
    /// stands for the task's hidden test files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub check: Vec<String>,
    /// Run in a fresh worktree before anything else (installing
    /// dependencies), as argv.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup: Vec<String>,
    /// What setup produces that a later attempt may reuse while the key
    /// files are unchanged (`node_modules` for `package-lock.json`). Mined
    /// into every task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_cache: Option<crate::task::SetupCache>,
    /// The wall-clock seconds a worker gets per task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_secs: Option<u64>,
    /// Paths a worker must never write (glob patterns, repo-relative).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protected: Vec<String>,
    /// Where the public task definitions live, repo-relative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_tasks: Option<String>,
    /// Where the hidden checks live: a path outside every checkout, or a git
    /// URL. Never inside the repository a worker can read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_checks: Option<String>,
    /// The label a maintainer puts on an issue to let an agent work it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trusted_issue_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<Worker>,
    /// What each model costs, by the name the worker reports for it
    /// (`model` in incode's result line). A model with no entry has no known
    /// cost, and `insi eval --max-dollars` stops rather than count it free;
    /// a model on a subscription is an entry of zeros.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub prices: std::collections::BTreeMap<String, Price>,
    /// The ways `insi improve` asks for a change, one per step, chosen by a
    /// bandit; [`Strategy::defaults`] when none are named.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strategies: Vec<Strategy>,
    /// The proposer `insi improve` asks for changes; the worker when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposer: Option<Worker>,
    /// Where issue work runs: setup, the worker and the check, each inside
    /// this command instead of on this machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<Sandbox>,
    /// Outside sources read for context (`insi context`), each a command
    /// that prints items as JSON lines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connectors: Vec<CommandConnector>,
}

/// An outside source read by running a command that prints one JSON object
/// per line: `{"title", "body", "url"?, "paths"?, "topic"?}`. The command
/// holds whatever login it needs; Insi holds none.
///
/// ```toml
/// [[connectors]]
/// name = "errors"
/// command = ["sentry-export", "--project", "shop", "--json-lines"]
/// trust = "untrusted"
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandConnector {
    pub name: String,
    pub command: Vec<String>,
    /// How far its items are believed; untrusted unless said otherwise —
    /// an outside source's text is anyone's until shown to be otherwise.
    #[serde(default = "untrusted")]
    pub trust: crate::Trust,
}

fn untrusted() -> crate::Trust {
    crate::Trust::Untrusted
}

/// What a sandbox command must contain: where the work's folder goes.
pub const TREE: &str = "{tree}";

/// A command every step of issue work runs inside, e.g. a container:
///
/// ```toml
/// [sandbox]
/// command = ["docker", "run", "--rm", "-v", "{tree}:/work", "-w", "/work", "node:22"]
/// ```
///
/// The step's own command is appended to it, and `{tree}` becomes the
/// worktree's path. What the container can reach is the command's to say:
/// the host's environment, credentials and network reach it only where the
/// command passes them on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sandbox {
    pub command: Vec<String>,
}

impl Sandbox {
    /// Why this sandbox cannot be used, if it cannot.
    pub fn problem(&self) -> Option<String> {
        if self.command.first().is_none_or(|p| p.trim().is_empty()) {
            return Some("sandbox.command is empty; name the program to run".to_string());
        }
        if !self.command.iter().any(|a| a.contains(TREE)) {
            return Some(format!(
                "sandbox.command has no {TREE} argument, so the work's folder would never be in it"
            ));
        }
        None
    }

    /// `argv` run inside this sandbox for the work in `tree`.
    pub fn wrap(&self, argv: &[String], tree: &Path) -> Vec<String> {
        let tree = tree.display().to_string();
        self.command
            .iter()
            .map(|a| a.replace(TREE, &tree))
            .chain(argv.iter().cloned())
            .collect()
    }
}

/// One way of asking the proposer for a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Strategy {
    pub name: String,
    /// What the proposer is told to do, in a sentence or two.
    pub prompt: String,
}

impl Strategy {
    /// The strategies used when the config names none.
    pub fn defaults() -> Vec<Strategy> {
        [
            (
                "from-failures",
                "Read the failing tasks and their failures. Add or sharpen ONE instruction \
                 that would have prevented the most common failure.",
            ),
            (
                "clarify",
                "Make ONE existing instruction clearer and more concrete, without adding a \
                 new requirement.",
            ),
            (
                "simplify",
                "Remove or shorten ONE instruction that may mislead or distract the agent.",
            ),
            (
                "checklist",
                "Add or improve a short checklist the agent works through before it stops.",
            ),
        ]
        .map(|(name, prompt)| Strategy {
            name: name.to_string(),
            prompt: prompt.to_string(),
        })
        .to_vec()
    }
}

/// US dollars per million tokens, by kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Price {
    #[serde(default)]
    pub input: f64,
    #[serde(default)]
    pub output: f64,
    #[serde(default)]
    pub cache_write: f64,
    #[serde(default)]
    pub cache_read: f64,
}

impl Price {
    /// What `usage` costs at this price, in US dollars.
    pub fn cost(&self, usage: &crate::task::Usage) -> f64 {
        (usage.input as f64 * self.input
            + usage.output as f64 * self.output
            + usage.cache_creation as f64 * self.cache_write
            + usage.cache_read as f64 * self.cache_read)
            / 1_000_000.0
    }
}

/// The worker adapter: a command run inside the task's worktree, with the
/// task's prompt in place of `{prompt}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Worker {
    pub command: Vec<String>,
    /// Files the harness writes for itself in the worktree — its state, its
    /// generated docs, its backups — as git glob pathspecs. They are left out
    /// of the worker's recorded diff, which is what a lesson is written from:
    /// a lesson about the harness's bookkeeping teaches nothing about the
    /// code. The check still runs on the whole worktree.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub own_files: Vec<String>,
}

impl Worker {
    /// The default adapter: incode, printing its events as JSON lines so the
    /// run's token use can be read back.
    ///
    /// `--spec-gate=off` because a task is already scoped: with the gate on,
    /// every attempt on a repository without incode's spec set first wrote
    /// five documents from guesses (130–170k tokens an attempt, repeated in
    /// every worktree), so the pass rate, tokens and wall time measured
    /// doc-writing as well as the fix. incode says `"spec_gate":"off"` in its
    /// result line.
    pub fn incode() -> Self {
        Worker {
            command: [
                "incode",
                "-t",
                PROMPT,
                "--output-format",
                "stream-json",
                "--spec-gate=off",
            ]
            .map(String::from)
            .to_vec(),
            // incode's state, the spec set its spec step writes when the gate
            // is on (incode src/spec.rs SPEC_SET), and the backups its edit
            // hook keeps beside each edited file.
            own_files: [
                ".incode/**",
                ".incord/**",
                "**/.backups/**",
                "docs/ARCHITECTURE.md",
                "docs/CONVENTIONS.md",
                "docs/VERIFICATION.md",
                "docs/ASSUMPTIONS.md",
            ]
            .map(String::from)
            .to_vec(),
        }
    }

    /// A worker given as a bare command, with no files of its own declared.
    pub fn command(command: Vec<String>) -> Self {
        Worker {
            command,
            own_files: Vec::new(),
        }
    }

    /// The command for one prompt. Refused when the program is missing or no
    /// argument takes the prompt: a worker that never sees the task would
    /// still be scored, as a failure the task did not earn.
    pub fn render(&self, prompt: &str) -> Result<Vec<String>, String> {
        if self.command.first().is_none_or(|p| p.trim().is_empty()) {
            return Err("worker.command is empty; name the program to run".to_string());
        }
        if !self.command.iter().any(|a| a.contains(PROMPT)) {
            return Err(format!(
                "worker.command has no {PROMPT} argument, so the worker would never see the task"
            ));
        }
        Ok(self
            .command
            .iter()
            .map(|a| a.replace(PROMPT, prompt))
            .collect())
    }

    /// The command for one prompt run in `dir`: [`Self::render`], except that
    /// a prompt too long for a command line ([`ARGV_PROMPT_MAX`]) is written
    /// whole to [`TASK_FILE`] in `dir` and the worker is told to read it.
    /// Measured: Insi's lesson writer once handed incode a prompt past
    /// Windows' 32,767-character command line, which failed to start at all
    /// (os error 206). Called for every worker Insi runs: attempts
    /// (insi-eval run.rs), the lesson writer (insi learn.rs), the proposer
    /// (insi improve.rs) and issue work (insi issue.rs).
    pub fn render_in(&self, prompt: &str, dir: &Path) -> Result<Vec<String>, String> {
        if prompt.len() <= ARGV_PROMPT_MAX {
            return self.render(prompt);
        }
        let path = dir.join(TASK_FILE);
        std::fs::write(&path, prompt).map_err(|e| {
            format!(
                "the task is {} bytes, more than a command line can carry, and could not \
                 be written to {} for the worker to read: {e}",
                prompt.len(),
                path.display()
            )
        })?;
        self.render(&format!(
            "This task is {} bytes, more than a command line can carry, so the whole of it \
             is in the file `{TASK_FILE}` in your working folder ({}). Read that file in \
             full, from its first line to its last, before doing anything else: it is the \
             task, and what it asks is what you do.",
            prompt.len(),
            path.display()
        ))
    }
}

/// The longest prompt passed on a command line, in bytes (each at least one
/// UTF-16 unit, so this is under Windows' 32,767-character limit with room
/// for the program, its flags and quoting).
pub const ARGV_PROMPT_MAX: usize = 24_000;

/// Where [`Worker::render_in`] writes a prompt too long for a command line,
/// in the worker's folder; left out of an attempt's recorded diff.
pub const TASK_FILE: &str = ".insi-task.md";

impl Config {
    /// Read `<repo>/.insi/config.toml`; a repo without one has the defaults.
    pub fn load(repo: &Path) -> Result<Config, String> {
        let path = repo.join(DIR).join(FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(format!("could not read {}: {e}", path.display())),
        }
    }

    pub fn parse(text: &str) -> Result<Config, String> {
        let config: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        if let Some(why) = config
            .topic
            .as_deref()
            .and_then(crate::topic::topic_problem)
            .or_else(|| config.topic_rules.iter().find_map(|r| r.problem()))
        {
            return Err(why);
        }
        if let Some(why) = config.sandbox.as_ref().and_then(Sandbox::problem) {
            return Err(why);
        }
        let mut sources = std::collections::BTreeSet::new();
        for c in &config.connectors {
            if c.name.trim().is_empty() || c.command.first().is_none_or(|p| p.trim().is_empty()) {
                return Err("a connector needs a name and a command".to_string());
            }
            if matches!(c.name.as_str(), "docs" | "ci") || !sources.insert(c.name.as_str()) {
                return Err(format!(
                    "connector \"{}\" is named twice (docs and ci are built in)",
                    c.name
                ));
            }
        }
        let mut names = std::collections::BTreeSet::new();
        for s in &config.strategies {
            if s.name.trim().is_empty() || s.prompt.trim().is_empty() {
                return Err("a strategy needs a name and a prompt".to_string());
            }
            if !names.insert(s.name.as_str()) {
                return Err(format!("strategy \"{}\" is named twice", s.name));
            }
        }
        Ok(config)
    }

    /// The base topic and this repo's rules, `base` standing in when no
    /// `topic` is set.
    pub fn topics<'a>(&'a self, base: &'a str) -> crate::topic::Topics<'a> {
        crate::topic::Topics {
            base: self.topic.as_deref().unwrap_or(base),
            rules: &self.topic_rules,
        }
    }

    /// The worker this repo names, else incode.
    pub fn worker(&self) -> Worker {
        self.worker.clone().unwrap_or_else(Worker::incode)
    }

    /// The strategies this repo names, else [`Strategy::defaults`].
    pub fn strategies(&self) -> Vec<Strategy> {
        if self.strategies.is_empty() {
            Strategy::defaults()
        } else {
            self.strategies.clone()
        }
    }

    /// The config as the file `insi init` writes.
    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string(self).map_err(|e| e.to_string())
    }

    /// Where tasks and hidden tests live by default, given the folder the
    /// config was read from: `private_checks` resolved against it, with
    /// `tasks/` and `hidden/` inside. `None` when it names none.
    pub fn private_dirs(&self, repo: &Path) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        let base = repo.join(self.private_checks.as_deref()?);
        Some((base.join("tasks"), base.join("hidden")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_config_reads_and_unknown_keys_are_refused() {
        let config = Config::parse(
            r#"
            topic = "incord/memory-service"
            test_cmd = ["cargo", "test"]
            protected = [".github/workflows/**", "evals/**"]
            eval_tasks = "evals/tasks"
            private_checks = "../insi-private"
            trusted_issue_label = "agent-ok"
            [worker]
            command = ["my-agent", "--task", "{prompt}"]
            own_files = [".my-agent/**"]
            "#,
        )
        .expect("valid");
        assert_eq!(config.test_cmd, ["cargo", "test"]);
        assert_eq!(config.worker().command[0], "my-agent");
        assert_eq!(config.worker().own_files, [".my-agent/**"]);

        let why = Config::parse("tset_cmd = [\"x\"]").expect_err("misspelt");
        assert!(why.contains("tset_cmd"), "{why}");
    }

    /// **A CONNECTOR'S ITEMS ARE UNTRUSTED UNLESS THE CONFIG SAYS OTHERWISE.**
    #[test]
    fn connectors_default_to_untrusted_and_need_a_unique_name() {
        let config = Config::parse(
            "[[connectors]]\nname = \"errors\"\ncommand = [\"export\"]\n\
             [[connectors]]\nname = \"wiki\"\ncommand = [\"wiki\"]\ntrust = \"trusted\"\n",
        )
        .expect("valid");
        assert_eq!(config.connectors[0].trust, crate::Trust::Untrusted);
        assert_eq!(config.connectors[1].trust, crate::Trust::Trusted);
        assert!(Config::parse("[[connectors]]\nname = \"docs\"\ncommand = [\"x\"]").is_err());
        assert!(Config::parse("[[connectors]]\nname = \"e\"\ncommand = []").is_err());
        assert!(Config::parse(
            "[[connectors]]\nname = \"e\"\ncommand = [\"x\"]\n[[connectors]]\nname = \"e\"\ncommand = [\"y\"]"
        )
        .is_err());
    }

    /// **A SANDBOX HOLDS THE WORK'S FOLDER, OR IS REFUSED.** The step's
    /// command follows the sandbox's, with `{tree}` filled in.
    #[test]
    fn a_sandbox_wraps_each_command_around_the_work_folder() {
        let config = Config::parse(
            "[sandbox]\ncommand = [\"docker\", \"run\", \"-v\", \"{tree}:/work\", \"img\"]",
        )
        .expect("valid");
        let wrapped = config
            .sandbox
            .expect("sandbox")
            .wrap(&["npm".into(), "test".into()], Path::new("/w/issue-1"));
        assert_eq!(
            wrapped,
            [
                "docker",
                "run",
                "-v",
                "/w/issue-1:/work",
                "img",
                "npm",
                "test"
            ]
        );
        let why = Config::parse("[sandbox]\ncommand = [\"docker\", \"run\", \"img\"]")
            .expect_err("no tree");
        assert!(why.contains("{tree}"), "{why}");
        assert!(Config::parse("[sandbox]\ncommand = []").is_err());
    }

    /// **TOPIC RULES ARE READ, AND ONE THAT CAN NEVER APPLY IS REFUSED.**
    #[test]
    fn topic_rules_are_read_and_checked() {
        let config = Config::parse(
            r#"
            topic = "shop"
            [[topic_rules]]
            topic = "shop/api"
            paths = ["src/api"]
            commands = ["npm run test:api"]
            "#,
        )
        .expect("valid");
        let topics = config.topics("unused");
        assert_eq!(topics.base, "shop");
        assert_eq!(topics.for_paths(&["src/api/a.ts".to_string()]), "shop/api");
        assert_eq!(Config::default().topics("repo").base, "repo");

        let why = Config::parse("[[topic_rules]]\ntopic = \"shop/api\"").expect_err("empty");
        assert!(why.contains("no paths"), "{why}");
        assert!(Config::parse("topic = \"shop//x\"").is_err());
        assert!(Config::parse("[[topic_rules]]\ntopic = \"a\"\npath = [\"x\"]").is_err());
    }

    /// **A PRICE IS PER MILLION TOKENS, PER KIND.** Read from the config by
    /// model name; a kind left out costs nothing; a misspelt kind is refused.
    #[test]
    fn prices_are_read_by_model_and_applied_per_kind() {
        let config = Config::parse(
            r#"
            [prices."claude-sonnet"]
            input = 3.0
            output = 15.0
            cache_write = 3.75
            cache_read = 0.3
            [prices."subscription-model"]
            "#,
        )
        .expect("valid");
        let usage = crate::task::Usage {
            input: 1_000_000,
            output: 100_000,
            cache_creation: 0,
            cache_read: 2_000_000,
        };
        let cost = config.prices["claude-sonnet"].cost(&usage);
        assert!((cost - (3.0 + 1.5 + 0.6)).abs() < 1e-9, "{cost}");
        assert_eq!(config.prices["subscription-model"].cost(&usage), 0.0);
        assert!(Config::parse("[prices.m]\ninptu = 1.0").is_err());
    }

    #[test]
    fn an_absent_config_is_the_defaults_with_incode_as_the_worker() {
        let dir = std::env::temp_dir().join(format!("insi-config-{}", std::process::id()));
        let config = Config::load(&dir).expect("absent is fine");
        assert_eq!(config, Config::default());
        assert_eq!(config.worker().command[0], "incode");
    }

    #[test]
    fn a_worker_must_be_handed_the_prompt() {
        let argv = Worker::incode().render("fix the bug").expect("renders");
        assert!(argv.contains(&"fix the bug".to_string()), "{argv:?}");
        assert!(
            argv.contains(&"--spec-gate=off".to_string()),
            "every attempt would write incode's spec set before the fix: {argv:?}"
        );

        let blind = Worker::command(vec!["agent".into(), "--go".into()]);
        assert!(blind
            .render("x")
            .expect_err("no prompt")
            .contains("{prompt}"));
        let empty = Worker::command(vec![]);
        assert!(empty.render("x").is_err());
    }

    /// **A PROMPT TOO LONG FOR A COMMAND LINE STILL REACHES THE WORKER.**
    /// Handed straight to the program, a 200 KB prompt cannot even start it
    /// (os error 206 on Windows, E2BIG elsewhere): the failure the lesson
    /// writer hit. Through `render_in` the program starts, is told where the
    /// task is, and the file holds every byte of it; a short prompt still goes
    /// on the command line and writes nothing.
    #[test]
    fn a_prompt_too_long_for_a_command_line_reaches_the_worker_whole() {
        let dir = std::env::temp_dir().join(format!(
            "insi-render-in-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("dir");
        // A real program that keeps what it was given: git writes the
        // argument into a config file.
        let worker = Worker::command(
            ["git", "config", "--file", "told.ini", "task.text", PROMPT]
                .map(String::from)
                .to_vec(),
        );
        let spawn = |argv: &[String]| {
            std::process::Command::new(&argv[0])
                .args(&argv[1..])
                .current_dir(&dir)
                .output()
        };
        let prompt = format!("Write one lesson.\n{}\nTHE END", "x".repeat(200_000));

        let direct = worker.render(&prompt).expect("renders");
        assert!(
            spawn(&direct).is_err(),
            "the failure being fixed: too long to start"
        );

        let argv = worker.render_in(&prompt, &dir).expect("renders");
        let ran = spawn(&argv).expect("starts");
        assert!(
            ran.status.success(),
            "{}",
            String::from_utf8_lossy(&ran.stderr)
        );
        let told = std::fs::read_to_string(dir.join("told.ini")).expect("told");
        assert!(
            told.contains(TASK_FILE) && told.contains("Read that file in full"),
            "{told}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(TASK_FILE)).expect("task file"),
            prompt,
            "every byte of the task"
        );

        std::fs::remove_file(dir.join(TASK_FILE)).expect("clear");
        assert_eq!(
            worker.render_in("fix it", &dir).expect("renders"),
            worker.render("fix it").expect("renders")
        );
        assert!(
            !dir.join(TASK_FILE).exists(),
            "a short prompt writes no file"
        );
    }
}
