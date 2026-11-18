# Copyright (c) 2026-2027 Incord Ai.
# https://incord.ai
#
# SPDX-License-Identifier: BUSL-1.1

"""Require sanitizers to detect leaking/corrupt builds and accept equivalent fixed builds."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import uuid


# Only immutable, preloaded images run; memory tools never get host mounts or a network.
def assess(image):
    if not re.fullmatch(r"(?:[a-zA-Z0-9./:_-]+@)?sha256:[a-f0-9]{64}", image):
        raise ValueError("memory image must be immutable")
    rows = []
    for mode, marker in [("leak", "LeakSanitizer: detected memory leaks"), ("uaf", "heap-use-after-free"), ("overflow", "heap-buffer-overflow")]:
        results = []
        for fixed in (False, True):
            # Unique resource names protect concurrent lab runs from each other's cleanup.
            name = "insi-memory-" + uuid.uuid4().hex
            argv = ["docker", "run", "--pull=never", "--name", name, "--network=none", "--cap-drop=ALL", "--security-opt=no-new-privileges", "--read-only", "--tmpfs", "/tmp:rw,noexec,nosuid,size=32m", "--memory=256m", "--pids-limit=64", image, "/probe-fixed" if fixed else "/probe-leak", mode]
            try:
                ran = subprocess.run(argv, capture_output=True, timeout=30)
                # A fixed control must also be free of diagnostics even if a sanitizer exits zero.
                stderr = ran.stderr.decode(errors="replace")
                matched = marker in stderr
                diagnostic = any(text in stderr for text in ("AddressSanitizer", "LeakSanitizer", "runtime error:"))
                passed = ran.returncode == 0 and not diagnostic if fixed else ran.returncode == 23 and matched
                results.append({"fixed": fixed, "exit": ran.returncode, "expected_marker": matched, "passed": passed})
            except (OSError, subprocess.TimeoutExpired):
                results.append({"fixed": fixed, "passed": False, "incomplete": True})
            finally:
                # Explicit removal also kills a timed-out container instead of leaving its workload running.
                # Cleanup errors are incomplete outcomes, not successful sanitizer controls.
                try:
                    cleanup = subprocess.run(["docker", "rm", "--force", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15, check=False)
                    if cleanup.returncode != 0:
                        results[-1]["passed"] = False
                        results[-1]["cleanup_failed"] = True
                except (OSError, subprocess.TimeoutExpired):
                    results[-1]["passed"] = False
                    results[-1]["cleanup_failed"] = True
        rows.append({"case": mode, "controls": results, "complete": all(row["passed"] for row in results)})
    return {"image": image, "cases": rows, "complete": all(row["complete"] for row in rows), "scope": "seeded native-memory controls, not arbitrary-program leak coverage"}


# The receipt preserves exits and matched diagnostic types without publishing raw sanitizer traces.
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = assess(args.image)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    return 0 if report["complete"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
