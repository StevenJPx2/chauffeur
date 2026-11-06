mod cooldown;
mod log_entry;

use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use chauffeur_protocol::{Event, Rule};
use chauffeur_runtime::{Ran, run};
use cooldown::Cooldowns;
use serde_json::{Map, Value, json};

use crate::audit::Audit;
use crate::state::Loaded;

/// Routes events to the rules that hook them, and writes what they did to
/// the audit log.
pub struct Router {
    loaded: RwLock<Arc<Loaded>>,
    cooldowns: Cooldowns,
    audit: Audit,
}

impl Router {
    /// A router over `loaded`, writing each run to `audit`.
    #[must_use]
    pub fn new(loaded: Loaded, audit: Audit) -> Self {
        Self {
            loaded: RwLock::new(Arc::new(loaded)),
            cooldowns: Cooldowns::new(),
            audit,
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
            let mut hooked = hooked.into_iter();
            // The last rule runs on this thread, so one hooked rule starts none.
            let here = hooked.next_back();
            let workers: Vec<_> = hooked
                .map(|rule| scope.spawn(|| self.run_one(&loaded, rule, event, deadline)))
                .collect();
            // As a worker's panic is dropped at its join, so is this one's.
            let last = here.and_then(|rule| {
                panic::catch_unwind(AssertUnwindSafe(|| {
                    self.run_one(&loaded, rule, event, deadline)
                }))
                .ok()
            });
            workers
                .into_iter()
                .filter_map(|worker| worker.join().ok())
                .chain(last)
                .collect()
        });
        let mut returned = Vec::new();
        for (entry, action) in results {
            // A run still returns its action when the log cannot be written.
            if let Err(error) = self.audit.append(entry) {
                eprintln!("chauffeur: a run was not written to the audit log: {error}");
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
        if self.cooldowns.cooling(&key, rule.cooldown()) {
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
                    log_entry::steps(&run.verdict.iter().collect::<Vec<_>>()),
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
            self.cooldowns.fire(key);
            json!({ "rule": rule.id(), "action": then.name(), "payload": then.payload() })
        });
        (Value::Object(entry), action)
    }
}
