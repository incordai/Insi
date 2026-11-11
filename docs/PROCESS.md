## Index

- changes: how a change reaches main
- dependencies: automated updates

## changes

Work happens on a branch and lands through a pull request. `main` is protected:
a merge needs a passing set of checks and a review (observed: `REVIEW_REQUIRED`
blocks `gh pr merge`). Do not bypass with admin merge without the owner saying so.
Pull requests start as drafts for large work.

## dependencies

Dependabot opens version-bump PRs (for example #46). For GitHub Actions bumps,
check each workflow for options the new major version changed before merging.
