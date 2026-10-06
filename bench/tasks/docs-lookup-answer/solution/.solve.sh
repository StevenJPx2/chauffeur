#!/bin/sh
set -eu
# Stands in for the agent's handbook search, as the stub server would log it.
printf '%s\n' '["mcp", "handbook", "search", {"query": "sev-1 escalation"}]' >> "$BENCH_LOG"
