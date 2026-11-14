"""Regression coverage for lab isolation, controls and incomplete outcomes."""

import subprocess
import unittest

import lab


# Test manifests use fake digests only with a recording executor, never a real daemon.
def case():
    return {"id": "sql-injection", "vulnerable_image": "lab/vuln@sha256:" + "a" * 64,
            "fixed_image": "lab/fixed@sha256:" + "b" * 64,
            "probe_image": "lab/probe@sha256:" + "c" * 64,
            "ready": ["probe", "ready", "target"],
            "probe": ["probe", "sql-injection", "target"], "seconds": 10}


# Exercise the Docker policy and actual decision branches through one recording executor.
class LabTests(unittest.TestCase):
    def test_controls_and_container_boundaries(self):
        commands = []

        # Only the fixed probe rejects exploitation; readiness succeeds for both targets.
        def execute(argv, seconds=30):
            commands.append(argv)
            return 1 if "sql-injection" in argv and any("-probe-fixed" in a for a in argv) else 0

        value = case()
        lab.validate({"version": 1, "cases": [value]})
        result = lab.run_case(value, execute)
        self.assertTrue(result["complete"])
        self.assertEqual([row["exit"] for row in result["controls"]], [0, 1])
        self.assertIn("--internal", commands[0])
        for argv in commands:
            if argv[0] != "run":
                continue
            for flag in ("--pull=never", "--cap-drop=ALL", "--read-only", "--pids-limit=128", "--memory=256m", "--user=65534:65534"):
                self.assertIn(flag, argv)
            self.assertNotIn("--privileged", argv)
            self.assertNotIn("-p", argv)
            self.assertNotIn("-v", argv)
        self.assertEqual(commands[-1][:2], ["network", "rm"])

    def test_bad_controls_and_timeouts_are_incomplete_and_cleaned(self):
        for timeout in (False, True):
            commands = []

            # Infrastructure errors never become evidence of a successful fix.
            def execute(argv, seconds=30):
                commands.append(argv)
                if argv[0] == "run" and "sql-injection" in argv:
                    if timeout:
                        raise subprocess.TimeoutExpired(argv, seconds)
                    return 2
                return 0

            self.assertFalse(lab.run_case(case(), execute)["complete"])
            self.assertEqual(commands[-1][:2], ["network", "rm"])
            self.assertTrue(any(argv[0] == "rm" for argv in commands))

    def test_mutable_images_and_unbounded_budgets_are_refused(self):
        for key, value in (("vulnerable_image", "lab:latest"), ("seconds", 0), ("seconds", 301), ("seconds", True), ("probe", "sh -c anything")):
            value_case = case()
            value_case[key] = value
            with self.assertRaises(ValueError):
                lab.validate({"version": 1, "cases": [value_case]})


if __name__ == "__main__":
    unittest.main()
