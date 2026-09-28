from datetime import datetime, timedelta, timezone

from .render import render_digest


def day_bounds(user, day):
    """Start (inclusive) and end (exclusive) of the user's calendar `day`."""
    start = datetime(day.year, day.month, day.day, tzinfo=timezone.utc)
    return start, start + timedelta(days=1)


def events_for_day(events, user, day):
    start, end = day_bounds(user, day)
    mine = (e for e in events if e.user_id == user.id and start <= e.starts_at < end)
    return sorted(mine, key=lambda e: e.starts_at)


def digest_day(user, now):
    """The day a digest sent at `now` covers: the user's today."""
    return now.date()


def build_digest(user, events, now):
    day = digest_day(user, now)
    return render_digest(user, day, events_for_day(events, user, day))
