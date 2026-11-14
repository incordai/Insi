#!/usr/bin/env bash
# Install the security tool list in a dev container or on macOS; never run a scan.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
if [[ "$(uname -s)" == Darwin ]]; then
  brew bundle --file "$root/scripts/security/Brewfile"
elif [[ "$(uname -s)" == Linux ]]; then
  # Development containers provide Python, Go, Rust and Docker through their features.
  sudo apt-get update
  # PHP runtime and native-memory tools accompany the Go/Node features in the dev container.
  sudo apt-get install -y nmap sqlmap python3-venv php-cli gcc g++ valgrind
  python3 -m venv "$root/.incord/temp/security-venv"
  "$root/.incord/temp/security-venv/bin/python" -m pip install -r "$root/scripts/security/requirements.txt"
  go install github.com/aquasecurity/trivy/cmd/trivy@v0.76.2
  # The upstream repository was renamed; its Go module still uses the historical path.
  go install github.com/zricethezav/gitleaks/v8@v8.30.1
  cargo install cargo-audit --version 0.23.0 --locked
  printf '\nSemgrep: %s/.incord/temp/security-venv/bin/semgrep\nAdd that directory and go env GOPATH/bin to PATH.\n' "$root"
else
  printf 'Use the Linux dev container for this platform.\n' >&2
  exit 1
fi
# Exact installed tool versions belong in each benchmark's evidence receipt.
printf '\nSecurity tools installed. Start your local Docker daemon before lab tests.\n'
