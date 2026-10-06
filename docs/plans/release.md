# Release

Chauffeur ships publicly as one OpenCode plugin package that carries its own
daemon. A user with OpenCode 2 and a TypeSafe API key installs it in one step:

```sh
opencode plugin add @fdcn/chauffeur
```

No Rust, Node, clone, or symlinks. Every release is built, tested, and staged
by CI from a tag; it goes public when you approve it with 2FA.

## Packages

| Package | Contents |
| --- | --- |
| `@fdcn/chauffeur` | The plugin bundle (`dist/index.js`), the shipped `skills/` folder, `README.md`, `LICENSE`, `CHANGELOG.md` |
| `@fdcn/chauffeur-darwin-arm64` | The `chauffeur` binary for Apple silicon |
| `@fdcn/chauffeur-darwin-x64` | The binary for Intel Macs |
| `@fdcn/chauffeur-linux-x64` | A static (musl) binary for x86-64 Linux |
| `@fdcn/chauffeur-linux-arm64` | A static (musl) binary for arm64 Linux |

The main package lists the four platform packages as `optionalDependencies`
at its exact version, each declaring its `os` and `cpu`. OpenCode's plugin
installer keeps only the matching one (its cache holds only the
`darwin-arm64` builds of other plugins' native dependencies). Windows is not
supported.

## Runtime

**Binary.** The plugin starts the daemon from the platform package it
resolves at load time. `CHAUFFEUR_BIN` overrides it; with neither, the plugin
falls back to `chauffeur` on `PATH`.

**Shipped skills.** The plugin starts the daemon with `CHAUFFEUR_SKILLS_DIR`
set to the package's `skills/`, unless the user sets it. Personal rules,
rulebooks, and overrides stay in `~/.config/chauffeur`, untouched by upgrades.

**Hand-over skills.** The plugin registers `skills/handoff/*` (`slack-cli`,
`jira-cli`, `twitter-cli`) with `ctx.skill.transform`, so nothing is linked
into `~/.agents/skills`.

**Version handshake.** `health` returns the daemon's version:

```sh
curl -s localhost:18790/rpc -d '{"id":1,"method":"health","params":{}}'
# → {"id":1,"result":{"ok":true,"version":"0.2.0"}}
```

A plugin that finds a daemon of another version replaces it: it sends the new
`shutdown` method, waits for the port to free, and starts its own. A daemon the
user runs (`CHAUFFEUR_DAEMON_URL` set) is never replaced; the plugin logs the
mismatch and stands aside, as it does when the daemon is unreachable.

**Key.** The daemon reads `TYPESAFE_API_KEY` from the environment OpenCode
runs in; without it the daemon exits and Chauffeur stands aside.

## Versions

One version for everything: the Cargo workspace and all five packages move
together, so the plugin and daemon never mix. Before 1.0:

- **Minor** (`0.2.0`): a change that needs the user to act or changes default
  behaviour: rule, rulebook, contract, or config format; a default turned on or
  off; a removed setting.
- **Patch** (`0.2.1`): fixes and wording.

`CHANGELOG.md` is generated from the conventional commits the repo already
uses, with changelogen as in sourcefed; a release's section becomes its GitHub
release notes.

**Channels.** Tag `vX.Y.Z` publishes to `latest`; `vX.Y.Z-rc.N` publishes to
`next`. Unpinned installs pick up `latest` through `opencode plugin update`;
pinned ones (`@fdcn/chauffeur@0.2.0`) stay put.

## CI

**`check.yml`** on every push and pull request:

- `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`
- `npm run check` in `adapters/opencode` (types, lint, tests, fallow)
- `chauffeur-bench verify` (offline task checks)
- a packaging dry run: build the Linux binary, assemble the packages, `npm pack`
  each, and start the packed daemon with a dummy key to call `health` and
  `texts`

**`release.yml`** on a `v*` tag, in a `release` environment that requires your
approval:

1. **Build** the binary on each target (`macos-14` for both macOS targets,
   `ubuntu-24.04` and `ubuntu-24.04-arm` with musl) and the plugin bundle.
2. **Smoke** each binary: `chauffeur daemon` with a dummy key answers `health`
   with the tag's version.
3. **Stage** each package with `npm stage publish`, platform packages before
   the main one. Trusted publishing (OIDC) authenticates; npm adds provenance.
4. **Release** on GitHub: the four binaries, `SHA256SUMS`, and the changelog
   section.

You then approve the staged packages on npmjs.com with 2FA, platform packages
first.

## Cutting a release

```sh
npm run release -- --minor   # bumps all versions, writes CHANGELOG.md, tags, pushes
```

Before tagging `latest`, an `rc` goes through:

- `chauffeur-bench prompt --variants base,full` on Luna and Opus: first-request
  tokens within 2% of the previous release
- the outcome suite on Luna, `full` only: pass rate within one run of the
  previous release
- `opencode plugin add @fdcn/chauffeur@next` on your machine for a day of use

## One-time setup

1. **Repository:** add `LICENSE` (MIT, as `Cargo.toml` declares); set
   `repository.url` in every `package.json` to
   `https://github.com/StevenJPx2/chauffeur` exactly, which trusted publishing
   requires; drop `"private": true`; rename the package `@fdcn/chauffeur`.
2. **First publish:** npm configures trusted publishing per existing package,
   so `0.1.0` of all five packages is published once from your terminal, from
   CI's build artifacts, with your OTP.
3. **Trusted publishers:** on each package, a GitHub Actions publisher for
   `StevenJPx2/chauffeur`, workflow `release.yml`, environment `release`,
   stage-only, with dist-tag management allowed; then "Require two-factor
   authentication and disallow tokens".
4. **GitHub:** a `release` environment with you as required reviewer, and a tag
   protection rule for `v*`.

## Your machine

Your setup moves to the published package:

- `~/.config/opencode/opencode.jsonc` (dotfiles) lists `@fdcn/chauffeur`;
  `~/.config/opencode/plugins/chauffeur.js` and the `~/.config/chauffeur/skills`
  link go.
- The launchd job runs the package's binary, or goes, since the plugin starts
  the daemon.
- **Development** points the plugin entry at the checkout
  (`~/Documents/Projects/chauffeur/adapters/opencode`) and sets
  `CHAUFFEUR_BIN` and `CHAUFFEUR_SKILLS_DIR` to the checkout's builds, so hot
  reload keeps working on the repo's skills.

## Order of work

1. Runtime: bundled-binary resolution, `CHAUFFEUR_SKILLS_DIR` from the package,
   hand-over skill registration, `health` version and `shutdown`, plugin
   handshake. Tests for each.
2. Packaging: `scripts/package.mjs` assembles the five packages from a binary
   directory and the bundle; `check.yml` with the dry run.
3. Repository setup, then `release.yml`; cut `v0.1.0-rc.1` to `next` and check
   it on your machine.
4. First publish of `0.1.0`, trusted publishers, then `0.1.1` through CI to
   prove the path.
5. README install section and your machine's migration.

## Open questions

- Intel Macs and Linux arm64: built and smoke-tested, but no one runs them
  day to day. Keep them, or ship macOS arm64 and Linux x64 only until asked?
- The daemon has only run on macOS. Linux needs one live session (CI's smoke
  test does not exercise Jev) before `0.1.0`.
