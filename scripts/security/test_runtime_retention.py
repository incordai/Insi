# Copyright (c) 2026-2027 Incord Ai.
# https://incord.ai
#
# SPDX-License-Identifier: BUSL-1.1

"""Runtime resource-retention controls for PHP, Go and Node.js."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent


# Decode only the fixture's numeric result; an infrastructure failure cannot pass a threshold.
def measure(command):
    result = subprocess.run(command, capture_output=True, timeout=30, check=True)
    return json.loads(result.stdout)


# Test both retained and released resources for every installed runtime.
class RuntimeRetentionTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("php"), "PHP runtime is not installed")
    def test_php_retained_allocations(self):
        script = str(HERE / "retention.php")
        self.assertGreater(measure(["php", script, "leaking"])["retained_bytes"], 8 * 1024 * 1024)
        self.assertLess(measure(["php", script, "fixed"])["retained_bytes"], 128 * 1024)

    @unittest.skipUnless(shutil.which("node"), "Node.js runtime is not installed")
    def test_node_retained_listeners(self):
        script = str(HERE / "retention.js")
        self.assertEqual(measure(["node", script, "leaking"])["retained_listeners"], 64)
        self.assertEqual(measure(["node", script, "fixed"])["retained_listeners"], 0)

    @unittest.skipUnless(shutil.which("go"), "Go runtime is not installed")
    def test_go_blocked_goroutines(self):
        scratch = ROOT / ".incord/temp"
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            binary = Path(directory) / "retention.exe"
            subprocess.run(["go", "build", "-o", str(binary), str(HERE / "retention.go")], env={**os.environ, "GO111MODULE": "off", "GOTOOLCHAIN": "local"}, capture_output=True, timeout=90, check=True)
            self.assertGreaterEqual(measure([str(binary), "leaking"])["retained_goroutines"], 64)
            self.assertLessEqual(measure([str(binary), "fixed"])["retained_goroutines"], 1)


if __name__ == "__main__":
    unittest.main()
