from dataclasses import dataclass
from datetime import datetime


@dataclass(frozen=True)
class User:
    id: str
    name: str
    email: str
    # Offset of the user's local time from UTC, e.g. 600 for Sydney (UTC+10).
    utc_offset_minutes: int = 0


@dataclass(frozen=True)
class Event:
    id: str
    user_id: str
    title: str
    # Timezone-aware; stored and loaded as UTC.
    starts_at: datetime
