use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use chauffeur_core::Outcome;
use chauffeur_judges::{Answer, Settled};
use chauffeur_protocol::{Event, Rule};
use chauffeur_runtime::{Log, Ran, run};
use serde_json::{Map, Value, json};

use crate::state::Loaded;

/// How many runs the log keeps.
const LOG_BOUND: usize = 256;
/// How many rule and session pairs keep a cooldown.
const COOLDOWN_BOUND: usize = 4_096;

/// Routes events to the rules that hook them, and keeps what they did.
pub struct Router {
    loaded: RwLock<Arc<Loaded>>,
    fired: Mutex<BTreeMap<(String, String), Instant>>,
    log: Mutex<Log<Value>>,
}

impl Router {
    /// A router over `loaded`.
    #[must_use]
    pub fn new(loaded: Loaded) -> Self {
        Self {
            loaded: RwLock::new(Arc::new(loaded)),
            fired: Mutex::new(BTreeMap::new()),
            log: Mutex::new(Log::new(LOG_BOUND)),
        }
    }

    /// Replaces the loaded rules, models and redaction; the next event uses them.
    pub fn replace(&self, loaded: Loaded) {
        if let Ok(mut current) = self.loaded.write() {
            *current = Arc::new(loaded);
        }
    }

    /// The current rules, models and redaction.
    #[must_use]
    pub fn loaded(&self) -> Option<Arc<Loaded>> {
        self.loaded.read().ok().map(|loaded| Arc::clone(&loaded))
    }

    /// Runs every rule that hooks `event`, at once, each within `deadline`,
    /// and returns what each rule returned.
    pub fn emit(&self, event: &Event, deadline: Duration) -> Vec<Value> {
        let Some(loaded) = self.loaded() else {
            return Vec::new();
        };
        let hooked: Vec<&Arc<Rule>> = loaded
            .rules
            .iter()
            .filter(|rule| rule.hooks(event))
            .collect();
        let results: Vec<(Value, Option<Value>)> = thread::scope(|scope| {
            let workers: Vec<_> = hooked
                .into_iter()
                .map(|rule| scope.spawn(|| self.run_one(&loaded, rule, event, deadline)))
                .collect();
            workers
                .into_iter()
                .filter_map(|worker| worker.join().ok())
                .collect()
        });
        let mut returned = Vec::new();
        let mut log = self.log.lock().ok();
        for (entry, action) in results {
            if let Some(log) = log.as_mut() {
                log.push(entry);
            }
            returned.extend(action);
        }
        returned
    }

    /// One rule's run: its log entry, and what it returned.
    fn run_one(
        &self,
        loaded: &Arc<Loaded>,
        rule: &Arc<Rule>,
        event: &Event,
        deadline: Duration,
    ) -> (Value, Option<Value>) {
        let session = event.session().unwrap_or_default().to_owned();
        let key = (rule.id().to_owned(), session.clone());
        let mut entry = Map::new();
        entry.insert("rule".into(), json!(rule.id()));
        entry.insert("event".into(), json!(event.name()));
        entry.insert("session".into(), json!(session));
        if self.cooling(rule, &key) {
            entry.insert("cooldown".into(), json!(true));
            return (Value::Object(entry), None);
        }
        let started = Instant::now();
        let ran = run(&loaded.context, rule, event, deadline);
        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        entry.insert("elapsed_ms".into(), json!(elapsed));
        let returned = match ran {
            Ran::Settled { run, then } => {
                entry.insert(
                    "steps".into(),
                    steps(&run.verdict.iter().collect::<Vec<_>>()),
                );
                then
            }
            Ran::Refused(reason) => {
                entry.insert("refused".into(), json!(reason));
                None
            }
            Ran::Deadline => {
                entry.insert("deadline".into(), json!(true));
                None
            }
        };
        let action = returned.map(|then| {
            entry.insert("returned".into(), json!(then.name()));
            self.fire(key);
            json!({ "rule": rule.id(), "action": then.name(), "payload": then.payload() })
        });
        (Value::Object(entry), action)
    }

    /// Whether `rule` returned something for this session within its cooldown.
    fn cooling(&self, rule: &Rule, key: &(String, String)) -> bool {
        let (Some(cooldown), Ok(fired)) = (rule.cooldown(), self.fired.lock()) else {
            return false;
        };
        fired.get(key).is_some_and(|at| at.elapsed() < cooldown)
    }

    fn fire(&self, key: (String, String)) {
        let Ok(mut fired) = self.fired.lock() else {
            return;
        };
        if fired.len() >= COOLDOWN_BOUND
            && let Some(oldest) = fired
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(key, _)| key.clone())
        {
            fired.remove(&oldest);
        }
        fired.insert(key, Instant::now());
    }

    /// The logged runs, oldest first, and how many were dropped.
    #[must_use]
    pub fn events(&self) -> Value {
        self.log.lock().map_or(
            Value::Null,
            |log| json!({ "runs": log.iter().collect::<Vec<_>>(), "dropped": log.dropped() }),
        )
    }
}

/// Each step's answer or failure, for the log.
fn steps(verdict: &[(&str, &Outcome<Answer>)]) -> Value {
    let steps = verdict
        .iter()
        .map(|(step, outcome)| match outcome {
            Outcome::Value(answer) => {
                let p: Map<String, Value> = answer.p().map(|(value, p)| (value.to_owned(), json!(p.to_f64()))).collect();
                let settled = match answer.outcome() {
                    Settled::Outcome(value) => value.clone(),
                    Settled::Unsure => "unsure".to_owned(),
                };
                json!({ "step": step, "answer": { "outcome": settled, "p": p, "level": answer.level(),
                        "model": answer.by().model, "placeholder": answer.by().placeholder } })
            }
            Outcome::Failed(failure) => json!({ "step": step, "failed": failure.name() }),
        })
        .collect();
    Value::Array(steps)
}
