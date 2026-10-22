import { expect, test } from "bun:test"
import { fileURLToPath } from "node:url"

test("permission and background RPCs cannot be starved by the host's fetch streams", async () => {
  // Bun reads this limit at startup; isolate it from the rest of the test runner.
  const child = Bun.spawn([process.execPath, fileURLToPath(new URL("./fixtures/rpc-starvation.ts", import.meta.url))], {
    env: { ...process.env, BUN_CONFIG_MAX_HTTP_REQUESTS: "4" },
    stdout: "pipe",
    stderr: "pipe",
  })

  try {
    const [code, stdout, stderr] = await Promise.all([
      child.exited,
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
    ])

    expect({ code, stderr }).toEqual({ code: 0, stderr: "" })
    expect(stdout).toContain("permission and background RPCs succeeded with the fetch pool saturated")
  } finally {
    child.kill()
  }
}, 10_000)
