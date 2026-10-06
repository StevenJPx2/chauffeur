import json
import os
import sys
from pathlib import Path

answer = Path("ANSWER.md")
if not answer.is_file():
    sys.exit("ANSWER.md is missing")
text = answer.read_text(encoding="utf-8").lower()
for needle in ("10 minutes", "secondary"):
    if needle not in text:
        sys.exit(f"ANSWER.md does not mention {needle!r}")

log = os.environ.get("BENCH_LOG")
searched = False
if log and Path(log).is_file():
    for line in Path(log).read_text(encoding="utf-8").splitlines():
        try:
            call = json.loads(line)
        except json.JSONDecodeError:
            continue
        searched = searched or (isinstance(call, list) and call[:3] == ["mcp", "handbook", "search"])
if not searched:
    sys.exit("the handbook was never searched")
print("ok")
