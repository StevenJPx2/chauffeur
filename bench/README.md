# chauffeur-bench

Compares OpenCode setups on a suite of coding tasks:

- **base**: plain OpenCode, with the Chauffeur plugin disabled.
- **full**: OpenCode with Chauffeur.
- **handicaps**: Chauffeur with one capability left out (`no-rules`, …).

Each run gives one task's prompt to `opencode run` in a fresh copy of the task
repo, then runs the task's hidden check. The harness records whether the check
passed, the tokens, cost, wall time, steps, and tool calls from OpenCode's JSON
events, and for Chauffeur variants what Chauffeur did (Jev calls and latency,
skills attached, tools hidden or revealed, steers, permission decisions) from
the daemon's audit log.

## Prerequisites

- `opencode` and `chauffeur` on `PATH` (or pass `--opencode BIN` / `--chauffeur BIN`).
- The Chauffeur OpenCode plugin installed globally. Base runs disable it per
  project with `"-chauffeur"`.
- `TYPESAFE_API_KEY` in the environment: each Chauffeur run starts its own
  daemon, which needs it for Jev. `CHAUFFEUR_CONFIG_DIR` and
  `CHAUFFEUR_SKILLS_DIR` are passed through when set.
- Credentials for the model provider (the default model is `openai/gpt-6-luna`).
- `git`, `sh`, `pgrep`, and `kill`.

## Inputs

- `variants.json`: `[{ "id": "base", "chauffeur": false }, { "id": "no-rules", "chauffeur": true, "disable": ["rules"] }, …]`.
  `disable` is required when `chauffeur` is true. An optional `env` object adds
  environment variables for OpenCode, such as `hybrid`'s
  `"CHAUFFEUR_HOST_SKILLS": "keep"`: Chauffeur attaches skills and OpenCode keeps
  its own skill tool and skill list.
- `tasks/<id>/task.json`: `{ "prompt", "check": [argv…], "timeout_seconds", "tags" }`, beside:
  - `repo/`: starting files.
  - `hidden/` (optional): copied over the agent's work before the check.
  - `bin/` (optional): stubs prepended to `PATH` for the agent and the check; they append their argv to `$BENCH_LOG`.
  - `solution/` (optional): an overlay that makes the check pass, with an optional `.solve.sh` run in the repo copy.

Both files are parsed strictly: unknown fields are errors.

## Commands

```sh
# Tasks (id, timeout, tags) and variants
cargo run -q -p chauffeur-bench -- list

# Offline sanity: each task's check fails on the starting repo and passes with
# its solution (solution, then hidden, then the check). Exits 1 on a bad task.
cargo run -q -p chauffeur-bench -- verify
cargo run -q -p chauffeur-bench -- verify --tasks fix-failing-test

# Smoke run: one task, base against full, once
cargo run -q -p chauffeur-bench -- run --tasks fix-failing-test --variants base,full --repeats 1

# The full suite (defaults: all tasks, all variants, --repeats 3, --parallel 2)
cargo run -q -p chauffeur-bench --release -- run --out ~/.local/state/chauffeur/bench/suite-1

# Rebuild the report from results.jsonl
cargo run -q -p chauffeur-bench -- report ~/.local/state/chauffeur/bench/suite-1
```

`run` flags: `--tasks a,b`, `--variants a,b`, `--repeats N`, `--parallel N`
(1–16), `--model provider/model`, `--out DIR` (default
`$XDG_STATE_HOME/chauffeur/bench/<UTC timestamp>`, else under
`~/.local/state`), `--chauffeur BIN`, `--opencode BIN`. Every command takes
`--root DIR` to use a suite outside `bench/`.

`--out` must not be inside a git checkout: the agent would see the checkout
(such as `bench/tasks/`) and could fix the source task instead of its copy.

Runs share OpenCode's session database, so they appear in its session list: a
private `OPENCODE_DB` loses the provider logins and `-m` falls back to the
default model.

Jobs are ordered round-robin across variants (repeat, then task, then variant),
so a partial run still compares the variants on the same tasks.

**Resuming.** Rerun with the same `--out`: a job whose `result.json` exists is
skipped, and an unfinished job folder is cleared and rerun. A run that failed in
the harness (for example, a daemon that never became healthy) also writes
`result.json` with its `error`; delete that run folder to retry it.

**Stopping.** Ctrl-C reaches OpenCode and the daemons through the terminal's
process group. On errors, timeouts, or panics the harness kills each child's
whole process tree. Nothing survives a `kill -9` of the harness itself; check
with `pgrep -fl "chauffeur daemon"` after one.

## One run

1. Copy `repo/`, write `.opencode/opencode.jsonc` with the plugin list
   (base: `["-chauffeur", "-ntfy-notify", "-sourcefed"]`; Chauffeur variants:
   `["-ntfy-notify", "-sourcefed"]`), and commit it in a fresh git repo.
2. Chauffeur variants: start `chauffeur daemon --port <free port>` with
   `CHAUFFEUR_STATE_DIR=<run>/state`, `CHAUFFEUR_DISABLE=<csv>`,
   `CHAUFFEUR_SOURCEFED=off`, `CHAUFFEUR_IDLE_STEERING=true`, and wait up to 15 s
   for `health`.
3. `opencode run --standalone --format json -m <model> --auto <prompt>` with
   `BENCH_LOG`, the task's `bin/` on `PATH`, and for
   Chauffeur variants `CHAUFFEUR_DAEMON_URL` and `CHAUFFEUR_DISABLE`. Killed at
   `timeout_seconds`. The daemon is stopped when OpenCode exits.
4. Copy `hidden/` over the repo and run the check (at most 10 minutes); exit 0
   passes. The check runs even when the agent timed out.
5. Write `result.json` and append it to `results.jsonl`.

## Output

```
<out>/
  results.jsonl          one result per finished run, appended as runs finish
  report.md              summary, pass matrix, Chauffeur activity, vs base
  runs/<variant>/<task>/<repeat>/
    repo/                the agent's working copy
    events.jsonl         OpenCode's JSON events (stdout)
    stderr.log           OpenCode's stderr
    check.log            the check's output
    bench.log            $BENCH_LOG: calls to the task's stub commands
    daemon.log           the daemon's output (Chauffeur variants)
    state/               the daemon's state, including audit.jsonl
    result.json
```

`report.md` is built only from `results.jsonl`. Pass rates and means cover
completed runs; harness errors are counted separately.

## Cost and time

The full suite is 8 tasks × 7 variants × 3 repeats = **168 runs**. With the
default `--parallel 2` and runs of a few minutes each, expect several hours;
the model cost is roughly 168 × one run's cost (see the Cost column of a smoke
run). Start with a smoke run, check `report.md`, then launch the suite with an
explicit `--out` so it can be resumed. Raise `--parallel` only as far as the
provider's rate limits allow: throttled runs inflate wall time and retries.
