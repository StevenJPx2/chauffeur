mod pool;

use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use chauffeur_core::{Panel, Run};
use chauffeur_judges::{Answer, Evidence, Fallback, Models, NamedJudge};
use chauffeur_protocol::{Event, Rule, Then};

use crate::redact::{Masking, Redactor};

pub use pool::{Job, Pool};

/// The most workers kept waiting between runs: enough for every rule hooked
/// by one event to start at once.
const KEEP: usize = 8;

/// The workers every run in this process shares.
static WORKERS: LazyLock<Pool> = LazyLock::new(|| Pool::new(KEEP));

/// Finds a model's key by its environment variable's name.
pub type Keys = dyn Fn(&str) -> Option<String> + Send + Sync;

/// What a rule needs to run: the models, the redactor and the keys.
pub struct Context {
    /// Registered models and tiers.
    pub models: Models,
    /// Redacts evidence before any judge sees it.
    pub redactor: Redactor,
    /// Finds model keys.
    pub keys: Box<Keys>,
}

/// How a rule's run ended.
#[derive(Debug)]
pub enum Ran {
    /// Every step settled: the run, and the first `then` entry it met.
    Settled {
        /// The run's events and verdict.
        run: Run<Answer>,
        /// What the rule returns, if any entry was met.
        then: Option<Then>,
    },
    /// The rule could not run, such as a placeholder whose answers would not
    /// load, or a worker that stopped.
    Refused(String),
    /// The deadline passed first; a late result is dropped.
    Deadline,
}

/// Runs `rule` for `event` on a pooled worker thread, waiting at most
/// `deadline`. The worker is free again within its models' timeouts.
#[must_use]
pub fn run(context: &Arc<Context>, rule: &Arc<Rule>, event: &Event, deadline: Duration) -> Ran {
    let mut masking = Masking::default();
    let blocks = event
        .evidence()
        .into_iter()
        .map(|(name, content)| (name, context.redactor.redact(&content, &mut masking)));
    let evidence = Evidence::new(blocks);
    let (send, receive) = mpsc::sync_channel(1);
    let (context, rule) = (Arc::clone(context), Arc::clone(rule));
    let started = WORKERS.execute(Box::new(move || {
        let _ = send.send(run_now(&context, &rule, &evidence));
    }));
    if let Err(error) = started {
        return Ran::Refused(format!("cannot start a worker: {error}"));
    }
    match receive.recv_timeout(deadline) {
        Ok(ran) => ran,
        Err(RecvTimeoutError::Timeout) => Ran::Deadline,
        // The worker dropped its sender without sending: it stopped.
        Err(RecvTimeoutError::Disconnected) => Ran::Refused("the run's worker stopped".to_owned()),
    }
}

fn run_now(context: &Context, rule: &Rule, evidence: &Evidence) -> Ran {
    let mut fallbacks = Vec::with_capacity(rule.judges().len());
    for (name, spec) in rule.judges() {
        match Fallback::build(
            &context.models,
            spec.tier,
            spec.model.as_deref(),
            &context.keys,
        ) {
            Ok(fallback) => fallbacks.push((name, spec, fallback)),
            Err(error) => return Ran::Refused(error.to_string()),
        }
    }
    let judges: Vec<(&String, NamedJudge<'_>)> = fallbacks
        .iter()
        .map(|(name, spec, fallback)| {
            (*name, NamedJudge::new(fallback, &spec.confidence, evidence))
        })
        .collect();
    let panel = judges.iter().fold(Panel::new(), |panel, (name, judge)| {
        panel.with(name.as_str(), judge)
    });
    match rule.plan().run(&panel) {
        Ok(run) => {
            let then = Then::first_met(rule.then(), &run.verdict).cloned();
            Ran::Settled { run, then }
        }
        Err(error) => Ran::Refused(error.to_string()),
    }
}
