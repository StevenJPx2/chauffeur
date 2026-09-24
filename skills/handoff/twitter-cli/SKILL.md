---
name: twitter-cli
description: Use the twitter command (twitter-cli) for ALL X/Twitter work — reading the timeline, searching, reading posts and replies, looking up users, and posting or replying. Never drive a browser or call X's web API by hand for these.
---

# twitter-cli — X/Twitter from the terminal

**Binary:** `twitter` (install with `uv tool install twitter-cli`; it reads your browser's
X session). Check access first: `twitter status`.

Pass `-c` for compact output when you only need to read, or `--json` when you parse it.

## Read

```sh
twitter -c feed --max 20                 # home timeline; -t following for Following
twitter -c search "release notes" -t Latest --max 20
twitter tweet https://x.com/user/status/12345 --full-text   # a post and its replies
twitter -c user-posts someone --max 20
twitter user someone                     # profile
twitter bookmarks --max 20
```

There is no notifications command. To find mentions, search for the handle:
`twitter -c search "@yourhandle" -t Latest --max 20` (`twitter whoami` shows yours).

## Post

Only post, reply, like, or follow when the user asked you to.

```sh
twitter post "Shipped v1.2: faster sync."
twitter post "Chart attached" -i chart.png           # up to 4 images
twitter reply 1234567890 "Thanks, fixed in v1.2."
twitter quote 1234567890 "Worth reading."
twitter delete 1234567890
```
