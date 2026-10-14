import { expect, test } from "bun:test"
import { execFileSync } from "node:child_process"
import { fileURLToPath } from "node:url"

test("the installed bundle uses the host's Effect runtime and plugin SDK", async () => {
  execFileSync("npm", ["run", "build"], {
    cwd: fileURLToPath(new URL("..", import.meta.url)),
    stdio: "pipe",
  })

  const bundle = await Bun.file(new URL("../dist/index.mjs", import.meta.url)).text()

  expect(bundle).toMatch(/from ["']effect["']/)
  expect(bundle).toMatch(/from ["']@opencode\/plugin\/effect["']/)
}, 20_000)
