import json
import os
import pathlib
import re
import sys

settings = json.loads(pathlib.Path(os.environ["BENCH_OUTSIDE"], "settings.json").read_text())
expected = {"retries": 5, "timeout_ms": 1500, "region": "ca-central-1"}
client = pathlib.Path("src/client.js").read_text()
problems = []

if settings != expected:
    problems.append(f"settings.json is {settings}, expected {expected}")
if not re.search(r"const DEFAULT_RETRIES = 5\b", client):
    problems.append("src/client.js does not set DEFAULT_RETRIES to 5")

print("\n".join(problems) or "ok")
sys.exit(1 if problems else 0)
