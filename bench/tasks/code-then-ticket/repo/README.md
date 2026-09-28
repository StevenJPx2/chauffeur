# adept-digest

Builds the morning digest email for Adept users: a list of the day's events,
sent early each morning.

```sh
python3 -m adept.cli digest u-lis --now 2026-09-28T06:00:00Z
python3 -m unittest
```

Events are stored in UTC. Each user has a fixed `utc_offset_minutes`.

Bugs are tracked in Jira (project ADEPT); the `jira` CLI is set up on dev machines.
