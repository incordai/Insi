"""Installed scanner smoke tests; absent optional tools are explicit skips."""
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


# Integration tests run only against generated local files, never external targets.
class ScannerTests(unittest.TestCase):
    # Scan only a test-owned loopback listener, never an external host or a port range.
    @unittest.skipUnless(shutil.which("nmap"), "Nmap is not installed")
    def test_nmap_observes_test_owned_loopback_port(self):
        import nmap_report
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen(4)
            port = listener.getsockname()[1]
            result = subprocess.run(["nmap", "-sT", "-Pn", "-p", str(port), "-oX", "-", "127.0.0.1"], capture_output=True, timeout=30)
            self.assertEqual(result.returncode, 0)
            findings = nmap_report.convert(result.stdout, "compose.yaml", 1)["findings"]
            self.assertEqual(len(findings), 1)
            self.assertIn(str(port), findings[0]["message"])

    @unittest.skipUnless(shutil.which("gitleaks"), "Gitleaks is not installed")
    def test_gitleaks_clean_directory(self):
        scratch = ROOT / ".incord/temp"
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            folder = Path(directory)
            (folder / "app.py").write_text('message = "hello"\n')
            report = folder / "report.json"
            result = subprocess.run(["gitleaks", "dir", directory, "--no-banner", "--log-level", "error", "--report-format", "json", "--report-path", str(report)], capture_output=True, timeout=30)
            self.assertEqual(result.returncode, 0, "Gitleaks failed to scan a clean fixture")
            self.assertEqual(json.loads(report.read_text()), [])
            # Insi captures stdout through pipes; Gitleaks' portable stdout target is '-', not /dev/stdout.
            streamed = subprocess.run(["gitleaks", "dir", directory, "--no-banner", "--log-level", "error", "--report-format", "json", "--report-path", "-", "--redact"], capture_output=True, timeout=30)
            self.assertEqual(streamed.returncode, 0, "Gitleaks stdout report failed")
            self.assertEqual(json.loads(streamed.stdout), [])

    @unittest.skipUnless(shutil.which("semgrep"), "Semgrep is not installed")
    def test_semgrep_detects_positive_and_rejects_parameterized_negative(self):
        scratch = ROOT / ".incord/temp"
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            folder = Path(directory)
            (folder / "bad.py").write_text('db.execute("SELECT note WHERE name = " + name)\n')
            (folder / "good.py").write_text('db.execute("SELECT note WHERE name = ?", (name,))\n')
            # The scratch root is git-ignored; scan it explicitly instead of inheriting repository ignores.
            result = subprocess.run(["semgrep", "scan", "--metrics=off", "--disable-version-check", "--no-git-ignore", "--config", str(ROOT / "scripts/security/semgrep.yml"), "--json", "."], cwd=directory, env={**os.environ, "SEMGREP_SEND_METRICS": "off"}, capture_output=True, timeout=60)
            self.assertEqual(result.returncode, 0, "Semgrep fixture scan failed")
            report = json.loads(result.stdout)
            self.assertFalse(report["errors"])
            self.assertEqual([Path(row["path"]).name for row in report["results"]], ["bad.py"])


if __name__ == "__main__":
    unittest.main()
