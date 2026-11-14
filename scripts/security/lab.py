"""Run trusted security probes against disposable targets on an internal Docker network.

Images must be preloaded and digest-pinned. A probe returns 0 for a demonstrated
vulnerability and 1 for its absence; any other exit or timeout is incomplete.
This runner never publishes ports, mounts host files or accepts external targets.
"""

import argparse
import hashlib
import json
import re
import subprocess
import time
import uuid
from pathlib import Path


# Validate the complete manifest before creating any Docker resources.
def validate(plan):
    if plan.get("version") != 1 or not isinstance(plan.get("cases"), list) or not plan["cases"]:
        raise ValueError("lab requires version 1 and nonempty cases")
    seen = set()
    for case in plan["cases"]:
        name = case.get("id", "")
        if not re.fullmatch(r"[a-z0-9][a-z0-9-]{0,63}", name) or name in seen:
            raise ValueError("case IDs must be unique lowercase identifiers")
        seen.add(name)
        for key in ("vulnerable_image", "fixed_image", "probe_image"):
            # Accept immutable local image IDs as well as registry manifest digests.
            if not re.fullmatch(r"(?:[a-zA-Z0-9./:_-]+@)?sha256:[a-f0-9]{64}", case.get(key, "")):
                raise ValueError("all images must be pinned by sha256 digest")
        for key in ("probe", "ready"):
            command = case.get(key)
            if not isinstance(command, list) or not command or any(not isinstance(arg, str) for arg in command):
                raise ValueError("probe and readiness commands must be argv arrays")
            if any("{target}" in arg for arg in command):
                raise ValueError("use the fixed Docker DNS name target, not target interpolation")
        if type(case.get("seconds")) is not int or not 1 <= case["seconds"] <= 300:
            raise ValueError("each phase needs a 1..300 second budget")
    if len(seen) > 32:
        raise ValueError("lab case limit is 32")


# Docker commands have finite deadlines; output is not copied into public reports.
def docker(args, seconds=30):
    return subprocess.run(["docker", *args], stdout=subprocess.DEVNULL,
                          stderr=subprocess.DEVNULL, timeout=seconds, check=False).returncode


# Both targets and probes share the same restricted container policy.
def container(name, network, image, command=(), detached=False):
    return ["run", "--pull=never", "--name", name, "--network", network,
            "--cap-drop=ALL", "--security-opt=no-new-privileges", "--read-only",
            "--tmpfs", "/tmp:rw,noexec,nosuid,size=64m", "--pids-limit=128",
            "--memory=256m", "--cpus=1", "--user=65534:65534",
            *(["--detach", "--network-alias", "target"] if detached else []),
            image, *command]


# A case is valid only when the identical probe detects the vulnerable image
# and rejects the fixed image after both targets pass readiness checks.
def run_case(case, execute=docker):
    token = "insi-lab-" + uuid.uuid4().hex
    network = token + "-net"
    result = {"id": case["id"], "complete": False, "controls": [],
              "images": {key: case[key] for key in ("vulnerable_image", "fixed_image", "probe_image")}}
    started = time.monotonic()
    try:
        if execute(["network", "create", "--internal", network]) != 0:
            return result
        for state, expected in (("vulnerable", 0), ("fixed", 1)):
            target = token + "-target-" + state
            probe = token + "-probe-" + state
            ready = token + "-ready-" + state
            try:
                if execute(container(target, network, case[state + "_image"], detached=True)) != 0:
                    return result
                if execute(container(ready, network, case["probe_image"], case["ready"]), case["seconds"]) != 0:
                    return result
                code = execute(container(probe, network, case["probe_image"], case["probe"]), case["seconds"])
                result["controls"].append({"state": state, "exit": code, "expected": expected})
                if code != expected:
                    return result
            finally:
                for name in (probe, ready, target):
                    execute(["rm", "--force", name])
        result["complete"] = True
        return result
    except (OSError, subprocess.TimeoutExpired):
        result["error"] = "infrastructure failure or deadline exceeded"
        return result
    finally:
        try:
            # Failed cleanup cannot be called a complete isolated lab run.
            if execute(["network", "rm", network]) != 0:
                result["complete"] = False
                result["error"] = "network cleanup failed"
        except (OSError, subprocess.TimeoutExpired):
            result["complete"] = False
            result["error"] = "cleanup failed"
        result["seconds"] = time.monotonic() - started


# Save only controlled metadata and normalized control outcomes, never scanner stdout.
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("plan", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    raw = args.plan.read_bytes()
    if len(raw) > 1024 * 1024:
        raise ValueError("lab manifest exceeds 1 MiB")
    plan = json.loads(raw)
    validate(plan)
    cases = [run_case(case) for case in plan["cases"]]
    report = {"version": 1, "manifest_sha256": hashlib.sha256(raw).hexdigest(),
              "complete": all(case["complete"] for case in cases), "cases": cases,
              "scope": "local disposable lab controls; no generalization claim"}
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    return 0 if report["complete"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
