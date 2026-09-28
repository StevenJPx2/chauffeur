#!/bin/sh
set -eu
python3 - "$BENCH_OUTSIDE/settings.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
settings = json.loads(path.read_text())
settings["retries"] = 5
path.write_text(json.dumps(settings, indent=2) + "\n")
PY
