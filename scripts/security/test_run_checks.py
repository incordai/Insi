# Copyright (c) 2026-2027 Incord Ai.
# https://incord.ai
#
# SPDX-License-Identifier: BUSL-1.1

"""Scheduled interpreter selection must not weaken or hide security test failures."""
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch
import run_checks


# Explicit interpreter selection is tested independently of this machine's PATH.
class LauncherTests(unittest.TestCase):
    def test_explicit_compatible_interpreter_is_selected(self):
        with patch.dict(os.environ, {"INSI_PYTHON": sys.executable}), patch("run_checks.subprocess.run", return_value=subprocess.CompletedProcess([], 0)):
            self.assertEqual(run_checks.choose_python(Path(".")), sys.executable)

    # A configured but incompatible interpreter fails clearly instead of changing the hash algorithm.
    def test_missing_scrypt_is_incomplete(self):
        with patch.dict(os.environ, {"INSI_PYTHON": sys.executable}), patch("run_checks.subprocess.run", return_value=subprocess.CompletedProcess([], 1)):
            with self.assertRaisesRegex(RuntimeError, "hashlib.scrypt"):
                run_checks.choose_python(Path("."))

    # An invalid explicit override must not silently fall back to an unrelated interpreter.
    def test_missing_override_is_not_ignored(self):
        with patch.dict(os.environ, {"INSI_PYTHON": "/nonexistent/insi-test-python"}):
            with self.assertRaises(RuntimeError):
                run_checks.choose_python(Path("."))

    # The wrapper preserves real test failures so the watch scheduler can file findings.
    def test_unittest_failure_exit_is_preserved(self):
        with patch("run_checks.choose_python", return_value=sys.executable), patch("run_checks.subprocess.call", return_value=17):
            self.assertEqual(run_checks.main(), 17)


if __name__ == "__main__":
    unittest.main()
