from datetime import timedelta, timezone


def render_digest(user, day, events):
    tz = timezone(timedelta(minutes=user.utc_offset_minutes))
    lines = [f"Hi {user.name}, here's your {day:%A %d %B}:", ""]
    if not events:
        lines.append("  Nothing scheduled. Enjoy!")
    for event in events:
        lines.append(f"  {event.starts_at.astimezone(tz):%H:%M}  {event.title}")
    return "\n".join(lines) + "\n"
