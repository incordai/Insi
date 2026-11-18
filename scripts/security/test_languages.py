# Copyright (c) 2026-2027 Incord Ai.
# https://incord.ai
#
# SPDX-License-Identifier: BUSL-1.1

"""Positive and fixed static-analysis controls; not runtime exploit coverage claims."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]

# Each pair uses the same API with unsafe construction versus bound parameters or safer sinks.
CASES = {
    "rust-retention": ("rs", 'fn main() { let value = vec![0_u8; 4096]; std::mem::forget(value); }', 'fn main() { let value = vec![0_u8; 4096]; drop(value); }'),
    "rust-shell": ("rs", 'fn main() { std::process::Command::new("sh").arg("-c").arg(input); }', 'fn main() { std::process::Command::new("echo").arg(input); }'),
    "php-sql": ("php", '<?php $db->query("SELECT * FROM users WHERE id=" . $id);', '<?php $db->prepare("SELECT * FROM users WHERE id=?");'),
    "go-sql": ("go", 'package main\nfunc query() { db.Query("SELECT * FROM users WHERE id=" + id) }', 'package main\nfunc query() { db.Query("SELECT * FROM users WHERE id=$1", id) }'),
    "javascript-sql": ("js", 'db.query("SELECT * FROM users WHERE id=" + id);', 'db.query("SELECT * FROM users WHERE id=?", [id]);'),
    "typescript-sql": ("ts", 'db.query("SELECT * FROM users WHERE id=" + id);', 'db.query("SELECT * FROM users WHERE id=?", [id]);'),
    "java-sql": ("java", 'class Query { void query() { stmt.executeQuery("SELECT * FROM users WHERE id=" + id); } }', 'class Query { void query() { var stmt = conn.prepareStatement("SELECT * FROM users WHERE id=?"); stmt.setInt(1, id); stmt.executeQuery(); } }'),
    "python-deserialization": ("py", 'import pickle\nvalue = pickle.loads(data)', 'import json\nvalue = json.loads(data)'),
    "python-shell": ("py", 'import subprocess\nsubprocess.run(command, shell=True)', 'import subprocess\nsubprocess.run(["echo", value], shell=False)'),
    "php-shell": ("php", '<?php shell_exec("echo " . $value);', '<?php proc_open(["echo", $value], [], $pipes);'),
    "javascript-xss": ("js", 'element.innerHTML = input;', 'element.textContent = input;'),
}


# Run all languages in one scanner process and require every expected positive and no negatives.
@unittest.skipUnless(shutil.which("semgrep"), "Semgrep is not installed")
class LanguageTests(unittest.TestCase):
    def test_language_and_weakness_controls(self):
        scratch = ROOT / ".incord/temp"
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            folder = Path(directory)
            expected = set()
            for name, (suffix, positive, fixed) in CASES.items():
                for label, text in (("positive", positive), ("fixed", fixed)):
                    filename = f"{name}-{label}.{suffix}"
                    (folder / filename).write_text(text + "\n")
                    if label == "positive":
                        expected.add(filename)
            result = subprocess.run(["semgrep", "scan", "--metrics=off", "--disable-version-check", "--no-git-ignore", "--config", str(ROOT / "scripts/security/semgrep.yml"), "--json", "."], cwd=directory, env={**os.environ, "SEMGREP_SEND_METRICS": "off"}, capture_output=True, timeout=90)
            self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace")[-2000:])
            report = json.loads(result.stdout)
            self.assertFalse(report["errors"])
            self.assertEqual({Path(row["path"]).name for row in report["results"]}, expected)


if __name__ == "__main__":
    unittest.main()
