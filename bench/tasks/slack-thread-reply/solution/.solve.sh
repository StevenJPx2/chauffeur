#!/bin/sh
set -eu
slackcli messages send --permalink "https://acme.slack.com/archives/C0123ABCD/p1700000000000000" --message "Deploy finished — all checks green."
