## Index

- rust: language rules
- docs: documentation rules

## rust

- Workspace clippy lints warn on `unwrap`, `expect` and `panic`
  (`Cargo.toml` `[workspace.lints.clippy]`); handle errors with `Result<_, String>`
  as the existing modules do.
- Prefer `serde_json::json!` values and explicit argv lists over shell strings.
- Doc comments name callers and callees of non-trivial functions.
- Keep the dependency set small; new dependencies go through the dependency
  policy check.

## docs

Each project document opens with `## Index`, then one `## name` section per
index line.
