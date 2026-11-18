# Copyright (c) 2026-2027 Incord Ai.
# https://incord.ai
#
# SPDX-License-Identifier: BUSL-1.1

"""Nmap imports must preserve uncertainty and reject unfinished or hostile reports."""

import unittest
import nmap_report


# Small native XML reports keep network activity out of parser regressions.
class NmapTests(unittest.TestCase):
    def test_open_ports_are_observations(self):
        report = b'<nmaprun><host><address addr="192.0.2.1"/><ports><port protocol="tcp" portid="8080"><state state="open"/></port><port protocol="tcp" portid="22"><state state="closed"/></port></ports></host><runstats><finished exit="success"/></runstats></nmaprun>'
        findings = nmap_report.convert(report, "compose.yaml", 5)["findings"]
        self.assertEqual(len(findings), 1)
        self.assertEqual(findings[0]["severity"], "note")
        self.assertIn("not exploit verification", findings[0]["message"])
        self.assertEqual(findings[0]["line"], 5)

    def test_invalid_or_incomplete_reports_are_not_clean(self):
        for report in (b'<nmaprun/>', b'<nmaprun><runstats><finished exit="error"/></runstats></nmaprun>', b'<!DOCTYPE x [<!ENTITY x "value">]><x/>'):
            with self.assertRaises(ValueError):
                nmap_report.convert(report, "compose.yaml", 1)
        with self.assertRaises(ValueError):
            nmap_report.convert(b'<nmaprun/>', "../outside", 1)


if __name__ == "__main__":
    unittest.main()
