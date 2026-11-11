## Index

- policy: where reporting rules live
- publishing: what may be posted publicly
- workflows: token and checkout rules

## policy

Vulnerability reporting is described in the repository root `SECURITY.md`.

## publishing

Text posted to a pull request (review comments, reports) is scanned with
`insi_core::secrets::scan_diff`, which inspects only lines that start with `+`.
Multi-line text must be scanned line by line (`review::leaks_secret`).

## workflows

- Pin third-party actions to a commit sha.
- Checkout trusted base code for jobs that hold a token; never run a PR's own
  workflow or configuration there.
- Give write permissions only to the job that needs them.
