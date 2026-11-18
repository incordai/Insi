# Copyright (c) 2026-2027 Incord Ai.
# https://incord.ai
#
# SPDX-License-Identifier: BUSL-1.1

"""Sanitizer evidence must distinguish a detected bug from a generic tool failure."""
import subprocess
import unittest
from unittest.mock import patch
import native_memory


# Exercise the container protocol without requiring native compilers in the unit suite.
class NativeMemoryTests(unittest.TestCase):
    def test_exit_code_without_evidence_is_incomplete(self):
        calls = []

        # A nonzero exit without the expected sanitizer diagnostic cannot establish detection.
        def execute(argv, **kwargs):
            calls.append(argv)
            return subprocess.CompletedProcess(argv, 0 if argv[1] == "rm" or "/probe-fixed" in argv else 23, b"", b"")

        with patch("native_memory.subprocess.run", side_effect=execute):
            report = native_memory.assess("sha256:" + "a" * 64)
        self.assertFalse(report["complete"])
        self.assertEqual(len([argv for argv in calls if argv[1] == "rm"]), 6)
        names = [argv[argv.index("--name") + 1] for argv in calls if argv[1] == "run"]
        self.assertEqual(len(set(names)), 6)

    # Refuse a changing compiler image before Docker sees it.
    def test_mutable_images_are_rejected(self):
        with self.assertRaises(ValueError):
            native_memory.assess("compiler:latest")


if __name__ == "__main__":
    unittest.main()
