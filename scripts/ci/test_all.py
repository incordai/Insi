"""Run the repository checks locally without service credentials."""
import pathlib
import subprocess
import sys


def main():
    root = pathlib.Path(__file__).resolve().parents[2]
    commands = [
        ['cargo', 'fmt', '--check'],
        ['cargo', 'clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings'],
        ['cargo', 'test', '--workspace', '--locked'],
    ]
    for directory in ['packaging/pip', 'scripts/checks', 'scripts/ci']:
        commands.append([sys.executable, '-m', 'unittest', 'discover', '-s', directory, '-p', 'test_*.py'])
    commands.append(['node', 'packaging/npm/test.js'])
    failed = []
    for command in commands:
        print('+ ' + ' '.join(command), flush=True)
        try:
            result = subprocess.run(command, cwd=root, check=False)
            if result.returncode:
                failed.append(command)
        except OSError as error:
            print(error, file=sys.stderr)
            failed.append(command)
    print(f'{len(commands) - len(failed)}/{len(commands)} checks passed')
    return bool(failed)


if __name__ == '__main__':
    sys.exit(main())
