import re
import sys
from pathlib import Path

EXPECTED = {
    ("app/main.py", 7): "load the store path from the command line",
    ("app/core/store.py", 16): "write to a temp file and rename for atomic saves",
    ("app/core/render.py", 10): "truncate very long titles",
    ("web/static/app.js", 7): "show an error banner when the request fails",
    ("scripts/deploy.sh", 4): "read the target host from an environment variable",
    ("tests/test_store.py", 12): "cover save() round-trips",
}

# Accept optional list bullets and backticks around the path:line prefix.
ENTRY = re.compile(r"^\s*(?:[-*+]|\d+\.)?\s*`?(?:\./)?([\w./-]+):(\d+)`?:\s*(.+?)\s*$")


def normalize(text):
    text = text.strip().strip("`").strip()
    text = re.sub(r"^(?:#|//)\s*", "", text)
    text = re.sub(r"^TODO:\s*", "", text)
    return " ".join(text.split()).lower()


todos = Path("TODOS.md")
if not todos.is_file():
    sys.exit("TODOS.md is missing")

found = {}
for line in todos.read_text(encoding="utf-8").splitlines():
    match = ENTRY.match(line)
    if match:
        found[(match.group(1), int(match.group(2)))] = normalize(match.group(3))

missing = sorted(set(EXPECTED) - set(found))
extra = sorted(set(found) - set(EXPECTED))
wrong = sorted(key for key in set(EXPECTED) & set(found) if EXPECTED[key] not in found[key])
if missing or extra or wrong:
    sys.exit(f"missing={missing} extra={extra} wrong_text={wrong}")
print("ok")
