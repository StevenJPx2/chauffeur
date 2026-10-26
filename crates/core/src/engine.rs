//! Sense → Classify → Act: batch each bounded round across capabilities.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use crate::backstop::Backstop;
use crate::capability::{Capability, Inference, PipeStep, Plan, Round, RunId};
use crate::effect::{Effect, PermissionDecision};
use crate::judge::{Docket, Judge, Routes, SYSTEM_ONE, SYSTEM_TWO, SystemOneJudge, SystemTwoJudge};
use crate::learning::{self, LearningConfig, Probe};
use crate::redact::{LearnedShapes, Masking, RedactionConfig, Redactor};
use crate::signal::{Signal, SignalKind};
use crate::situation::{Situation, SituationTexts};
use crate::system_one::{
    Answer, Question, QuestionKind, SystemOne, validate_answers, validate_some_answers,
};
use crate::system_two::{
    self, ACTIVITY, Evidence, Reasoned, Reasoning, ReasoningAttempt, ReasoningFailure, SystemTwo,
};
use crate::trace::{Trace, TracedAnswer, TracedQuestion, TracedReasoning};

pub const MAX_AGENTS: usize = 256;
/// Rounds per signal, at most, whether they ask System One, System Two, or
/// both.
pub const MAX_ROUNDS: usize = 2;
/// Signals in flight at once, at most; a host bounds its own below this.
pub const MAX_RUNS: usize = 64;
/// The state a round always keeps, however much its questions already ask.
const MIN_STATE_CHARS: usize = 2_048;
/// Bumped when the saved state's shape changes; older state is ignored.
const STATE_VERSION: u32 = 1;
/// The time a local judge is given; it decides without I/O.
const LOCAL_TIMEOUT: Duration = Duration::from_secs(1);

pub struct Engine {
    /// Every judge the engine holds, by name. Local judges rule while a round
    /// is planned; the engine's own [`SYSTEM_ONE`] and [`SYSTEM_TWO`] answer
    /// in [`Engine::ingest`], and a host that runs them itself asks them the
    /// rest. A missing System Two leaves every reasoning unavailable, so
    /// capabilities apply their failure posture.
    judges: Vec<Box<dyn Judge>>,
    /// Which judge answers each ask.
    routes: Routes,
    capabilities: Vec<Box<dyn Capability>>,
    situations: HashMap<String, Situation>,
    backstop: Backstop,
    redactor: Redactor,
    /// The backstop or redactor learned something since the host last asked.
    learning: LearningConfig,
    situation_texts: SituationTexts,
    learned_changed: bool,
    /// The trace of the signal that settled last.
    trace: Trace,
    /// Signals between their first round and their last.
    runs: HashMap<RunId, Run>,
    next_run: u64,
    /// The run [`Engine::begin`] started, which [`Engine::finish`] continues.
    solo: Option<RunId>,
}

/// What one signal needs next.
#[derive(Debug, PartialEq)]
pub enum Step {
    /// Decided without System One.
    Done(Vec<Effect>),
    /// Ask System One these questions, if any, and run each reasoning on
    /// System Two; finish with [`Engine::finish`], or [`Engine::finish_round`]
    /// when there are reasonings. Finishing may produce another round. Asks
    /// routed to a local judge are already answered and never appear here.
    Ask {
        state: String,
        questions: Vec<Question>,
        reasonings: Vec<Reasoning>,
    },
}

/// A signal between its first round and its last: everything its later
/// rounds need, owned by the run so other signals can be in flight too.
struct Run {
    signal: Signal,
    /// The agent's activity as the signal saw it; later rounds read the
    /// same, whatever else is recorded or evicted meanwhile.
    situation: Situation,
    settled: Vec<Effect>,
    asks: Vec<(usize, Inference)>,
    probe: Probe,
    round: usize,
    irreversible: bool,
    /// This round's asks for the host: System One questions and System Two
    /// reasonings.
    questions: Vec<Question>,
    reasonings: Vec<Reasoning>,
    /// This round's rulings from local judges.
    local: Local,
    trace: Trace,
}

/// What local judges ruled in one round, and the asks left for the host.
#[derive(Default)]
struct Local {
    answers: Vec<Answer>,
    reasoned: Vec<(String, Reasoned)>,
}

/// A round's asks once local judges have ruled.
struct Split {
    questions: Vec<Question>,
    reasonings: Vec<Reasoning>,
    local: Local,
}

impl Engine {
    pub fn new(
        system_one: Box<dyn SystemOne>,
        capabilities: Vec<Box<dyn Capability>>,
    ) -> Result<Self, String> {
        Ok(Self::hosted(capabilities)?.with_judge(Box::new(SystemOneJudge(system_one))))
    }

    /// An engine whose host asks System One: drive it with [`Engine::begin`]
    /// and [`Engine::finish`].
    pub fn hosted(capabilities: Vec<Box<dyn Capability>>) -> Result<Self, String> {
        let mut ids = HashSet::new();

        for capability in &capabilities {
            if !ids.insert(capability.id().to_string()) {
                return Err(format!("duplicate capability id {}", capability.id()));
            }
        }

        Ok(Self {
            judges: Vec::new(),
            routes: Routes::default(),
            capabilities,
            situations: HashMap::new(),
            trace: Trace::default(),
            backstop: Backstop::new(Vec::new()),
            redactor: Redactor::new(RedactionConfig::default(), LearnedShapes::default())?,
            learning: LearningConfig::default(),
            situation_texts: SituationTexts::default(),
            learned_changed: false,
            runs: HashMap::new(),
            next_run: 0,
            solo: None,
        })
    }

    /// Reason with `system_two`. Without one, every reasoning is unavailable.
    #[must_use]
    pub fn with_system_two(self, system_two: Box<dyn SystemTwo>) -> Self {
        self.with_judge(Box::new(SystemTwoJudge(system_two)))
    }

    /// Add `judge`, replacing any judge of the same name.
    #[must_use]
    pub fn with_judge(mut self, judge: Box<dyn Judge>) -> Self {
        self.judges.retain(|held| held.name() != judge.name());
        self.judges.push(judge);
        self
    }

    /// Send each ask its route names to that judge.
    #[must_use]
    pub fn with_routes(mut self, routes: Routes) -> Self {
        self.routes = routes;
        self
    }

    fn judge(&mut self, name: &str) -> Option<&mut Box<dyn Judge>> {
        self.judges.iter_mut().find(|judge| judge.name() == name)
    }

    fn is_local(&self, name: &str) -> bool {
        self.judges
            .iter()
            .any(|judge| judge.name() == name && judge.local())
    }

    /// Whether question `id`'s judge is the host's System One.
    fn host_answers(&self, id: &str) -> bool {
        let judge = self.routes.question(id);

        judge == SYSTEM_ONE && !self.is_local(judge)
    }

    /// Replace the shipped learning bars, for example with your `learning.json`.
    #[must_use]
    pub fn with_learning(mut self, learning: LearningConfig) -> Self {
        self.learning = learning;
        self
    }

    /// Replace the shipped wording of the state System One reads, for example
    /// with your `situation.json`.
    #[must_use]
    pub fn with_situation_texts(mut self, texts: SituationTexts) -> Self {
        self.situation_texts = texts;
        self
    }

    /// Replace the default backstop, for example with project patterns.
    #[must_use]
    pub fn with_backstop(mut self, backstop: Backstop) -> Self {
        self.backstop = backstop;
        self
    }

    /// Replace the default redactor, for example with project prefixes.
    #[must_use]
    pub fn with_redactor(mut self, redactor: Redactor) -> Self {
        self.redactor = redactor;
        self
    }

    /// Commands the backstop learned, and shapes the redactor learned.
    #[must_use]
    pub fn learned(&self) -> (Vec<String>, LearnedShapes) {
        (self.backstop.learned().to_vec(), self.redactor.learned())
    }

    /// Whether anything was learned since the last call, for persisting it.
    pub fn take_learned_changed(&mut self) -> bool {
        std::mem::take(&mut self.learned_changed)
    }

    /// Scrub audit fields with the active defaults, overrides, and learned
    /// shapes, including credential-like strings not yet judged by Jev.
    #[must_use]
    pub fn redact_for_audit(&self, text: &str) -> String {
        self.redactor.redact(text, &mut Masking::default())
    }

    /// Everything the engine remembers per agent, for persistence.
    #[must_use]
    pub fn save(&self) -> serde_json::Value {
        let capabilities: serde_json::Map<String, serde_json::Value> = self
            .capabilities
            .iter()
            .filter_map(|capability| Some((capability.id().to_string(), capability.save()?)))
            .collect();

        serde_json::json!({
            "version": STATE_VERSION,
            "situations": self.situations,
            "capabilities": capabilities,
        })
    }

    /// Restore [`Engine::save`] output. An unknown version, or a part that no
    /// longer parses, is ignored.
    pub fn load(&mut self, mut state: serde_json::Value) {
        if state["version"] != STATE_VERSION {
            return;
        }

        if let Ok(situations) =
            serde_json::from_value::<HashMap<String, Situation>>(state["situations"].take())
        {
            self.situations = situations.into_iter().take(MAX_AGENTS).collect();
        }

        for capability in &mut self.capabilities {
            let saved = state["capabilities"][capability.id()].take();

            if !saved.is_null() {
                capability.load(saved);
            }
        }
    }

    /// What the signal that settled last asked and received, for auditing.
    #[must_use]
    pub fn trace(&self) -> &Trace {
        &self.trace
    }

    /// Signals in flight.
    #[must_use]
    pub fn active(&self) -> usize {
        self.runs.len()
    }

    /// The run `agent_id` has in flight, if any.
    #[must_use]
    pub fn busy(&self, agent_id: &str) -> Option<RunId> {
        self.runs
            .iter()
            .find(|(_, run)| run.signal.agent_id == agent_id)
            .map(|(id, _)| *id)
    }

    /// Run one signal to completion with the engine's own judges.
    pub fn ingest(&mut self, signal: &Signal) -> Result<Vec<Effect>, String> {
        let (run, mut step) = self.start(signal)?;

        loop {
            let (state, questions, reasonings) = match step {
                Step::Done(effects) => return Ok(effects),
                Step::Ask {
                    state,
                    questions,
                    reasonings,
                } => (state, questions, reasonings),
            };
            let started = std::time::Instant::now();
            let result = match self.judge(SYSTEM_ONE) {
                _ if questions.is_empty() => Ok(Vec::new()),
                Some(judge) => {
                    judge
                        .judge(&Docket {
                            state: &state,
                            questions: &questions,
                            reasonings: &[],
                            timeout: Duration::MAX,
                        })
                        .answers
                }
                None => Err("no System One provider".to_string()),
            };
            let reasoned = match self.judge(SYSTEM_TWO) {
                Some(judge) if !reasonings.is_empty() => {
                    judge
                        .judge(&Docket {
                            state: &state,
                            questions: &[],
                            reasonings: &reasonings,
                            timeout: Duration::MAX,
                        })
                        .reasoned
                }
                _ => reasonings
                    .iter()
                    .map(|request| {
                        (
                            request.id.clone(),
                            system_two::run_attempt(None, request, Duration::MAX),
                        )
                    })
                    .collect(),
            };
            let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            step = self
                .resume_attempts(run, result, reasoned, elapsed)
                .unwrap_or(Step::Done(Vec::new()));
        }
    }

    /// Start a signal, abandoning the one [`Engine::begin`] started before.
    /// Hosts that call System One themselves ask the returned questions about
    /// the returned state, then call [`Engine::finish`].
    pub fn begin(&mut self, signal: &Signal) -> Result<Step, String> {
        if let Some(solo) = self.solo.take() {
            self.cancel(solo);
        }

        let (run, step) = self.start(signal)?;

        if matches!(step, Step::Ask { .. }) {
            self.solo = Some(run);
        }

        Ok(step)
    }

    /// Start a signal as its own run, beside any others in flight. Ask the
    /// returned step's questions and reasonings, then call
    /// [`Engine::resume`] with the run; a step that is done has no run to
    /// resume.
    ///
    /// # Errors
    ///
    /// An invalid signal; an agent with a run already in flight, since one
    /// agent's signals are judged in order; or [`MAX_RUNS`] in flight.
    pub fn start(&mut self, signal: &Signal) -> Result<(RunId, Step), String> {
        self.trace = Trace::default();
        signal.validate()?;

        if let Some(run) = self.busy(&signal.agent_id) {
            return Err(format!("{} is still judging {run}", signal.agent_id));
        }
        if self.runs.len() >= MAX_RUNS {
            return Err(format!("{MAX_RUNS} signals already in flight"));
        }

        self.next_run += 1;
        let id = RunId(self.next_run);
        let mut trace = Trace::default();

        self.record(signal);

        // Irreversible harm is vetoed before any capability or model call.
        if let Some(pattern) = self.backstop.veto(signal) {
            trace.veto = Some(pattern.to_string());
            self.trace = trace;

            return Ok((
                id,
                Step::Done(vec![Effect::Permission {
                    agent_id: signal.agent_id.clone(),
                    decision: PermissionDecision::Deny,
                    message: Some(self.backstop.blocked_message(pattern)),
                }]),
            ));
        }

        // A request you chose to confirm yourself asks, whatever a contract says.
        if let Some(pattern) = self.backstop.confirm(signal) {
            trace.veto = Some(pattern.to_string());
            self.trace = trace;

            return Ok((
                id,
                Step::Done(vec![Effect::Permission {
                    agent_id: signal.agent_id.clone(),
                    decision: PermissionDecision::Ask,
                    message: Some(self.backstop.confirm_message(pattern)),
                }]),
            ));
        }

        let situation = self
            .situations
            .get(&signal.agent_id)
            .cloned()
            .unwrap_or_default();
        let (settled, asks) = self.plan(id, &situation, signal);
        let harm = learning::harm(signal, &self.learning.texts);

        if asks.is_empty() && harm.is_none() {
            self.release(id);
            return Ok((id, Step::Done(settled)));
        }

        let (command, harm) = harm.unzip();
        let mut questions = self.namespaced(&asks);

        questions.extend(harm);

        // One numbering across state and questions, so a value keeps its label.
        let mut masking = Masking::default();
        let state = self.redactor.redact(
            &situation.render(&self.situation_texts, self.state_budget(&questions)),
            &mut masking,
        );
        let mut questions = self.redacted(questions, &mut masking);
        let reasonings = self.reasonings(&asks, &state, &mut masking);

        questions.extend(learning::secrets(&masking, &self.learning.texts));
        trace.questions = questions
            .iter()
            .map(|question| TracedQuestion::new(question, 1))
            .collect();

        let split = self.rule_locally(&state, questions, reasonings, 1, &mut trace);
        let to_host = !split.questions.is_empty() || !split.reasonings.is_empty();

        self.runs.insert(
            id,
            Run {
                signal: signal.clone(),
                situation,
                settled,
                asks,
                probe: Probe::new(command, &masking),
                round: 1,
                irreversible: false,
                questions: split.questions.clone(),
                reasonings: split.reasonings.clone(),
                local: split.local,
                trace,
            },
        );

        // Local judges answered everything: nothing to ask the host.
        if !to_host {
            let step = self
                .resume_attempts(id, Ok(Vec::new()), Vec::new(), 0)
                .unwrap_or(Step::Done(Vec::new()));

            return Ok((id, step));
        }

        Ok((
            id,
            Step::Ask {
                state,
                questions: split.questions,
                reasonings: split.reasonings,
            },
        ))
    }

    /// Hand each ask routed to a local judge to that judge, with each judge's
    /// asks in one call, and keep their rulings for the round. Questions for
    /// System One and reasonings for System Two are left for the host. An ask
    /// routed to a judge the engine does not hold goes unanswered: a question
    /// reads as unanswered and a reasoning as unavailable.
    fn rule_locally(
        &mut self,
        state: &str,
        questions: Vec<Question>,
        reasonings: Vec<Reasoning>,
        round: usize,
        trace: &mut Trace,
    ) -> Split {
        let mut host = Split {
            questions: Vec::new(),
            reasonings: Vec::new(),
            local: Local::default(),
        };
        let mut dockets: BTreeMap<String, (Vec<Question>, Vec<Reasoning>)> = BTreeMap::new();

        for question in questions {
            let judge = self.routes.question(&question.id).to_string();

            if self.is_local(&judge) {
                dockets.entry(judge).or_default().0.push(question);
            } else if judge == SYSTEM_ONE {
                host.questions.push(question);
            } else {
                trace
                    .error
                    .get_or_insert(format!("{}: no judge {judge}", question.id));
            }
        }

        for request in reasonings {
            let judge = self.routes.reasoning(&request.id).to_string();

            if self.is_local(&judge) {
                dockets.entry(judge).or_default().1.push(request);
            } else if judge == SYSTEM_TWO {
                host.reasonings.push(request);
            } else {
                let outcome: Reasoned =
                    Err(ReasoningFailure::Unavailable(format!("no judge {judge}")));

                trace
                    .reasonings
                    .push(TracedReasoning::new(&request, &outcome, round));
                host.local.reasoned.push((request.id, outcome));
            }
        }

        for (name, (questions, reasonings)) in dockets {
            let Some(judge) = self.judge(&name) else {
                continue;
            };
            let rulings = judge.judge(&Docket {
                state,
                questions: &questions,
                reasonings: &reasonings,
                timeout: LOCAL_TIMEOUT,
            });

            match rulings.answers.and_then(|answers| {
                validate_some_answers(&questions, &answers)
                    .map(|()| answers)
                    .map_err(|error| error.0)
            }) {
                Ok(answers) => {
                    trace.answers.extend(
                        answers
                            .iter()
                            .map(|answer| TracedAnswer::new(answer, round)),
                    );
                    host.local.answers.extend(answers);
                }
                Err(error) => {
                    trace.error.get_or_insert(format!("{name}: {error}"));
                }
            }

            let mut reasoned: HashMap<String, ReasoningAttempt> =
                rulings.reasoned.into_iter().collect();

            for request in &reasonings {
                let outcome = reasoned
                    .remove(&request.id)
                    .map_or_else(
                        || Err(ReasoningFailure::Unavailable("no outcome".into())),
                        |attempt| attempt.outcome,
                    )
                    .and_then(|verdict| {
                        system_two::validate_verdict(request, &verdict).map(|()| verdict)
                    });

                trace
                    .reasonings
                    .push(TracedReasoning::new(request, &outcome, round));
                host.local.reasoned.push((request.id.clone(), outcome));
            }
        }

        host
    }

    /// Abandon `run`: no capability acts on it, and a later result for it is
    /// ignored. Returns what it had asked and received so far.
    pub fn cancel(&mut self, run: RunId) -> Option<Trace> {
        if self.solo == Some(run) {
            self.solo = None;
        }

        let mut trace = self.runs.remove(&run)?.trace;

        self.release(run);
        trace.error.get_or_insert_with(|| "cancelled".into());

        Some(trace)
    }

    /// Every capability forgets `run`.
    fn release(&mut self, run: RunId) {
        for capability in &mut self.capabilities {
            capability.cancel(run);
        }
    }

    /// Every capability's reasonings, IDs namespaced and text redacted with
    /// the round's numbering; those that ask for it get the redacted `state`
    /// as evidence [`ACTIVITY`].
    fn reasonings(
        &self,
        asks: &[(usize, Inference)],
        state: &str,
        masking: &mut Masking,
    ) -> Vec<Reasoning> {
        let mut reasonings = Vec::new();

        for (index, inference) in asks {
            let prefix = self.prefix(*index);

            for request in &inference.reasonings {
                let mut evidence = Vec::with_capacity(request.evidence.len() + 1);

                if request.activity && !request.evidence.iter().any(|piece| piece.id == ACTIVITY) {
                    evidence.push(Evidence {
                        id: ACTIVITY.into(),
                        text: state.to_string(),
                    });
                }
                for piece in &request.evidence {
                    evidence.push(Evidence {
                        id: piece.id.clone(),
                        text: self.redactor.redact(&piece.text, masking),
                    });
                }

                reasonings.push(Reasoning {
                    id: format!("{prefix}{}", request.id),
                    instructions: self.redactor.redact(&request.instructions, masking),
                    evidence,
                    ..request.clone()
                });
            }
        }

        reasonings
    }

    /// Characters the state may use this round: its own bound, less what the
    /// questions already ask. One request counts against the provider's input
    /// limit, so a round that asks at length gets a shorter window to be read
    /// in. `MIN_STATE_CHARS` is always left, since the request being judged is
    /// the newest entry.
    fn state_budget(&self, questions: &[Question]) -> usize {
        let asked: usize = questions.iter().map(asked_chars).sum();

        self.situation_texts
            .max_chars
            .saturating_sub(asked)
            .max(MIN_STATE_CHARS)
    }

    /// Questions with their text redacted: capability questions can quote
    /// the agent's input.
    fn redacted(&self, questions: Vec<Question>, masking: &mut Masking) -> Vec<Question> {
        questions
            .into_iter()
            .map(|question| {
                let kind = match question.kind {
                    QuestionKind::Choice { options } => QuestionKind::Choice {
                        options: options
                            .into_iter()
                            .map(|option| crate::system_one::ChoiceOption {
                                description: self.redactor.redact(&option.description, masking),
                                ..option
                            })
                            .collect(),
                    },
                    other => other,
                };

                Question {
                    instructions: self.redactor.redact(&question.instructions, masking),
                    kind,
                    ..question
                }
            })
            .collect()
    }

    /// Finish a round of System One alone; another may be ready when a
    /// capability depends on its answer.
    pub fn finish(&mut self, result: Result<Vec<Answer>, String>, elapsed_ms: u64) -> Step {
        self.finish_round(result, Vec::new(), elapsed_ms)
    }

    /// Finish the round [`Engine::begin`] asked; another may be ready when a
    /// capability depends on its outcome. Without a run in flight, nothing
    /// is done.
    pub fn finish_round(
        &mut self,
        result: Result<Vec<Answer>, String>,
        reasoned: Vec<(String, Reasoned)>,
        elapsed_ms: u64,
    ) -> Step {
        let Some(run) = self.solo else {
            return Step::Done(Vec::new());
        };
        let step = self
            .resume(run, result, reasoned, elapsed_ms)
            .unwrap_or(Step::Done(Vec::new()));

        if matches!(step, Step::Done(_)) {
            self.solo = None;
        }

        step
    }

    /// Continue `run` with System One's result and each reasoning's outcome
    /// by ID. A round that asked System One nothing ignores `result`; a
    /// reasoning without an outcome is unavailable, and a verdict is checked
    /// against its request again before any capability reads it. `None` for
    /// a run not in flight: it settled, was cancelled, or this result is a
    /// duplicate; nothing changes.
    pub fn resume(
        &mut self,
        run: RunId,
        result: Result<Vec<Answer>, String>,
        reasoned: Vec<(String, Reasoned)>,
        elapsed_ms: u64,
    ) -> Option<Step> {
        self.resume_attempts(
            run,
            result,
            reasoned
                .into_iter()
                .map(|(id, outcome)| (id, outcome.into()))
                .collect(),
            elapsed_ms,
        )
    }

    /// Continue a hosted run, retaining each attempt's execution metadata in
    /// its trace while capabilities read validated outcomes only. A next
    /// round that local judges answer whole continues at once.
    pub fn resume_attempts(
        &mut self,
        run: RunId,
        result: Result<Vec<Answer>, String>,
        reasoned: Vec<(String, ReasoningAttempt)>,
        elapsed_ms: u64,
    ) -> Option<Step> {
        let mut step = self.resume_round(run, result, reasoned, elapsed_ms)?;

        while let Step::Ask {
            questions,
            reasonings,
            ..
        } = &step
            && questions.is_empty()
            && reasonings.is_empty()
        {
            step = self.resume_round(run, Ok(Vec::new()), Vec::new(), 0)?;
        }

        Some(step)
    }

    fn resume_round(
        &mut self,
        run: RunId,
        result: Result<Vec<Answer>, String>,
        reasoned: Vec<(String, ReasoningAttempt)>,
        elapsed_ms: u64,
    ) -> Option<Step> {
        let mut pending = self.runs.remove(&run)?;

        pending.trace.elapsed_ms = pending.trace.elapsed_ms.saturating_add(elapsed_ms);
        let answers = checked_answers(&mut pending, result);
        let mut reasoned = checked(&mut pending, reasoned);
        let local = std::mem::take(&mut pending.local);

        reasoned.extend(local.reasoned);
        if pending.round == 1 {
            pending.irreversible = answers.as_deref().is_some_and(|answers| {
                let (learned, irreversible) = self.learn(&pending.probe, answers);

                pending.trace.learned = learned;
                irreversible
            });
        }
        let round = Round {
            answers: answers.as_deref(),
            reasoned: &reasoned,
        };
        let (effects, next) = self.advance(
            run,
            &pending.signal,
            &pending.asks,
            &round,
            &local.answers,
            pending.round,
        );
        pending.settled.extend(effects);

        // A failed System One call ends System One's rounds; a next round
        // that asks System One nothing may still run.
        let next_allowed = answers.is_some()
            || next.iter().all(|(index, inference)| {
                let prefix = self.prefix(*index);

                inference
                    .questions
                    .iter()
                    .all(|question| !self.host_answers(&format!("{prefix}{}", question.id)))
            });

        if !next.is_empty() && pending.round < MAX_ROUNDS && next_allowed {
            pending.asks = next;
            pending.round += 1;
            let mut masking = Masking::default();
            let asked = self.namespaced(&pending.asks);
            let state = self.redactor.redact(
                &pending
                    .situation
                    .render(&self.situation_texts, self.state_budget(&asked)),
                &mut masking,
            );
            let questions = self.redacted(asked, &mut masking);
            let reasonings = self.reasonings(&pending.asks, &state, &mut masking);
            pending.trace.questions.extend(
                questions
                    .iter()
                    .map(|question| TracedQuestion::new(question, pending.round)),
            );
            let split = self.rule_locally(
                &state,
                questions,
                reasonings,
                pending.round,
                &mut pending.trace,
            );
            pending.questions = split.questions.clone();
            pending.reasonings = split.reasonings.clone();
            pending.local = split.local;
            self.runs.insert(run, pending);
            return Some(Step::Ask {
                state,
                questions: split.questions,
                reasonings: split.reasonings,
            });
        }

        // A command judged irreversible is denied, whatever a contract said.
        if pending.irreversible {
            pending
                .settled
                .retain(|effect| !matches!(effect, Effect::Permission { .. }));
            pending.settled.push(Effect::Permission {
                agent_id: pending.signal.agent_id.clone(),
                decision: PermissionDecision::Deny,
                message: Some(self.backstop.learned_message()),
            });
        }

        self.release(run);
        self.trace = pending.trace;

        Some(Step::Done(pending.settled))
    }

    /// Grow the safety lists from the core answers: what was learned, and
    /// whether the command was judged irreversible. Valid answers are kept
    /// even if their run is later cancelled; they judged the command, not
    /// the run.
    fn learn(&mut self, probe: &Probe, answers: &[Answer]) -> (Vec<String>, bool) {
        let (learned, irreversible) = probe.learn(
            &self.learning,
            answers,
            &mut self.backstop,
            &mut self.redactor,
        );

        self.learned_changed |= !learned.is_empty();

        (learned, irreversible)
    }

    fn record(&mut self, signal: &Signal) {
        // A skill review is no agent's activity: it leaves no memory behind.
        if matches!(signal.kind, SignalKind::SkillReview { .. }) {
            return;
        }

        if !self.situations.contains_key(&signal.agent_id) && self.situations.len() >= MAX_AGENTS {
            self.evict_oldest();
        }

        self.situations
            .entry(signal.agent_id.clone())
            .or_default()
            .record(signal, &self.situation_texts);
    }

    fn evict_oldest(&mut self) {
        let oldest = self
            .situations
            .iter()
            .min_by_key(|(_, situation)| situation.last_at())
            .map(|(id, _)| id.clone());

        if let Some(id) = oldest {
            self.situations.remove(&id);
        }
    }

    /// Settled effects, plus each asking capability's index and inference.
    fn plan(
        &mut self,
        run: RunId,
        situation: &Situation,
        signal: &Signal,
    ) -> (Vec<Effect>, Vec<(usize, Inference)>) {
        let mut effects = Vec::new();
        let mut asks = Vec::new();

        for (index, capability) in self.capabilities.iter_mut().enumerate() {
            let inference = match capability.start(run, situation, signal) {
                Plan::Skip => continue,
                Plan::Settled(settled) => {
                    effects.extend(settled);
                    continue;
                }
                Plan::Ask(questions) => Inference::from(questions),
                Plan::Infer(inference) => inference,
            };

            if !inference.is_empty() {
                asks.push((index, inference));
            }
        }

        (effects, asks)
    }

    /// Every capability's questions in one list, IDs namespaced by capability.
    fn namespaced(&self, asks: &[(usize, Inference)]) -> Vec<Question> {
        asks.iter()
            .flat_map(|(index, inference)| {
                let prefix = self.prefix(*index);

                inference.questions.iter().map(move |question| Question {
                    id: format!("{prefix}{}", question.id),
                    ..question.clone()
                })
            })
            .collect()
    }

    /// Hand each asking capability its own outcomes, IDs restored: System
    /// One's answers with local judges' answers beside them. One that asked
    /// System One nothing this round is not told of its failure.
    fn advance(
        &mut self,
        run: RunId,
        signal: &Signal,
        asks: &[(usize, Inference)],
        round: &Round<'_>,
        local_answers: &[Answer],
        number: usize,
    ) -> (Vec<Effect>, Vec<(usize, Inference)>) {
        let mut effects = Vec::new();
        let mut next = Vec::new();

        for (index, inference) in asks {
            let prefix = self.prefix(*index);
            let own = |answers: &[Answer]| -> Vec<Answer> {
                answers
                    .iter()
                    .filter_map(|answer| {
                        let id = answer.id.strip_prefix(&prefix)?;

                        Some(Answer {
                            id: id.to_string(),
                            ..answer.clone()
                        })
                    })
                    .collect()
            };
            let asked_host = inference
                .questions
                .iter()
                .any(|question| self.host_answers(&format!("{prefix}{}", question.id)));
            let local: Option<Vec<Answer>> = if asked_host {
                round.answers.map(|answers| {
                    let mut answers = own(answers);

                    answers.extend(own(local_answers));
                    answers
                })
            } else {
                Some(own(local_answers))
            };
            let reasoned: Vec<(String, Reasoned)> = round
                .reasoned
                .iter()
                .filter_map(|(id, outcome)| {
                    Some((id.strip_prefix(&prefix)?.to_string(), outcome.clone()))
                })
                .collect();
            let outcome = Round {
                answers: local.as_deref(),
                reasoned: &reasoned,
            };

            if let Some(capability) = self.capabilities.get_mut(*index) {
                let inference = match capability.resume(run, signal, &outcome, number) {
                    PipeStep::Done(done) => {
                        effects.extend(done);
                        continue;
                    }
                    PipeStep::Next(questions) => Inference::from(questions),
                    PipeStep::Infer(inference) => inference,
                };

                if !inference.is_empty() {
                    next.push((*index, inference));
                }
            }
        }

        (effects, next)
    }

    fn prefix(&self, index: usize) -> String {
        self.capabilities
            .get(index)
            .map_or_else(String::new, |capability| format!("{}/", capability.id()))
    }
}

/// Accept only answers matching the run's questions and trace their outcome.
fn checked_answers(run: &mut Run, result: Result<Vec<Answer>, String>) -> Option<Vec<Answer>> {
    let result = if run.questions.is_empty() {
        Ok(Vec::new())
    } else {
        result.and_then(|answers| {
            validate_answers(&run.questions, &answers)
                .map_err(|error| error.0)
                .map(|()| answers)
        })
    };
    match result {
        Ok(answers) => {
            run.trace.answers.extend(
                answers
                    .iter()
                    .map(|answer| TracedAnswer::new(answer, run.round)),
            );
            Some(answers)
        }
        Err(error) => {
            run.trace.error = Some(error);
            None
        }
    }
}

/// The characters one question spends in the request, its options included.
fn asked_chars(question: &Question) -> usize {
    let options: usize = match &question.kind {
        QuestionKind::Choice { options } => options
            .iter()
            .map(|option| option.value.chars().count() + option.description.chars().count())
            .sum(),
        QuestionKind::Score { levels } => levels.iter().map(|level| level.chars().count()).sum(),
        QuestionKind::Noul => 0,
    };

    question.id.chars().count() + question.instructions.chars().count() + options
}

/// Each of `run`'s reasonings with its outcome: a verdict checked against
/// its request again, unavailable when none came, and traced. Outcomes for
/// anything not asked are dropped.
fn checked(run: &mut Run, reasoned: Vec<(String, ReasoningAttempt)>) -> Vec<(String, Reasoned)> {
    let mut reasoned: HashMap<String, ReasoningAttempt> = reasoned.into_iter().collect();

    run.reasonings
        .iter()
        .map(|request| {
            let attempt = reasoned
                .remove(&request.id)
                .unwrap_or_else(|| Err(ReasoningFailure::Unavailable("no outcome".into())).into());
            let outcome = attempt.outcome.and_then(|verdict| {
                system_two::validate_verdict(request, &verdict).map(|()| verdict)
            });

            let traced =
                TracedReasoning::new(request, &outcome, run.round).with_metadata(attempt.metadata);
            run.trace.reasonings.push(traced);

            (request.id.clone(), outcome)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::judge::Facts;
    use crate::signal::SignalKind;
    use crate::system_one::{Answer, AnswerValue, QuestionKind, SystemOneError};

    struct Recorder {
        calls: Arc<Mutex<Vec<usize>>>,
        fail: bool,
    }

    impl SystemOne for Recorder {
        fn name(&self) -> &str {
            "recorder"
        }

        fn ask(&mut self, _: &str, questions: &[Question]) -> Result<Vec<Answer>, SystemOneError> {
            self.calls.lock().unwrap().push(questions.len());

            if self.fail {
                return Err(SystemOneError("down".into()));
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

    /// A harmless effect that names who produced it.
    fn keep(agent_id: &str) -> Effect {
        Effect::Model {
            agent_id: agent_id.into(),
            model: None,
        }
    }

    struct Probe {
        id: &'static str,
        interested: bool,
    }

    impl Capability for Probe {
        fn id(&self) -> &str {
            self.id
        }

        fn start(&mut self, _: RunId, _: &Situation, _: &Signal) -> Plan {
            if !self.interested {
                return Plan::Skip;
            }

            Plan::Ask(vec![
                Question {
                    id: "a".into(),
                    instructions: "?".into(),
                    kind: QuestionKind::Noul,
                    facts: Facts::new(),
                },
                Question {
                    id: "b".into(),
                    instructions: "?".into(),
                    kind: QuestionKind::Noul,
                    facts: Facts::new(),
                },
            ])
        }

        fn resume(&mut self, _: RunId, _: &Signal, round: &Round<'_>, _: usize) -> PipeStep {
            let ids = round.answers.map_or_else(
                || "none".to_string(),
                |answers| {
                    answers
                        .iter()
                        .map(|answer| answer.id.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                },
            );

            PipeStep::Done(vec![keep(&format!("{}:{ids}", self.id))])
        }
    }

    fn engine(fail: bool, calls: &Arc<Mutex<Vec<usize>>>) -> Engine {
        let capabilities: Vec<Box<dyn Capability>> = vec![
            Box::new(Probe {
                id: "one",
                interested: true,
            }),
            Box::new(Probe {
                id: "two",
                interested: true,
            }),
            Box::new(Probe {
                id: "off",
                interested: false,
            }),
        ];

        Engine::new(
            Box::new(Recorder {
                calls: Arc::clone(calls),
                fail,
            }),
            capabilities,
        )
        .unwrap()
    }

    fn signal() -> Signal {
        Signal {
            agent_id: "agent".into(),
            at: 1,
            kind: SignalKind::ToolResult {
                tool: "bash".into(),
                ok: true,
                workspace: String::new(),
                subagent: false,
                input: String::new(),
                error: String::new(),
                user_request: String::new(),
                evidence: String::new(),
                candidates: Vec::new(),
            },
        }
    }

    #[test]
    fn interested_capabilities_share_one_call() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let effects = engine(false, &calls).ingest(&signal()).unwrap();

        assert_eq!(*calls.lock().unwrap(), vec![4]);
        assert_eq!(effects, vec![keep("one:a,b"), keep("two:a,b")]);
    }

    /// Plain code as a judge: yes to every yes/no question, in place.
    struct Always;

    impl Judge for Always {
        fn name(&self) -> &str {
            "always"
        }

        fn local(&self) -> bool {
            true
        }

        fn judge(&mut self, docket: &Docket<'_>) -> crate::judge::Rulings {
            crate::judge::Rulings {
                answers: Ok(docket
                    .questions
                    .iter()
                    .map(|question| Answer {
                        id: question.id.clone(),
                        value: AnswerValue::Noul(1.0),
                        confidence: Some(1.0),
                    })
                    .collect()),
                reasoned: Vec::new(),
            }
        }
    }

    fn routed(fail: bool, calls: &Arc<Mutex<Vec<usize>>>, routes: &[(&str, &str)]) -> Engine {
        engine(fail, calls)
            .with_judge(Box::new(Always))
            .with_routes(Routes::new(
                routes
                    .iter()
                    .map(|(id, judge)| ((*id).to_string(), (*judge).to_string()))
                    .collect(),
            ))
    }

    #[test]
    fn a_question_routed_to_a_local_judge_is_answered_in_place_beside_system_one() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let effects = routed(false, &calls, &[("one/b", "always")])
            .ingest(&signal())
            .unwrap();

        // System One is asked only the three questions still routed to it.
        assert_eq!(*calls.lock().unwrap(), vec![3]);
        assert_eq!(effects, vec![keep("one:a,b"), keep("two:a,b")]);
    }

    #[test]
    fn a_capability_answered_wholly_by_a_local_judge_never_waits_on_system_one() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let routes = [
            ("one/a", "always"),
            ("one/b", "always"),
            ("two/a", "always"),
            ("two/b", "always"),
        ];
        let effects = routed(true, &calls, &routes).ingest(&signal()).unwrap();

        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(effects, vec![keep("one:a,b"), keep("two:a,b")]);

        // System One down: a capability that also asked it applies its
        // failure posture; one answered locally does not.
        let calls = Arc::new(Mutex::new(Vec::new()));
        let effects = routed(true, &calls, &routes[..2])
            .ingest(&signal())
            .unwrap();

        assert_eq!(effects, vec![keep("one:a,b"), keep("two:none")]);
    }

    #[test]
    fn a_question_routed_to_a_judge_the_engine_lacks_goes_unanswered() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut engine = routed(false, &calls, &[("one/a", "missing")]);
        let effects = engine.ingest(&signal()).unwrap();

        assert_eq!(effects, vec![keep("one:b"), keep("two:a,b")]);
        assert!(
            engine
                .trace()
                .error
                .as_deref()
                .unwrap()
                .contains("no judge missing")
        );
    }

    /// One request counts against the provider's input limit, so a round that
    /// asks at length leaves a shorter window of activity to read it in.
    #[test]
    fn a_round_that_asks_at_length_gets_a_shorter_state() {
        struct Lengths(Arc<Mutex<Vec<usize>>>);

        impl SystemOne for Lengths {
            fn name(&self) -> &str {
                "lengths"
            }

            fn ask(
                &mut self,
                state: &str,
                questions: &[Question],
            ) -> Result<Vec<Answer>, SystemOneError> {
                self.0.lock().unwrap().push(state.len());

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

        struct Ask {
            id: &'static str,
            chars: usize,
        }

        impl Capability for Ask {
            fn id(&self) -> &str {
                self.id
            }

            fn start(&mut self, _: RunId, _: &Situation, _: &Signal) -> Plan {
                Plan::Ask(vec![Question {
                    id: "q".into(),
                    instructions: "why ".repeat(self.chars),
                    kind: QuestionKind::Noul,
                    facts: Facts::new(),
                }])
            }

            fn resume(&mut self, _: RunId, _: &Signal, _: &Round<'_>, _: usize) -> PipeStep {
                PipeStep::Done(Vec::new())
            }
        }

        fn state_len(id: &'static str, chars: usize) -> usize {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let texts = SituationTexts {
                max_chars: 8_000,
                ..SituationTexts::default()
            };
            let mut engine = Engine::new(
                Box::new(Lengths(Arc::clone(&seen))),
                vec![Box::new(Ask { id, chars })],
            )
            .unwrap()
            .with_situation_texts(texts);

            for at in 1..=32 {
                let mut signal = signal();
                signal.at = at;

                if let SignalKind::ToolResult { input, .. } = &mut signal.kind {
                    *input = "x".repeat(1_000);
                }

                engine.record(&signal);
            }

            engine.ingest(&signal()).unwrap();

            seen.lock().unwrap()[0]
        }

        let quiet = state_len("quiet", 10);
        let loud = state_len("loud", 7_000);

        assert!(quiet > 7_000, "{quiet} characters of activity");
        assert!(loud <= MIN_STATE_CHARS, "{loud} characters");
        assert!(loud < quiet / 2, "{loud} against {quiet}");
    }

    #[test]
    fn classifier_failure_reaches_each_capability_posture() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let effects = engine(true, &calls).ingest(&signal()).unwrap();

        assert_eq!(effects.len(), 2);
        assert!(effects.contains(&keep("one:none")));
    }

    #[test]
    fn duplicate_capability_ids_are_rejected() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let capabilities: Vec<Box<dyn Capability>> = vec![
            Box::new(Probe {
                id: "one",
                interested: true,
            }),
            Box::new(Probe {
                id: "one",
                interested: true,
            }),
        ];

        assert!(Engine::new(Box::new(Recorder { calls, fail: false }), capabilities).is_err());
    }

    #[test]
    fn situations_survive_a_save_and_load_and_unknown_versions_are_ignored() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut before = engine(false, &calls);
        before.record(&signal());

        let saved = before.save();
        let mut after = engine(false, &calls);
        after.load(saved.clone());
        let texts = SituationTexts::default();
        assert_eq!(
            after.situations["agent"].render(&texts, texts.max_chars),
            before.situations["agent"].render(&texts, texts.max_chars)
        );

        let mut stale = saved;
        stale["version"] = serde_json::json!(0);
        let mut fresh = engine(false, &calls);
        fresh.load(stale);
        assert!(fresh.situations.is_empty());
    }

    #[test]
    fn each_ingest_leaves_a_trace_of_what_was_asked_and_answered() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut ok = engine(false, &calls);
        ok.ingest(&signal()).unwrap();

        let ids: Vec<&str> = ok
            .trace()
            .questions
            .iter()
            .map(|question| question.id.as_str())
            .collect();
        assert_eq!(ids, vec!["one/a", "one/b", "two/a", "two/b"]);
        assert_eq!(ok.trace().answers.len(), 4);
        assert!(ok.trace().error.is_none());

        let mut failing = engine(true, &calls);
        failing.ingest(&signal()).unwrap();
        assert_eq!(failing.trace().error.as_deref(), Some("down"));
        assert!(failing.trace().answers.is_empty());
    }

    #[test]
    fn a_hosted_engine_asks_its_host_then_finishes_with_the_answers() {
        let capabilities: Vec<Box<dyn Capability>> = vec![Box::new(Probe {
            id: "one",
            interested: true,
        })];
        let mut engine = Engine::hosted(capabilities).unwrap();

        let Step::Ask {
            state, questions, ..
        } = engine.begin(&signal()).unwrap()
        else {
            panic!("expected questions")
        };
        assert!(state.contains("Recent agent activity"));
        assert_eq!(questions.len(), 2);

        let answers = questions
            .iter()
            .map(|question| Answer {
                id: question.id.clone(),
                value: AnswerValue::Noul(0.9),
                confidence: None,
            })
            .collect();
        let Step::Done(effects) = engine.finish(Ok(answers), 42) else {
            panic!("expected done")
        };

        assert_eq!(effects, vec![keep("one:a,b")]);
        assert_eq!(engine.trace().elapsed_ms, 42);
        // Nothing is pending once finished.
        assert_eq!(engine.finish(Err("late".into()), 0), Step::Done(Vec::new()));
    }

    struct FollowUp;

    impl Capability for FollowUp {
        fn id(&self) -> &str {
            "follow-up"
        }

        fn start(&mut self, _: RunId, _: &Situation, _: &Signal) -> Plan {
            Plan::Ask(vec![Question {
                id: "first".into(),
                instructions: "first?".into(),
                kind: QuestionKind::Noul,
                facts: Facts::new(),
            }])
        }

        fn resume(
            &mut self,
            _: RunId,
            signal: &Signal,
            outcome: &Round<'_>,
            round: usize,
        ) -> PipeStep {
            let answers = outcome.answers;

            if round == 1 && answers.is_some_and(|answers| matches!(answers.first(), Some(Answer { value: AnswerValue::Noul(p), .. }) if *p > 0.7)) {
                return PipeStep::Next(vec![Question { id: "second".into(), instructions: "second?".into(), kind: QuestionKind::Noul, facts: Facts::new() }]);
            }

            PipeStep::Done(
                (round == 2 && answers.is_some())
                    .then(|| keep(&signal.agent_id))
                    .into_iter()
                    .collect(),
            )
        }
    }

    #[test]
    fn dependent_round_is_batched_only_when_requested_and_failures_settle() {
        let mut engine = Engine::hosted(vec![Box::new(FollowUp)]).unwrap();
        let Step::Ask { questions, .. } = engine.begin(&signal()).unwrap() else {
            panic!("expected first round")
        };
        assert_eq!(questions[0].id, "follow-up/first");
        let Step::Ask { questions, .. } = engine.finish(
            Ok(vec![Answer {
                id: questions[0].id.clone(),
                value: AnswerValue::Noul(0.9),
                confidence: None,
            }]),
            4,
        ) else {
            panic!("expected second round")
        };
        assert_eq!(questions[0].id, "follow-up/second");
        assert_eq!(
            engine.finish(Err("timeout".into()), 5),
            Step::Done(Vec::new())
        );
        assert_eq!(engine.trace().questions.len(), 2);
        assert_eq!(engine.trace().questions[1].round, 2);
        assert_eq!(engine.trace().answers[0].round, 1);
        assert_eq!(engine.trace().elapsed_ms, 9);

        let Step::Ask { questions, .. } = engine.begin(&signal()).unwrap() else {
            panic!("expected first round")
        };
        assert_eq!(
            engine.finish(
                Ok(vec![Answer {
                    id: questions[0].id.clone(),
                    value: AnswerValue::Noul(0.2),
                    confidence: None,
                }]),
                1
            ),
            Step::Done(Vec::new())
        );
        assert_eq!(engine.trace().questions.len(), 1);
    }

    #[test]
    fn an_irreversible_shell_command_is_denied_and_learned_for_the_next_request() {
        let mut engine = Engine::hosted(Vec::new()).unwrap();
        let command = "psql -c 'drop database production'";
        let request = Signal {
            agent_id: "agent".into(),
            at: 1,
            kind: SignalKind::PermissionRequest {
                action: "shell".into(),
                resources: vec![crate::signal::Resource {
                    requested: command.into(),
                    resolved: command.into(),
                }],
                request: command.into(),
                workspace: String::new(),
                user_requests: Vec::new(),
                host_decision: crate::effect::PermissionDecision::Allow,
            },
        };

        let Step::Ask { questions, .. } = engine.begin(&request).unwrap() else {
            panic!("expected the harm question")
        };
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].id, "core/irreversible");
        let Step::Done(denied) = engine.finish(
            Ok(vec![Answer {
                id: "core/irreversible".into(),
                value: AnswerValue::Noul(0.95),
                confidence: Some(0.9),
            }]),
            1,
        ) else {
            panic!("expected done")
        };

        assert!(matches!(
            denied.as_slice(),
            [Effect::Permission {
                decision: PermissionDecision::Deny,
                ..
            }]
        ));
        assert_eq!(engine.learned().0, vec![command]);
        assert!(engine.take_learned_changed());
        assert!(!engine.take_learned_changed());
        assert!(matches!(engine.begin(&request).unwrap(), Step::Done(_)));
    }
}
