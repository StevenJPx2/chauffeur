import json
import os
import sys
from pathlib import Path

issue = Path("ISSUE.md")
if not issue.is_file():
    sys.exit("ISSUE.md is missing")
if "WEB-418" not in issue.read_text(encoding="utf-8"):
    sys.exit("ISSUE.md does not name the created issue WEB-418")

log = os.environ.get("BENCH_LOG")
created = []
if log and Path(log).is_file():
    for line in Path(log).read_text(encoding="utf-8").splitlines():
        try:
            call = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(call, list) and call[:3] == ["mcp", "tracker", "create_issue"]:
            created.append(call[3])
if not created:
    sys.exit("the tracker's create_issue was never called")

described = [
    args
    for args in created
    if args.get("project") == "WEB"
    and any(word in json.dumps(args).lower() for word in ("plus", "+"))
]
if not described:
    sys.exit(f"no WEB issue mentions the plus sign: {created}")
print("ok")
