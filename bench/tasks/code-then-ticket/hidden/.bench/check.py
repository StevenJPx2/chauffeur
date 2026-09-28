import json
import os
import subprocess
import sys
from pathlib import Path

KEY = "ADEPT-481"
KEYWORDS = ("timezone", "time zone", "utc", "offset")

result = subprocess.run([sys.executable, "-m", "unittest", "-q"])
if result.returncode != 0:
    sys.exit("tests failed")


def split(args, value_flags):
    positionals, flags = [], {}
    i = 0
    while i < len(args):
        name, eq, inline = args[i].partition("=")
        if name in value_flags:
            if eq:
                flags[name] = inline
            elif i + 1 < len(args):
                flags[name] = args[i + 1]
                i += 1
        elif args[i].startswith("-") and args[i] != "-":
            flags[args[i]] = True
        else:
            positionals.append(args[i])
        i += 1
    return positionals, flags


def template_text(path):
    if not isinstance(path, str):
        return ""
    p = Path(path).expanduser()
    return p.read_text(encoding="utf-8", errors="replace") if p.is_file() else ""


log = os.environ.get("BENCH_LOG")
if not log or not Path(log).is_file():
    sys.exit("no tool invocations were logged")

comments, moves = [], []
for line in Path(log).read_text(encoding="utf-8").splitlines():
    try:
        argv = json.loads(line)
    except json.JSONDecodeError:
        continue
    if not isinstance(argv, list) or argv[:1] != ["jira"] or "--help" in argv or "-h" in argv:
        continue
    if argv[1:4] == ["issue", "comment", "add"]:
        pos, flags = split(argv[4:], {"--body", "-b", "--template", "-T"})
        if pos[:1] == [KEY]:
            body = flags.get("--body") or flags.get("-b") or " ".join(pos[1:])
            body = body if isinstance(body, str) else ""
            body += template_text(flags.get("--template") or flags.get("-T"))
            comments.append(body)
    elif argv[1:3] in (["issue", "move"], ["issue", "transition"]):
        pos, flags = split(argv[3:], {"--comment", "--resolution", "-R", "--assignee", "-a"})
        if pos[:1] == [KEY] and len(pos) == 2:
            moves.append(pos[1].strip().strip("'\"").lower())
            if isinstance(flags.get("--comment"), str):
                comments.append(flags["--comment"])

if not any(any(k in c.lower() for k in KEYWORDS) for c in comments):
    sys.exit(f"no comment on {KEY} describing the fix; comments={comments}")
if "in review" not in moves:
    sys.exit(f"{KEY} was not moved to In Review; moves={moves}")
print("ok")
