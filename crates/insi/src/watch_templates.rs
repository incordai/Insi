//! Shared Inmem watch starter: additive, no scheduler and no automatic tool installation.
use std::io::Write;
use std::path::Path;

const CATALOG: &str = include_str!("watch_templates.toml");
const BEGIN: &str = "# BEGIN INSI WATCH MODULE";
const END: &str = "# END INSI WATCH MODULE";

/// Add the catalog once; existing checks are preserved and receive no duplicate active suite.
pub fn install(repo: &Path, test_command: &[String]) -> Result<bool, String> {
    let path = repo.join(".incord/watch.toml");
    // Refuse redirecting generated writes through a symlinked config file or directory.
    for checked in [repo.join(".incord"), path.clone()] {
        if std::fs::symlink_metadata(&checked).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("watch template path must not be a symlink".into());
        }
    }
    if let Ok(existing) = std::fs::read_to_string(&path) {
        toml::from_str::<toml::Value>(&existing)
            .map_err(|e| format!("existing watch file is invalid: {e}"))?;
        // A partial section is not safe to repair automatically; preserve all agent and owner content.
        if existing.contains(BEGIN) != existing.contains(END) {
            return Err(
                "incomplete Insi watch module; preserve the file and repair its markers".into(),
            );
        }
        if existing.contains(BEGIN) || existing.contains("# INSI CHECK CATALOG v1") {
            return Ok(false);
        }
        // Append only comments to an owner's file; its commands and scheduling stay byte-for-byte intact.
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        file.write_all(format!("\n\n{CATALOG}").as_bytes())
            .map_err(|e| e.to_string())?;
        return Ok(true);
    }
    std::fs::create_dir_all(repo.join(".incord")).map_err(|e| e.to_string())?;
    // Keep every generated active check inside Insi's segment, not among another module's checks.
    let mut text = CATALOG
        .trim_end()
        .strip_suffix(END)
        .ok_or("watch catalog end marker missing")?
        .to_string();
    if !test_command.is_empty() {
        // Serialize TOML strings rather than interpolating paths or shell punctuation into TOML.
        let command = test_command
            .iter()
            .map(|arg| crate::init::shell_quoted(arg))
            .collect::<Vec<_>>()
            .join(" ");
        let value = toml::Value::String(command).to_string();
        text.push_str(&format!("\n# Detected full suite; enabled for this repository.\n[[check]]\nname = \"insi: project full test suite\"\ncommand = {value}\nevery_minutes = 1440\ntimeout_secs = 600\npaths = []\n"));
    }
    text.push_str(&format!("{END}\n"));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// All artifacts belong to this checkout's ignored scratch directory.
    fn scratch() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../.incord/temp/watch-{}-{stamp}-{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("scratch");
        path
    }

    /// The detected command remains a single TOML string and repeated runs preserve agent edits.
    #[test]
    fn starter_is_parseable_quoted_and_idempotent() {
        let root = scratch();
        let argv = ["runner", "path with spaces", "a'b", "$(not-executed)"].map(String::from);
        assert!(install(&root, &argv).expect("install"));
        let path = root.join(".incord/watch.toml");
        let text = std::fs::read_to_string(&path).expect("read");
        let value: toml::Value = toml::from_str(&text).expect("TOML");
        assert_eq!(value["check"].as_array().expect("checks").len(), 1);
        assert_eq!(
            value["check"][0]["command"].as_str(),
            Some("runner 'path with spaces' 'a'\\''b' '$(not-executed)'")
        );
        let edited = format!("{text}\n# Owner's added note\n");
        std::fs::write(&path, &edited).expect("owner edit");
        assert!(!install(&root, &argv).expect("again"));
        assert_eq!(std::fs::read_to_string(path).expect("read"), edited);
    }

    /// Existing checks retain their exact bytes and receive only a commented catalog once.
    #[test]
    fn existing_checks_are_preserved_without_duplicate_scheduling() {
        let root = scratch();
        std::fs::create_dir(root.join(".incord")).expect("dir");
        let path = root.join(".incord/watch.toml");
        let original = "[[check]]\nname='owner check'\ncommand='owner-tool'\nevery_minutes=17\ntimeout_secs=21\npaths=['src']\n";
        std::fs::write(&path, original).expect("write");
        assert!(install(&root, &["cargo".into(), "test".into()]).expect("install"));
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.starts_with(original));
        let value: toml::Value = toml::from_str(&text).expect("TOML");
        assert_eq!(value["check"].as_array().expect("checks").len(), 1);
        assert!(!install(&root, &[]).expect("again"));
        // Another agent may extend the Insi segment and append an independent module after it.
        let edited = text.replace(END, &format!("[[check]]\nname='insi: agent custom'\ncommand='custom-validator'\nevery_minutes=30\ntimeout_secs=45\npaths=[]\n{END}")) + "\n# OTHER AGENT MODULE\n[[check]]\nname='inmem: other'\ncommand='other-validator'\nevery_minutes=12\ntimeout_secs=30\npaths=[]\n";
        std::fs::write(&path, &edited).expect("agent edit");
        assert!(!install(&root, &["cargo".into(), "test".into()]).expect("preserve modules"));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), edited);
        let value: toml::Value = toml::from_str(&edited).expect("edited TOML");
        assert_eq!(value["check"].as_array().expect("checks").len(), 3);
    }

    /// Commented templates must remain valid Inmem-style check tables when enabled.
    #[test]
    fn optional_catalog_checks_are_valid_toml() {
        let enabled = CATALOG
            .lines()
            .filter_map(|line| line.strip_prefix("# "))
            .filter(|line| {
                line.starts_with("[[check]]")
                    || [
                        "name =",
                        "command =",
                        "every_minutes =",
                        "timeout_secs =",
                        "paths =",
                    ]
                    .iter()
                    .any(|key| line.starts_with(key))
            })
            .collect::<Vec<_>>()
            .join("\n");
        let value: toml::Value = toml::from_str(&enabled).expect("catalog TOML");
        let checks = value["check"].as_array().expect("checks");
        assert!(checks.len() >= 6);
        for check in checks {
            assert!(check["timeout_secs"].as_integer().expect("timeout") > 0);
        }
    }

    /// An unknown stack receives examples only; no guessed test command is scheduled.
    #[test]
    fn unknown_stack_has_no_active_check() {
        let root = scratch();
        install(&root, &[]).expect("install");
        let text = std::fs::read_to_string(root.join(".incord/watch.toml")).expect("read");
        let parsed: toml::Value = toml::from_str(&text).expect("TOML");
        assert!(parsed.get("check").is_none());
    }

    /// A redirected config path must not modify another checkout's owner file.
    #[cfg(unix)]
    #[test]
    fn symlinked_watch_files_are_rejected() {
        let root = scratch();
        std::fs::create_dir(root.join(".incord")).expect("dir");
        let outside = root.join("owner.toml");
        std::fs::write(&outside, "# owner's file\n").expect("write");
        std::os::unix::fs::symlink(&outside, root.join(".incord/watch.toml")).expect("link");
        assert!(install(&root, &[]).is_err());
        assert_eq!(
            std::fs::read_to_string(outside).expect("read"),
            "# owner's file\n"
        );
    }
}
