"""Find an existing crates.io baseline; only 404/no stable release may skip."""
import argparse
import json
import re
import sys
import urllib.error
import urllib.request

PACKAGES = ['insi-core', 'insi-eval', 'insi-memory', 'insi-loop', 'insi-connectors']


def baseline(package, opener=urllib.request.urlopen):
    request = urllib.request.Request(
        f'https://crates.io/api/v1/crates/{package}',
        headers={'User-Agent': 'insi-semver-baseline/0.1.0'},
    )
    try:
        with opener(request, timeout=30) as response:
            data = json.load(response)
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return None
        raise
    version = data['crate']['max_stable_version']
    if version is None:
        return None
    if not isinstance(version, str) or not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.+-]+)?', version):
        raise ValueError('invalid registry baseline version')
    return version


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('package', choices=PACKAGES)
    args = parser.parse_args()
    try:
        version = baseline(args.package)
    except (OSError, ValueError, KeyError, TypeError):
        print('Registry baseline lookup failed; refusing to skip compatibility checks.', file=sys.stderr)
        return 1
    if version is None:
        print('No published stable baseline; first publication has no API to compare.', file=sys.stderr)
        print('available=false')
    else:
        print(f'available=true\nversion={version}')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
