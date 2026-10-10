# Security

Please **do not** open a public issue for a vulnerability.

Report it privately with a GitHub Security Advisory on this repository
(**Security → Report a vulnerability**). Include the version you run
(`insi --version`), your platform, the command, and enough detail to reproduce
it. You will get an answer, and credit in the fix if you want it.

## What counts

Insi runs a coding agent on tasks, on issues anyone can write, and opens pull
requests from what it produces. The most serious reports are the ones where
one of its boundaries breaks:

- **The hidden check reaches the worker.** A task's hidden tests, or its
  reference commit, readable by the agent before or during its attempt. This
  breaks every measurement Insi makes.
- **Untrusted text acts.** An unlabelled issue worked by `insi issue`, or text
  from an issue, a connector or a CI log that makes Insi or its worker do
  something it should only have read.
- **A secret leaves the machine.** A credential or personal data in a branch
  that `insi issue` or `insi improve --pr` pushed despite the scan.
- **The sandbox does not hold.** Setup, the worker or the check reaching past
  the `[sandbox]` command that should contain them.
- **The MCP server or the hook writes.** `insi mcp` and `insi hook` only read;
  any input that makes either write a lesson or run a command is a bug.

## What to expect from Insi itself

Insi runs the worker command you configure, on your machine, with your
permissions, unless you name a `[sandbox]`. It reads, holds and sends no
GitHub token: `gh` holds the login. See the README for each command's limits.

## Supported versions

Fixes are made on `main` and in the next release.
