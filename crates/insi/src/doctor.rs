//! Offline bug-report diagnostics, dispatched before Work can start cleanup.
//! Only git's version command is executed. Worker/configured commands are
//! inspected as files, never run; arbitrary config strings are never emitted.
use std::path::Path;
use std::time::Duration;

use insi_core::Config;
use serde_json::Value;

/// Collect a shareable report without creating a work folder or lock file.
pub fn report(repo: &Path, work: &Path, markdown: bool) -> String {
    let mut lines = vec![
        format!("Insi: {}", env!("CARGO_PKG_VERSION")),
        format!(
            "Build commit: {} (working tree modifications not recorded)",
            env!("INSI_BUILD_COMMIT")
        ),
        format!("Target: {}", env!("INSI_BUILD_TARGET")),
        format!("OS: {} / {}", std::env::consts::OS, std::env::consts::ARCH),
        format!("Git: {}", git_version(repo)),
        "Config path: <repo>/.insi/config.toml".into(),
    ];
    match Config::load(repo) {
        Ok(config) => {
            lines.push(
                if repo.join(".insi/config.toml").exists() {
                    "Config: loaded"
                } else {
                    "Config: absent (defaults)"
                }
                .into(),
            );
            let program = config.worker().command.first().cloned().unwrap_or_default();
            let status = executable_found(&program, repo);
            // Never print arbitrary program names, which may themselves be secrets.
            let name = match program.as_str() {
                "incode" | "claude" | "codex" | "aider" | "opencode" => &program,
                _ => "<custom worker>",
            };
            lines.push(format!(
                "Configured worker {name}: {}",
                if status { "found" } else { "not found" }
            ));
            let mut value = serde_json::to_value(&config).unwrap_or(Value::Null);
            if let Some(prices) = value.get_mut("prices") {
                *prices = Value::String("<redacted model names and prices>".into());
            }
            redact(&mut value);
            lines.push(format!(
                "Config (all strings redacted): {}",
                serde_json::to_string_pretty(&value).unwrap_or_default()
            ));
        }
        Err(_) => {
            lines.push("Config: invalid or unreadable (details omitted to protect secrets)".into())
        }
    }
    for lock in [
        "git.lock",
        "results.jsonl.lock",
        "lessons.jsonl.lock",
        "loop/audit.jsonl.lock",
        "loop/bandit.json.lock",
    ] {
        lines.push(format!(
            "Work lock {lock}: {}",
            lock_state(&work.join(lock))
        ));
    }
    lines.push(
        "Lock states are a snapshot; absent lock files do not prove no worker is running.".into(),
    );
    let body = lines.join("\n");
    if markdown {
        format!("### Insi diagnostics\n\n```text\n{body}\n```")
    } else {
        body
    }
}

fn redact(value: &mut Value) {
    match value {
        Value::String(s) => *s = "<redacted>".into(),
        Value::Array(values) => values.iter_mut().for_each(redact),
        Value::Object(values) => values.values_mut().for_each(redact),
        _ => {}
    }
}

fn git_version(repo: &Path) -> String {
    let args = vec!["git".into(), "--version".into()];
    let Ok(out) = insi_eval::process::run(&args, repo, Duration::from_secs(5)) else {
        return "unavailable".into();
    };
    if !out.success() {
        return "unavailable".into();
    }
    let version = out
        .stdout
        .strip_prefix("git version ")
        .unwrap_or_default()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .take(32)
        .collect::<String>();
    if version.is_empty() {
        "unavailable".into()
    } else {
        version.trim_end_matches('.').into()
    }
}

fn executable_found(program: &str, repo: &Path) -> bool {
    let path = Path::new(program);
    if program.is_empty() {
        return false;
    }
    let usable = |p: &Path| {
        let Ok(meta) = p.metadata() else {
            return false;
        };
        if !meta.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            meta.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    };
    if path.is_absolute() || path.components().count() > 1 {
        return usable(&repo.join(path));
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        let dir = if dir.is_absolute() {
            dir
        } else {
            repo.join(dir)
        };
        if usable(&dir.join(program)) {
            return true;
        }
        #[cfg(windows)]
        for extension in ["exe", "cmd", "bat"] {
            if usable(&dir.join(format!("{program}.{extension}"))) {
                return true;
            }
        }
        false
    })
}

fn lock_state(path: &Path) -> &'static str {
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return "absent",
        Err(_) => return "unreadable",
    };
    match file.try_lock() {
        Ok(()) => "available",
        Err(std::fs::TryLockError::WouldBlock) => "held",
        Err(_) => "unavailable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> std::path::PathBuf {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.claude/temp")
            .join(format!(
                "doctor-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock")
                    .as_nanos()
            ));
        std::fs::create_dir_all(&root).expect("scratch");
        root
    }

    #[test]
    fn report_redacts_strings_and_dynamic_model_names_without_creating_work() {
        let root = folder();
        std::fs::create_dir(root.join(".insi")).expect("config dir");
        std::fs::write(root.join(".insi/config.toml"), "topic = 'private-topic'\nprivate_checks = '/secret/path'\n[worker]\ncommand = ['secret-binary', '--token', 'secret-argument', '{prompt}']\n[prices.'secret-model']\ninput = 3.0\n").expect("config");
        let work = root.join("work");
        let report = report(&root, &work, true);
        for secret in [
            "private-topic",
            "/secret/path",
            "secret-binary",
            "secret-argument",
            "secret-model",
        ] {
            assert!(!report.contains(secret), "leaked {secret}");
        }
        assert!(report.contains("```text"));
        assert!(!work.exists());
        std::fs::write(root.join(".insi/config.toml"), "invalid secret-parse-error")
            .expect("invalid");
        let invalid = super::report(&root, &work, false);
        assert!(invalid.contains("invalid or unreadable"));
        assert!(!invalid.contains("secret-parse-error"));
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn lock_probe_never_creates_files_and_does_not_wait_for_holders() {
        let root = folder();
        let lock = root.join("probe.lock");
        assert_eq!(lock_state(&lock), "absent");
        assert!(!lock.exists());
        let file = std::fs::File::create(&lock).expect("lock");
        file.lock().expect("held");
        assert_eq!(lock_state(&lock), "held");
        drop(file);
        assert_eq!(lock_state(&lock), "available");
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn executable_worker_is_detected_without_running_it() {
        use std::os::unix::fs::PermissionsExt;
        let root = folder();
        std::fs::create_dir(root.join(".insi")).expect("config dir");
        let program = root.join("worker");
        std::fs::write(&program, "#!/bin/sh\ntouch worker-ran\n").expect("worker");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
            .expect("executable");
        std::fs::write(
            root.join(".insi/config.toml"),
            "[worker]\ncommand = ['./worker', '{prompt}']\n",
        )
        .expect("config");
        assert!(report(&root, &root.join("work"), false).contains("<custom worker>: found"));
        assert!(!root.join("worker-ran").exists());
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
