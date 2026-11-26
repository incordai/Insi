// Copyright (c) 2026-2027 Incord Ai.
// https://incord.ai
//
// SPDX-License-Identifier: BUSL-1.1

//! Insi's watch module: the catalog, the checks it enables by itself, and the owner's checks kept intact.
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

/// A git Rust project: what the secrets, dependency and lint checks look for.
fn rust_git_project() -> std::path::PathBuf {
    let root = scratch();
    std::fs::create_dir(root.join(".git")).expect("git");
    std::fs::write(root.join("Cargo.toml"), "[package]\nname='p'\n").expect("toml");
    std::fs::write(root.join("Cargo.lock"), "version = 4\n").expect("lock");
    root
}

fn names(text: &str) -> Vec<String> {
    active_checks(&toml::from_str(text).expect("TOML"))
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

const EVERY_PROGRAM: &dyn Fn(&Auto) -> bool = &|_| true;

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

/// Commented templates must remain valid Inmem-style check tables when enabled, and each check Insi enables by itself
/// is the same as its template, so the two never drift apart.
#[test]
fn optional_catalog_checks_are_valid_toml_and_match_what_insi_enables() {
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
    for auto in AUTO {
        let template = checks
            .iter()
            .find(|c| c["name"].as_str() == Some(auto.name))
            .unwrap_or_else(|| panic!("no template named {}", auto.name));
        assert_eq!(
            template["command"].as_str(),
            Some(auto.command),
            "{}",
            auto.name
        );
        assert_eq!(
            template["timeout_secs"].as_integer(),
            Some(auto.timeout_secs as i64),
            "{}",
            auto.name
        );
        assert_eq!(
            template["every_minutes"].as_integer(),
            Some(auto.every_minutes as i64),
            "{}",
            auto.name
        );
    }
}

/// An unknown stack receives examples only; no guessed test command is scheduled.
#[test]
fn unknown_stack_has_no_active_check() {
    let root = scratch();
    install_with(&root, &[], EVERY_PROGRAM).expect("install");
    let text = std::fs::read_to_string(root.join(".incord/watch.toml")).expect("read");
    let parsed: toml::Value = toml::from_str(&text).expect("TOML");
    assert!(parsed.get("check").is_none());
}

/// A new git Rust project gets the secrets, dependency and lint checks enabled, active, with the ledger naming them;
/// the checks whose files the project lacks stay templates.
#[test]
fn a_new_project_gets_the_checks_that_fit_enabled() {
    let root = rust_git_project();
    assert!(install_with(&root, &["cargo".into(), "test".into()], EVERY_PROGRAM).expect("install"));
    let text = std::fs::read_to_string(root.join(".incord/watch.toml")).expect("read");
    assert_eq!(
        names(&text),
        [
            "insi: security secrets",
            "insi: security dependencies and configuration",
            "insi: rust strict lint",
            "insi: project full test suite"
        ]
    );
    assert!(text.contains(&format!(
        "{LEDGER}insi: security secrets | insi: security dependencies and configuration | insi: rust strict lint\n"
    )));
    // Everything Insi wrote is inside its module.
    let after_end = text.split(END).nth(1).expect("end marker");
    assert_eq!(after_end.trim(), "");
}

/// A check whose program does not answer here is left a template, never scheduled to fail.
#[test]
fn a_check_whose_program_is_missing_stays_a_template() {
    let root = rust_git_project();
    install_with(&root, &[], &|c: &Auto| c.program != "trivy").expect("install");
    let text = std::fs::read_to_string(root.join(".incord/watch.toml")).expect("read");
    assert_eq!(
        names(&text),
        ["insi: security secrets", "insi: rust strict lint"]
    );
}

/// An owner's file keeps every byte; Insi's module with the fitting checks goes after it, once. A check the owner then
/// removes is never enabled again, and one whose program later appears is added without touching the rest.
#[test]
fn an_owners_file_gets_the_fitting_checks_once_and_a_removed_one_stays_removed() {
    let root = rust_git_project();
    std::fs::create_dir(root.join(".incord")).expect("dir");
    let path = root.join(".incord/watch.toml");
    let original = "[[check]]\nname = \"builds\"\ncommand = \"cargo check\"\n";
    std::fs::write(&path, original).expect("write");
    assert!(install_with(&root, &[], &|c: &Auto| c.program != "trivy").expect("install"));
    let text = std::fs::read_to_string(&path).expect("read");
    assert!(text.starts_with(original));
    assert_eq!(
        names(&text),
        ["builds", "insi: security secrets", "insi: rust strict lint"]
    );
    assert!(!install_with(&root, &[], &|c: &Auto| c.program != "trivy").expect("again"));

    // The owner removes the lint check; trivy becomes available.
    let lint = "[[check]]\nname = \"insi: rust strict lint\"\ncommand = \"cargo clippy --workspace --all-targets --locked -- -D warnings\"\nevery_minutes = 1440\ntimeout_secs = 600\n\n";
    assert!(text.contains(lint));
    let edited = text.replace(lint, "");
    std::fs::write(&path, &edited).expect("owner removes");
    assert!(install_with(&root, &[], EVERY_PROGRAM).expect("trivy now"));
    let text = std::fs::read_to_string(&path).expect("read");
    assert!(text.starts_with(original));
    assert_eq!(
        names(&text),
        [
            "builds",
            "insi: security secrets",
            "insi: security dependencies and configuration"
        ]
    );
    assert_eq!(ledger(&text).len(), 3, "{text}");
    assert_eq!(text.matches(LEDGER).count(), 1);
    assert!(!install_with(&root, &[], EVERY_PROGRAM).expect("settled"));
}

/// A project set up before Insi enabled checks (its module all comments) gets them inside the module; the bytes before
/// the module and from its end marker on are unchanged.
#[test]
fn an_older_module_gets_its_checks_inside_it_and_nothing_else_moves() {
    let root = rust_git_project();
    std::fs::create_dir(root.join(".incord")).expect("dir");
    let path = root.join(".incord/watch.toml");
    let before = "[[check]]\nname = \"tests\"\ncommand = \"npm test\"\n\n";
    let after = "\n# OTHER MODULE\n[[check]]\nname = \"other\"\ncommand = \"other-tool\"\n";
    let old = format!("{before}{}{after}", CATALOG.trim_end());
    std::fs::write(&path, &old).expect("write");
    assert!(install_with(&root, &[], EVERY_PROGRAM).expect("install"));
    let text = std::fs::read_to_string(&path).expect("read");
    assert!(text.starts_with(before));
    assert!(text.ends_with(&format!("{END}{after}")));
    assert_eq!(names(&text).len(), 5);
}

/// A check is not scheduled twice: an active check that already runs the program keeps the job.
#[test]
fn an_active_check_running_the_same_program_is_not_doubled() {
    let root = rust_git_project();
    std::fs::create_dir(root.join(".incord")).expect("dir");
    let path = root.join(".incord/watch.toml");
    std::fs::write(
        &path,
        "[[check]]\nname = \"mine\"\ncommand = \"gitleaks detect --source .\"\n[[check]]\nname = \"lint\"\ncommand = \"cargo clippy -- -D warnings\"\n",
    )
    .expect("write");
    install_with(&root, &[], EVERY_PROGRAM).expect("install");
    let text = std::fs::read_to_string(&path).expect("read");
    assert_eq!(
        names(&text),
        [
            "mine",
            "lint",
            "insi: security dependencies and configuration"
        ]
    );
}

/// A file with Windows line endings keeps them in what Insi adds and still parses.
#[test]
fn windows_line_endings_are_kept() {
    let root = rust_git_project();
    std::fs::create_dir(root.join(".incord")).expect("dir");
    let path = root.join(".incord/watch.toml");
    std::fs::write(
        &path,
        "[[check]]\r\nname = \"builds\"\r\ncommand = \"cargo check\"\r\n",
    )
    .expect("write");
    install_with(&root, &[], EVERY_PROGRAM).expect("install");
    let text = std::fs::read_to_string(&path).expect("read");
    assert_eq!(text.matches('\n').count(), text.matches("\r\n").count());
    assert_eq!(names(&text).len(), 4);
}

/// The dependency check fits a lockfile or a container file up to three folders deep, but not one inside a package or
/// build folder.
#[test]
fn dependency_files_are_found_in_the_project_not_in_its_package_folders() {
    let root = scratch();
    assert!(!has_dependency_or_config_files(&root));
    std::fs::create_dir_all(root.join("node_modules/x")).expect("dir");
    std::fs::write(root.join("node_modules/x/package-lock.json"), "{}").expect("lock");
    assert!(!has_dependency_or_config_files(&root));
    std::fs::create_dir_all(root.join("services/api")).expect("dir");
    std::fs::write(root.join("services/api/Dockerfile"), "FROM scratch\n").expect("docker");
    assert!(has_dependency_or_config_files(&root));
}

/// Live: a program that is here answers its probe; one that is not is never enabled.
#[test]
fn the_probe_runs_the_real_program() {
    let real = Auto {
        program: "cargo",
        probe: &["--version"],
        ..AUTO[2]
    };
    assert!(program_answers(&real));
    let missing = Auto {
        program: "no-such-program-insi-test",
        ..AUTO[2]
    };
    assert!(!program_answers(&missing));
    // A program the command also starts must answer too.
    let needs_missing = Auto {
        program: "cargo",
        probe: &["--version"],
        needs: &["no-such-program-insi-test"],
        ..AUTO[2]
    };
    assert!(!program_answers(&needs_missing));
    let needs_cargo = Auto {
        program: "cargo",
        probe: &["--version"],
        needs: &["cargo"],
        ..AUTO[2]
    };
    assert!(program_answers(&needs_cargo));
}

/// A repository configured with `.insi/` enables the runtime-errors watch check when `insi` is available.
#[test]
fn insi_project_enables_runtime_errors_check() {
    let root = scratch();
    std::fs::create_dir(root.join(".insi")).expect("insi dir");
    assert!(install_with(&root, &[], EVERY_PROGRAM).expect("install"));
    let text = std::fs::read_to_string(root.join(".incord/watch.toml")).expect("read");
    assert_eq!(names(&text), ["insi: runtime errors"]);
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
