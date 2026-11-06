import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from project_health import check


class Health(unittest.TestCase):
    def setUp(self):
        scratch = Path(__file__).resolve().parents[2] / '.claude/temp'
        scratch.mkdir(parents=True, exist_ok=True)
        self.folder = tempfile.TemporaryDirectory(dir=scratch)
        self.root = Path(self.folder.name)

    def tearDown(self):
        self.folder.cleanup()

    def test_records_control(self):
        check('records', self.root)
        path = self.root / '.insi-work/results.jsonl'
        path.parent.mkdir()
        path.write_text('{"task":"example","passed":false}\n')
        check('records', self.root)
        path.write_text('broken json\n')
        with self.assertRaises(ValueError):
            check('records', self.root)

    def test_schedule_control(self):
        path = self.root / '.github/workflows/dependencies.yml'
        path.parent.mkdir(parents=True)
        path.write_text('schedule:\n  - cron: "17 8 * * 1"\nuses: EmbarkStudios/cargo-deny-action@abc\n')
        check('jobs', self.root)
        path.write_text('name: dependencies\n')
        with self.assertRaises(ValueError):
            check('jobs', self.root)

    def test_disk_control(self):
        with patch('project_health.shutil.disk_usage') as usage:
            usage.return_value.free = 100
            check('disk', self.root, 99)
            with self.assertRaises(ValueError):
                check('disk', self.root, 101)
