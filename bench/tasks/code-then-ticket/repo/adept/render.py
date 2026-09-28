def render_digest(user, day, events):
    lines = [f"Hi {user.name}, here's your {day:%A %d %B}:", ""]
    if not events:
        lines.append("  Nothing scheduled. Enjoy!")
    for event in events:
        lines.append(f"  {event.starts_at:%H:%M}  {event.title}")
    return "\n".join(lines) + "\n"
