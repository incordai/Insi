# Review product research and implementation priorities

Checked against primary product/API sources on 2026-11-07. Vendor descriptions
are feature claims, not evidence that one reviewer is more accurate than another.

## Research applied in this implementation

| Observation | Insi implementation |
|---|---|
| Review needs a useful PR surface, not just local output | Exact-head summary and inline review comments; separate publication command and opt-in workflow |
| Repeated bot noise reduces usefulness | Stable finding identity, author-scoped prior-comment checks, result-sensitive review markers and per-PR workflow concurrency |
| Passing policy checks is not equivalent to testing behavior | Distinct tool-reported, reproduced-regression, verified-fix, incomplete and policy outcomes |
| Context must include callers and affected tests | Reuse Incord's blast-radius API; frozen graph inputs with conservative full-test fallback |
| Trust differs between PR code and reviewer code | Build on trusted base; sandbox PR tools; isolate the publication job's write token |
| Quality must be evaluated rather than asserted | Explicit feedback to existing memory, held-out corpus metrics, overlap/duplicate rejection and no automatic superiority claim |

CodeRabbit documents pre-merge checks as policy evaluation and explicitly
distinguishes them from running a build/test suite. That supports keeping Insi's
executable evidence separate from an AI explanation or policy verdict.
[CodeRabbit's maintained pre-merge examples](https://github.com/coderabbitai/awesome-coderabbit/blob/main/configs/pre-mergechecks/README.md).

Qodo documents review across defined repository relationships, including APIs,
shared packages and schemas. Its documented feature is marked beta. The useful
next addition for Insi is executing consumer contract checks using Incord's
existing relationships, rather than creating a separate context index.
[Qodo cross-repository review](https://docs.qodo.ai/governance/cross-repo-code-review).

GitHub supports reviews associated with a commit and grouped inline comments.
It also documents publication notifications/rate limits. Insi therefore batches
findings, verifies revisions, scopes duplicate checks to its own author and uses
COMMENT instead of asserting approval.
[GitHub review API](https://docs.github.com/en/rest/pulls/reviews?apiVersion=2022-11-28).

## Further additions to evaluate

These remain research directions, not capabilities claimed as implemented:

1. **Consumer contract execution across repositories.** Pin provider/consumer
   commits, run declared OpenAPI/schema/SDK compatibility checks in independent
   sandboxes, and link failures to the consumer. Reuse Incord relationships.
2. **Mutation testing for generated reproductions.** Check whether the new test
   detects relevant broken behavior and still passes equivalent implementations.
   This helps detect a test that merely recognizes one implementation.
3. **Observed security exploitability.** Combine existing local analyzers with
   tightly scoped reproducible data-flow/exploit tests. Do not label every pattern
   match exploitable or replace human security review with a model explanation.
4. **Crash-safe execution resume.** The candidate claim already prevents duplicate
   edits. Add stored service receipts and deterministic recovery after a lost
   response before offering automatic execution retry.
5. **Freshness-aware thread lifecycle.** Resolve the reviewer's own PR threads only
   when evidence at the new head supports the fix, and reflag regressions after a
   previously fixed issue returns. Empty scans alone are insufficient.
6. **Blind, independently labeled quality studies.** Compare actionable defect
   precision/recall, latency and false-positive rates on matched unseen changes,
   using existing experiment statistics and explicit disagreement adjudication.

Do not duplicate the memory graph, owner/proxy rules, coder settings, scheduler
or audit finding store. Prioritize measured reliability and a low-noise workflow
before a large language/tool checklist. No paid API dependency was added.
