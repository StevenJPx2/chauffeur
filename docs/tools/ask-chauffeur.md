# `ask_chauffeur`

The agent asks, in plain words, for a tool or skill it lacks. Chauffeur
answers with the best match: a hidden tool group revealed, a Code Mode
namespace with its matching tools, or a skill's body. How it decides is on
the [`ask-chauffeur` plugin's page](../plugins/ask-chauffeur.md).

| Input  | Type   |                                                                   |
| ------ | ------ | ----------------------------------------------------------------- |
| `need` | string | What the agent has to do, in plain words, not the tool it guesses |

## A hidden tool group

The agent was told to publish a page, but the artifact tools were hidden at
the start of the conversation:

```json
{ "need": "publish this report as a shareable page" }
```

```text
Chauffeur revealed the artifact tools, available from your next step:
- artifact_publish: Publish an artifact from a Markdown string or file
- artifact_edit: Edit an existing artifact while keeping its URL
- artifact_delete: Delete an existing artifact
```

## A Code Mode namespace

```json
{ "need": "list my Cloudflare Workers" }
```

```text
Code Mode tools that fit this, under cloudflare:
- tools.cloudflare.workers_list: List the account's Workers
- tools.cloudflare.workers_get: Get a Worker's settings
Find exact paths with search({ namespace: "cloudflare", query: "…" }) inside execute.
```

## A skill

```json
{ "need": "post a reply in a Slack thread" }
```

```text
Chauffeur attached the slack-cli skill. Its base directory is
~/.config/opencode/skills/slack-cli.

# slack-cli
Use slackcli to read and post Slack messages …
```

A skill already in the conversation is not sent again; the reply says so.

## Nothing fits

```json
{ "need": "book a meeting room" }
```

```text
Nothing Chauffeur has clearly does this. Closest: the google-workspace tools,
the calendar skill. Carry on with your own tools, or ask again naming one.
```

## What to put in `need`

| Instead of           | Write                                                      |
| -------------------- | ---------------------------------------------------------- |
| `"browser tools"`    | `"read the comments on this Jira issue"`                   |
| `"slack"`            | `"post a reply in the #releases thread"`                   |
| `"a skill for this"` | `"bump the version and write the changelog for a release"` |

The need is judged against what each tool and skill is for, so a task finds
a dedicated tool over one that only shares a topic with it.
