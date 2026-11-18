# Copyright (c) 2026-2027 Incord Ai.
# https://incord.ai
#
# SPDX-License-Identifier: BUSL-1.1

"""Compile and execute Rust retention controls with the installed rustc."""
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


# A measured global allocator must catch forgotten ownership and accept ordinary Drop cleanup.
@unittest.skipUnless(shutil.which("rustc"), "rustc is not installed")
class RustMemoryTests(unittest.TestCase):
    def test_forgotten_allocations_and_drop_control(self):
        scratch = ROOT / ".incord/temp"
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            for leaking in (False, True):
                binary = Path(directory) / ("leak.exe" if leaking else "fixed.exe")
                argv = ["rustc", "--edition=2021", str(ROOT / "scripts/security/rust_memory.rs"), "-o", str(binary)]
                if leaking:
                    argv.extend(["--cfg", "leaking"])
                compiled = subprocess.run(argv, capture_output=True, timeout=60)
                self.assertEqual(compiled.returncode, 0, compiled.stderr.decode(errors="replace")[-1000:])
                result = subprocess.run([str(binary)], capture_output=True, timeout=20, check=True)
                retained = json.loads(result.stdout)["retained_bytes"]
                if leaking:
                    self.assertGreaterEqual(retained, 8 * 1024 * 1024)
                else:
                    self.assertLess(retained, 128 * 1024)


if __name__ == "__main__":
    unittest.main()
