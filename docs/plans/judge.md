# Judge: a judgment state machine in core

Core provides one generic state machine for System One judgments, `Judge<T>`, and a small
factory of primitives to build it. Single questions, fan-outs, chains, and any other
judging strategy are compositions of those primitives, written where they are needed. A
capability composes a judge from a signal's facts and acts on the value it finishes with;
the engine drives every capability's judge round by round.

## The state machine

A `Judge<T>` is in one of two states:

```rust
enum Judge<T> {
    Done(T),                                   // finished, with its value
    Asking { questions: Vec<Question>, next: Box<dyn FnOnce(Option<&[Answer]>) -> Judge<T> + Send> },
}
```

Each round, every asking judge's questions go into the engine's one System One call. The
answers, or `None` when the call failed, feed `next`, which yields the judge for the
following round. A judge still asking after the engine's last round is fed `None`, so it
settles through its failure path. Nothing else can make a judge run longer.

## The factory

Four primitives build every judge:

| Primitive | Meaning | Rounds |
|---|---|---|
| `Judge::done(value)` | finished, no question | 0 |
| `Judge::ask(question, read)` | one question; `read` turns its answer, or `None`, into a value | 1 |
| `judge.then(f)` | when `judge` finishes with `v`, continue as `f(v)` | sum |
| `Judge::all(judges)` | run judges side by side, questions in the same rounds; finishes with all their values | max |

`map` follows from `then`, and `a.zip(b)` is `all` for two judges of different value
types. `judge.unless_failed()` finishes with `None` when a round's call failed, so a
capability can tell an unanswered judgment from rules that refused. Question IDs within a
round must be unique; `all` and `zip` check this.

Answers are read through **rules**, the one place thresholds live:

```rust
Rule::yes(0.7, 0.4).holds(answer)        // P ≥ 0.7, confident
Rule::no(0.3, 0.4).holds(answer)         // P ≤ 0.3, confident
Rule::unless_no(0.3, 0.4).holds(answer)  // not a confident P ≤ 0.3
Rule::pick(0.4).chosen(answer)           // Some(option) other than `none`, confident
```

Every rule is false, or `None`, for a failed call.

## Strategies are compositions

`judge::strategy` provides the common ones, built only from the factory:

```rust
// one question, one rule
strategy::single(question, Rule::unless_no(0.3, 0.4))            // → Judge<bool>

// one question per candidate in a single round; the keys that hold, most likely first
strategy::fan_out(candidates)                                    // → Judge<Vec<K>>

// per candidate, each step asked only when the previous held
strategy::chain(candidates)                                      // → Judge<Vec<K>>
```

A capability composes its own when these do not fit. Tool recovery picks a hidden tool,
then confirms the pick in the next round:

```rust
Judge::ask(choose_tool, |answer| Rule::pick(0.4).chosen(answer))
    .then(|tool| match tool {
        Some(tool) => Judge::ask(confirm(&tool), move |answer| Rule::yes(0.7, 0.4).holds(answer).then_some(tool)),
        None => Judge::done(None),
    })
```

## Capabilities

A judged capability implements two methods; `Judging` adapts it to `Capability`, holding the
in-flight judge between rounds:

```rust
impl Judged for SkillExposure {
    type Verdict = Vec<String>;

    fn judge(&mut self, situation: &Situation, signal: &Signal) -> Option<Judge<Vec<String>>> {
        let skills = self.offerable(signal);

        Some(strategy::fan_out(skills.iter().map(|skill| candidate(signal, skill))))
    }

    fn act(&mut self, signal: &Signal, skills: Vec<String>) -> Vec<Effect> {
        self.attach(signal, &skills)   // within the 64 KiB skill budget
    }
}

let skills = Judging::new(SkillExposure::default());   // a Capability, composed by the daemon
```

The engine's contract is unchanged: capabilities still ask through `plan` and `advance`,
and one System One call per round carries every capability's questions. `Judging` asks
again only after a successful call with rounds remaining; otherwise it settles the judge,
so a verdict is always acted on.

## Migration

| Capability | Composition |
|---|---|
| skill exposure — prompt, `ask_chauffeur` | `fan_out` over offered skills; facts choose each skill's wording and rule |
| skill exposure — drift | `single` with `Rule::pick` |
| tool exposure — hide / reveal / Code Mode | `fan_out` over groups and namespaces |
| tool exposure — `ask_chauffeur` | `fan_out` over hidden groups and namespaces |
| tool exposure — missing-tool recovery | pick, `then` confirm |
| rules | `chain` over admitted rules |
| monitors | `fan_out` over candidates |
| event gate | `single` with `Rule::no`: withhold on a confident no |
| model router | `single` with `Rule::pick` |
| permission | stays on its contracts, which map answers to allow, ask, or deny |

## Order

Each step lands on the branch, then `main`, once every check passes.

1. `judge` in core: the state machine, the four primitives, rules, and `strategy`, with unit
   tests for each primitive, the round bound, and failure.
2. `Judged` and `Judging`, tested through the engine.
3. Skill exposure on `fan_out`, which gives a prompt every skill it needs; then tool
   exposure's `ask_chauffeur` and recovery.
4. Rules, monitors, event gate, and model router, one commit each.
