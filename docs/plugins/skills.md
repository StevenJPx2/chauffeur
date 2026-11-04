# `skills`

Attaches the skills a message needs, and hands over a better skill when the
agent drifts to a worse way of doing the work. It judges the skills the
harness already knows and screens third-party ones before offering them.
Slice 6.

| Hook                                                             | Does                                                                                                                                      |
| ---------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| Start                                                            | Removes the harness's `skill` tool and its list from the prompt, through the tool registry; attached skills and `@skill-id` still resolve |
| `ctx.before("prompt.sent")`, budget 400 ms, `onTimeout: "allow"` | Ranks the catalog, asks one yes/no per candidate, attaches the best as `skill` parts                                                      |
| `ctx.on("tool.after")`, at most once a minute                    | One `choice`: a skill that does the current work better, delivered `now`                                                                  |
| `ctx.on("response.finished")`                                    | The same `choice`, delivered `next`                                                                                                       |

```ts
import { Plugin } from "@fdcn/chauffeur/plugin"
import { judge, noul, strategy } from "@fdcn/chauffeur/judge"
import { skill } from "@fdcn/chauffeur/harness"

const quick = judge({ tier: "system-one", confidence: 0.7 })
const pick = strategy([
  {
    id: "needs",
    judge: quick,
    forEach: "candidates",
    ask: noul("Does this request need this skill?")
  },
  {
    id: "best",
    judge: "facts",
    after: ["needs[*]"],
    ask: { select: { from: "needs[*]", top: 4, min: 0.7, by: "yes" } }
  }
])

export default Plugin.define({
  id: "skills",
  setup(ctx) {
    ctx.before(
      "prompt.sent",
      async (e) => {
        // The harness's catalog, cut to the 16 closest by the bm25 fact,
        // plus the pinned, named and project skills.
        const candidates = await closest(
          await skill.list(e.session),
          e.text,
          ctx.options.ranking
        )

        const best = await pick.run({ evidence: { message: e.text }, candidates })
        if (best.ok) e.append(best.outcome.map((id) => ({ skill: id })))
      },
      { budget: 400, onTimeout: "allow" }
    )
  }
})
```

## Where skills come from

The candidates are the harness's catalog: each skill's ID, description,
base directory and source. A skill the user adds to the harness is a
candidate at the next message, with nothing to configure in Chauffeur:

```jsonc
// opencode.jsonc
{
  "skills": ["~/shared/opencode-skills", "https://example.com/opencode/skills/"]
}
```

- **Project skills** come from the session's project folders, such as
  `.opencode/skills/`. They are asked "does this request involve this
  project's work?" and attach unless the judge is confident they do not.
- **Skills the message names**, such as `slack` for `slack-cli`, are asked
  whether the agent needs them.
- **Pinned skills** in `ranking.always` are always asked about.
- **Third-party skills**, from an HTTP catalog or a folder outside the user's
  own, are screened once per version: System One first, and System Two for
  what it leaves unsure from slice 11. Until then an unsure screen withholds
  the skill.
- **A Git repository** of skills is cloned and listed as a local folder.
- **A harness without skill sources** offers what its own folders hold, such
  as `~/.claude/skills/` in Claude Code.

At most four skills and 64 KiB attach per message, likeliest first.

## Commands

```sh
# Every skill the harness offers, its source and screen.
chauffeur skills list

# Its description, base directory, source and screen.
chauffeur skills show jira-cli

# Offer a third-party skill its screen withheld.
chauffeur skills approve some-skill
```

```text
$ chauffeur skills list
ID             SOURCE                                   SCREEN
jira-cli       ~/.config/opencode/skills                own
review-pr      .opencode/skills                         project
deploy-docs    https://example.com/opencode/skills/ v3  passed
yolo-shell     https://example.com/opencode/skills/ v1  withheld: runs remote scripts
```

`approve` records the skill and its version in the plugin's storage; a new
version is screened again.

## Options

```jsonc
{
  "skills": {
    "options": {
      "attach": { "confidence": 0.7, "top": 4, "bytes": 65536 },
      "project": { "confidence": { "yes": 0.3, "no": 0.7 } },
      "ranking": { "fact": "bm25", "judged": 16, "always": [] },
      "drift": {
        "confidence": 0.7,
        "cooldown_seconds": 60,
        "browser_only": ["browser-harness"]
      }
    }
  }
}
```

`ranking.fact` is `bm25` or `embedding`. A project skill's `no` needs 0.7, so
`unsure` attaches it. Which skills passed their screen, per version, is kept
in the plugin's storage; [its commands](#commands) list and approve them.
