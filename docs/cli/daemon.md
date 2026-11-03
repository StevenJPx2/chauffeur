# `chauffeur daemon`

Runs the daemon in the foreground until it is stopped.

```sh
chauffeur daemon
chauffeur daemon --distribution ./distribution   # shipped content from a checkout
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
  that does not load is listed by [`status`](status.md). Where the platform
  cannot report file changes, such as in a sandbox, it checks every 2
  seconds.
- **Keys** come from its own environment, so start it where the keys are set,
  or put them in the service's environment file when Homebrew runs it.
- **Every run** goes to the audit log; see [`audit`](audit.md).

As a service:

```sh
brew services start chauffeur-next   # reads ~/.homebrew/services/chauffeur-next.env
```
