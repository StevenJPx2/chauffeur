//! Runs the engine on its own thread and schedules its rounds. Several
//! agents' signals are judged at once, each agent's in order, and every
//! judge call runs on a worker thread, so a slow call holds up only its own
//! signal. A permission request is admitted first and keeps a System One
//! worker for itself, since the host holds a tool call until it is answered.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use chauffeur_core::engine::MAX_ROUNDS;
use chauffeur_core::{
    Answer, Effect, Engine, Question, Reasoned, Reasoning, ReasoningFailure, Reloading, RunId,
    Signal, SignalKind, Step, SystemOne, SystemTwo, system_two,
};
use tokio::sync::oneshot;

use crate::judge::JudgeConfig;
use crate::workers::Pool;

/// Signals waiting to be judged, at most; more are refused.
const QUEUE_DEPTH: usize = 64;
/// Signals judged at once.
const MAX_ACTIVE: usize = 8;
const ONE_WORKERS: usize = 4;
/// System One workers only permission requests may use.
const RESERVED_FOR_PERMISSION: usize = 1;
const TWO_WORKERS: usize = 2;
/// How often a busy scheduler checks deadlines and hosts that gave up.
const TICK: Duration = Duration::from_millis(50);
/// How long the host waits for a signal's effects.
pub(crate) const REPLY_TIMEOUT: Duration = Duration::from_secs(15);
/// A signal's own deadline, from when it arrives: settled through its
/// failure posture before the host stops waiting.
pub(crate) const RUN_DEADLINE: Duration = Duration::from_secs(14);

/// Builds one System Two executor per worker.
pub type SystemTwoFactory = Arc<dyn Fn() -> Result<Box<dyn SystemTwo>, String> + Send + Sync>;

/// Where the scheduler keeps the engine's memory and its decision log.
pub(crate) struct Files {
    pub(crate) state: Option<std::path::PathBuf>,
    pub(crate) audit: Option<std::path::PathBuf>,
}

/// What the scheduler is handed: an engine to build on its thread, the
/// judges to build on its workers, and each signal's deadline.
pub(crate) struct Setup {
    pub(crate) judge: JudgeConfig,
    pub(crate) system_two: Option<SystemTwoFactory>,
    pub(crate) deadline: Duration,
}

enum Event {
    Job(Job),
    Done(Done),
    /// Every handle is gone.
    Closed,
}

struct Job {
    signal: Signal,
    reply: oneshot::Sender<Result<Vec<Effect>, String>>,
    received: Instant,
}

enum Done {
    Answers {
        run: RunId,
        round: usize,
        result: Result<Vec<Answer>, String>,
    },
    Reasoned {
        run: RunId,
        round: usize,
        id: String,
        outcome: Reasoned,
    },
}

struct OneTask {
    run: RunId,
    round: usize,
    state: String,
    questions: Vec<Question>,
    urgent: bool,
}

struct TwoTask {
    run: RunId,
    round: usize,
    request: Reasoning,
}

/// The daemon's way in to the engine.
#[derive(Clone)]
pub struct EngineHandle {
    inner: Arc<Inner>,
}

struct Inner {
    events: Sender<Event>,
    queued: Arc<AtomicUsize>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let _ = self.events.send(Event::Closed);
    }
}

impl EngineHandle {
    /// Judge `signal` and return its effects.
    ///
    /// # Errors
    ///
    /// The queue is full or the engine stopped; the signal is invalid; or
    /// no effects came within the host's wait.
    pub async fn ingest(&self, signal: Signal) -> Result<Vec<Effect>, String> {
        let (reply, result) = oneshot::channel();

        if self.inner.queued.fetch_add(1, Ordering::SeqCst) >= QUEUE_DEPTH {
            self.inner.queued.fetch_sub(1, Ordering::SeqCst);
            return Err("engine unavailable: queue full".into());
        }

        let job = Job {
            signal,
            reply,
            received: Instant::now(),
        };

        if self.inner.events.send(Event::Job(job)).is_err() {
            self.inner.queued.fetch_sub(1, Ordering::SeqCst);
            return Err("engine unavailable: stopped".into());
        }

        tokio::time::timeout(REPLY_TIMEOUT, result)
            .await
            .map_err(|_| "engine timed out".to_string())?
            .map_err(|_| "engine dropped the signal".to_string())?
    }

    /// Start the scheduler thread, which builds the engine with `make` and
    /// the judges' workers; resolves once all are built, or with the first
    /// error.
    pub(crate) async fn launch(
        make: impl FnOnce() -> Result<(Reloading<Engine>, Files), String> + Send + 'static,
        setup: Setup,
    ) -> Result<Self, String> {
        let (events, inbox) = mpsc::channel();
        let queued = Arc::new(AtomicUsize::new(0));
        let (ready, started) = oneshot::channel();
        let done = events.clone();
        let counter = Arc::clone(&queued);

        std::thread::Builder::new()
            .name("chauffeur-engine".into())
            .spawn(move || {
                let built = (|| {
                    let (engine, files) = make()?;
                    let judge = setup.judge;
                    let one = Pool::spawn(
                        "chauffeur-judge",
                        ONE_WORKERS,
                        move || judge.build(),
                        ask,
                        &done,
                    )?;
                    let two = setup
                        .system_two
                        .map(|factory| {
                            Pool::spawn(
                                "chauffeur-reason",
                                TWO_WORKERS,
                                move || factory(),
                                reason,
                                &done,
                            )
                        })
                        .transpose()?;

                    Ok::<_, String>(Scheduler {
                        engine,
                        files,
                        inbox,
                        queued: counter,
                        deadline: setup.deadline,
                        waiting: VecDeque::new(),
                        active: HashMap::new(),
                        one,
                        one_backlog: VecDeque::new(),
                        two,
                        two_backlog: VecDeque::new(),
                    })
                })();

                match built {
                    Ok(scheduler) => {
                        let _ = ready.send(Ok(()));
                        drop(done);
                        scheduler.run();
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error));
                    }
                }
            })
            .map_err(|error| format!("spawn engine thread: {error}"))?;

        started
            .await
            .map_err(|_| "engine thread exited during startup".to_string())??;

        Ok(Self {
            inner: Arc::new(Inner { events, queued }),
        })
    }
}

fn ask(judge: &mut Box<dyn SystemOne>, task: OneTask) -> Event {
    let result = judge
        .ask(&task.state, &task.questions)
        .map_err(|error| error.0);

    Event::Done(Done::Answers {
        run: task.run,
        round: task.round,
        result,
    })
}

fn reason(executor: &mut Box<dyn SystemTwo>, task: TwoTask) -> Event {
    let outcome = system_two::run(Some(executor.as_mut()), &task.request);

    Event::Done(Done::Reasoned {
        run: task.run,
        round: task.round,
        id: task.request.id,
        outcome,
    })
}

/// A signal being judged, between its rounds.
struct Active {
    job: Job,
    round: usize,
    asked_at: Instant,
    /// System One's result for this round, once back.
    answers: Option<Result<Vec<Answer>, String>>,
    /// Reasonings asked this round.
    asked: Vec<String>,
    reasoned: Vec<(String, Reasoned)>,
    /// Calls of this round still out.
    awaiting: usize,
    /// The engine's memory before a signal that runs alone, restored if its
    /// host stops waiting.
    before: Option<serde_json::Value>,
}

struct Scheduler {
    engine: Reloading<Engine>,
    files: Files,
    inbox: Receiver<Event>,
    queued: Arc<AtomicUsize>,
    deadline: Duration,
    /// Signals not yet started, in arrival order.
    waiting: VecDeque<Job>,
    active: HashMap<RunId, Active>,
    one: Pool<OneTask>,
    /// Permission requests' calls first.
    one_backlog: VecDeque<OneTask>,
    two: Option<Pool<TwoTask>>,
    two_backlog: VecDeque<TwoTask>,
}

fn is_permission(signal: &Signal) -> bool {
    matches!(signal.kind, SignalKind::PermissionRequest { .. })
}

/// A model error runs alone: if its host stops waiting, the engine's memory
/// is restored to before it, which must not undo another signal.
fn runs_alone(signal: &Signal) -> bool {
    matches!(signal.kind, SignalKind::ModelError { .. })
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

impl Scheduler {
    fn run(mut self) {
        loop {
            self.sweep();
            self.admit();
            self.dispatch();

            let event = if self.active.is_empty() && self.waiting.is_empty() {
                self.inbox
                    .recv()
                    .map_err(|_| RecvTimeoutError::Disconnected)
            } else {
                self.inbox.recv_timeout(TICK)
            };

            match event {
                Ok(Event::Job(job)) => {
                    self.queued.fetch_sub(1, Ordering::SeqCst);
                    self.waiting.push_back(job);
                }
                Ok(Event::Done(done)) => self.collect(done),
                Ok(Event::Closed) | Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    /// Drop signals whose host stopped waiting, and settle those past their
    /// deadline.
    fn sweep(&mut self) {
        let now = Instant::now();
        let deadline = self.deadline;

        for job in std::mem::take(&mut self.waiting) {
            if job.reply.is_closed() {
                continue;
            }
            if now >= job.received + deadline {
                let _ = job.reply.send(Err("engine timed out".into()));
                continue;
            }
            self.waiting.push_back(job);
        }

        let gone: Vec<RunId> = self
            .active
            .iter()
            .filter(|(_, active)| active.job.reply.is_closed())
            .map(|(run, _)| *run)
            .collect();

        for run in gone {
            self.abandon(run);
        }

        let late: Vec<RunId> = self
            .active
            .iter()
            .filter(|(_, active)| now >= active.job.received + deadline)
            .map(|(run, _)| *run)
            .collect();

        for run in late {
            self.expire(run);
        }
    }

    /// Start waiting signals while there is room: a permission request
    /// first, otherwise the oldest; never one whose agent has a signal in
    /// flight or waiting ahead of it.
    fn admit(&mut self) {
        if self.waiting.is_empty() {
            return;
        }

        // Config and skills reload only between signals.
        if self.active.is_empty() {
            self.engine.current();
        }

        while self.active.len() < MAX_ACTIVE {
            if self.active.values().any(|active| active.before.is_some()) {
                return;
            }

            let Some(index) = self.next() else { return };

            if runs_alone(&self.waiting[index].signal) && !self.active.is_empty() {
                return;
            }

            if let Some(job) = self.waiting.remove(index) {
                self.begin(job);
            }
        }
    }

    fn next(&self) -> Option<usize> {
        let eligible = |index: usize| {
            let agent = &self.waiting[index].signal.agent_id;

            !self
                .active
                .values()
                .any(|active| active.job.signal.agent_id == *agent)
                && !self
                    .waiting
                    .iter()
                    .take(index)
                    .any(|earlier| earlier.signal.agent_id == *agent)
        };

        (0..self.waiting.len())
            .filter(|index| eligible(*index))
            .find(|index| is_permission(&self.waiting[*index].signal))
            .or_else(|| (0..self.waiting.len()).find(|index| eligible(*index)))
    }

    fn begin(&mut self, job: Job) {
        let engine = self.engine.peek();
        let before = runs_alone(&job.signal).then(|| engine.save());

        match engine.start(&job.signal) {
            Err(error) => self.complete(job, Err(error), None),
            Ok((_, Step::Done(effects))) => self.complete(job, Ok(effects), before),
            Ok((
                run,
                Step::Ask {
                    state,
                    questions,
                    reasonings,
                },
            )) => {
                self.active.insert(
                    run,
                    Active {
                        job,
                        round: 0,
                        asked_at: Instant::now(),
                        answers: None,
                        asked: Vec::new(),
                        reasoned: Vec::new(),
                        awaiting: 0,
                        before,
                    },
                );
                self.ask(run, 1, state, questions, reasonings);
            }
        }
    }

    /// Queue a round's calls. Without a System Two executor, each reasoning
    /// is unavailable at once.
    fn ask(
        &mut self,
        run: RunId,
        round: usize,
        state: String,
        questions: Vec<Question>,
        reasonings: Vec<Reasoning>,
    ) {
        let Some(active) = self.active.get_mut(&run) else {
            return;
        };
        let urgent = is_permission(&active.job.signal);

        active.round = round;
        active.asked_at = Instant::now();
        active.answers = None;
        active.reasoned.clear();
        active.asked = reasonings
            .iter()
            .map(|request| request.id.clone())
            .collect();
        active.awaiting = 0;

        if !questions.is_empty() {
            active.awaiting += 1;

            let task = OneTask {
                run,
                round,
                state,
                questions,
                urgent,
            };
            let at = if urgent {
                self.one_backlog
                    .iter()
                    .position(|queued| !queued.urgent)
                    .unwrap_or(self.one_backlog.len())
            } else {
                self.one_backlog.len()
            };

            self.one_backlog.insert(at, task);
        }

        for request in reasonings {
            if self.two.is_some() {
                active.awaiting += 1;
                self.two_backlog.push_back(TwoTask {
                    run,
                    round,
                    request,
                });
            } else {
                let outcome = system_two::run(None, &request);

                active.reasoned.push((request.id, outcome));
            }
        }

        if active.awaiting == 0 {
            self.advance(run);
        }
    }

    /// Hand queued calls to free workers, skipping those of signals already
    /// settled or abandoned. The last free System One worker serves only
    /// permission requests.
    fn dispatch(&mut self) {
        while self.one.free() > 0 {
            let index = if self.one.free() <= RESERVED_FOR_PERMISSION {
                self.one_backlog.iter().position(|task| task.urgent)
            } else {
                (!self.one_backlog.is_empty()).then_some(0)
            };
            let Some(task) = index.and_then(|index| self.one_backlog.remove(index)) else {
                break;
            };

            if self.active.contains_key(&task.run) {
                self.one.send(task);
            }
        }

        if let Some(two) = &mut self.two {
            while two.free() > 0 {
                let Some(task) = self.two_backlog.pop_front() else {
                    break;
                };

                if self.active.contains_key(&task.run) {
                    two.send(task);
                }
            }
        }
    }

    /// A worker's result: kept for its signal's round, which continues once
    /// all its calls are back. A result for a signal that settled or was
    /// abandoned only frees its worker.
    fn collect(&mut self, done: Done) {
        let (run, round) = match &done {
            Done::Answers { run, round, .. } => {
                self.one.finished();
                (*run, *round)
            }
            Done::Reasoned { run, round, .. } => {
                if let Some(two) = &mut self.two {
                    two.finished();
                }
                (*run, *round)
            }
        };
        let Some(active) = self
            .active
            .get_mut(&run)
            .filter(|active| active.round == round)
        else {
            return;
        };

        match done {
            Done::Answers { result, .. } => active.answers = Some(result),
            Done::Reasoned { id, outcome, .. } => active.reasoned.push((id, outcome)),
        }
        active.awaiting = active.awaiting.saturating_sub(1);

        if active.awaiting == 0 {
            self.advance(run);
        }
    }

    /// Resume the engine with a finished round.
    fn advance(&mut self, run: RunId) {
        let Some(active) = self.active.get_mut(&run) else {
            return;
        };
        let answers = active.answers.take().unwrap_or_else(|| Ok(Vec::new()));
        let reasoned = std::mem::take(&mut active.reasoned);
        let elapsed = millis(active.asked_at.elapsed());
        let round = active.round;

        match self.engine.peek().resume(run, answers, reasoned, elapsed) {
            Some(Step::Ask {
                state,
                questions,
                reasonings,
            }) => self.ask(run, round + 1, state, questions, reasonings),
            Some(Step::Done(effects)) => {
                if let Some(active) = self.active.remove(&run) {
                    self.complete(active.job, Ok(effects), active.before);
                }
            }
            None => {
                if let Some(active) = self.active.remove(&run) {
                    self.complete(active.job, Err("engine dropped the signal".into()), None);
                }
            }
        }
    }

    /// The host stopped waiting: no capability acts on the signal.
    fn abandon(&mut self, run: RunId) {
        let Some(active) = self.active.remove(&run) else {
            return;
        };
        let engine = self.engine.peek();

        engine.cancel(run);

        if let Some(before) = active.before {
            engine.load(before);
        }
    }

    /// Past its deadline: settle the signal with what came back, every call
    /// still out read as timed out, so each capability applies its failure
    /// posture and the host gets effects before it stops waiting.
    fn expire(&mut self, run: RunId) {
        let Some(mut active) = self.active.remove(&run) else {
            return;
        };
        let engine = self.engine.peek();
        let answers = active
            .answers
            .take()
            .unwrap_or_else(|| Err("timed out".into()));
        let mut reasoned = std::mem::take(&mut active.reasoned);

        for id in &active.asked {
            if !reasoned.iter().any(|(answered, _)| answered == id) {
                reasoned.push((id.clone(), Err(ReasoningFailure::TimedOut)));
            }
        }

        let mut step = engine.resume(run, answers, reasoned, millis(active.asked_at.elapsed()));

        for _ in 0..MAX_ROUNDS {
            match step {
                Some(Step::Ask { .. }) => {
                    step = engine.resume(run, Err("timed out".into()), Vec::new(), 0);
                }
                _ => break,
            }
        }

        let result = match step {
            Some(Step::Done(effects)) => Ok(effects),
            Some(Step::Ask { .. }) => {
                engine.cancel(run);
                Err("engine timed out".into())
            }
            None => Err("engine timed out".into()),
        };

        self.complete(active.job, result, active.before);
    }

    /// Reply to the host, log the decision and save the engine's memory. If
    /// the host stopped waiting, nothing is logged and a signal that ran
    /// alone is undone.
    fn complete(
        &mut self,
        job: Job,
        result: Result<Vec<Effect>, String>,
        before: Option<serde_json::Value>,
    ) {
        let engine = self.engine.peek();

        if job.reply.is_closed() {
            if let Some(before) = before {
                engine.load(before);
            }
            return;
        }

        if let Some(path) = &self.files.audit {
            crate::audit::append(path, &job.signal, engine.trace(), &result, |text| {
                engine.redact_for_audit(text)
            });
        }

        if job.reply.send(result).is_err() {
            if let Some(before) = before {
                engine.load(before);
            }
            return;
        }

        if let Some(path) = &self.files.state {
            crate::engine::persist(engine, path);
            if engine.take_learned_changed() {
                crate::learned::save(engine, path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread::sleep;

    use chauffeur_core::{
        AnswerValue, Delivery, Judge, Judged, Judging, PermissionDecision, Schema, Situation,
        SystemOneError, Watch,
    };
    use serde_json::json;

    use super::*;

    const SLOW_ONE: Duration = Duration::from_millis(600);
    const SLOW_TWO: Duration = Duration::from_millis(1_500);
    /// Well under either slow call, well over the scheduler's tick.
    const PROMPT: Duration = Duration::from_millis(400);

    /// A permission request asks System One; a turn end whose summary is
    /// `reason` asks System Two; any other turn end asks System One, slowly
    /// when its summary starts with `slow`.
    struct Probe;

    fn label(agent: String, label: &str) -> Effect {
        Effect::Context {
            agent_id: agent,
            delivery: Delivery::Wait,
            label: label.into(),
            skills: Vec::new(),
            text: None,
        }
    }

    impl Judged for Probe {
        type Verdict = Effect;

        fn id(&self) -> &str {
            "probe"
        }

        fn judge(&mut self, _: &Situation, signal: &Signal) -> Option<Judge<Effect>> {
            let agent = signal.agent_id.clone();
            let question = |instructions: &str| Question {
                id: "q".into(),
                instructions: instructions.into(),
                kind: chauffeur_core::QuestionKind::Noul,
            };

            match &signal.kind {
                SignalKind::PermissionRequest { .. } => {
                    Some(Judge::ask(question("allow?"), move |answer| {
                        Effect::Permission {
                            agent_id: agent,
                            decision: if answer.is_some() {
                                PermissionDecision::Allow
                            } else {
                                PermissionDecision::Ask
                            },
                            message: None,
                        }
                    }))
                }
                SignalKind::TurnEnd { summary, .. } if summary == "reason" => {
                    let request = Reasoning {
                        id: "review".into(),
                        profile: "test".into(),
                        instructions: "Done?".into(),
                        evidence: Vec::new(),
                        activity: false,
                        schema: Schema::Object(vec![(
                            "verdict".into(),
                            Schema::Enum(vec!["complete".into()]),
                        )]),
                    };

                    Some(Judge::reason(request, move |outcome| {
                        label(
                            agent,
                            if outcome.is_ok() {
                                "reviewed"
                            } else {
                                "unsettled"
                            },
                        )
                    }))
                }
                SignalKind::TurnEnd { summary, .. } => {
                    let summary = summary.clone();

                    Some(Judge::ask(question(&summary), move |answer| {
                        label(agent, if answer.is_some() { &summary } else { "failed" })
                    }))
                }
                _ => None,
            }
        }

        fn act(&mut self, _: &Signal, effect: Effect) -> Vec<Effect> {
            vec![effect]
        }
    }

    struct One;

    impl SystemOne for One {
        fn name(&self) -> &str {
            "one"
        }

        fn ask(&mut self, _: &str, questions: &[Question]) -> Result<Vec<Answer>, SystemOneError> {
            if questions
                .iter()
                .any(|question| question.instructions.starts_with("slow"))
            {
                sleep(SLOW_ONE);
            }

            Ok(questions
                .iter()
                .map(|question| Answer {
                    id: question.id.clone(),
                    value: AnswerValue::Noul(0.9),
                    confidence: None,
                })
                .collect())
        }
    }

    struct Two;

    impl SystemTwo for Two {
        fn name(&self) -> &str {
            "two"
        }

        fn reason(&mut self, _: &Reasoning) -> Reasoned {
            sleep(SLOW_TWO);
            Ok(json!({ "verdict": "complete" }))
        }
    }

    async fn scheduler(deadline: Duration) -> EngineHandle {
        let setup = Setup {
            judge: JudgeConfig::custom(Arc::new(|| Ok(Box::new(One) as Box<dyn SystemOne>))),
            system_two: Some(Arc::new(|| Ok(Box::new(Two) as Box<dyn SystemTwo>))),
            deadline,
        };

        EngineHandle::launch(
            || {
                let engine = Engine::hosted(vec![Box::new(Judging::new(Probe))])?;
                let engine = Reloading::new(
                    "test",
                    engine,
                    Watch::new(Vec::new()),
                    Box::new(|_| Err("no reload".into())),
                );

                Ok((
                    engine,
                    Files {
                        state: None,
                        audit: None,
                    },
                ))
            },
            setup,
        )
        .await
        .unwrap()
    }

    fn turn_end(agent: &str, summary: &str) -> Signal {
        Signal {
            agent_id: agent.into(),
            at: 1,
            kind: SignalKind::TurnEnd {
                workspace: String::new(),
                subagent: false,
                user_request: String::new(),
                summary: summary.into(),
                todos: Vec::new(),
            },
        }
    }

    fn permission(agent: &str) -> Signal {
        Signal {
            agent_id: agent.into(),
            at: 1,
            kind: SignalKind::PermissionRequest {
                action: "shell".into(),
                resources: Vec::new(),
                request: "ls".into(),
                workspace: String::new(),
                user_requests: Vec::new(),
                host_decision: PermissionDecision::Ask,
            },
        }
    }

    fn labels(effects: &[Effect]) -> Vec<&str> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Context { label, .. } => Some(label.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Judge `signal` in the background: its effects, how long they took, and
    /// when they came.
    fn judge(
        engine: &EngineHandle,
        signal: Signal,
    ) -> tokio::task::JoinHandle<(Vec<Effect>, Duration, Instant)> {
        let engine = engine.clone();

        tokio::spawn(async move {
            let started = Instant::now();
            let effects = engine.ingest(signal).await.unwrap();

            (effects, started.elapsed(), Instant::now())
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_slow_reasoning_holds_up_only_its_own_signal() {
        let engine = scheduler(RUN_DEADLINE).await;
        let review = judge(&engine, turn_end("a", "reason"));

        tokio::time::sleep(Duration::from_millis(50)).await;

        let (allowed, took, _) = judge(&engine, permission("b")).await.unwrap();

        assert!(took < PROMPT, "the permission request waited {took:?}");
        assert!(matches!(
            allowed.as_slice(),
            [Effect::Permission {
                decision: PermissionDecision::Allow,
                ..
            }]
        ));

        let (reviewed, took, _) = review.await.unwrap();

        assert_eq!(labels(&reviewed), ["reviewed"]);
        assert!(took >= SLOW_TWO, "{took:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn each_agents_signals_are_judged_in_order_and_others_pass_them() {
        let engine = scheduler(RUN_DEADLINE).await;
        let first = judge(&engine, turn_end("a", "slow first"));

        tokio::time::sleep(Duration::from_millis(20)).await;

        let second = judge(&engine, turn_end("a", "second"));
        let other = judge(&engine, turn_end("b", "other"));
        let (other, other_took, _) = other.await.unwrap();
        let (first, _, first_done) = first.await.unwrap();
        let (second, _, second_done) = second.await.unwrap();

        assert_eq!(labels(&other), ["other"]);
        assert!(other_took < PROMPT, "{other_took:?}");
        assert_eq!(labels(&first), ["slow first"]);
        assert_eq!(labels(&second), ["second"]);
        // The second waited for the first, though its own call is quick.
        assert!(second_done >= first_done);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_last_judge_worker_is_kept_for_permission_requests() {
        let engine = scheduler(RUN_DEADLINE).await;
        let slow: Vec<_> = (0..ONE_WORKERS)
            .map(|agent| judge(&engine, turn_end(&format!("slow-{agent}"), "slow")))
            .collect();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let (_, took, _) = judge(&engine, permission("p")).await.unwrap();

        assert!(took < PROMPT, "the permission request waited {took:?}");

        let mut took: Vec<Duration> = Vec::new();

        for run in slow {
            took.push(run.await.unwrap().1);
        }
        took.sort();

        // Three ran at once; the fourth waited for a worker, not the reserved one.
        assert!(took[ONE_WORKERS - 2] < SLOW_ONE + PROMPT, "{took:?}");
        assert!(took[ONE_WORKERS - 1] >= SLOW_ONE * 2, "{took:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_host_that_stops_waiting_frees_its_agent() {
        let engine = scheduler(RUN_DEADLINE).await;
        let gave_up = tokio::time::timeout(
            Duration::from_millis(100),
            engine.ingest(turn_end("a", "reason")),
        )
        .await;

        assert!(gave_up.is_err());

        let (effects, took, _) = judge(&engine, turn_end("a", "next")).await.unwrap();

        assert_eq!(labels(&effects), ["next"]);
        assert!(took < PROMPT, "the next signal waited {took:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_signal_past_its_deadline_settles_through_its_failure_posture() {
        let deadline = Duration::from_millis(300);
        let engine = scheduler(deadline).await;
        let (effects, took, _) = judge(&engine, turn_end("a", "reason")).await.unwrap();

        assert_eq!(labels(&effects), ["unsettled"]);
        assert!(took >= deadline && took < SLOW_TWO, "{took:?}");
    }
}
