---
name: jira-cli
description: Use the jira command line (ankitpokhrel/jira-cli) for ALL Jira work — viewing issues and comments, listing and searching with JQL, commenting, assigning, and moving issues between statuses. Never drive a browser or call Jira's web API by hand for these.
---

# jira-cli — Jira from the terminal

**Binary:** `jira` (configured in `~/.config/.jira/.config.yml`; `-p KEY` picks another project)

The list view is interactive by default. Always pass `--plain` (or `--raw` for JSON)
so the command returns instead of waiting for input.

## Read

```sh
jira issue view DEMO-7 --plain --comments 10
jira issue view DEMO-7 --raw            # full JSON, e.g. for custom fields

jira issue list --plain -a "$(jira me)" -s "In Progress"
jira issue list --plain --no-truncate -q 'labels = checkout AND updated >= -7d'
jira issue list --plain --history        # issues you touched recently
```

## Change

Only change an issue when the user asked, or a Chauffeur reminder says the work
requires it (for example: the PR merged, so the ticket moves on).

```sh
jira issue comment add DEMO-7 "Fixed in PR #42." --no-input
jira issue comment add DEMO-7 --template reply.md --no-input   # longer comments
jira issue assign DEMO-7 "Priya Shah"
jira issue move DEMO-7 "In Review" --comment "PR #42 is up."
```

`jira issue move` takes the target status by name; if it fails, `jira issue view DEMO-7 --raw`
shows the issue's current status, and the error lists the valid transitions.
