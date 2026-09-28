import argparse
import sys
from datetime import datetime, timezone

from .digest import build_digest
from .store import load, parse_utc


def main(argv=None):
    parser = argparse.ArgumentParser(prog="adept")
    sub = parser.add_subparsers(dest="command", required=True)
    digest = sub.add_parser("digest", help="print a user's morning digest")
    digest.add_argument("user_id")
    digest.add_argument("--now", type=parse_utc, help="send time (ISO 8601, default: now)")
    digest.add_argument("--data", default="data/fixtures.json")
    args = parser.parse_args(argv)

    users, events = load(args.data)
    if args.user_id not in users:
        print(f"error: unknown user {args.user_id}", file=sys.stderr)
        return 1
    now = args.now or datetime.now(timezone.utc)
    print(build_digest(users[args.user_id], events, now), end="")
    return 0


if __name__ == "__main__":
    sys.exit(main())
