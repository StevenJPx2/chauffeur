from datetime import datetime, timedelta, timezone

from .render import render_digest


def user_tz(user):
    return timezone(timedelta(minutes=user.utc_offset_minutes))


def day_bounds(user, day):
    """Start (inclusive) and end (exclusive) of the user's calendar `day`, in UTC."""
    start = datetime(day.year, day.month, day.day, tzinfo=user_tz(user))
    return start.astimezone(timezone.utc), (start + timedelta(days=1)).astimezone(timezone.utc)


def events_for_day(events, user, day):
    start, end = day_bounds(user, day)
    mine = (e for e in events if e.user_id == user.id and start <= e.starts_at < end)
    return sorted(mine, key=lambda e: e.starts_at)


def digest_day(user, now):
    """The day a digest sent at `now` covers: the user's today."""
    return now.astimezone(user_tz(user)).date()


def build_digest(user, events, now):
    day = digest_day(user, now)
    return render_digest(user, day, events_for_day(events, user, day))
