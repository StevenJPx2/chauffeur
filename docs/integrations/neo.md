# neo

neo drives a real iPhone through Ghosthand and runs fixed phone actions such
as `send_money` and `send_whatsapp_text`. Before and after each irreversible
tap it checks the screen twice: exact values in code (the payee, the amount)
and the screen's overall meaning, asked of Jev with the screenshot and its OCR
text. A yes below 0.8 stops the action.

With Chauffeur, the meaning check is a rule. neo keeps its exact value checks,
its `complete: true` flag and the Approve dialog on the Mac: those are its
safety boundary, and Chauffeur never replaces them.

## The rule

```jsonc
// ~/.config/chauffeur/rules/neo/screen-check.json, installed by neo
{
  "on": "neo.screen-check",
  "judges": {
    "screen": {
      "tier": "system-one",
      "model": "jev",
      "confidence": { "yes": 0.8, "no": 0.5 }
    }
  },
  "strategy": [
    {
      "id": "shows",
      "judge": "screen",
      "ask": {
        "outcomes": ["yes", "no"],
        "evidence": ["screenshot", "ocr_text", "question"],
        "instructions": "Looking at the iPhone screen in screenshot (its OCR text is in ocr_text), answer the question in question."
      }
    }
  ],
  "then": [{ "if": { "shows": "yes" }, "confirm": "screen" }]
}
```

## The call

```ts
import { navigate } from "@fdcn/chauffeur/client"

const { intents } = await navigate(
  {
    on: "neo.screen-check",
    evidence: {
      screenshot: { image: await phone.screenshot(), media_type: "image/png" },
      ocr_text: screenText,
      question: check.question
    }
  },
  { signal: AbortSignal.timeout(30_000) }
)
const confirmed = intents.some((intent) => intent.confirm === "screen")
```

No intent (a `no`, an `unsure`, a failure, no daemon) means the screen is not
confirmed, so the action stops: neo's default for an unconfirmed screen.

## What neo gains

- The bar (0.8) and the wording live in a file the user can tune and replay.
- `chauffeur audit` shows every check: Jev's probabilities, the model that
  answered and how long it took.
- The model is configuration: a second System One model, or GLiDE for what
  Jev leaves unsure, is an edit to the rule, not to neo.

## What it needs

| Piece                                                                                                                                                                       | Slice |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----- |
| `navigate` and the TypeScript client                                                                                                                                        | 2     |
| `image` evidence: bytes and a media type, sent only to a format that accepts images (Jev, with its opt-in), only when a step names the block, never redacted, at most 4 MiB | 3     |
