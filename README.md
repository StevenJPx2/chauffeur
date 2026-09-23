# Chauffeur

Chauffeur progressively enhances a coding agent's context. It observes the
agent's events, classifies the situation with a System One model (TypeSafe's
Jev), and routes the judgment to capabilities:

- **Skill exposure** attaches the one skill that best fits each user message,
  and hands the agent a better-fitting skill when it drifts, such as driving a
  browser for X or Jira where a dedicated skill exists.
- **Tool exposure** hides tool groups a context won't need and brings them
  back when a later message does. With Code Mode, it points the agent at the
  Code Mode tools a request needs, which Code Mode's catalog shows only in
  part, and it removes OpenCode's own skill list (about 4k tokens), since
  Chauffeur loads skills itself.
- **Permission** enforces JSON skill contracts on the agent's edits and shell
  commands.
- **Model router** switches to an equivalent model on a usage limit, and back
  once the limit has likely cleared.
- **Idle reminders** (optional) nudge an idle agent with plugin rules, including
  follow-through on sourcefed events.
- **Tool misuse** steers the agent after a tool call that a misuse contract
  flags, such as a hand-rolled Python patch script instead of the edit tool, or
  the browser for Slack, and hands over the right skill.
- **Event gate** decides which sourcefed events reach the agent, so only ones
  it needs to act on arrive.

See [`ARCHITECTURE.md`](./ARCHITECTURE.md) for the design, what is built, and
its limits.

## Prerequisites

Set `TYPESAFE_API_KEY` to a TypeSafe API key. Jev is Chauffeur's only System
One provider, so the daemon refuses to start without it. If Jev is unreachable,
Chauffeur stands aside: prompts go through unenhanced and permission requests
fall back to asking you.

## Build and validate

```sh
cargo build --release
ln -s "$PWD/skills" ~/.config/chauffeur/skills
ln -s "$PWD"/skills/handoff/* ~/.agents/skills/
./target/release/chauffeur skill validate skills/misuse/slack-via-browser.json
```

Contracts and Chauffeur's own skills live apart from the code in `skills/`
(`permission/`, `misuse/`, `handoff/`). The daemon loads that folder from
`CHAUFFEUR_SKILLS_DIR`, by default `~/.config/chauffeur/skills`. Link
`skills/handoff/*` into a directory OpenCode reads, such as
`~/.agents/skills`, so hand-overs can find them. Validation uses the same strict JSON schema and size
limits as daemon startup, and rejects unknown fields, invalid thresholds,
duplicate IDs, and unsupported schema versions.

The daemon keeps what it has learned about each session in
`~/.local/state/chauffeur/state.json` (`CHAUFFEUR_STATE_DIR`), so a restart
loses nothing.

Every decision is logged to `~/.local/state/chauffeur/audit.jsonl`. To see
why Chauffeur did something:

```sh
chauffeur audit      # the last 20 decisions
chauffeur audit 100
```

```text
00:16:40 UTC ses_f32a1b… user_message Check my latest Twitter mentions → attach_skills ["twitter-cli"] | 1 asked, 493 ms
00:16:42 UTC ses_f32a1b… permission_request shell rm -rf / → permission deny | 0 asked, 0 ms; vetoed: rm -rf /
```

## Run

```sh
./target/release/chauffeur daemon
./target/release/chauffeur signal --file permission.json
```

The OpenCode adapter reports events to the daemon automatically.
`chauffeur signal` sends one signal by hand and prints the effects it produced,
for example a permission request:

```json
{
  "agent_id": "demo-session",
  "at": 1780000000,
  "kind": {
    "type": "permission_request",
    "action": "edit",
    "resources": [{ "requested": "README.md", "resolved": "/work/app/README.md" }],
    "request": "Update the install section",
    "workspace": "/work/app",
    "user_requests": ["please update README.md"]
  }
}
```

The permission capability computes each contract's evidence
(`resource_present`, `resource_within_workspace`, `resource_named_by_user`)
before any model call. Missing evidence returns the contract's fixed
`missing_evidence` outcome; low confidence selects `uncertain`; a System One
failure selects `judge_failure`. The most restrictive matching outcome answers
the host, and a host denial always stands.

## Load the OpenCode adapter

Install the adapter's dependencies once (`cd adapters/opencode && npm install`),
then load it with a one-line plugin file, either for one project in
`.opencode/plugins/chauffeur.ts` or for every project in
`~/.config/opencode/plugins/chauffeur.ts`:

```ts
export { default } from "/path/to/chauffeur/adapters/opencode/src/index.ts"
```

OpenCode 2.0.15 does not load the adapter when its package directory is listed
under `plugins` in `opencode.json(c)`; the plugin file form loads it.

## Author a skill

Add a `.json` file under `skills/` with these required sections:

1. `schema_version` and `identity` (`id`, `name`, numeric `major.minor.patch` `version`)
2. `match.events` and `match.actions` with exact host values
3. `predicates.required_evidence` and `predicates.forbidden_evidence`
4. A typed `decision` (`choice`, `score`, or `noul`) with confidence and explicit
   numeric thresholds or criterion-to-branch mappings
5. Six static outcomes: `positive`, `negative`, `uncertain`,
   `missing_evidence`, `judge_failure`, and `cooldown`
6. `cooldown_seconds`

All objects reject unknown fields. Skill files may be at most 65,536 bytes; a
directory may contain at most 64 JSON skill files and 256 entries total. Skill
contexts are limited to 65,536 encoded bytes, and the daemon caps RPC request
bodies at 128 KiB. See
[`skills/permission/workspace-edit-gate.json`](./skills/permission/workspace-edit-gate.json)
for a complete permission-boundary example.

## Model failover

When a model hits a usage limit, the model router asks System One to pick an
equivalent model from the same tier (or to stay), and the OpenCode adapter
switches the session and retries. Pin preferred fallbacks in
`~/.config/chauffeur/model-router.json`:

```json
{ "pins": ["openai/gpt-6-sol", "openai/gpt-6-luna"] }
```

Once the limit has likely cleared (judged no sooner than 5 minutes later), the
router switches the session back to the model it left.

## sourcefed events

To let Chauffeur filter your sourcefed monitors, and follow through on them
(for example: PR merged, so remind the agent to transition the Jira ticket),
start sourcefed's daemon with:

```sh
SOURCEFED_GATE_URL=http://127.0.0.1:18790/integrations/sourcefed
```

sourcefed then asks Chauffeur before delivering each event, and only events the
agent needs to act on reach the session; bot comments, approvals, and status
churn are withheld. If Chauffeur is down, sourcefed delivers as before.
sourcefed's messages in the session are not treated as yours.

## Configuration

| Variable | Default |
|---|---|
| `TYPESAFE_API_KEY` | required |
| `CHAUFFEUR_CONFIG_DIR` | `~/.config/chauffeur` |
| `CHAUFFEUR_SKILLS_DIR` | `~/.config/chauffeur/skills` (a link to `skills/`) |
| `CHAUFFEUR_STATE_DIR` | `~/.local/state/chauffeur` |
| `CHAUFFEUR_DAEMON_URL` | `http://127.0.0.1:18790` |
| `CHAUFFEUR_DAEMON_TOKEN` | unset |
| `CHAUFFEUR_BIN` | `chauffeur` for the OpenCode adapter |
| `CHAUFFEUR_IDLE_STEERING` | `false`; set to `true` on the daemon to enable idle reminders |

## Development checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cd adapters/opencode && npm run typecheck
```
