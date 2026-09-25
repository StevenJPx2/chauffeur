# Chauffeur architecture

Chauffeur autonomously and progressively enhances a coding agent's context. It
observes the agent's event stream, classifies the evolving situation with a
System One model, and routes that judgment to decoupled capabilities that shape
what the agent sees, may do, and runs on — without invalidating the model's
prompt cache.

The pitch is one line: **detect the situation, classify it, enhance the
context.** Permission gates, skill contracts, model routing, and integrations
are capabilities or plugins that hang off that core.

## Status

| Part | State |
|---|---|
| Sense → Classify → Act engine, System One interface, Jev provider (HTTPS via rustls), secret redaction | built |
| Model router capability, Anthropic and OpenAI provider plugins, OpenCode signal/effect adapter | built |
| Skill exposure, tool exposure, permission (skill contract), and rules (tool-result steers, idle reminders, project rules) capabilities | built |
| Irreversible-harm backstop | built |
| Integration events and the sourcefed event gate, model switch-back, tool-group reveal, rules, persistence across restarts | built |
| Ephemeral enhancements | not yet built |
| Code Mode tool surfacing, skill-list removal, audit log, child-session inheritance | built |


## Three planes

```text
host events: user prompt · agent response · tool call/result · idle · model error · integrations
        │  adapter sends Signals
   ┌────▼──────┐  SENSE      bounded rolling Situation per agent (32 entries)
   │ Situation │
   └────┬──────┘
        │  each capability plans: skip · settled effects · a question
    ┌────▼──────┐  CLASSIFY   one System One request per ready round of capability questions
   │  Router   │             deterministic facts are features in the question, never a separate router
   └────┬──────┘
        │  typed answers
   ┌────▼──────┐  ACT        each capability turns its answer, or the failure posture, into effects
   │Capabilities│
   └────┬──────┘
        │  adapter applies effects at host seams
        ▼
   agent context (cached prefix intact)
```

The engine is synchronous, owns no I/O, and runs on a dedicated daemon thread
behind a bounded queue. Capabilities know nothing about routing; the engine
knows nothing about any capability.

## Routing

Every capability is reached through System One. Facts such as a 429, a
secret-pattern hit, or idle duration become features of the question. A
capability may ask several questions (one per skill, say). The engine
namespaces them as `capability/question` and batches every ready capability's
questions into one request per round. Most capabilities finish in one Jev
call. A dependent tool-recovery judgment adds a second call only when its
first answer selects a candidate; the bounded tool-result hook then waits up
to eight seconds. Other effects from that signal wait for the final round.

A capability may settle without a question only on facts that make a judgment
moot: an empty candidate set, or a veto such as "a tool already ran in the
failed step, so retrying could repeat it." The **safety backstop** is the only
other deterministic gate: a last-resort veto on irreversible harm. The engine
checks every permission request's action, resources, and stated reason against
shipped patterns in `skills/safety/backstop.json` (compiled into the binary)
and optional `$CHAUFFEUR_CONFIG_DIR/backstop.json` patterns. The config adds by
default or replaces shipped patterns with `"replace": true`. Shell permission
requests also ask Jev about irreversible harm; a confident yes denies the call
and adds its command to `$CHAUFFEUR_STATE_DIR/learned-backstop.json` immediately.
A match denies before any capability or model call, whatever a contract would
decide. Patterns that name a root match only the root itself, so `rm -rf
/tmp/build` is not vetoed. The backstop vetoes; it never routes.

The same file's `confirm` patterns hand a request to the user: a match asks,
before any model call, whether the host would allow it or a contract approve
it. Each must end at a word boundary. The shipped ones cover force-pushes to
`main` or `master` (`--force`, `-f`, `--force-with-lease`, `+main`), which
rewrite history others have pulled; force-pushing a rebased feature branch
matches none.

## System One

`SystemOne` is the single inference interface: typed questions (choice, score,
noul) in one call, typed answers out, validated against the questions before
any capability sees them.

- **Jev** (TypeSafe AI, hosted) is the only provider, and the daemon requires
  `TYPESAFE_API_KEY` to start. When Jev fails, each capability applies its
  failure posture: nothing is hidden or attached, permission falls to ask, and
  the host keeps its retry decision. There is no local fallback: on 58
  labelled Chauffeur questions Jev scored 98% with no wrong actions at
  P ≥ 0.7, while the local Laya model scored 83% with 3, though ~5× faster.
  Standing aside is safer than acting on a weaker judgment.
- **Confidence is optional.** Jev omits it for noul; the engine then uses the
  margin `|2p − 1|`.
- **Egress:** shipped token prefixes in `skills/safety/redaction.json` (compiled
  in) and private-key blocks are redacted locally. Optional
  `$CHAUFFEUR_CONFIG_DIR/redaction.json` prefixes add or replace (`"replace":
  true`) the shipped list. Unknown credential-like strings are masked locally
  before state and question text reach Jev; Jev judges their shapes, never
  their values. Secret and context-bound safe shapes are saved immediately in
  `$CHAUFFEUR_STATE_DIR/learned-redaction.json`. Audit fields use the same
  redactor. Both learned files are separate from config so they can be reviewed.
- **No training on Jev output.** TypeSafe's customer agreement (§2.3(b))
  forbids distillation from Jev. Distilled students, when built, train on
  human labels only.

## Effects

Effects are the host's actions, not capabilities' concepts. Core defines five,
and no effect names the capability that asked, so a new capability needs no
new effect and no adapter change:

| Effect | What the host does | Used by |
|---|---|---|
| `permission` | answers the pending permission request: allow, deny, or ask with a message | permission contracts, the backstop |
| `model` | switches the model (with its thinking variant) and retries, or keeps it and applies its own retry policy | model router |
| `tools` | hides or shows named tools in this context | tool exposure |
| `context` | adds skills (the host resolves their bodies) and text to the conversation, at a `delivery`: `prompt` (with the user message being admitted), `steer` (the running turn), `resume` (wakes an idle agent), or `wait` (for the next turn) | skill exposure, tool exposure (Code Mode notes), rules |
| `gate` | delivers or withholds the integration event being gated | event gate |

Every `context` delivery lands in history at the tail, so the cached prefix
stays intact. Ephemeral context, appended to one request in the `context` hook,
is not built: it needs `@opencode/ai`'s `Message` class, a direct dependency not
yet added.

A subagent's first prompt continues its parent's context rather than starting
one: the adapter attaches the parent's skills to it and keeps the parent's
hidden tools hidden, so the engine judges only what the subagent adds.

What a context has already received is rebuilt from its own history: attached
skills are read from user messages and from synthetic messages carrying
`chauffeur.skills` since the last compaction, and
the hidden tools from the latest `chauffeur.hidden` metadata. A skill larger
than 16 KiB is never attached. Everything else the engine remembers per agent
is persisted by the daemon (see Failure and bounds).

## Cache discipline

Prompt caching keys on a stable prefix per (provider, model). Tool definitions
come first in that prefix.

- **Tool set.** Tools are judged by **group**, the ID prefix before the first
  `_` (`artifact` for `artifact_publish`). At a context's first user message,
  before any cache exists, the tool-exposure capability keeps a base set,
  OpenCode's built-in tools (read, edit, patch, write, shell, grep, glob,
  question, subagent, webfetch, websearch; override in `tool-exposure.json` as
  `{"base": [...]}`), and asks one question per other group. A group is hidden
  only when System One is confident it is not needed (P ≤ 0.3, confidence
  ≥ 0.4); the `skill` tool is always hidden. Effects name tools, so a tool the
  engine never judged is never removed. The adapter records the hidden list on
  the message's metadata and removes exactly those tools in the `context` hook
  on every request. If classification fails, nothing is hidden.
- **Reveal.** On later user messages the host sends the currently hidden tools,
  and one question per hidden group asks whether the latest request needs it
  now (P ≥ 0.7, confidence ≥ 0.4). Revealing changes the tool list at the front
  of the cached prefix, so the provider re-reads the conversation once; the
  question says so, and the adapter records the smaller hidden list. After a
  tool result reporting a missing tool, a bounded catalog of registered,
  hidden direct tools can enter a two-step pipe: Jev selects a plausible
  candidate, then confirms a confident need to reveal it. The adapter saves
  the smaller hidden list in plugin storage and queues a synthetic session
  marker before the next model request.
- **Code Mode.** Only tools the host puts in the request's tool record can be
  hidden. The adapter learns that set from the `context` hook, before anything
  is removed, and offers System One only those tools; before the first request
  it leaves out tools flagged `codemode`. With Code Mode on, plugin and MCP
  tools reach the model through `execute`, whose catalog shows each namespace
  only in part (browser: 4 of 45) and is updated by appended messages.
  Chauffeur never edits the system prompt, so it does not hide Code Mode tools;
  it **surfaces** them. On each user message the adapter sends every Code Mode
  namespace with its size and its best matches for the request (up to 8, by
  word overlap, as Code Mode's own search would find them). Tool exposure asks
  one question per namespace not yet surfaced in the context (P ≥ 0.7,
  confidence ≥ 0.4) and, for the chosen ones, writes a note listing those
  matches with the `search({ namespace })` call for exact paths. It arrives as
  a `context` effect with `prompt` delivery, a text attachment that reaches
  the first step and adds nothing to the system prompt. Tool exposure
  remembers what it surfaced per agent, resets at a new context, and persists
  it with the rest of its state.
- **Skill list.** At startup the adapter adds a `skill` deny rule to every
  agent through `agent.transform`. OpenCode then leaves both the `skill` tool
  and its skill list (about 4k tokens with 43 skills) out of every request,
  while skills Chauffeur attaches to prompts still resolve. The rule lives only
  in the plugin, so disabling Chauffeur restores both; OpenCode's config files
  are untouched.
- **Skills.** Chauffeur owns skill loading. On each user message the
  skill-exposure capability asks one choice: the single skill not yet attached
  (up to 64) that would most help, or none. A choice at confidence ≥ 0.4 is
  attached to that prompt, where it resolves like a user-selected skill and
  enters history at the tail.
- **Drift.** The agent can lose track, for example driving a browser for X,
  Slack, GA4, or Jira where a dedicated skill exists. After a tool result (at
  most once a minute per agent) and at every turn end, one choice asks whether
  a remaining skill gives a dedicated, better way to do the current work. A
  chosen skill is delivered as a synthetic message carrying its body: `steer`
  mid-turn, and `resume: false` at turn end so a finished task is not redone.
- **Reveals are monotonic** for the session. At compaction, which rebuilds the
  prefix anyway, session-start classification runs again and only still-relevant
  skills are preloaded.
- **Model switches are cache-cold** on the target provider. The model router
  weighs that cost; each provider keeps its own warm prefix.

## Capability catalog

| Capability | Question | Triggered by | Effect | Status |
|---|---|---|---|---|
| Model router | switch, and to which same-tier model, or stay? has the limit on the model left behind cleared? | model usage-limit error; user message after a switch | decision | built |
| Permission / skill contract | each matching contract's typed question | permission request | decision | built |
| Skill exposure | which one skill helps most, or none? is the agent using a generic approach where a skill fits? | user message; tool result and turn end | persistent | built |
| Tool exposure | will the task need this tool group? does the latest request need a hidden group now? | first user message of a context; later user messages | tool set | built |
| Rules | each admitted rule's step, one or two rounds | tool result for a watched tool; turn end | persistent (steer, resume, or wait; skill hand-over) | built |
| Event gate | does this integration event need the agent to act now? | integration event | decision | built |

Classification passes run on user message, tool result, turn end, permission
request, model error, and integration event.

## Skills

Contracts and Chauffeur-owned skills live apart from the code, in the
top-level `skills/` folder, loaded from `CHAUFFEUR_SKILLS_DIR` (default
`~/.config/chauffeur/skills`, usually a symlink to that folder):
`permission/` holds strict JSON permission contracts, `rules/` strict JSON
rules, and `handoff/` skills that rules hand over (`slack-cli`, `jira-cli`, `twitter-cli`), linked into a
directory OpenCode reads. A missing directory means no contracts of that
kind.

### Permission

Contracts in `skills/permission/` (strict JSON, validated at daemon start;
`chauffeur skill validate PATH` checks one) are the permission capability's
configuration. The adapter reports every request the host does not deny,
with the host's own decision. Contracts judge only requests the host would
**ask** about; one the host allows reaches only the backstop and the
irreversible-harm question. For each ask, every contract matching the
`permission.evaluate` event and the requested action:

1. checks its required and forbidden evidence, computed in core from the
   request's facts: `resource_present`, `resource_within_workspace` (lexical
   containment over host-resolved absolute paths), and `resource_named_by_user`
   (path or file name appears in a recent user message; sourcefed's messages
   are not the user's);
2. applies its cooldown per agent, if it has one;
3. otherwise contributes its typed question to the shared System One call.

Missing evidence and cooldown select fixed outcomes without a model call; a
failed or low-confidence judgment selects `judge_failure` or `uncertain`,
which cannot allow. The most restrictive outcome across contracts answers the
host (deny, then ask, then allow; prompt and remind ask with their reminder). A
host denial always stands; the adapter asks when the engine fails or misses
the 600 ms budget. No matching contract leaves the host's decision.

Three contracts ship, none with a cooldown. Each asks Jev whether the request
the host would ask about clearly serves the user's stated task, and approves
it only on a **confident yes** (P ≥ 0.7, confidence ≥ 0.4). A no, an unsure
answer, or a failed judgment leaves the ask to the user:

- `workspace-edit-gate`: edits, inside or outside the workspace. Credentials,
  secrets, and configuration the user did not ask to change are a no.
- `workspace-shell-gate`: shell commands. Unrelated, destructive,
  secret-exposing, and workaround commands are a no (tool results carry their
  error text, so a host denial is visible, which closes writing a denied file
  through `echo > file`).
- `external-directory-gate`: directories outside the project, such as a
  sibling repository the task depends on. Credential stores (`~/.ssh`,
  `~/.aws`), system directories, and unrelated directories are a no.

### Rules

One capability, `capabilities/rules`, and one strict JSON format
(`schema_version: 2`) cover every "facts admit it, Jev confirms it, deliver
context" behavior: tool-misuse steers, idle reminders, and a project's own
follow-through. A rule declares:

- `on`: `tool_result` (after each call; the question sees the call, and a call
  the user explicitly asked for, exactly as asked, does not count) or
  `turn_end` (the question sees the latest user request, so an explicit
  instruction such as "do not create a PR" can end the rule).
- `when`, exact facts checked before any model call. For a `tool_result`
  rule, `tools` names the calls it watches: other tools are never judged.
  `tools_called`, `tools_called_any`, and `tools_not_called` read the agent's
  history, as far back as `history` reaches: `turn` (the default; since the
  latest user message, in the current workspace) or `session`. `status`
  (`implementing`, or `in_review` once a PR was opened or any GitHub event
  arrived), `source` (`jira` or `github`, from tools and events), and `hooks`
  (integration events as `source:kind`, such as `github:merged`) read the
  session.
- `steps`: one or two yes/no judgments, each with `yes_at_or_above` (0.5-1)
  and `minimum_confidence`. The second is asked in a later round only after
  the first holds.
- `then`: the context delivered, `steer` for a tool result or `resume` or
  `wait` at a turn end, with its `text`, an optional `label`, and an optional
  `skill` to hand over.
- `priority`, `once` (per agent, renewed at the next user message for a
  `turn` rule, never for a `session` rule), and `cooldown_seconds`.

A signal delivers at most two rules, highest priority first, chosen after
every round. A failed judgment confirms nothing new; rules an earlier round
confirmed still deliver.

Rules come from two places. `skills/rules/` ships nine: seven steers
(hand-rolled patch scripts, `grep -r`, `cat` to read files, and X, Slack,
Jira, or GitHub through a browser, handing over `twitter-cli`, `slack-cli`,
and `jira-cli`; on 13 labelled calls they made no false nudges and one near
miss) and two session reminders, `git-conflict-loop` and
`jira-transition-after-merge`. sourcefed already delivers CI failures and
review requests, so reminders add follow-through only. Shipped `turn_end`
rules run only when the daemon has `CHAUFFEUR_IDLE_STEERING=true`. Code Mode
tools are called through `execute`, so browser use there is caught by
watching `execute`.

A project keeps its own in `.chauffeur/rules/*.json`, read at each tool result
and turn end from the Git worktree root down to the session's directory; a
sibling repository's rules never apply, and a symlinked folder or file is
rejected. A project rule cannot reuse a shipped rule's ID. The HPDP Overlay
worktrees keep verification and PR follow-through rules there: the first PR
judgment checks the latest user instruction; the second, whether the work is
verified and ready. `chauffeur skill validate PATH` checks one rule, and
`chauffeur skill validate-project WORKTREE` the rules a session there would
load.

### Integration events and the event gate

Chauffeur decides which sourcefed events reach the agent, with no
configuration. The adapter registers an OpenCode plugin RPC, `chauffeur.gate`
(`ChauffeurRpc`, plain JSON Schema). sourcefed's OpenCode plugin, which
delivers each event into a session in the same OpenCode process, calls it
first with the session, source, kind, summary, body, and actionable flag. The
adapter sends an `integration_event` signal for that session to the daemon,
recorded in the Situation as `github merged event: PR #42 merged`, and the
event-gate capability asks whether the agent needs to act on it now. Only a
confident no (P ≤ 0.3, confidence ≥ 0.4) withholds the event; everything else,
including a failed judgment, delivers. sourcefed delivers too when Chauffeur's
plugin is not loaded, fails, or is slower than 3 s. Other integrations can
POST sourcefed's event shape `{target, monitorID, source, event}` to the
daemon's `/integrations/sourcefed` and receive `{"deliver": bool}`. On 12 labelled events Jev withheld every piece of noise (bot
comments, "LGTM", status and assignee churn, thanks) and showed every event
that needed action.

sourcefed tags what it delivers with `metadata.sourcefed`, and the adapter
never reports those messages as the user's.

### Model router

On a usage-limit error the router acts. A limit is HTTP 402, 429, 503, or 529;
a limit error type (OpenCode's `provider.quota`, `rate_limit`, `overloaded`,
and similar); or wording in the message ("credit balance", "insufficient
funds", "quota", "rate limit", "billing", "high demand", "capacity"), which
catches providers that report an empty balance as an invalid request. A model
the router switched to that cannot serve the agent (401, 403, 404, or an auth,
permission, or not-found error) is a failed switch: the router moves on to the
next candidate. The same error on a model the user chose is left to the host.

**Tiers depend on thinking.** A model reference carries the host's thinking
variant (`anthropic/claude-opus-5-5#high`), and provider plugins' tier tables
map a model, at one variant or at any, to a tier:

| Tier | Anthropic | OpenAI |
|---|---|---|
| Frontier | `claude-opus-5-5#high` | `gpt-6-sol` |
| Balanced | `claude-opus-5-5#low` | `gpt-6-luna#max` |
| Fast | `claude-sonnet-4-6` | `gpt-6-luna` (other variants) |

The router:

1. declines to switch if a tool already ran in the failed step;
2. computes candidates: each usable host model at every variant its table
   names, untried, in the current model's tier (any tier when the current
   model is unknown), plus every pinned model regardless of tier, ordered by
   pins, then other providers, then host order, at most 8. Another variant of
   the current model is never a candidate, because a usage limit applies to
   the whole model;
3. asks one choice question over the candidates plus `stay`, stating the error
   and that an exhausted quota or balance does not clear by waiting;
4. switches to the chosen model, or keeps the current one. On System One
   failure or confidence below 0.2 it switches to the first candidate, keeping
   the agent unblocked.

A completed model step clears the session's tried set; an HTTP 200 alone does
not. A cancelled execution invalidates pending model effects, and an
abandoned daemon reply restores the router's prior state. A switch sets the
model's thinking variant too. `$CHAUFFEUR_CONFIG_DIR/model-router.json` pins
preferred fallbacks, with a variant where it matters:
`{"pins": ["openai/gpt-6-sol", "anthropic/claude-opus-5-5#low"]}`.

**Switch back.** The router remembers the model the agent left (the first in a
chain of switches). On a user message at least 5 minutes later, while the
agent is still on the model the router chose, one question asks whether that
original limit has most likely cleared, as a fact about the limit rather than
a trade-off against the cache: Jev judges the fact far better. At P ≥ 0.7 and
confidence ≥ 0.4 the adapter switches back before the prompt runs. A model the
user picked themselves forgets the origin.

## Coupling

The adapter sends **Signals** and applies **Effects**; it holds no policy.

- Transport: HTTP on `127.0.0.1:18790`: `POST /rpc` for hosts, where `signal`
  returns the effects that signal produced and the adapter applies them at
  once; `POST /integrations/sourcefed` for sourcefed.
- Latency: permission decisions answer within 600 ms or fall back (ask).
  Skill and tool exposure run during prompt admission, so their content lands
  before the model replies; admission waits at most 6 s, then proceeds
  unenhanced. The `context` hook never calls a model.
- Hosts: designed for many, OpenCode first.

| Method | Purpose |
|---|---|
| `signal` | report one observation, receive its effects |
| `health` | readiness |

### OpenCode seams

- **Sense:** `session.hook("prompt")` (user messages, with `skill.list()` and
  `tool.list()` catalogs and `session.context` history), `session.hook("retry")`
  (model errors), `session.hook("model.request")` (per-request tool state),
  `tool.hook("execute.after")` (tool results), `event.subscribe`
  (`session.step.ended` reports model success;
  `session.execution.started` resets per-step state;
  `session.execution.succeeded` and `.failed` end a turn; OpenCode 2.0.15
  emits no `session.status` idle event). Permission resources are file paths
  for `read`, `edit`, `write`, and `patch`, and command text for `shell`.
- **`permission`:** `permission.hook("evaluate")`, answered within 600 ms.
- **`model`:** `session.switchModel` (with the variant) plus the retry
  decision; the session's model comes from `session.get`.
- **`tools`:** the hidden list is recorded on the user message's
  `chauffeur.hidden` metadata and deleted from `tools` in
  `session.hook("context")`.
- **`context`, `prompt` delivery:** skills go to `prompt.skills`, text to a
  `data:text/plain` attachment in `prompt.files`. The prompt waits up to 6 s
  for the engine, then proceeds unenhanced.
- **`context`, other deliveries:** one `session.synthetic` message with the
  text and each skill's body, recording the skills in `chauffeur.skills`
  metadata: `delivery: "steer"`, `resume: true`, or `resume: false`. The
  `execute.after` hook waits at most 3 s.
- **`gate`:** the `chauffeur.gate` plugin RPC's reply.
- **Skill loading:** a `skill` deny rule on every agent via `agent.transform`.

## Workspace

| Path | Role |
|---|---|
| `crates/core/src` | `lib.rs`, engine, RPC protocol, optional client, and config helpers at the root; `contracts/` holds the host/capability interfaces; `state/` holds rolling context and traces; `features/` holds backstop, redaction, and learning. No capability concepts |
| `crates/daemon` | engine thread, Jev wiring, capability composition, HTTP RPC, sourcefed surface |
| `skills` | permission contracts, shipped rules, the backstop, hand-over skills |
| `crates/cli`, `crates/mcp` | drive the daemon |
| `capabilities/model-router` | model-router capability and the provider tier-table contract |
| `capabilities/skill-exposure` | skill-exposure capability |
| `capabilities/tool-exposure` | tool-exposure capability |
| `capabilities/permission` | permission capability and the skill-contract format |
| `capabilities/rules` | rules capability, the rule format, and shipped and project rule loading |
| `capabilities/event-gate` | event-gate capability |
| `plugins/anthropic`, `plugins/openai` | provider tier tables |
| `judges/jev` | Jev System One provider |
| `adapters/opencode` | signals in, effects out; an Effect plugin whose hooks share the plugin scope |

## Failure and bounds

- Permission fails closed (ask); enhancements fail open (omitted); the model
  router fails toward unblocking (first candidate); with the daemon down, the
  host's own retry decision stands.
- Bounds: 256 agents per engine (oldest evicted), 32 Situation entries, 2 KiB
  per signal field, 400 B per catalog description, 128 skills and 128 tools per
  signal, 256 available models per signal, a 64-deep engine queue,
  3 s Jev timeout, 15 s engine reply timeout, 256 KiB Jev response cap, 128 KiB
  RPC body cap.
- **Persistence.** After every signal the daemon writes each capability's
  per-agent memory and the Situations to `$CHAUFFEUR_STATE_DIR/state.json`
  (default `$XDG_STATE_HOME/chauffeur` or `~/.local/state/chauffeur`),
  atomically, and loads it at start. A state file over 8 MiB, from another
  version, or that no longer parses is ignored, so the daemon starts fresh.
- **Audit log.** Every signal that asked System One, was vetoed, produced
  effects, or failed appends one JSONL record to
  `$CHAUFFEUR_STATE_DIR/audit.jsonl`: the signal's kind and a short summary,
  the namespaced questions, the answers or System One's error, the veto, the
  effects, and the time spent waiting. Text is secret-redacted and clipped; the
  file rotates once to `audit.1.jsonl` past 8 MiB. `chauffeur audit [N]` prints
  the last N decisions, one line each.

## Configuration

| Variable | Default |
|---|---|
| `TYPESAFE_API_KEY` | required; the daemon refuses to start without it |
| `TYPESAFE_BASE_URL` | `https://api.typesafe.ai` |
| `TYPESAFE_DEFAULT_MODEL` | `jev-latest` |
| `CHAUFFEUR_CONFIG_DIR` | `~/.config/chauffeur` |
| `CHAUFFEUR_SKILLS_DIR` | `$CHAUFFEUR_CONFIG_DIR/skills` (a link to `skills/`) |
| `CHAUFFEUR_STATE_DIR` | `$XDG_STATE_HOME/chauffeur`, else `~/.local/state/chauffeur` |
| `CHAUFFEUR_DAEMON_URL` | `http://127.0.0.1:18790` |
| `CHAUFFEUR_DAEMON_TOKEN` | unset |
| `CHAUFFEUR_IDLE_STEERING` | `false`; `true` enables idle reminders in the daemon |
| `CHAUFFEUR_BIN` | `chauffeur` |
