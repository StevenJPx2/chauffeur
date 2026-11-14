use std::sync::{Arc, Mutex};
use std::time::Instant;

use chauffeur_judges::Fallback;
use chauffeur_protocol::{Intent, Navigate, Rejection, Rule, Strategy, Target, progress};
use chauffeur_runtime::{Cancel, Context, EventLog, MAX_EVENTS, Ran, run_strategy};
use serde_json::{Map, Value, json};

use super::{Call, Router, log_entry};
use crate::state::Loaded;

/// What an inline strategy's run is logged as.
const INLINE: &str = "inline";

impl Router {
    /// Answers a navigation: the intents of every rule its event hooks, or an
    /// inline strategy's verdict. `notify` is given each `chauffeur.event`'s
    /// params as the run goes, at most [`MAX_EVENTS`] of them.
    ///
    /// # Errors
    ///
    /// A [`Rejection`]: `deadline` or `judge_unavailable` for an inline
    /// strategy, `cancelled` once the caller cancels.
    pub fn navigate(
        &self,
        request: Navigate,
        cancel: &Cancel,
        notify: &(dyn Fn(Value) + Sync),
    ) -> Result<Value, Rejection> {
        let loaded = self
            .loaded()
            .ok_or_else(|| Rejection::JudgeUnavailable("no configuration is loaded".to_owned()))?;
        let deadline = Instant::now()
            .checked_add(request.deadline())
            .ok_or(Rejection::Deadline)?;
        let log = Mutex::new(EventLog::new(MAX_EVENTS));
        let send = |event: Value| {
            let kept = log.lock().is_ok_and(|mut log| log.push(()));
            if kept {
                notify(event);
            }
        };
        let evidence = request.evidence().to_vec();
        let answered = match (request.event(), request.into_target()) {
            (Some(event), Target::On(on)) => {
                let hooked: Vec<&Arc<Rule>> = loaded
                    .rules
                    .iter()
                    .filter(|rule| rule.hooks(&event))
                    .collect();
                let call = Call {
                    event: &on,
                    session: "",
                    evidence: &evidence,
                    deadline,
                    cancel,
                };
                Ok(self.intents(&loaded, &hooked, &call, &send))
            }
            (_, Target::Strategy(strategy)) => {
                self.inline(&loaded, strategy, &evidence, deadline, cancel, &send)
            }
            (None, Target::On(_)) => Err(Rejection::Invalid("no event to match".to_owned())),
        };
        if cancel.is_cancelled() {
            return Err(Rejection::Cancelled);
        }
        answered
    }

    /// Every hooked rule's intents; a rule that misses the deadline, fails or
    /// meets no `then` entry gives none, and the rest still return.
    fn intents(
        &self,
        loaded: &Arc<Loaded>,
        hooked: &[&Arc<Rule>],
        call: &Call<'_>,
        send: &(dyn Fn(Value) + Sync),
    ) -> Value {
        for rule in hooked {
            send(progress::rule_matched(rule.id()));
        }
        let ran = |rule: &Rule, done: &super::RuleRun| {
            for event in done
                .run
                .iter()
                .flat_map(|run| progress::run_events(Some(rule.id()), &run.events))
            {
                send(event);
            }
        };
        let intents: Vec<Value> = self
            .run_all(loaded, hooked, call, &ran)
            .into_iter()
            .filter_map(|(rule, then)| then.map(|then| Intent::new(rule.id(), &then).to_json()))
            .collect();
        json!({ "intents": intents })
    }

    /// An inline strategy's verdict, once every judge it names has a model.
    fn inline(
        &self,
        loaded: &Arc<Loaded>,
        strategy: Strategy,
        evidence: &[(String, String)],
        deadline: Instant,
        cancel: &Cancel,
        send: &(dyn Fn(Value) + Sync),
    ) -> Result<Value, Rejection> {
        available(&loaded.context, &strategy)?;
        let mut entry = Map::new();
        entry.insert("rule".into(), json!(INLINE));
        entry.insert("event".into(), json!("chauffeur.navigate"));
        entry.insert("session".into(), json!(""));
        let started = Instant::now();
        let ran = run_strategy(
            &loaded.context,
            &Arc::new(strategy),
            evidence,
            deadline,
            cancel,
        );
        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        entry.insert("elapsed_ms".into(), json!(elapsed));
        let refused = match &ran {
            Ran::Refused(reason) => Some(reason.clone()),
            Ran::Settled(_) | Ran::Deadline | Ran::Cancelled => None,
        };
        let run = log_entry::ended(&mut entry, ran);
        if let Err(error) = self.audit.append(Value::Object(entry)) {
            eprintln!("chauffeur: a run was not written to the audit log: {error}");
        }
        match (run, refused) {
            (Some(run), _) => {
                for event in progress::run_events(None, &run.events) {
                    send(event);
                }
                Ok(json!({ "verdict": progress::verdict(&run.verdict) }))
            }
            (None, Some(reason)) => Err(Rejection::JudgeUnavailable(reason)),
            // A cancelled run is answered as cancelled by `navigate`.
            (None, None) => Err(Rejection::Deadline),
        }
    }
}

/// Refuses a strategy naming a judge with no model to ask, before any runs.
fn available(context: &Context, strategy: &Strategy) -> Result<(), Rejection> {
    for (name, spec) in strategy.judges() {
        let fallback = Fallback::build(
            &context.models,
            spec.tier,
            spec.model.as_deref(),
            &context.keys,
        )
        .map_err(|error| Rejection::JudgeUnavailable(error.to_string()))?;
        if fallback.is_empty() {
            return Err(Rejection::JudgeUnavailable(format!(
                "judge `{name}` has no {} model to ask",
                spec.tier.name()
            )));
        }
    }
    Ok(())
}
