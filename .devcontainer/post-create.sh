#!/usr/bin/env bash
# Prepares a dev container or Codespace to run the checks in CONTRIBUTING.md:
# the three CI commands, the Rust 1.89 MSRV check and the Python/Node
# packaging tests. Callers: devcontainer.json (postCreateCommand).
set -euo pipefail

# The tests commit to real git repositories, so git needs an identity. Keep
# one that is already set (Codespaces sets the GitHub user's).
git config --global user.name >/dev/null || git config --global user.name "Insi Dev"
git config --global user.email >/dev/null || git config --global user.email "dev@example.invalid"

rustup component add rustfmt clippy
# CONTRIBUTING.md: `cargo +1.89.0 check --workspace --all-targets --locked`.
rustup toolchain install 1.89.0 --profile minimal

cargo fetch --locked

cat <<'MSG'

Insi dev container ready. Before opening a pull request:
  cargo fmt --check
  cargo clippy --workspace --all-targets --locked -- -D warnings
  cargo test --workspace --locked
MSG
