mod log;
mod pool;
mod wait;

use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use chauffeur_core::{Panel, Run};
use chauffeur_judges::{Answer, Evidence, Fallback, Models, NamedJudge};
use chauffeur_protocol::{Event, Rule, Strategy, Then};

use crate::redact::{Masking, Redactor};

pub use log::{EventLog, MAX_EVENTS};
pub use pool::{Job, Pool};
pub use wait::{Cancel, Waited, within};

/// The most workers kept waiting between runs: enough for every rule hooked
/// by one event to start at once.
const KEEP: usize = 8;

/// The most workers alive: twice the daemon's capacity, so runs that outlive
/// their deadline, still waiting on their models, leave room for new ones.
const MAX_WORKERS: usize = 32;

/// The workers every run in this process shares.
static WORKERS: LazyLock<Pool> = LazyLock::new(|| Pool::new(KEEP, MAX_WORKERS));

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

/// How a strategy's run ended.
#[derive(Debug)]
pub enum Ran {
    /// Every step settled: the run's events and verdict.
    Settled(Run<Answer>),
    /// The strategy could not run, such as a placeholder whose answers would
    /// not load, or no worker free.
    Refused(String),
    /// The deadline passed first; a late result is dropped.
    Deadline,
    /// The caller cancelled first; a late result is dropped.
    Cancelled,
}

impl Ran {
    /// The first `then` entry of `rule` its verdict meets, if it settled.
    #[must_use]
    pub fn then<'a>(&self, rule: &'a Rule) -> Option<&'a Then> {
        match self {
            Self::Settled(run) => Then::first_met(rule.then(), &run.verdict),
            Self::Refused(_) | Self::Deadline | Self::Cancelled => None,
        }
    }
}

/// Runs `rule` for `event`, waiting at most `deadline`.
#[must_use]
pub fn run(context: &Arc<Context>, rule: &Arc<Rule>, event: &Event, deadline: Duration) -> Ran {
    let deadline = Instant::now().checked_add(deadline);
    let Some(deadline) = deadline else {
        return Ran::Refused("the deadline is too far away".to_owned());
    };
    run_strategy(context, rule, &event.evidence(), deadline, &Cancel::new())
}

/// Runs `strategy` over `evidence` on a pooled worker thread, until
/// `deadline` or `cancel`. The evidence is redacted here, before any judge
/// sees it. The worker is free again within its models' timeouts.
#[must_use]
pub fn run_strategy<S: AsRef<Strategy> + Send + Sync + 'static>(
    context: &Arc<Context>,
    strategy: &Arc<S>,
    evidence: &[(String, String)],
    deadline: Instant,
    cancel: &Cancel,
) -> Ran {
    let mut masking = Masking::default();
    let blocks = evidence
        .iter()
        .map(|(name, content)| (name, context.redactor.redact(content, &mut masking)));
    let evidence = Evidence::new(blocks);
    let (context, strategy) = (Arc::clone(context), Arc::clone(strategy));
    let job = move |deadline: Instant, cancel: &Cancel| {
        run_now(&context, (*strategy).as_ref(), &evidence, deadline, cancel)
    };
    match within(&WORKERS, deadline, cancel, job) {
        Waited::Done(ran) => ran,
        Waited::Deadline => Ran::Deadline,
        Waited::Cancelled => Ran::Cancelled,
        Waited::Refused(reason) => Ran::Refused(reason),
    }
}

fn run_now(
    context: &Context,
    strategy: &Strategy,
    evidence: &Evidence,
    deadline: Instant,
    cancel: &Cancel,
) -> Ran {
    // Checked before the round: the caller may have gone, or waited too long.
    if cancel.is_cancelled() {
        return Ran::Cancelled;
    }
    if Instant::now() >= deadline {
        return Ran::Deadline;
    }
    let mut fallbacks = Vec::with_capacity(strategy.judges().len());
    for (name, spec) in strategy.judges() {
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
    match strategy.plan().run(&panel) {
        Ok(run) => Ran::Settled(run),
        Err(error) => Ran::Refused(error.to_string()),
    }
}
