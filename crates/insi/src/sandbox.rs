// Copyright (c) 2026-2027 Incord Ai.
// https://incord.ai
//
// SPDX-License-Identifier: BUSL-1.1

//! `insi sandbox run [-q] [--best-effort] (-w PATH | --deny-read PATH)... -- PROGRAM [ARGS...]`: the worker's
//! sandbox, built into Insi. It is the incord-sandbox library (github.com/incordai/Incode, crates/sandbox), the same
//! sandbox as that repository's `insbx` binary, taking the same flags, so nothing else is installed for it (the owner,
//! 2026-11-17). On macOS it is the system's own sandbox (Seatbelt), so the worker keeps the signed-in agent's
//! sign-in; on Linux Landlock and seccomp; on Windows a write-restricted token.
//!
//! Reached from: `main`, before Insi's own flag parsing (which stops at `--`), through the `[worker] sandbox` prefix
//! that the presets write (`insi_core::config::Worker::preset`, program `{insi}`); `Sandbox::wrap` there puts it before
//! each command that `issue.rs`, `autonomous.rs`, `benchmark.rs` and `review.rs` run. Calls
//! `incord_sandbox::Sandbox::run`, whose helper is this binary again: `main` calls `incord_sandbox::enter_if_requested`
//! before anything else. A timeout is Insi's: `insi_eval::process::run` kills the process group, which the sandboxed
//! command stays in (no wall-clock limit is set here, so it gets no group of its own).
//! Sibling: a hand-written `[worker] sandbox` may still name `insbx` (or any other wrapper) directly.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use incord_sandbox::{FsMode, Policy, Sandbox, EXIT_SANDBOX};

const USAGE: &str =
    "usage: insi sandbox run [-q] [--best-effort] (-w PATH | --deny-read PATH)... -- PROGRAM [ARGS...]";

/// What `insi sandbox run` was asked for.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Request {
    pub write: Vec<PathBuf>,
    pub deny_read: Vec<PathBuf>,
    pub quiet: bool,
    pub best_effort: bool,
    pub command: Vec<String>,
}

/// `args` are what follows `insi sandbox`.
pub(crate) fn parse(args: &[String]) -> Result<Request, String> {
    let Some((verb, rest)) = args.split_first() else {
        return Err(USAGE.into());
    };
    if verb != "run" {
        return Err(format!("sandbox: unknown command `{verb}`\n{USAGE}"));
    }
    let mut request = Request::default();
    let mut i = 0;
    while i < rest.len() {
        let arg = rest[i].as_str();
        match arg {
            "--" => {
                request.command = rest[i + 1..].to_vec();
                break;
            }
            "-q" | "--quiet" => request.quiet = true,
            "--best-effort" => request.best_effort = true,
            "-w" | "--write" | "--deny-read" => {
                i += 1;
                let path = rest
                    .get(i)
                    .ok_or_else(|| format!("sandbox run: {arg} needs a path\n{USAGE}"))?;
                if arg == "--deny-read" {
                    request.deny_read.push(PathBuf::from(path));
                } else {
                    request.write.push(PathBuf::from(path));
                }
            }
            other => return Err(format!("sandbox run: unknown option `{other}`\n{USAGE}")),
        }
        i += 1;
    }
    if request.command.is_empty() {
        return Err(format!("sandbox run: no command after `--`\n{USAGE}"));
    }
    Ok(request)
}

/// The policy `request` asks for, as `insbx`'s own flags make it (incord-code crates/sandbox/src/main.rs
/// `apply_flag`): a writable path confines writes to the writable paths, a hidden path alone makes the run read-only;
/// the network, the environment and `.git` hook protection keep the library's defaults (open, inherited, on).
pub(crate) fn policy(request: &Request) -> Policy {
    let mut policy = Policy::default();
    if !request.deny_read.is_empty() {
        policy.fs.mode = FsMode::ReadOnly;
    }
    if !request.write.is_empty() {
        policy.fs.mode = FsMode::WorkspaceWrite;
    }
    policy.fs.write.clone_from(&request.write);
    policy.fs.deny_read.clone_from(&request.deny_read);
    policy
}

/// Runs `insi sandbox …` and ends with the command's own exit code, or 125 when the sandbox refused or failed.
pub(crate) fn main(args: &[String]) -> ExitCode {
    let request = match parse(args) {
        Ok(request) => request,
        Err(why) => {
            eprintln!("insi: {why}");
            return exit(EXIT_SANDBOX);
        }
    };
    // Strict unless asked: a policy this machine cannot enforce in full is refused, not run with less.
    let sandbox = Sandbox::new(policy(&request)).strict(!request.best_effort);
    if request.best_effort && !request.quiet {
        for gap in sandbox.enforcement().gaps() {
            eprintln!("insi sandbox: not enforced here: {gap}");
        }
    }
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(why) => {
            eprintln!("insi sandbox: no current directory: {why}");
            return exit(EXIT_SANDBOX);
        }
    };
    let (program, rest) = request
        .command
        .split_first()
        .expect("parse refuses an empty command");
    let rest: Vec<OsString> = rest.iter().map(OsString::from).collect();
    match sandbox.run(program, &rest, &cwd) {
        Ok(outcome) => exit(outcome.code()),
        Err(why) => {
            eprintln!("insi sandbox: {why}");
            exit(EXIT_SANDBOX)
        }
    }
}

fn exit(code: i32) -> ExitCode {
    ExitCode::from(u8::try_from(code & 0xff).unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn the_presets_flags_parse_as_insbx_takes_them() {
        let request = parse(&args(&[
            "run",
            "-q",
            "-w",
            "/w/tree",
            "-w",
            "~/.claude",
            "--deny-read",
            "/src",
            "--",
            "claude",
            "-p",
            "fix it",
        ]))
        .expect("parses");
        assert!(request.quiet && !request.best_effort);
        assert_eq!(
            request.write,
            [PathBuf::from("/w/tree"), PathBuf::from("~/.claude")]
        );
        assert_eq!(request.deny_read, [PathBuf::from("/src")]);
        assert_eq!(request.command, args(&["claude", "-p", "fix it"]));
    }

    #[test]
    fn everything_after_the_separator_is_the_command_even_flags() {
        let request =
            parse(&args(&["run", "-w", ".", "--", "git", "-w", "--deny-read"])).expect("parses");
        assert_eq!(request.command, args(&["git", "-w", "--deny-read"]));
        assert_eq!(request.write, [PathBuf::from(".")]);
    }

    #[test]
    fn a_wrong_request_is_refused_with_the_usage() {
        for bad in [
            &["run", "-w", ".", "claude"][..],
            &["run", "-w"][..],
            &["run", "--net", "deny", "--", "x"][..],
            &["exec", "--", "x"][..],
            &[][..],
        ] {
            let why = parse(&args(bad)).expect_err("refused");
            assert!(why.contains("usage: insi sandbox run"), "{why}");
        }
    }

    #[test]
    fn the_policy_is_insbxs_for_the_same_flags() {
        let write = policy(
            &parse(&args(&["run", "-w", "/t", "--deny-read", "/s", "--", "x"])).expect("parses"),
        );
        assert_eq!(write.fs.mode, FsMode::WorkspaceWrite);
        assert_eq!(write.fs.write, [PathBuf::from("/t")]);
        assert_eq!(write.fs.deny_read, [PathBuf::from("/s")]);
        assert!(write.fs.protect_git);
        let hidden =
            policy(&parse(&args(&["run", "--deny-read", "/s", "--", "x"])).expect("parses"));
        assert_eq!(hidden.fs.mode, FsMode::ReadOnly);
        let open = policy(&parse(&args(&["run", "--", "x"])).expect("parses"));
        assert_eq!(open.fs.mode, FsMode::Off);
    }
}
