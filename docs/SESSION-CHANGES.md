# Review and memory integration changes

This change adds:

- Isolated PR/local reviews, summaries, inline comments and duplicate suppression.
- Paired reproduction tests and normalized JSON/SARIF/ESLint findings.
- Graph-assisted test selection with full-check fallback, plus Incord impact lookup.
- Feedback to existing Incord memory, offline quality metrics and setup verification.
- An opt-in GitHub workflow with separate collection and publication jobs.
- Candidate edit delegation through Incord's existing owner/proxy and audit coder.

The project watcher also runs the candidate-approval denial regression through
`scripts/checks/audit_bridge.py`; existing watcher registrations are preserved.

Usage: [review guide](REVIEW.md). Service setup: [candidate API](INCORD-CANDIDATE-API.md).
Further research: [review priorities](REVIEW-RESEARCH.md).

## Memory-service changes

The sibling service folder is not a separate Git checkout; its files are untracked
under the home directory's repository. To avoid committing unrelated service code,
the Insi commit preserves only this session's service delta in
[candidate-api.patch](../integrations/incord-memory/candidate-api.patch).
It covers `src/api/insi_api.rs`, `src/api/mod.rs`, `src/router.rs` and `src/audit.rs`.
The patch was verified in a fresh scratch tree against the pre-change form
reconstructed from this session's exact edits; applying it reproduced the current
service files byte for byte, and its reverse check passed. Review target-version
compatibility before transferring it to another service checkout.

These service edits are already applied locally; do not apply the patch twice.
The active engine has not been restarted. PR automation remains disabled until
configured. No live AI worker or GitHub review was invoked, and no push was made.

## Checks

Seven local project check groups and 74 binary tests passed. Review-specific tests,
targeted engine approval/scope/HTTP/gate tests, workflow lint and documentation links
passed. CLI trials checked real Git review/retry behavior and actual Insi signature
tests with live memory impact lookup. These do not establish model superiority or
production readiness. The Codecov page was inaccessible; no Codecov result is claimed.
