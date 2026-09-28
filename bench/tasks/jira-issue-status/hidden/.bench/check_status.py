import json
import os
import sys
from pathlib import Path

status = Path("STATUS.md")
if not status.is_file():
    sys.exit("STATUS.md is missing")
text = status.read_text(encoding="utf-8").lower()
for needle in ("in review", "priya natarajan"):
    if needle not in text:
        sys.exit(f"STATUS.md does not mention {needle!r}")

log = os.environ.get("BENCH_LOG")
calls = []
if log and Path(log).is_file():
    for line in Path(log).read_text(encoding="utf-8").splitlines():
        try:
            argv = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(argv, list) and argv[:1] == ["jira"] and "ADEPT-123" in argv:
            calls.append(argv)
if not calls:
    sys.exit("jira was never called for ADEPT-123")
print("ok")
