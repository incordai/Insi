# Copyright (c) 2026-2027 Incord Ai.
# https://incord.ai
#
# SPDX-License-Identifier: BUSL-1.1

"""The local workflow gate must not silently downgrade or skip CI's online checks."""
import contextlib
import io
import os
from pathlib import Path
import subprocess
import unittest
from unittest import mock

import test_all
import workflow_security


# Isolate credentials and external processes so these tests need no scanner or GitHub account.
class WorkflowSecurity(unittest.TestCase):
    def setUp(self):
        self.env = mock.patch.dict(os.environ, {}, clear=True)
        self.env.start()
        self.addCleanup(self.env.stop)

    # A missing scanner is an incomplete verification, never a green result.
    def test_missing_scanner_fails(self):
        with mock.patch.object(workflow_security, 'scanner', return_value=None), mock.patch.object(subprocess, 'run') as run, contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(workflow_security.main(), 2)
        run.assert_not_called()

    # An offline environment cannot make the supposedly online gate skip provenance audits.
    def test_offline_flags_are_rejected(self):
        for flag in ['ZIZMOR_OFFLINE', 'ZIZMOR_NO_ONLINE_AUDITS']:
            with self.subTest(flag=flag), mock.patch.dict(os.environ, {flag:'true', 'GH_TOKEN':'fixture'}):
                with self.assertRaisesRegex(RuntimeError, 'requires online audits'):
                    workflow_security.environment()

    # Missing authentication must be explicit because Zizmor otherwise skips online audits.
    def test_missing_authentication_fails(self):
        with mock.patch.object(workflow_security.shutil, 'which', return_value=None):
            with self.assertRaisesRegex(RuntimeError, 'GitHub token'):
                workflow_security.environment()

    # An unusable existing login cannot accidentally authorize a reduced scan.
    def test_failed_gh_login_is_an_error(self):
        with mock.patch.object(workflow_security.shutil, 'which', return_value='gh'), mock.patch.object(subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, stdout='', stderr='unavailable')):
            with self.assertRaisesRegex(RuntimeError, 'existing gh login'):
                workflow_security.environment()

    # Reuse the existing login without exposing its token in command arguments or terminal output.
    def test_existing_login_is_forwarded_only_in_environment(self):
        login = subprocess.CompletedProcess([], 0, stdout='fixture-credential\n', stderr='')
        with mock.patch.object(workflow_security.shutil, 'which', return_value='gh'), mock.patch.object(subprocess, 'run', return_value=login) as run:
            env = workflow_security.environment()
        self.assertEqual(env['GH_TOKEN'], 'fixture-credential')
        self.assertEqual(run.call_args.args[0], ['gh', 'auth', 'token', '--hostname', 'github.com'])

    # A different local scanner version must not be described as equivalent to CI.
    def test_wrong_version_fails_before_authentication(self):
        version = subprocess.CompletedProcess([], 0, stdout='zizmor 0.0.0\n')
        with mock.patch.object(workflow_security, 'scanner', return_value='zizmor'), mock.patch.object(subprocess, 'run', return_value=version) as run, contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(workflow_security.main(), 2)
        self.assertEqual(run.call_count, 1)

    # Findings must preserve the scanner's failing exit, exact scope and online execution.
    def test_findings_fail_without_leaking_credentials(self):
        version = subprocess.CompletedProcess([], 0, stdout='zizmor ' + workflow_security.VERSION + '\n')
        found = subprocess.CompletedProcess([], 14)
        output = io.StringIO()
        with mock.patch.dict(os.environ, {'GH_TOKEN':'fixture-credential'}), mock.patch.object(workflow_security, 'scanner', return_value='zizmor'), mock.patch.object(subprocess, 'run', side_effect=[version, found]) as run, contextlib.redirect_stdout(output):
            self.assertEqual(workflow_security.main(), 14)
        args = run.call_args.args[0]
        self.assertEqual(args[-2:], ['--', '.github'])
        self.assertIn('--persona=regular', args)
        self.assertIn('--collect=default', args)
        self.assertNotIn('--offline', args)
        self.assertNotIn('--no-online-audits', args)
        self.assertEqual(run.call_args.kwargs['env']['GH_TOKEN'], 'fixture-credential')
        self.assertNotIn('fixture-credential', str(args) + output.getvalue())

    # Tool timeouts are infrastructure errors, never successful scans.
    def test_timeout_fails(self):
        with mock.patch.object(workflow_security, 'scanner', return_value='zizmor'), mock.patch.object(subprocess, 'run', side_effect=subprocess.TimeoutExpired('zizmor', 15)), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(workflow_security.main(), 2)

    # Install instructions and the CI action must keep the local scanner at the same version.
    def test_local_version_matches_ci(self):
        root = Path(__file__).resolve().parents[2]
        ci = (root / '.github/workflows/ci.yml').read_text(encoding='utf-8')
        workflow_job = ci.split('\n  workflows:', 1)[1]
        self.assertIn('version: ' + workflow_security.VERSION, workflow_job)
        self.assertIn('zizmor==' + workflow_security.VERSION, (root / 'scripts/ci/requirements.txt').read_text(encoding='utf-8'))

    # The common preflight must actually invoke this check and propagate its failure.
    def test_common_preflight_includes_online_audit_and_fails_with_it(self):
        def execute(command, **_):
            return subprocess.CompletedProcess(command, 14 if command[-1] == 'scripts/ci/workflow_security.py' else 0)
        with mock.patch.object(subprocess, 'run', side_effect=execute) as run, contextlib.redirect_stdout(io.StringIO()):
            self.assertTrue(test_all.main())
        self.assertEqual(run.call_args_list[0].args[0][-1], 'scripts/ci/workflow_security.py')


if __name__ == '__main__':
    unittest.main()
