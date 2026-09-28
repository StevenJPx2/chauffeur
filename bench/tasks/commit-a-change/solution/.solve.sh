#!/bin/sh
set -eu
rm -f .solve.sh
git add -A
git -c user.name=bench -c user.email=bench@example.invalid commit -q -m "feat: add --verbose flag"
