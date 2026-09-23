---
name: slack-cli
description: Use slackcli for ALL Slack work — reading channels and threads, searching messages, looking up people, and sending replies. Never drive a browser or call Slack's web API by hand for these.
---

# slack-cli — Slack from the terminal

**Binary:** `slackcli` (already authenticated; `slackcli auth --help` if not)

Prefer `--json` whenever you parse the output.

## Read

```sh
# A thread or channel from a Slack link (most common: someone pasted a URL)
slackcli conversations read --permalink "https://…slack.com/archives/C0123/p1700000000000000" --json

# A channel's recent history, or one thread in it
slackcli conversations read C0123ABCD --limit 50 --json
slackcli conversations read C0123ABCD --thread-ts 1700000000.000000 --json

# One message, and what is unread
slackcli conversations get C0123ABCD 1700000000.000000
slackcli conversations unread
```

Find channel IDs with `slackcli conversations list` or `slackcli search channels <name>`.

## Search

```sh
slackcli search messages "deploy failed" --in eng --limit 20 --json
slackcli search messages "from:@priya checkout" --sort timestamp
slackcli search people "priya"
```

`search messages` accepts Slack's search operators (`from:`, `in:`, `before:`, `after:`).

## Reply and send

Only send when the user asked you to post. Reply in the thread you were asked about:

```sh
slackcli messages send --permalink "https://…/p1700000000000000" --message "Fixed in #42."
slackcli messages send --recipient-id C0123ABCD --thread-ts 1700000000.000000 --message-file reply.md
slackcli messages react --help   # add a reaction instead of a message when that is enough
```

Use `--message-file` for anything longer than a sentence, so quoting cannot break the shell.
