#!/usr/bin/env python3
"""Verify that denied candidate approval never reaches edit execution."""
from pathlib import Path
import subprocess


def main():
    root = Path(__file__).resolve().parents[2]
    return subprocess.call(
        ["cargo", "test", "-p", "insi", "--bin", "insi", "--locked",
         "audit_bridge::tests::denial_stops_before_execution_and_never_uses_a_direct_worker",
         "--", "--exact"],
        cwd=root,
    )


if __name__ == "__main__":
    raise SystemExit(main())
