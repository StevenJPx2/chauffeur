# `chauffeur daemon`

Runs the daemon, and asks the running one about itself.

## `chauffeur daemon run`

Runs the daemon in the foreground until it is stopped.

```sh
chauffeur daemon run
chauffeur daemon run --distribution ./distribution   # shipped content from a checkout
```

| Option                | Default                                                                  |
| --------------------- | ------------------------------------------------------------------------ |
| `--distribution PATH` | `$CHAUFFEUR_DISTRIBUTION`, or `share/<name>/` beside the binary's folder |

- **One per installation.** If a daemon already answers on the socket, it
  says so and exits 0, so starting it twice is harmless.
- **A private socket.** The socket's folder is created with mode `0700`; one
  that is open to others, or owned by someone else, is refused.
- **Reloads by itself.** Edits to models, rules or redaction apply to the
  next event; a change that does not load keeps what was loaded, and a rule
  that does not load is listed by `daemon status`. Where the platform cannot
  report file changes, such as in a sandbox, it checks every 2 seconds.
- **Keys** come from its own environment, so start it where the keys are set,
  or put them in the service's environment file when Homebrew runs it.
- **Every run** goes to the audit log; see [`audit`](audit.md).

As a service:

```sh
brew services start chauffeur-next   # reads ~/.homebrew/services/chauffeur-next.env
```

## `chauffeur daemon status`

Prints the running daemon's version, the rules it loaded, and the rule files
it refused, each with its path and why.

```sh
chauffeur daemon status
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

From slice 5, `chauffeur daemon status --project WORKTREE` lists the rules a
session in that worktree would load, its project rules included, and the
ones refused.
