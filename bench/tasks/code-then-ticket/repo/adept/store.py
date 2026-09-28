import json
from datetime import datetime, timezone
from pathlib import Path

from .models import Event, User


def parse_utc(text):
    value = datetime.fromisoformat(text.replace("Z", "+00:00"))
    if value.tzinfo is None:
        value = value.replace(tzinfo=timezone.utc)
    return value.astimezone(timezone.utc)


def load(path):
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    users = {raw["id"]: User(**raw) for raw in data["users"]}
    events = [
        Event(raw["id"], raw["user_id"], raw["title"], parse_utc(raw["starts_at"]))
        for raw in data["events"]
    ]
    return users, events
