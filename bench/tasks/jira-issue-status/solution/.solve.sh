#!/bin/sh
set -eu
jira issue view ADEPT-123 --plain >/dev/null
