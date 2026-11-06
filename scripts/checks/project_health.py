"""Read-only checks for this CLI; no persistent server is expected."""
import argparse
import json
import shutil
from pathlib import Path


def check(kind, root, minimum=1024 ** 3):
    if kind == 'records':
        path = root / '.insi-work/results.jsonl'
        if path.exists():
            with path.open(encoding='utf-8') as stream:
                for number, line in enumerate(stream, 1):
                    if not line.strip():
                        continue
                    row = json.loads(line)
                    if not isinstance(row, dict) or not isinstance(row.get('task'), str) or not isinstance(row.get('passed'), bool):
                        raise ValueError(f'invalid evaluation record at line {number}')
    elif kind == 'jobs':
        text = (root / '.github/workflows/dependencies.yml').read_text(encoding='utf-8')
        if 'schedule:' not in text or 'cron:' not in text or 'EmbarkStudios/cargo-deny-action@' not in text:
            raise ValueError('dependency audit schedule or action is missing')
    elif shutil.disk_usage(root).free < minimum:
        raise ValueError(f'workspace free space is below {minimum} bytes')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('kind', choices=['records', 'jobs', 'disk'])
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument('--min-free-bytes', type=int, default=1024 ** 3)
    args = parser.parse_args()
    try:
        check(args.kind, args.root, args.min_free_bytes)
    except (OSError, ValueError) as error:
        print(f'{args.kind}: {str(error).splitlines()[0]}')
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
