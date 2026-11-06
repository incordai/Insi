"""Regression checks for the CI configuration that CI itself depends on.

Plain text checks, so they need no YAML library in the setup-python job.
"""

import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
CI = (ROOT / '.github' / 'workflows' / 'ci.yml').read_text(encoding='utf-8')
DEPENDABOT = (ROOT / '.github' / 'dependabot.yml').read_text(encoding='utf-8')


def job_block(text, name):
    match = re.search(rf'^  {re.escape(name)}:\n(.*?)(?=^  \w+:\n|\Z)', text, re.M | re.S)
    if match is None:
        raise AssertionError(f'job {name} not found')
    return match.group(1)


class PackagingRunner(unittest.TestCase):
    def test_packaging_is_pinned_to_a_runner_with_python_39(self):
        block = job_block(CI, 'packaging')
        self.assertIn('runs-on: ${{ matrix.os }}', block)
        self.assertIn('os: [ubuntu-24.04, windows-latest]', block)

    def test_one_python_failure_does_not_cancel_the_other(self):
        block = job_block(CI, 'packaging')
        self.assertIn('fail-fast: false', block)
        self.assertIn('python: ["3.9", "3.12"]', block)


class ZizmorScope(unittest.TestCase):
    def test_ci_scans_the_whole_github_directory(self):
        block = job_block(CI, 'workflows')
        self.assertRegex(block, r'inputs: \.github\n')


class DependabotCooldown(unittest.TestCase):
    def test_every_ecosystem_has_a_seven_day_cooldown(self):
        updates = re.split(r'^  - package-ecosystem:', DEPENDABOT, flags=re.M)[1:]
        self.assertEqual(len(updates), 4)
        for update in updates:
            ecosystem = update.split('\n', 1)[0].strip()
            match = re.search(r'^    cooldown:\n      default-days: (\d+)\n', update, re.M)
            self.assertIsNotNone(match, f'{ecosystem} has no cooldown')
            self.assertGreaterEqual(int(match.group(1)), 7, ecosystem)

    def test_security_updates_are_not_disabled(self):
        self.assertNotIn('open-pull-requests-limit: 0', DEPENDABOT)
        self.assertNotIn('versioning-strategy: "lockfile-only"', DEPENDABOT)


if __name__ == '__main__':
    unittest.main()
