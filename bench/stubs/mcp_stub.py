#!/usr/bin/env python3
"""A stub MCP server for chauffeur-bench tasks, over stdio.

Usage: mcp_stub.py SPEC.json

SPEC names the server's tools and their canned replies:

    {"tools": [{"name": "create_issue", "description": "...",
                "input": {"type": "object", "properties": {...}},
                "reply": "Created {title} as TRK-7."}]}

A reply's {field} is filled from the call's string arguments. Each call is
appended to $BENCH_LOG as one JSON line, `["mcp", server, tool, arguments]`,
like the argv lines `bin/` stubs write, so a task's check reads both alike.
"""

import json
import os
import string
import sys

SERVER = os.path.splitext(os.path.basename(sys.argv[1]))[0]
SPEC = json.load(open(sys.argv[1], encoding="utf-8"))
TOOLS = {tool["name"]: tool for tool in SPEC["tools"]}
LOG = os.environ.get("BENCH_LOG")


def fill(template, arguments):
    """The reply with each {field} replaced; unknown fields stay as written."""
    values = {key: value for key, value in arguments.items() if isinstance(value, str)}

    return string.Formatter().vformat(template, (), _Keep(values))


class _Keep(dict):
    def __missing__(self, key):
        return "{" + key + "}"


def log_call(tool, arguments):
    if LOG:
        with open(LOG, "a", encoding="utf-8") as out:
            out.write(json.dumps(["mcp", SERVER, tool, arguments], sort_keys=True) + "\n")


def call(params):
    tool = TOOLS.get(params.get("name"))
    arguments = params.get("arguments") or {}

    if tool is None:
        return {"content": [{"type": "text", "text": f"unknown tool {params.get('name')}"}], "isError": True}

    log_call(tool["name"], arguments)

    return {"content": [{"type": "text", "text": fill(tool["reply"], arguments)}]}


def listing(_params):
    return {
        "tools": [
            {
                "name": tool["name"],
                "description": tool["description"],
                "inputSchema": tool.get("input", {"type": "object", "properties": {}}),
            }
            for tool in SPEC["tools"]
        ]
    }


def initialize(params):
    return {
        "protocolVersion": params.get("protocolVersion", "2025-06-18"),
        "capabilities": {"tools": {}},
        "serverInfo": {"name": SERVER, "version": "1.0.0"},
    }


HANDLERS = {"initialize": initialize, "tools/list": listing, "tools/call": call, "ping": lambda _params: {}}


def respond(message):
    handler = HANDLERS.get(message.get("method"))

    if handler is None:
        return {"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32601, "message": "method not found"}}

    return {"jsonrpc": "2.0", "id": message["id"], "result": handler(message.get("params") or {})}


def main():
    for line in sys.stdin:
        if not line.strip():
            continue

        message = json.loads(line)

        # Notifications carry no id and get no reply.
        if "id" in message:
            sys.stdout.write(json.dumps(respond(message)) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
