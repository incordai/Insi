"""Run operator-authored HTTP security assertions against an explicit local app.

Headers may reference environment variables so credentials never enter reports.
Only normalized assertion outcomes are written; response bodies remain private.
"""
import argparse
import json
import os
from pathlib import Path
from urllib.parse import urlsplit
import urllib.request
import urllib.error


# Refuse redirects so a tested origin cannot silently move a credentialed request elsewhere.
class NoRedirect(urllib.request.HTTPRedirectHandler):
    # Preserve the original response as an HTTP error for explicit status assertions.
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


# Local targets include loopback listeners and the existing isolated Docker DNS alias.
def validate(plan):
    base = urlsplit(plan.get("base", ""))
    if base.scheme not in ("http", "https") or base.hostname not in ("127.0.0.1", "localhost", "::1", "target") or base.username or base.password or base.query or base.fragment or base.path not in ("", "/"):
        raise ValueError("contract base must be an explicit local test origin")
    cases = plan.get("cases")
    if plan.get("version") != 1 or not isinstance(cases, list) or not 1 <= len(cases) <= 100:
        raise ValueError("contract needs version 1 and 1..100 cases")
    seen = set()
    for case in cases:
        name, path = case.get("id"), case.get("path", "")
        if not isinstance(name, str) or not name or name in seen:
            raise ValueError("unique case IDs required")
        seen.add(name)
        if not path.startswith("/") or path.startswith("//") or "\\" in path or any(ord(c) < 32 for c in path):
            raise ValueError("contract paths must stay on the tested origin")
        if case.get("method", "GET") not in ("GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"):
            raise ValueError("unsupported method")
        if type(case.get("status")) is not int or not 100 <= case["status"] <= 599:
            raise ValueError("expected status required")


# Failed requests and violated assertions are explicit incomplete or failing cases, never clean scans.
def run(plan):
    validate(plan)
    opener = urllib.request.build_opener(NoRedirect)
    rows = []
    for case in plan["cases"]:
        row = {"id": case["id"], "passed": False}
        try:
            headers = dict(case.get("headers", {}))
            for name, variable in case.get("headers_from_env", {}).items():
                headers[name] = os.environ[variable]
            body = case.get("body")
            request = urllib.request.Request(plan["base"].rstrip("/") + case["path"], data=body.encode() if body is not None else None, headers=headers, method=case.get("method", "GET"))
            try:
                response = opener.open(request, timeout=10)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                raw = response.read(1024 * 1024 + 1)
                if len(raw) > 1024 * 1024:
                    raise ValueError("response exceeds limit")
                text = raw.decode("utf-8", errors="replace")
                row["status"] = response.status
                row["passed"] = (response.status == case["status"] and all(value in text for value in case.get("contains", [])) and all(value not in text for value in case.get("forbids", [])) and all(response.headers.get(name) == value for name, value in case.get("response_headers", {}).items()))
        except (OSError, ValueError, KeyError, TypeError):
            row["incomplete"] = True
        rows.append(row)
    return {"version": 1, "complete": not any(row.get("incomplete") for row in rows), "passed": all(row["passed"] for row in rows), "cases": rows, "scope": "declared HTTP assertions only; not an exhaustive application audit"}


# Credentials and request/response content are never copied into the exported receipt.
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("plan", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    raw = args.plan.read_bytes()
    if len(raw) > 1024 * 1024:
        raise ValueError("HTTP contract exceeds 1 MiB")
    report = run(json.loads(raw))
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    return 0 if report["passed"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
