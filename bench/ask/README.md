# ask_chauffeur test set

Replays agent requests against the running daemon and scores what each one
hands over, so a change to wording, bars, caps or ranking can be measured.

- `catalog.json`: the skills and Code Mode namespaces an agent saw, taken from
  the audit log's questions (descriptions as the judge reads them, at most 400
  bytes).
- `code-mode.json`: each case's namespaces with the tools Code Mode's own search
  finds for its need, as the plugin sends them.
- `cases.json`: each request (`need`, `user_request`), the skills already in
  its context (`attached`), and its labels. `want` lists groups of answers, at
  least one of each must be handed over (`[]` means nothing fits); `allow`
  lists other acceptable answers; `max` caps how many items. Items are
  `skill:<id>`, `ns:<namespace>` and `tools:<tool>`.
- `ranking.json`: requests that each clearly need one skill, for `rank.mjs`.

```sh
# Each case sends a "continue" prompt with the catalog, then the request, under
# a fresh agent ID; the daemon's own config and judges answer.
node bench/ask/run.mjs --repeats 2 --out /tmp/ask.jsonl
node bench/ask/run.mjs --cases slack-thread-reply,worker-logs

# Grade a saved run again after changing labels
node bench/ask/run.mjs --rescore /tmp/ask.jsonl

# Where each request's skill lands in the ranking that decides which skills
# are judged: the embedding service, BM25, and both fused
node bench/ask/rank.mjs
```

A case passes when every `want` group is met, nothing outside `want` and
`allow` is handed over, and `max` holds. The closest names a reply would give
when nothing fits are printed, not graded.

Results on 2026-11-03, two repeats each (26 runs):

| Version | Pass | Missed | Unneeded items | Items per request | Median |
|---|---|---|---|---|---|
| Before (every match, "any part" wording) | 7 | 3 | 47 | 3.7 | 1.7 s |
| Need-only wording, best few, closest named | 19 | 4 | 5 | 1.4 | 1.1 s |
| + 24 skills judged, a pick among several, no `skill` namespace | 23 | 2 | 1 | 1.3 | 1.3 s |

The first row used fixed namespace examples; the others use `code-mode.json`.
On `ranking.json`, BM25 put the right skill first as often as the embeddings
(35 of 45) and ranked it lower otherwise; fusing both changed nothing.
