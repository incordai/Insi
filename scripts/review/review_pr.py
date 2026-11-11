#!/usr/bin/env python3
"""Review open pull requests with Insi and the local `claude -p` login.

  review_pr.py 47            review PR 47, print the findings, post nothing
  review_pr.py 47 --post     also post them as one COMMENT-only GitHub review
  review_pr.py --all --post  do that for every open, non-draft PR

It never approves or merges: `insi review-publish` only ever posts COMMENT
reviews and skips a head it already reviewed. The PR code is never executed:
the plan has no checks and the only tool is scripts/review/claude_p_review.py,
which reads the diff. It runs on this machine with its own `gh` and `claude`
logins; no token or key is stored in the repo.
"""
import json
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
ADAPTER = os.path.join(HERE, "claude_p_review.py")
INSI = os.environ.get("INSI_BIN", os.path.join(REPO, "target", "debug", "insi"))


# Run a command and return its stdout; a failure stops the review with the
# command's own error text rather than posting a half result. Callers: all
# functions below.
def run(cmd, cwd=REPO):
    done = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
    if done.returncode != 0:
        raise SystemExit(f"{' '.join(cmd[:3])} failed: {done.stderr.strip() or done.stdout.strip()}")
    return done.stdout.strip()


# Build a throwaway checkout of the PR head with the PR base kept as a branch,
# because `insi review` clones it and can only see branches. Nothing in the
# owner's own checkout changes. Callers: review.
def checkout(number, base, head, work):
    src = os.path.join(work, "src")
    run(["git", "clone", "--quiet", "--no-local", REPO, src])
    run(["git", "remote", "set-url", "origin", run(["git", "remote", "get-url", "origin"])], src)
    run(["git", "fetch", "--quiet", "origin", f"refs/pull/{number}/head"], src)
    if run(["git", "rev-parse", "FETCH_HEAD"], src) != head:
        raise SystemExit("PR changed during fetch; run again")
    run(["git", "fetch", "--quiet", "origin", base], src)
    run(["git", "update-ref", "refs/heads/insi-review-base", base], src)
    run(["git", "checkout", "--quiet", "-B", "insi-review-head", head], src)
    return src


# True when the logged-in `gh` user already posted an Insi review for this
# exact head. The marker `<!-- insi-review:<head>:` is the one
# `insi review-publish` writes (review.rs, review_marker). Model findings vary
# between runs, so without this check a scheduled run would post a new review
# on the same head every time. Callers: review.
def reviewed(number, head):
    login = run(["gh", "api", "user", "--jq", ".login"])
    pages = json.loads(run(["gh", "api", f"repos/{{owner}}/{{repo}}/pulls/{number}/reviews", "--paginate", "--slurp"]))
    marker = f"<!-- insi-review:{head}:"
    return any(
        r.get("user", {}).get("login") == login and marker in (r.get("body") or "")
        for page in pages
        for r in page
    )


# Review one PR: build the checkout, run `insi review` with the model plan,
# print a summary, and with post=True publish the report as a COMMENT review.
# A posting run skips a head this login already reviewed unless force=True.
# Callers: main.
def review(number, post, force=False):
    info = json.loads(run(["gh", "pr", "view", str(number), "--json", "baseRefOid,headRefOid,title"]))
    base, head = info["baseRefOid"], info["headRefOid"]
    if post and not force and reviewed(number, head):
        print(f"PR #{number}: head {head[:7]} already reviewed; skipped")
        return
    work = os.path.join(REPO, ".incord", "temp", f"review-pr-{number}-{int(time.time())}")
    os.makedirs(work)
    src = checkout(number, base, head, work)
    plan = {
        "version": 1,
        "tools": [
            {
                "name": "claude-p-review",
                "command": ["python3", ADAPTER, base],
                "format": "json",
                "accepted_exits": [0],
                "timeout_secs": 1800,
            }
        ],
        # Insi requires a fallback check; this one only reads and never runs PR code.
        "full_checks": [["git", "rev-parse", "HEAD"]],
        "tests": [],
        "graph": [],
        "reproductions": [],
    }
    plan_path = os.path.join(work, "plan.json")
    with open(plan_path, "w") as f:
        json.dump(plan, f)
    out = os.path.join(work, "out")
    run([INSI, "review", "--repo", src, "--base", "insi-review-base", "--plan", plan_path, "--output", out])
    with open(os.path.join(out, "report.json")) as f:
        report = json.load(f)
    findings = report.get("findings", [])
    print(f"PR #{number} {info['title']!r}: {len(findings)} finding(s); report {out}/report.json")
    for item in findings:
        print(f"  {item.get('file')}:{item.get('line')} [{item.get('severity')}] {item.get('message')}")
    if post:
        print(run([INSI, "review-publish", "--repo", REPO, "--pr", str(number), "--report", os.path.join(out, "report.json")]))


# Read the command line: a PR number or --all, --post to publish and --force
# to review a head again. One failing PR does not stop the others; the exit
# code is 1 if any failed. Callers: the script entry point below.
def main():
    args = sys.argv[1:]
    post = "--post" in args
    force = "--force" in args
    args = [a for a in args if a not in ("--post", "--force")]
    if args == ["--all"]:
        listed = json.loads(run(["gh", "pr", "list", "--state", "open", "--json", "number,isDraft"]))
        numbers = [p["number"] for p in listed if not p["isDraft"]]
    elif len(args) == 1 and args[0].isdigit():
        numbers = [int(args[0])]
    else:
        raise SystemExit(__doc__)
    failed = 0
    for number in numbers:
        try:
            review(number, post, force)
        except SystemExit as err:
            print(f"PR #{number}: {err}", file=sys.stderr)
            failed = 1
    return failed


if __name__ == "__main__":
    sys.exit(main())
