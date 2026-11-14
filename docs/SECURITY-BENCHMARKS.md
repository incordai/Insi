# Security review and disposable lab benchmarks

## Language and weakness coverage

Coverage below describes checked-in seeded controls, not every framework in a
language. Scanner language support alone is not evidence of a working security
rule. Every static rule is tested against a positive and a safer negative sample.

| Language | Static controls | Runtime controls |
| --- | --- | --- |
| Rust | Forgotten ownership and shell interpretation | Instrumented global allocator: forgotten `Box` versus `Drop`; workspace RustSec audit |
| PHP | SQL concatenation, shell execution | Retained string allocations versus released allocations |
| Go | SQL concatenation | Blocked goroutines versus cancellation and joined cleanup |
| JavaScript | SQL construction, HTML sink | Event listeners retained versus removed |
| TypeScript | SQL construction, HTML sink | Parser controls only; no TypeScript application runtime claim |
| Java | SQL construction | Parser controls only; no JVM heap-leak detector added |
| Python | SQL construction, pickle deserialization, shell execution | SQL lab, tracemalloc retention, ownership/IDOR, traversal and symlink boundaries, HTML serialization |
| C | Native controls compiled inside Linux | LeakSanitizer leak, AddressSanitizer use-after-free and heap-buffer-overflow, each with a clean fixed build |

Additional classes covered by existing tool adapters include credentials,
dependency advisories, container configuration and exposed-service observations.
The IDOR/path/HTML tests exercise small boundary implementations. The HTML test
checks parsing/escaping, not JavaScript execution in a browser. Shell and pickle
rules flag review obligations, not proof that attacker-controlled input reaches
the sink. `mem::forget` may be intentional and is reported as a warning.

Not yet established: arbitrary C++/C#/Ruby application coverage, JVM heap profiling,
Rust unsafe-code/Miri coverage, cross-browser/framework behavior, production
session/CSRF integration, arbitrary file-type decoding, an actual DNS-rebinding
transport, deployment-wide TLS policy, cryptographic protocols and comprehensive
application-specific authorization/business-logic coverage.
These require application-specific targets and expected behavior. No finite suite
can prove that arbitrary software or a dataset has no exploitable behavior.

## Application boundary and HTTP contracts

### End-to-end authentication/API, SSRF and upload fixtures

Run `python3 -m unittest discover -s scripts/security -p 'test_api_fixture.py'`.
The four scenario methods run the same real HTTP requests against deliberately
vulnerable and fixed modes of `api_fixture.py`. All services bind only to
loopback and use test-owned users, storage and upstreams; no external deployment
or production credential is contacted.

| Scenario | Vulnerable effect | Fixed control |
| --- | --- | --- |
| Authentication and sessions | Wrong password issues a usable Alice session; replay works after logout | Salted verification before issuance; logout revokes; expiry denies old credentials |
| API authorization and CSRF | Alice reads/updates Bob's record; forged mutation requests change data | Ownership checks on read/write, session-bound proof, exact origin, legitimate own updates still work |
| SSRF over real HTTP | Direct and redirected requests reach a separate private canary service and return its body | Exact allowed upstream, numeric connection pinning, no proxy use, no redirect following; private hit count remains zero |
| Upload/download boundaries | Traversal writes outside the upload folder; PHP/type/size cases are accepted; Bob reads Alice's file | Random stored names, text/type/size checks, confinement and owner checks; legitimate upload/download still work |

The SSRF fixture now uses an actual HTTP connector, not the earlier mocked
transport. Public/private roles are represented by separate test-owned loopback
origins. It demonstrates direct and redirect SSRF prevention with a strict
origin allowlist; it does not emulate Internet DNS rebinding or a general-purpose
HTTPS proxy. The vulnerable upload escape is itself confined to the test's parent
scratch directory. No uploaded PHP is executed. The target uses HTTP for local
cookie/request tests, while real certificate validation has separate TLS controls;
it does not establish production Secure-cookie or reverse-proxy behavior.

Upload probes send actual multipart file bodies with per-file names and media
types. Fixed mode requires one non-nested file part and applies the same bounded
text storage policy; these tests do not claim arbitrary multipart/parser-fuzz or
archive/image decoder coverage.

The ordinary security unittest discovery includes these scenarios, so existing
local and CI commands exercise them without extra scanners, Docker or paid APIs.
Application-specific SSO/MFA, permission models, storage backends and arbitrary
file types still require their own contracts and deployments.

`boundary_controls.py` and `test_boundaries.py` add narrow reference policies and
adversarial controls: salted scrypt verification before session issuance; session
rotation, expiry and logout; session-bound CSRF and same-origin checking; host-only
Secure/HttpOnly/SameSite cookies; object/method authorization; all-address SSRF
filtering with a pinned numeric destination and refused redirects; randomized
text-only uploads; finite positive money amounts, idempotent replay and concurrent
balance updates. Password work factors are lab settings, not production guidance.
Roles passed to the authorization helper must come from a trusted server identity.

The earlier `fetch_public` helper uses an injected resolver/transport: it tests
decisions and pinning, not an actual HTTPS connector or DNS-rebinding attack.
The end-to-end SSRF fixture above separately tests a real HTTP connector.
The upload helper writes text
outside the served directory; it does not validate arbitrary image/archive formats.
The ledger is an in-memory control, not evidence of a database transaction's isolation.

`test_tls.py` performs real loopback TLS requests: a trusted issuer/hostname passes,
unknown issuers and hostname mismatches fail. Its intentionally unverified client
demonstrates the unsafe contrast. Temporary certificates and keys stay in ignored
scratch storage and are removed after the test.

`browser_lab.py` serves loopback-only vulnerable/fixed reflected-HTML pages. The
visible Watchman browser triggered one local canary request on the unsafe page
and zero on the escaped/CSP-protected page. The fixed page visibly rendered the
payload as text. Browser execution is manually verified here, not automated by
the ordinary unittest run. Its CSP also blocked the inspector's eval operation;
the server-side canary count avoided weakening the policy for instrumentation.

For application-specific checks, author a local HTTP contract and run:

```sh
python3 scripts/security/http_contract.py contract.json \
  --output .incord/temp/http-security-report.json
```

```json
{
  "version": 1,
  "base": "http://127.0.0.1:8080",
  "cases": [
    {"id":"anonymous-private-route", "path":"/private", "status":401},
    {"id":"other-user-object", "path":"/records/bob", "status":403,
     "headers_from_env":{"Authorization":"ALICE_TEST_AUTH"},
     "forbids":["bob-canary"]}
  ]
}
```

Targets are limited to explicit loopback origins or the internal Docker alias
`target`. Redirects are not followed, bodies have a size limit and requests have
deadlines. Reports omit credentials and response content. Missing credentials or
failed transports are incomplete. Assertions can check status, required/forbidden
text and exact response headers. The same contract passed a live fixed two-user
HTTP fixture and failed its deliberately broken ownership check. It does not
discover routes, provision users or invent an application's authorization policy.

## Memory check commands

`python3 -m unittest discover -s scripts/security -p 'test_*.py'` runs Rust,
Python, Node, PHP and Go retention controls when their runtimes are installed;
missing tools are explicit skips. PHP and Go are included in the installer.
Counts of live allocations, listeners and goroutines avoid confusing allocator
high-water RSS with a leak. The seeded leaking workload must cross a threshold
and its fixed workload must stay below it; this does not select a universal
production-service threshold.

Native checks run separately in disposable Linux containers:

```sh
docker build -t insi-memory-check -f scripts/security/memory.Dockerfile scripts/security
docker image inspect --format '{{.Id}}' insi-memory-check
python3 scripts/security/native_memory.py --image sha256:<image-id> \
  --output .incord/temp/security-native-memory.json
```

The runner refuses mutable image tags, uses no network or host mounts, limits
resources and removes timed-out containers. Both a sanitizer diagnostic and its
expected failure exit are required; generic crashes are incomplete results.
Clean fixed runs must exit zero without sanitizer diagnostics. The current
native build recipe installs GCC/libc inside Debian; record the resulting image
digest and compiler versions for comparisons because package repository content
can change between builds. The recorded local Linux run detected all three
positive native faults and accepted their fixed counterparts.

The reference is [vxcontrol/PentAGI](https://github.com/vxcontrol/pentagi).
PentAGI dynamically selects tools; this integration does not claim to reproduce
every penetration-testing capability or to demonstrate equivalent effectiveness.

## Install the tool inventory

Run `bash scripts/security/install.sh`. On macOS the checked-in Brewfile installs
Semgrep, Trivy, Gitleaks, cargo-audit, Nmap, sqlmap, Docker CLI, Colima, Go and PHP.
Run `colima start` to start the local Docker VM. The Linux development container
provides Go and Docker-in-Docker, and its post-create hook installs the scanners.
Linux Semgrep is isolated in `.incord/temp/security-venv`; add its `bin` directory
to PATH along with the Go and Cargo binary directories. No scanner needs an LLM
subscription for these deterministic tests. Installation accesses package
registries; the lab execution network has no external connectivity.

The Linux tool list pins Semgrep 1.185.0, Trivy 0.76.2, Gitleaks 8.30.1 and
cargo-audit 0.23.0. Homebrew and apt resolve their package versions at install
time. Record actual versions and advisory-database snapshots when comparing runs;
the install list alone is not a reproducible vulnerability-database snapshot.

Metasploit is an optional probe in a dedicated lab image. Supply that image by
digest and make its probe exit 0 only after observing the intended lab effect,
1 when absent, and another code for infrastructure errors. It is not installed
as a host service and no exploitation module is enabled by default. Nmap and
sqlmap likewise run only against declared disposable targets in this harness.

## Native review formats

Review tools now accept `semgrep`, `trivy`, `gitleaks` and `cargo-audit` in addition
to `json`, `sarif` and `eslint`. Use native JSON stdout and declare accepted exits
according to the command used. Missing tools, rejected exits, malformed reports
and Semgrep partial-scan errors make review incomplete. A positive scan remains
`tool-reported`; only the existing frozen base/head reproduction can establish
a regression. Dependency findings anchor to line 1 of the dependency manifest,
not a fabricated source-code location. Trivy imports target filesystem reports,
not arbitrary image names or remote endpoints as repository paths.

Examples of operator-owned command arguments:

| Format | Command |
| --- | --- |
| semgrep | `semgrep scan --config /trusted/rules.yml --json .` |
| trivy | `trivy fs --scanners vuln,misconfig,secret --format json .` |
| gitleaks | `gitleaks dir --report-format json --report-path - --redact .` |
| cargo-audit | `cargo audit --json --no-fetch` |

Keep rule files and advisory databases pinned in the trusted scanner image;
do not let PR-controlled rules suppress required checks. Secrets' matched values
are omitted by the Gitleaks and Trivy adapters. Existing public-report redaction
still applies to all other imported text. Raw tool logs remain private artifacts.

`scripts/security/nmap_report.py report.xml --file compose.yaml --line 5`
imports completed Nmap XML as informational open-port observations tied to an
operator-declared service configuration. It does not execute a scan or assert
that an exposed service is vulnerable. Failed XML scans are not clean results.

## First executable lab: SQL injection

Load a trusted Python Alpine base image, then pass its immutable image ID or
manifest digest to the builder. Floating references are refused by the builder:

```sh
python3 scripts/security/build_lab.py --base sha256:<loaded-image-id> \
  --output .incord/temp/security-lab.json
python3 scripts/security/lab.py .incord/temp/security-lab.json \
  --output .incord/temp/security-lab-report.json
```

The builder produces vulnerable and fixed images differing only in query
parameterization. The same probe must expose an in-memory canary on the vulnerable
image and fail to expose it on the fixed image. It also checks an ordinary absent
record as a clean negative. Readiness failure, timeout, unexpected exit and failed
network cleanup are incomplete, not successful fixes. The manifest hash, immutable
image references, elapsed time and control exits are saved without raw payloads.

Each case runs on a fresh internal Docker network with no published ports, host
mounts or Docker socket. Targets/probes run as UID 65534, drop capabilities, use
read-only filesystems, and have memory, CPU, PID and time limits. Cleanup removes
the case's containers and network. Images must already be loaded (`--pull=never`).
These restrictions require compatible lab images, not arbitrary production images.

## Coverage and remaining work

| Capability from the earlier research | Delivery and evidence |
| --- | --- |
| Source, dependency, secret and container/configuration scans | Four native import adapters with parser regression tests; existing SARIF remains supported |
| Network discovery | Nmap XML importer; open ports are observations |
| SQL injection and fix retest | Runnable disposable positive/fixed controls and a clean negative |
| Exploit validation with other tools | Generic digest-pinned lab probe interface; no recorded live sqlmap/Metasploit comparison yet |
| Execution control | Internal networks, immutable images, restricted containers, bounded phases, cleanup and failure tests |
| Evidence reporting and scoring | Existing `review-evidence` and `review-quality` paths; reuse exact file/line/rule labels, false positives, misses and incomplete denominators |
| Memory and held-out improvement | Existing Incord lesson/experiment flow; these sample controls do not constitute an independent held-out study |

`python3 scripts/ci/test_all.py` includes security parser/orchestration regressions
without Docker. Live scans and Docker control runs are separate checks and must
be reported separately. Research/planning behavior, arbitrary exploit modules,
and comparative PentAGI effectiveness are not established by these tests.

## Local verification, 2026-11-14

Real API fixture extension: the full local runner passed 8/8 groups with 32
security test methods and no skips. After switching the upload probes to actual
multipart bodies, all four end-to-end scenario methods passed again. Effects
were checked through issued credentials, stored records, real upstream hit counts
and files on disk. All temporary services and storage are cleaned up by the tests.
Validation was performed locally before publishing this extension.

Boundary extension: 28 security unittest methods passed without skips and the
full local runner passed 8/8 groups. This includes credential/cookie, session/CSRF,
authorization, simulated SSRF, upload and concurrent-transfer controls; real
loopback TLS verification; and a live HTTP contract that detects a planted
ownership regression. Browser verification was separate: one canary hit for the
unsafe page and zero for the fixed page, with the payload visibly rendered as
text. Both temporary browser servers were stopped after inspection. Validation
was performed locally before publishing this extension.

Expanded verification: all 19 security unittest methods passed with no skips,
including native Rust allocator instrumentation, PHP allocation retention, Go
goroutine cleanup and Node listener cleanup. The complete local runner passed
8/8 groups. The separate Linux sanitizer receipt passed leak, use-after-free and
heap-buffer-overflow positives and their fixed controls. Native test containers
were removed. Workflow validation with actionlint passed; the newly added CI job
has not yet run on GitHub because these changes remain local.

All eight local check groups passed. Installed-scanner tests exercised Gitleaks
8.30.1 on a clean directory, Semgrep 1.185.0 on vulnerable/parameterized examples,
and Nmap 7.993 on a test-owned loopback port. cargo-audit 0.23.0 reported zero
workspace vulnerabilities and no warnings after fetching its advisory database.
Trivy 0.76.2 found a missing health check in the new lab Dockerfile; adding it
reduced configuration findings to zero under the embedded checks. The downloaded
check-bundle update failed because the host's Docker credential helper was absent,
so this is not evidence of a fresh downloaded Trivy policy bundle.

The live Docker SQL-injection controls completed: the same probe returned 0 for
the vulnerable image and 1 for the parameterized fixed image. All case containers
and the internal network were removed. The base was
`python@sha256:c5ba3deef00c7758561a0f03c26f44f10589263d40e732b8748e0d1f38f6dd89`.
Machine-readable evidence is in `.incord/temp/security-lab-report.json` (ignored).
Live sqlmap/Metasploit probes and a comparative PentAGI study remain unverified.
