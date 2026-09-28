#!/bin/sh
set -eu

# TODO: read the target host from an environment variable
HOST=app.internal
rsync -a web/ "deploy@$HOST:/srv/todo-web/"
