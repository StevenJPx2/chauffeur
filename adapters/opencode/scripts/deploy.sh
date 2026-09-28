#!/bin/sh
# Check, build, and install the plugin into OpenCode's global plugins folder.
# The file is replaced atomically, so a running service never loads half of it;
# restart the service (`opencode service restart`) to load a new build.
set -eu

cd "$(dirname "$0")/.."
plugins="${XDG_CONFIG_HOME:-$HOME/.config}/opencode/plugins"

npm run check
npx tsdown

mkdir -p "$plugins"

# The old entry loaded this repository's sources directly. It goes first, so
# the host never sees two plugins with the id `chauffeur`.
if [ -f "$plugins/chauffeur.ts" ] && grep -q "adapters/opencode/src" "$plugins/chauffeur.ts"; then
  rm "$plugins/chauffeur.ts"
fi

cp dist/index.mjs "$plugins/.chauffeur.js.tmp"
mv "$plugins/.chauffeur.js.tmp" "$plugins/chauffeur.js"

echo "installed $plugins/chauffeur.js ($(wc -c < "$plugins/chauffeur.js" | tr -d ' ') bytes)"
