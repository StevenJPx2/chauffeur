import unittest
from datetime import date, datetime, timezone

from adept.digest import build_digest, digest_day, events_for_day
from adept.models import Event, User

ANA = User("u-lis", "Ana", "ana@example.com", 0)


def at(text):
    return datetime.fromisoformat(text).replace(tzinfo=timezone.utc)


EVENTS = [
    Event("e2", "u-lis", "Lunch with Tom", at("2026-09-28T12:00")),
    Event("e1", "u-lis", "Design review", at("2026-09-28T09:30")),
    Event("e3", "u-lis", "Flight to Porto", at("2026-09-29T07:15")),
    Event("x1", "u-other", "Not Ana's", at("2026-09-28T10:00")),
]


class DigestTest(unittest.TestCase):
    def test_events_for_day_filters_and_sorts(self):
        titles = [e.title for e in events_for_day(EVENTS, ANA, date(2026, 9, 28))]
        self.assertEqual(titles, ["Design review", "Lunch with Tom"])

    def test_digest_day(self):
        self.assertEqual(digest_day(ANA, at("2026-09-28T06:00")), date(2026, 9, 28))

    def test_build_digest(self):
        text = build_digest(ANA, EVENTS, at("2026-09-28T06:00"))
        self.assertIn("Monday 28 September", text)
        self.assertIn("09:30  Design review", text)
        self.assertIn("12:00  Lunch with Tom", text)
        self.assertNotIn("Flight to Porto", text)

    def test_empty_day(self):
        text = build_digest(ANA, EVENTS, at("2026-09-30T06:00"))
        self.assertIn("Nothing scheduled", text)
