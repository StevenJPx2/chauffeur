# `chauffeur status`

Prints the daemon's version, the rules it loaded, and the rule files it
refused, each with its path and why.

```sh
chauffeur status
```

```json
{
  "version": "1.0.0-rc.1",
  "protocol": 1,
  "rules": ["conventional-commits", "team/release"],
  "refused": [
    "/Users/you/.config/chauffeur/rules/team/broken.json: rule `team/broken`: step `s` names judge `nobody`, which is neither in `judges` nor a tier"
  ]
}
```

A rule file is checked when it loads, so `refused` is where a mistake in a
rule shows, with no separate validation step.

From slice 5, `chauffeur status --project WORKTREE` lists the rules a session
in that worktree would load, its project rules included, and the ones
refused.
