"""Text helpers. See README.md for the specification."""

import re
import unicodedata

MAX_LEN = 60
_SPECIAL = {"ß": "ss", "æ": "ae", "œ": "oe", "ø": "o", "ł": "l"}


def slugify(text):
    lowered = text.lower()
    for src, dst in _SPECIAL.items():
        lowered = lowered.replace(src, dst)
    ascii_text = unicodedata.normalize("NFKD", lowered).encode("ascii", "ignore").decode()
    slug = re.sub(r"[^a-z0-9]+", "-", ascii_text).strip("-")
    if len(slug) <= MAX_LEN:
        return slug
    if slug[MAX_LEN] == "-":
        return slug[:MAX_LEN]
    head = slug[:MAX_LEN]
    cut = head.rfind("-")
    return head[:cut] if cut > 0 else head
