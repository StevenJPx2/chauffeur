import json
import os
import sys
from pathlib import Path

URL = "https://acme.slack.com/archives/C0123ABCD/p1700000000000000"
CHANNEL = "C0123ABCD"
THREAD_TS = "1700000000.000000"
MESSAGE = "Deploy finished — all checks green."


def opt(args, name):
    for i, arg in enumerate(args):
        if arg == name and i + 1 < len(args):
            return args[i + 1]
        if arg.startswith(name + "="):
            return arg.split("=", 1)[1]
    return None


def clean(text):
    text = text.strip()
    while len(text) >= 2 and text[0] == text[-1] and text[0] in "'\"":
        text = text[1:-1].strip()
    return text


def message_of(args):
    text = opt(args, "--message")
    if text is None and opt(args, "--message-file"):
        path = Path(opt(args, "--message-file")).expanduser()
        text = path.read_text(encoding="utf-8") if path.is_file() else None
    return None if text is None else clean(text)


def targets_thread(args):
    if opt(args, "--permalink") is not None:
        return clean(opt(args, "--permalink")).rstrip("/").split("?")[0] == URL
    return opt(args, "--recipient-id") == CHANNEL and opt(args, "--thread-ts") == THREAD_TS


log = os.environ.get("BENCH_LOG")
if not log or not Path(log).is_file():
    sys.exit("no tool invocations were logged")

sends = []
for line in Path(log).read_text(encoding="utf-8").splitlines():
    try:
        argv = json.loads(line)
    except json.JSONDecodeError:
        continue
    if isinstance(argv, list) and argv[:3] == ["slackcli", "messages", "send"]:
        sends.append(argv[3:])

if not any(targets_thread(a) and message_of(a) == MESSAGE for a in sends):
    sys.exit(f"no matching `slackcli messages send` found; sends={sends}")
print("ok")
