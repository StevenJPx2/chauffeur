import unittest
from datetime import date, datetime, timezone
from pathlib import Path

from adept.digest import build_digest, digest_day, events_for_day
from adept.models import Event, User
from adept.render import render_digest
from adept.store import load

FIXTURES = Path(__file__).resolve().parent.parent / "data" / "fixtures.json"

SYD = User("u-syd", "Jack", "jack@example.com", 600)
SFO = User("u-sfo", "Maria", "maria@example.com", -420)
BLR = User("u-blr", "Asha", "asha@example.com", 330)


def at(text):
    return datetime.fromisoformat(text).replace(tzinfo=timezone.utc)


def titles(events):
    return [e.title for e in events]


class PositiveOffsetTest(unittest.TestCase):
    EVENTS = [
        Event("e4", "u-syd", "Dinner with the Okafors", at("2026-09-28T09:00")),
        Event("e7", "u-syd", "Gym", at("2026-09-29T08:00")),
        Event("e5", "u-syd", "Standup", at("2026-09-28T22:00")),
        Event("e6", "u-syd", "1:1 with Priya", at("2026-09-29T00:30")),
        Event("e8", "u-syd", "Next day", at("2026-09-29T14:00")),
    ]

    def test_digest_day_uses_local_date(self):
        self.assertEqual(digest_day(SYD, at("2026-09-28T21:30")), date(2026, 9, 29))
        self.assertEqual(digest_day(SYD, at("2026-09-28T13:59")), date(2026, 9, 28))
        self.assertEqual(digest_day(SYD, at("2026-09-28T14:00")), date(2026, 9, 29))

    def test_events_for_local_day(self):
        got = events_for_day(self.EVENTS, SYD, date(2026, 9, 29))
        self.assertEqual(titles(got), ["Standup", "1:1 with Priya", "Gym"])

    def test_digest_from_ticket(self):
        text = build_digest(SYD, self.EVENTS, at("2026-09-28T21:30"))
        self.assertIn("Tuesday 29 September", text)
        self.assertIn("08:00  Standup", text)
        self.assertIn("10:30  1:1 with Priya", text)
        self.assertIn("18:00  Gym", text)
        self.assertNotIn("Dinner", text)
        self.assertNotIn("Next day", text)

    def test_cli_fixture_repro(self):
        users, events = load(FIXTURES)
        text = build_digest(users["u-syd"], events, at("2026-09-28T21:30"))
        self.assertIn("Tuesday 29 September", text)
        self.assertIn("08:00  Standup", text)
        self.assertLess(text.index("Standup"), text.index("1:1 with Priya"))


class NegativeOffsetTest(unittest.TestCase):
    EVENTS = [
        Event("a", "u-sfo", "Board prep", at("2026-09-28T16:00")),
        Event("b", "u-sfo", "School pickup", at("2026-09-28T22:30")),
        Event("c", "u-sfo", "Late dinner", at("2026-09-29T03:00")),
        Event("d", "u-sfo", "Investor call", at("2026-09-29T15:00")),
        Event("e", "u-sfo", "Too early", at("2026-09-28T06:59")),
    ]

    def test_evening_digest_is_still_today(self):
        self.assertEqual(digest_day(SFO, at("2026-09-29T02:00")), date(2026, 9, 28))

    def test_events_for_local_day(self):
        got = events_for_day(self.EVENTS, SFO, date(2026, 9, 28))
        self.assertEqual(titles(got), ["Board prep", "School pickup", "Late dinner"])

    def test_rendered_times_are_local(self):
        text = build_digest(SFO, self.EVENTS, at("2026-09-28T14:00"))
        self.assertIn("Monday 28 September", text)
        self.assertIn("09:00  Board prep", text)
        self.assertIn("15:30  School pickup", text)
        self.assertIn("20:00  Late dinner", text)
        self.assertNotIn("Investor call", text)


class BoundaryTest(unittest.TestCase):
    def test_half_hour_offset_midnight_boundaries(self):
        events = [
            Event("m0", "u-blr", "At midnight", at("2026-09-28T18:30")),
            Event("m1", "u-blr", "Just before", at("2026-09-28T18:29")),
            Event("m2", "u-blr", "Next midnight", at("2026-09-29T18:30")),
            Event("m3", "u-blr", "Last minute", at("2026-09-29T18:29")),
        ]
        got = events_for_day(events, BLR, date(2026, 9, 29))
        self.assertEqual(titles(got), ["At midnight", "Last minute"])
        text = render_digest(BLR, date(2026, 9, 29), got)
        self.assertIn("00:00  At midnight", text)
        self.assertIn("23:59  Last minute", text)

    def test_other_users_events_excluded(self):
        events = [Event("x", "u-other", "Someone else", at("2026-09-29T00:30"))]
        self.assertEqual(events_for_day(events, SYD, date(2026, 9, 29)), [])

    def test_utc_user_unchanged(self):
        ana = User("u-lis", "Ana", "ana@example.com", 0)
        events = [Event("a", "u-lis", "Design review", at("2026-09-28T09:30"))]
        text = build_digest(ana, events, at("2026-09-28T06:00"))
        self.assertIn("09:30  Design review", text)
