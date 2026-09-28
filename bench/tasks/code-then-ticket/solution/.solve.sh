#!/bin/sh
set -eu
jira issue view ADEPT-481 --plain --comments 5 >/dev/null
jira issue comment add ADEPT-481 "Fixed: the digest used UTC day boundaries and printed event times in UTC, ignoring the user's utc_offset_minutes. digest_day/day_bounds now use the user's local timezone offset and render_digest shows local times. Added tests for positive, negative and half-hour offsets around local midnight."
jira issue move ADEPT-481 "In Review"
