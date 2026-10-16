# Configured skill sources

Chauffeur follows maintained skills from their upstream instead of making you
maintain local copies. Add trusted sources to
`$CHAUFFEUR_CONFIG_DIR/skill-sources.json` (normally
`~/.config/chauffeur/skill-sources.json`):

```json
{
  "refresh_seconds": 86400,
  "sources": [
    {
      "id": "team",
      "type": "git",
      "url": "https://github.com/example/team-skills.git",
      "ref": "main",
      "paths": ["skills"]
    },
    {
      "id": "vendor",
      "type": "http",
      "url": "https://example.com/skills/"
    }
  ]
}
```

Only this user-owned configuration introduces sources. A project's files cannot
silently enroll a new upstream.

This repository publishes its own skills the same way. To follow the
`chauffeur` skill and the hand-over skills:

```json
{
  "id": "chauffeur",
  "type": "git",
  "url": "https://github.com/StevenJPx2/chauffeur.git",
  "ref": "main",
  "paths": ["skills/chauffeur", "skills/handoff"]
}
```

### Your descriptions

The judge reads only the first 400 bytes of a skill's description, and an
upstream description may run longer or lead with the wrong thing. Replace it
without forking the skill:

```json
{
  "sources": [ … ],
  "descriptions": {
    "mobile-native": "Make a web app feel native on a phone: … Use when …"
  }
}
```

Only selection reads your wording; the instructions and files still come from
upstream. Editing a description downloads nothing.

## How a remote skill stays current

Chauffeur keeps two things apart:

- **Descriptions**, which choosing a skill needs all the time. They are kept on
  disk and refreshed only when upstream changed.
- **A skill's files** (its instructions, references and scripts). They are
  fetched from upstream when the skill is attached to a session, for exactly
  the revision that was listed.

Every `refresh_seconds` (default once a day), the daemon asks each source
whether anything changed. A source you add or edit is checked right away:

- **Git:** one `ls-remote` for the commit the ref points at. A pinned commit
  needs no request at all.
- **HTTP:** one conditional request for `index.json`. An unchanged catalog
  answers "not modified" with no body.

An unchanged answer downloads nothing. When upstream did change, Chauffeur
fetches only the `SKILL.md` files, for their descriptions. Git does this
through a persistent partial clone: folder trees arrive without file contents,
and each file's contents are fetched the first time they are needed.

When the judge attaches a remote skill (on a prompt, through `ask_chauffeur`,
or as a mid-turn hand-over), the daemon fetches that skill's files before
replying, waiting at most ten seconds. Skills that are never attached never
download their references or scripts. A skill already fetched at that
revision is not fetched again.

A running session keeps the revision it was given, so its instructions and
scripts never change underneath it. The next attach after an upstream change
gets the new revision.

## Using a skill's files

Each attached skill's text names its source and revision, its folder, and every
file in it by full path:

```text
Source: team (https://github.com/example/team-skills.git)
Revision: 3f9c…
Base directory: ~/.local/state/chauffeur/skill-cache/team/3f9c…/review
Files:
- ~/.local/state/chauffeur/skill-cache/team/3f9c…/review/references/policy.md
- ~/.local/state/chauffeur/skill-cache/team/3f9c…/review/scripts/check.sh (executable)
```

- Fetched files are read-only, so an agent cannot edit a cached skill.
- Git keeps each script's executable bit, so instructions such as "run
  `scripts/check.sh`" work as written. HTTP catalogs carry no file modes; run
  their scripts through an interpreter (`sh scripts/check.sh`).
- Reading these files and running these scripts go through your normal
  permission rules. The cache sits outside the project, so a rule that asks
  before `external_directory` access asks here too; allow
  `~/.local/state/chauffeur/skill-cache/*` to skip those prompts.

## Source formats

### Git

- `url`: HTTPS or SSH. SCP-style URLs such as
  `git@example.com:team/skills.git` work too.
- `ref`: a branch, tag, or commit. Defaults to `HEAD`. A branch or tag tracks
  updates; a commit pins a revision.
- `paths`: repository directories to search for skills. Defaults to `["."]`.
- Each skill is a folder with a `SKILL.md` containing YAML `name` and
  `description`, followed by its instructions. Folded and multiline YAML
  descriptions work. Every regular file in the folder belongs to the skill.
- Authentication uses Git's existing credential helpers and SSH configuration.
  Interactive credential prompts are disabled. Put no secrets in source URLs.
- Fetching reads Git objects directly. It does not check out files, run source
  scripts, apply checkout filters, follow symlinks, or fetch submodules.
- The server must support partial clone (`blob:none` filters), as GitHub and
  GitLab do.

### HTTP catalogs

The base URL serves an OpenCode-compatible `index.json`:

```json
{
  "skills": [
    {
      "name": "review",
      "version": "3",
      "files": ["SKILL.md", "references/policy.md"]
    }
  ]
}
```

- Files above live at `<base>/review/SKILL.md` and
  `<base>/review/references/policy.md`.
- A `<name>.md` entry is also accepted instead of `SKILL.md`.
- Bump `version` whenever a skill's files change, as OpenCode's own catalogs
  require: a change check compares `index.json`, so an edit to a reference
  alone is not noticed until `index.json` changes.
- Only HTTPS is accepted for remote HTTP catalogs. Plain HTTP and `git://` are
  allowed only for loopback development servers.
- Redirects, embedded credentials, query parameters, traversal, cross-origin
  files, and duplicate catalog names are refused.
- Custom authorization headers are not supported. Use Git with configured
  credentials for private sources.

## Precedence and failures

- The daemon owns one worker shared by all harnesses. Source work never blocks
  the synchronous judgment engine.
- Configuration changes are noticed within five seconds.
- A failed check is retried after `refresh_seconds`, not on every prompt, and
  keeps the last good descriptions.
- A failed or slow file fetch still delivers the skill's instructions; its
  files are fetched again on the next attach.
- Warm starts expose cached descriptions immediately. A cold catalog waits for
  one bounded initial check.
- Among remote sources, the first configured source wins a name collision.
- In OpenCode 2.0.28, a remote skill shadows a discovered local skill of the
  same name while its source is configured. Removing the source restores the
  local one.
- Removing a source withdraws its skills without deleting files a running
  session may still use. Invalid configuration edits keep the previous catalog.

## Cache

Under `$CHAUFFEUR_STATE_DIR/skill-cache/<source-id>/`, normally
`~/.local/state/chauffeur/skill-cache/`:

- `listing.json`: the current revision and every skill's description, body and
  file list, replaced atomically.
- `repo.git`: the Git source's partial clone.
- `<revision>/<skill>/`: one attached skill's files at one revision. Each is
  staged and published by rename, so a partial download never appears.

Revision folders are never removed beneath a session. Each source keeps at most
32; at the ceiling, attaching a skill at a new revision fails visibly and the
instructions still arrive. Remove old revision folders after sessions using
them have closed. Automatic reclamation is not implemented yet.

Inspect the catalog and source status independently of any harness:

```sh
chauffeur skills
```

The same result is available through the daemon's `skills` RPC and the Rust
daemon client's `skills()` method. Source states are:

- `ready`: the current revision and its last successful check time
- `stale`: the last good revision plus the latest check error
- `unavailable`: no usable listing yet

## Bounds

- At most 16 sources, with unique IDs, and 256 description overrides of at
  most 4 KiB each.
- Check intervals from 60 seconds to seven days.
- At most 16 Git directories searched per source.
- At most 512 files per skill and 16 MiB per fetched skill.
- At most 1 MiB per downloaded file.
- At most 64 KiB per `SKILL.md`, 16 KiB of frontmatter, and 4 KiB of description.
- At most 100 files named in a skill's text; the folder holds them all.
- At most 64 remote skills offered, in configured source and file order. The
  host's existing 64-candidate selection bound also includes local skills.
- At most ten seconds per source check, twenty seconds of checking per cycle,
  and ten seconds of file fetching per reply.

## Public discovery

Searching public registries for a skill you do not have is not built.
[skilld](https://github.com/skilld-dev/skilld) is the likely basis: its
`skilld search` and `skilld trending` commands have JSON output, and it builds
verified artifacts from exact Git commits.

Remote skills are trusted instructions from sources you enroll, not verified
safe programs. Fetching never executes them; an agent following their
instructions still passes through the normal tool and permission policies.
