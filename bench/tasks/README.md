# Benchmark tasks

Each task is a directory `bench/tasks/<id>/`:

| path          | required | purpose |
|---------------|----------|---------|
| `task.json`   | yes | `{ "prompt", "check", "timeout_seconds", "tags" }`, exactly these fields. Runs have no `--auto`: a request that would ask the user is rejected unless Chauffeur allows it. |
| `outside/`    | no  | Copied beside the repo, outside the project. The prompt names it as `{outside}`; commands see `BENCH_OUTSIDE`. |
| `repo/`       | yes | Starting files. The runner copies it to a fresh directory, runs `git init`, and commits it. |
| `hidden/`     | no  | Copied over the repo copy after the agent finishes, before the check (hidden tests, check scripts under `.bench/`). |
| `bin/`        | no  | Executable stubs prepended to `PATH` for the agent and the check. |
| `solution/`   | yes | Overlay that makes the check pass; stands in for a correct agent. |

- `check` is an argv run in the repo copy after the agent finishes; exit 0 means pass. It gets the
  agent's environment: `PATH` starting with `bin/`, and `BENCH_LOG` pointing at a log file that may
  not exist.
- Stubs are POSIX sh or `python3` (stdlib only), never touch the network, and append one line per
  invocation to `$BENCH_LOG`: a JSON array `["<tool>", ...args]`.
- `solution/.solve.sh`, if present, is not copied into the repo. It runs with the repo copy as cwd
  and the same `PATH`/`BENCH_LOG`, after the overlay, to make the stub calls or commits a correct
  agent would make.
- Prompts read like a real user request. They never mention the benchmark, the check, or tooling.
- Only `python3`, `node` (no packages), `git`, and `sh` may be assumed.

## Adding a task

1. Create `repo/` with a few small files and write `task.json`.
2. Put anything the agent must not see or edit in `hidden/`. A task whose check needs a clean git
   tree must not use `hidden/`; inline its check instead (see `commit-a-change`).
3. Add `solution/` and, for stub or git tasks, `solution/.solve.sh`. Make stubs and `.solve.sh`
   executable.
4. Run the verifier.

## Verifying

```sh
cargo run -q -p chauffeur-bench -- verify [--tasks a,b]
```

For each task it prepares a fresh copy (repo, `git init`, initial commit, `PATH`/`BENCH_LOG` set):

1. copies `hidden/` and runs the check, which must **fail**;
2. prepares a new copy, copies `solution/`, runs `solution/.solve.sh`, copies `hidden/`, then runs
   the check, which must **pass**.

It exits non-zero if any task misbehaves.
