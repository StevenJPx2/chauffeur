import { expect, test } from "bun:test"
import { Effect } from "effect"
import { ASK_TOOL, installAskTool, nothingFound } from "../src/ask-tool.js"
import { type DaemonClient, DaemonError } from "../src/daemon.js"
import type { ExposureControl } from "../src/exposure.js"
import type { HostEffect, Signal } from "../src/protocol.js"
import { hostLoadsSkills } from "../src/skills.js"
import { SHIPPED_TEXTS } from "../src/texts.js"
import { fakeExposure, fakeHost, install, noRulebooks, noSkills, shippedTexts } from "./support.js"

const sessionID = "ses_ask"

test("the host keeps skill loading when skill exposure is off or kept on request", () => {
  expect(hostLoadsSkills({})).toBe(false)
  expect(hostLoadsSkills({ CHAUFFEUR_DISABLE: "rules, skill-exposure" })).toBe(true)
  expect(hostLoadsSkills({ CHAUFFEUR_DISABLE: "skill-exposures" })).toBe(false)
  expect(hostLoadsSkills({ CHAUFFEUR_HOST_SKILLS: "keep" })).toBe(true)
})

test("an empty answer points at the skill tool only while the host keeps it", () => {
  expect(nothingFound(SHIPPED_TEXTS, true)).toContain("load it with the skill tool")
  expect(nothingFound(SHIPPED_TEXTS, false)).not.toContain("skill tool")
})

/** The host has decoded the call's input by the time `execute` runs. */
type Need = { readonly need: string }

type Registered = {
  readonly name: string
  readonly options?: { readonly codemode?: boolean }
  readonly execute: (input: Need, context: { readonly sessionID: string; readonly agent: string }) => Effect.Effect<{ readonly content: string; readonly metadata?: { readonly "chauffeur.skills"?: ReadonlyArray<string> } }>
}

const CONTEXT = [
  { type: "user", text: "Check the landing page renders", metadata: {} },
  { type: "user", text: "CI failed", metadata: { sourcefed: { kind: "ci" } } },
]

/** Install the tool and return its registered definition. */
async function registered(daemon: DaemonClient, exposure: ExposureControl, context: ReadonlyArray<unknown> = CONTEXT, skills: ReadonlyArray<{ id: string; content: string; path?: string }> = [{ id: "jira-cli", content: "Use jira issue create.", path: "/skills/jira-cli/SKILL.md" }]) {
  const tools: Registered[] = []

  const host = fakeHost({
    tool: {
      transform: (callback: (editor: { add: (tool: Registered) => void }) => void) => Effect.sync(() => {
        callback({ add: (tool) => { tools.push(tool) } })

        return { dispose: Effect.void }
      }),
    },
    session: {
      context: () => Effect.succeed(context),
    },
    skill: { list: () => Effect.succeed({ data: skills }) },
  })

  const plugin = await install(installAskTool(exposure), host, daemon)

  const [tool] = tools

  if (tool === undefined) throw new Error("ask_chauffeur was not registered")

  const call = (need: Need) => Effect.runPromise(tool.execute(need, { sessionID, agent: "build" }))
  const ask = (need: Need) => call(need).then((result) => result.content)

  return { tool, ask, call, close: plugin.close }
}

function replying(sent: Signal[], effects: ReadonlyArray<HostEffect>): DaemonClient {
  return { skills: noSkills, rulebooks: noRulebooks, texts: shippedTexts, signal: (value) => Effect.sync(() => {
    sent.push(value)

    return effects
  }) }
}

test("a request carries the need, the user's words, hidden tools, and Code Mode, and replies with what was granted", async () => {
  const sent: Signal[] = []
  const revealed: Array<ReadonlyArray<string>> = []

  const exposure = fakeExposure({
    candidates: () => Effect.succeed([{ id: "browser_open", description: "Open a tab", bytes: 0 }]),
    reveal: (_, names) => Effect.sync(() => {
      revealed.push(names)

      return true
    }),
    codeMode: (text) => Effect.succeed([{ name: "safari", size: 3, tools: [{ id: text.slice(0, 5), description: "", bytes: 0 }] }]),
  })

  const { tool, ask, close } = await registered(replying(sent, [
    { type: "tools", agent_id: sessionID, hide: [], reveal: ["browser_open"] },
    { type: "context", agent_id: sessionID, delivery: "steer", label: "skill jira-cli", skills: ["jira-cli"], text: null },
    { type: "context", agent_id: "ses_other", delivery: "steer", label: "x", skills: [], text: "not for this session" },
  ]), exposure)

  const reply = await ask({ need: "drive a browser" })

  expect(tool).toMatchObject({ name: ASK_TOOL, options: { codemode: false } })
  expect(sent[0]?.kind).toEqual({
    type: "agent_request",
    need: "drive a browser",
    user_request: "Check the landing page renders",
    tools: [{ id: "browser_open", description: "Open a tab", bytes: 0 }],
    code_mode: [{ name: "safari", size: 3, tools: [{ id: "drive", description: "", bytes: 0 }] }],
  })
  expect(revealed).toEqual([["browser_open"]])
  expect(reply).toContain("revealed these tools; they are available from your next step: browser_open")
  expect(reply).toContain("Use jira issue create.")
  // Without it the agent cannot find the skill's scripts or references.
  expect(reply).toContain("Base directory for this skill: /skills/jira-cli\n")
  expect(reply).not.toContain("not for this session")

  await close()
})

test("a skill already in the context is not sent again, and the reply records the skills it carries", async () => {
  // Before the compaction, jira-cli was attached: the current context lacks it.
  const context = [
    { type: "user", text: "Earlier", skills: [{ id: "jira-cli" }], metadata: {} },
    { type: "compaction", metadata: {} },
    { type: "user", text: "Open a PR", skills: [{ id: "deploy" }], metadata: {} },
    { type: "synthetic", text: "Hand-over", metadata: { "chauffeur.skills": ["browser"] } },
    { type: "skill", skill: "gh", name: "gh", text: "Use gh pr view.", metadata: {} },
    { type: "assistant", content: [
      { type: "text", text: "Asking" },
      { type: "tool", name: ASK_TOOL, state: { status: "completed", input: {}, content: [{ type: "text", text: "…" }], metadata: { "chauffeur.skills": ["linear"] } } },
      { type: "tool", name: "read", state: { status: "running", input: {} } },
    ] },
  ]

  const skills = ["deploy", "browser", "gh", "linear", "jira-cli"].map((id) => ({ id, content: `Use ${id} well.` }))

  const { call, close } = await registered(replying([], [
    { type: "context", agent_id: sessionID, delivery: "steer", label: "present", skills: ["deploy", "browser", "gh", "linear"], text: null },
    { type: "context", agent_id: sessionID, delivery: "steer", label: "skill jira-cli", skills: ["jira-cli"], text: null },
    { type: "context", agent_id: sessionID, delivery: "steer", label: "again", skills: ["jira-cli"], text: null },
  ]), fakeExposure(), context, skills)

  const result = await call({ need: "file an issue" })

  expect(result.content.match(/<skill_content name="([^"]+)"/g)).toEqual(['<skill_content name="jira-cli"'])
  expect(result.metadata).toEqual({ "chauffeur.skills": ["jira-cli"] })

  await close()
})

test("the closest matches are named only when nothing is handed over", async () => {
  const suggest: HostEffect = { type: "suggest", agent_id: sessionID, names: ["the jira-cli skill", "the safari Code Mode tools"] }

  const nothing = await registered(replying([], [suggest]), fakeExposure())

  const something = await registered(replying([], [
    suggest,
    { type: "context", agent_id: sessionID, delivery: "steer", label: "skill jira-cli", skills: ["jira-cli"], text: null },
  ]), fakeExposure())

  const named = await nothing.ask({ need: "file an issue" })

  expect(named).toStartWith("Nothing Chauffeur has clearly fits. The closest were the jira-cli skill, the safari Code Mode tools;")
  expect(named).toContain("nothing restricts them")
  expect(await something.ask({ need: "file an issue" })).not.toContain("closest")
  await nothing.close()
  await something.close()
})

test("nothing granted, a failed reveal, or a daemon failure still replies", async () => {
  const nothing = await registered(replying([], []), fakeExposure())
  const unrevealed = await registered(replying([], [{ type: "tools", agent_id: sessionID, hide: [], reveal: ["browser_open"] }]), fakeExposure())
  const down = await registered({ skills: noSkills, rulebooks: noRulebooks, texts: shippedTexts, signal: () => Effect.fail(new DaemonError({ message: "daemon down" })) }, fakeExposure())

  expect(await nothing.ask({ need: "a time machine" })).toContain("nothing restricts them")
  expect(await unrevealed.ask({ need: "a browser" })).toContain("nothing restricts them")
  expect(await down.ask({ need: "a browser" })).toContain("could not answer")

  await Promise.all([nothing.close(), unrevealed.close(), down.close()])
})
