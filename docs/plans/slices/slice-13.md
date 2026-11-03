# Slice 13: Cutover

**Status:** Planned.

v2 replaces v1, with no layer that reads v1's files. The
[migration guide's prompt](../../migration/README.md#rewriting-your-configuration)
has an agent rewrite a v1 configuration into v2's files, keep the v1 copy,
and list anything with no v2 home; every shipped v1 rule is already ported.
When
every parity row is done, `v1` becomes a hotfix branch, v2 merges into `main`,
and `1.0.0` is tagged and published to Homebrew.

Verification: upgrade from the current v1 release on a machine with personal
rules, rewritten by an agent from the prompt until `chauffeur daemon status`
refuses none; the bench `verify` fixtures and release-gate runs pass; a day of real
use.
