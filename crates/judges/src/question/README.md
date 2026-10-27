# `question`

The only thing a judge is given: instructions, the outcomes to choose from,
and the names of the evidence blocks it may see.

| File | Holds |
|---|---|
| `mod.rs` | `Question::new` and `Question::noul` (yes/no, Jev's name for it) |
| `outcomes.rs` | `Outcomes`: 2 to 64 labels, unique, none blank, none `unsure` or `failed`. Their order is a scale's order |
| `label.rs` | `Label`: an outcome's value and an optional description the model sees |
| `error.rs` | `QuestionError`: why a question or its outcomes were refused |

```rust
use chauffeur_judges::{Label, Outcomes, Question};

let outcomes = Outcomes::new([
    Label::new("low"),
    Label::described("high", "Could lose data or money"),
])?;
let question = Question::new("How risky is this command?", outcomes, ["tool_call"])?;
assert_eq!(question.outcomes().position("high"), Some(1));
```

- `unsure` and `failed` are reserved because a step's result can take
  those names; an outcome with either name would be ambiguous.
