mod cooldown;
mod log_entry;
mod navigate;

use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use chauffeur_judges::Answer;
use chauffeur_protocol::{Event, Rule, Then};
use chauffeur_runtime::{Admission, Busy, Cancel, Capacity, Permit, Source, run_strategy};
use cooldown::Cooldowns;
use serde_json::{Map, Value, json};

use crate::audit::Audit;
use crate::state::Loaded;

/// Routes events and navigations to the rules that hook them, admits each
/// within its source's share, and writes what they did to the audit log.
pub struct Router {
    loaded: RwLock<Arc<Loaded>>,
    cooldowns: Cooldowns,
    admission: Admission,
    audit: Audit,
}

/// One rule's run for an event or a navigation: what it was given.
struct Call<'a> {
    event: &'a str,
    session: &'a str,
    evidence: &'a [(String, String)],
    deadline: Instant,
    cancel: &'a Cancel,
}

/// One rule's run: its log entry, what it returned, and its run, if it ran.
struct RuleRun {
    entry: Value,
    then: Option<Then>,
    run: Option<chauffeur_core::Run<Answer>>,
}

impl Router {
    /// A router over `loaded`, writing each run to `audit`.
    #[must_use]
    pub fn new(loaded: Loaded, audit: Audit) -> Self {
        Self {
            loaded: RwLock::new(Arc::new(loaded)),
            cooldowns: Cooldowns::new(),
            admission: Admission::new(Capacity::DAEMON),
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

    /// A place for one call from `source`, held until its reply is sent.
    ///
    /// # Errors
    ///
    /// [`Busy`] while the source's share or the capacity is in use.
    pub fn admit(&self, source: Source) -> Result<Permit, Busy> {
        self.admission.admit(source)
    }

    /// Runs every rule that hooks `event`, at once, each within `deadline`
    /// or until `cancel`, and returns what each rule returned.
    pub fn emit(&self, event: &Event, deadline: Duration, cancel: &Cancel) -> Vec<Value> {
        let Some(loaded) = self.loaded() else {
            return Vec::new();
        };
        let Some(deadline) = Instant::now().checked_add(deadline) else {
            return Vec::new();
        };
        let evidence = event.evidence();
        let call = Call {
            event: event.name(),
            session: event.session().unwrap_or_default(),
            evidence: &evidence,
            deadline,
            cancel,
        };
        let hooked: Vec<&Arc<Rule>> = loaded
            .rules
            .iter()
            .filter(|rule| rule.hooks(event))
            .collect();
        self.run_all(&loaded, &hooked, &call, &|_, _| {})
            .into_iter()
            .filter_map(|(rule, then)| {
                then.map(|then| {
                    json!({ "rule": rule.id(), "action": then.name(), "payload": then.payload() })
                })
            })
            .collect()
    }

    /// Runs `hooked` at once, calling `ran` as each finishes, and returns
    /// what each returned, in the order given. Each run is written to the log.
    fn run_all<'r>(
        &self,
        loaded: &Arc<Loaded>,
        hooked: &[&'r Arc<Rule>],
        call: &Call<'_>,
        ran: &(dyn Fn(&Rule, &RuleRun) + Sync),
    ) -> Vec<(&'r Arc<Rule>, Option<Then>)> {
        let results: Vec<(&Arc<Rule>, RuleRun)> = thread::scope(|scope| {
            let one = |rule: &'r Arc<Rule>| {
                let done = self.run_one(loaded, rule, call);
                ran(rule, &done);
                (rule, done)
            };
            let mut hooked = hooked.iter().copied();
            // The last rule runs on this thread, so one hooked rule starts none.
            let here = hooked.next_back();
            // Every worker starts before any is joined.
            let mut workers = Vec::with_capacity(hooked.len());
            for rule in hooked {
                workers.push(scope.spawn(move || one(rule)));
            }
            // As a worker's panic is dropped at its join, so is this one's.
            let last =
                here.and_then(|rule| panic::catch_unwind(AssertUnwindSafe(|| one(rule))).ok());
            workers
                .into_iter()
                .filter_map(|worker| worker.join().ok())
                .chain(last)
                .collect()
        });
        results
            .into_iter()
            .map(|(rule, done)| {
                // A run still returns when the log cannot be written.
                if let Err(error) = self.audit.append(done.entry) {
                    eprintln!("chauffeur: a run was not written to the audit log: {error}");
                }
                (rule, done.then)
            })
            .collect()
    }

    /// One rule's run: its log entry, what it returned, and its run.
    fn run_one(&self, loaded: &Arc<Loaded>, rule: &Arc<Rule>, call: &Call<'_>) -> RuleRun {
        let key = (rule.id().to_owned(), call.session.to_owned());
        let mut entry = Map::new();
        entry.insert("rule".into(), json!(rule.id()));
        entry.insert("event".into(), json!(call.event));
        entry.insert("session".into(), json!(call.session));
        if self.cooldowns.cooling(&key, rule.cooldown()) {
            entry.insert("cooldown".into(), json!(true));
            return RuleRun {
                entry: Value::Object(entry),
                then: None,
                run: None,
            };
        }
        let started = Instant::now();
        let ran = run_strategy(
            &loaded.context,
            rule,
            call.evidence,
            call.deadline,
            call.cancel,
        );
        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        entry.insert("elapsed_ms".into(), json!(elapsed));
        let returned = ran.then(rule).cloned();
        let run = log_entry::ended(&mut entry, ran);
        // Runs that settle at once in one session may all have passed
        // `cooling`; only the first to fire returns its action.
        let then = returned.and_then(|then| {
            if !self.cooldowns.try_fire(key, rule.cooldown()) {
                entry.insert("cooldown".into(), json!(true));
                return None;
            }
            entry.insert("returned".into(), json!(then.name()));
            Some(then)
        });
        RuleRun {
            entry: Value::Object(entry),
            then,
            run,
        }
    }
}
