import unittest
from datetime import timezone
from pathlib import Path

from adept.store import load, parse_utc

FIXTURES = Path(__file__).resolve().parent.parent / "data" / "fixtures.json"


class StoreTest(unittest.TestCase):
    def test_parse_utc(self):
        value = parse_utc("2026-09-28T09:30:00Z")
        self.assertEqual(value.tzinfo, timezone.utc)
        self.assertEqual(value.hour, 9)

    def test_parse_utc_converts_offsets(self):
        self.assertEqual(parse_utc("2026-09-28T19:30:00+10:00").hour, 9)

    def test_load_fixtures(self):
        users, events = load(FIXTURES)
        self.assertEqual(users["u-syd"].utc_offset_minutes, 600)
        self.assertTrue(all(e.starts_at.tzinfo is not None for e in events))
