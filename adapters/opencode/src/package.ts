import { existsSync } from "node:fs"
import { createRequire } from "node:module"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import manifest from "../package.json" with { type: "json" }

/** This plugin's version; the daemon it starts carries the same one. */
export const PLUGIN_VERSION: string = manifest.version

/** Platforms a binary package ships for, as `<process.platform>-<process.arch>`. */
const PLATFORMS = new Set(["darwin-arm64", "darwin-x64", "linux-x64", "linux-arm64"])

const PLATFORM = `${process.platform}-${process.arch}`

/**
 * The daemon binary installed with this plugin for `platform`: the
 * `@fdcn/chauffeur-<platform>` optional dependency, resolved from `from`.
 * Absent outside a package install, such as a plugin copied into place.
 */
export function bundledBinary(from: string = import.meta.url, platform: string = PLATFORM): string | undefined {
  if (!PLATFORMS.has(platform)) return undefined

  try {
    const manifestPath = createRequire(from).resolve(`@fdcn/chauffeur-${platform}/package.json`)
    const binary = join(dirname(manifestPath), "bin", "chauffeur")

    return existsSync(binary) ? binary : undefined
  } catch {
    return undefined
  }
}

/** The shipped skills beside the bundle (`<package>/skills`), when present. */
export function packageSkills(from: string = import.meta.url): string | undefined {
  const folder = fileURLToPath(new URL("../skills", from))

  return existsSync(join(folder, "rulebooks")) ? folder : undefined
}

/** How to start the daemon: the binary, and an environment naming the shipped skills. */
export type Launch = { readonly bin: string; readonly env: NodeJS.ProcessEnv }

/**
 * `CHAUFFEUR_BIN`, else the bundled binary, else `chauffeur` on `PATH`; and
 * `CHAUFFEUR_SKILLS_DIR`, else the package's skills, else the daemon's own
 * default. The user's settings always win.
 */
export function daemonLaunch(env: NodeJS.ProcessEnv, from: string = import.meta.url): Launch {
  const skills = env.CHAUFFEUR_SKILLS_DIR ?? packageSkills(from)

  return {
    bin: env.CHAUFFEUR_BIN ?? bundledBinary(from) ?? "chauffeur",
    env: skills === undefined ? env : { ...env, CHAUFFEUR_SKILLS_DIR: skills },
  }
}
