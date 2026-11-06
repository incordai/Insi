import io
import unittest
import urllib.error
from semver_baseline import baseline


class Baseline(unittest.TestCase):
    def test_first_publication_skips_but_outage_does_not(self):
        def missing(*args, **kwargs):
            raise urllib.error.HTTPError('fixture', 404, 'missing', {}, None)
        self.assertIsNone(baseline('insi-core', missing))
        def outage(*args, **kwargs):
            raise urllib.error.HTTPError('fixture', 503, 'outage', {}, None)
        with self.assertRaises(urllib.error.HTTPError):
            baseline('insi-core', outage)

    def test_existing_baseline_and_output_injection(self):
        self.assertEqual(baseline('insi-core', lambda *a, **k: io.BytesIO(b'{"crate":{"max_stable_version":"0.1.0"}}')), '0.1.0')
        with self.assertRaises(ValueError):
            baseline('insi-core', lambda *a, **k: io.BytesIO(b'{"crate":{"max_stable_version":"0.1.0\\navailable=false"}}'))
        with self.assertRaises(KeyError):
            baseline('insi-core', lambda *a, **k: io.BytesIO(b'{}'))
