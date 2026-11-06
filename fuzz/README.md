# Parser fuzzing

Install `cargo-fuzz`, then run these bounded smoke checks from the repository root:

```sh
cargo +nightly-2026-11-05 fuzz run task -- -runs=1000 -max_len=4096
cargo +nightly-2026-11-05 fuzz run connector -- -runs=1000 -max_len=4096
```

The targets parse task/config TOML and connector JSON without executing
commands. A panic or sanitizer finding fails the run. Keep generated artifacts
and corpora local; promote a minimized failing input to a regression test before
fixing its parser. The ordinary workspace tests also run bounded proptest
round-trips, malformed inputs and path/topic boundary checks on stable Rust.

The separate fuzz workspace uses `libfuzzer-sys` (Apache-2.0/NCSA) only for
development. It adds no runtime dependency to Insi. The root workspace's
`proptest` dependency is test-only and disables subprocess/fork features.
