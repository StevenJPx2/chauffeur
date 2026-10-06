#!/bin/sh
set -eu
# Stands in for the agent's tracker call, as the stub server would log it.
printf '%s\n' '["mcp", "tracker", "create_issue", {"project": "WEB", "title": "Login fails for passwords containing a plus sign", "type": "bug"}]' >> "$BENCH_LOG"
