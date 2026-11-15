"""Run the same online workflow audit as CI; missing tools or authentication are failures."""
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
VERSION = '1.30.1'


# Prefer installed tools, with the documented project-local virtual environment as a fallback.
def scanner():
    installed = shutil.which('zizmor')
    local = ROOT / '.incord/temp/workflow-security-venv' / ('Scripts/zizmor.exe' if os.name == 'nt' else 'bin/zizmor')
    return installed or (str(local) if local.is_file() else None)


# Credentials travel only in the child environment, never in argv, logs or repository files.
def environment():
    env = os.environ.copy()
    # Use the repository's normal config discovery, matching CI rather than a personal override.
    env.pop('ZIZMOR_CONFIG', None)
    for flag in ['ZIZMOR_OFFLINE', 'ZIZMOR_NO_ONLINE_AUDITS']:
        if env.get(flag, '').lower() not in ('', 'false', '0'):
            raise RuntimeError('workflow security requires online audits; remove ' + flag)
        env.pop(flag, None)
    token = next((env.get(name) for name in ['GH_TOKEN', 'GITHUB_TOKEN', 'ZIZMOR_GITHUB_TOKEN'] if env.get(name)), None)
    if not token:
        gh = shutil.which('gh')
        if not gh:
            raise RuntimeError('workflow security needs a GitHub token or an existing gh login')
        auth = subprocess.run([gh, 'auth', 'token', '--hostname', 'github.com'], capture_output=True, text=True, timeout=15, check=False)
        if auth.returncode or not auth.stdout.strip():
            raise RuntimeError('workflow security could not use the existing gh login')
        token = auth.stdout.strip()
    env['GH_TOKEN'] = token
    return env


# Fail closed when the CI-equivalent online scan cannot run; retain Zizmor's finding exit code.
def main():
    try:
        binary = scanner()
        if not binary:
            raise RuntimeError('install the local workflow scanner: python3 -m venv .incord/temp/workflow-security-venv; then use its pip install -r scripts/ci/requirements.txt')
        version = subprocess.run([binary, '--version'], capture_output=True, text=True, timeout=15, check=False)
        if version.returncode or version.stdout.strip() != 'zizmor ' + VERSION:
            raise RuntimeError('workflow security requires zizmor ' + VERSION + ', matching CI')
        env = environment()
        cache = ROOT / '.incord/temp/workflow-security-cache'
        print('workflow security: zizmor ' + VERSION + ', online audits, scope .github', flush=True)
        return subprocess.run([binary, '--persona=regular', '--collect=default', '--color=never', '--no-progress', '--cache-dir', str(cache), '--', '.github'], cwd=ROOT, env=env, timeout=300, check=False).returncode
    except (OSError, subprocess.TimeoutExpired, RuntimeError) as error:
        print('workflow security could not complete: ' + str(error), file=sys.stderr)
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
