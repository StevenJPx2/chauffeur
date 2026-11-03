# Proclaimify

Proclaimify suggests a song's key from its chord sheet with one Jev `choice`
question over 17 major keys. It runs on Netlify, where no Chauffeur daemon
exists, so production keeps calling TypeSafe directly.

Chauffeur serves the same question on the machine where the question is
developed: the rule replays Proclaimify's test songs, shows Jev's probability
for every key in `chauffeur events`, and compares models, such as Jev against
GLiDE, by changing one line.

## The rule

```jsonc
// ~/.config/chauffeur/rules/proclaimify/key.json
{
  "on": "proclaimify/key",
  "judges": { "keys": { "tier": "system-one", "confidence": 0.5 } },
  "strategy": [
    { "id": "root", "judge": "keys",
      "ask": { "outcomes": [
                 { "value": "A", "description": "A major key" },
                 { "value": "Bb", "description": "B flat major key" }
                 /* … the other 15 keys */
               ],
               "evidence": ["stanzas"],
               "instructions": "The evidence is JSON: each key names a stanza, and each array holds the chord string for each line. Which note is the tonic of the whole song as a major key? …" } }
  ],
  "then": [{ "key": "root" }]
}
```

## The call

```sh
jq -n --slurpfile song song.json '{ on: "proclaimify/key", evidence: { stanzas: $song[0] } }' | chauffeur navigate
# → { "intents": [{ "rule": "proclaimify/key", "key": "G" }] }
```

An `unsure` answer returns no intent; Proclaimify then suggests nothing, as
it does when Jev fails.

## What it needs

| Piece | Slice |
|---|---|
| `navigate` and `chauffeur navigate` | 2 |
| `choice` with every outcome's probability | 3 |
