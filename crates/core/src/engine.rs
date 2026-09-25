//! Sense → Classify → Act: batch each bounded round across capabilities.

use std::collections::{HashMap, HashSet};

use crate::backstop::Backstop;
use crate::capability::{Capability, PipeStep, Plan};
use crate::effect::{Effect, PermissionDecision};
use crate::learning::{self, Probe};
use crate::redact::{LearnedShapes, Masking, RedactionConfig, Redactor};
use crate::signal::Signal;
use crate::situation::Situation;
use crate::system_one::{Answer, Question, QuestionKind, SystemOne, validate_answers};
use crate::trace::{Trace, TracedAnswer, TracedQuestion};

pub const MAX_AGENTS: usize = 256;
const MAX_ROUNDS: usize = 2;
/// Bumped when the saved state's shape changes; older state is ignored.
const STATE_VERSION: u32 = 1;

pub struct Engine {
    system_one: Option<Box<dyn SystemOne>>,
    capabilities: Vec<Box<dyn Capability>>,
    situations: HashMap<String, Situation>,
    backstop: Backstop,
    redactor: Redactor,
    /// The backstop or redactor learned something since the host last asked.
    learned_changed: bool,
    trace: Trace,
    pending: Option<Pending>,
}

/// What one signal needs next.
#[derive(Debug, PartialEq)]
pub enum Step {
    /// Decided without System One.
    Done(Vec<Effect>),
    /// Ask System One these questions; finishing may produce another round.
    Ask {
        state: String,
        questions: Vec<Question>,
    },
}

/// A signal between [`Engine::begin`] and [`Engine::finish`].
struct Pending {
    signal: Signal,
    settled: Vec<Effect>,
    asks: Vec<(usize, Vec<Question>)>,
    probe: Probe,
    round: usize,
    irreversible: bool,
    questions: Vec<Question>,
}

impl Engine {
    pub fn new(
        system_one: Box<dyn SystemOne>,
        capabilities: Vec<Box<dyn Capability>>,
    ) -> Result<Self, String> {
        let mut engine = Self::hosted(capabilities)?;

        engine.system_one = Some(system_one);

        Ok(engine)
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
            system_one: None,
            capabilities,
            situations: HashMap::new(),
            trace: Trace::default(),
            backstop: Backstop::new(Vec::new()),
            redactor: Redactor::new(RedactionConfig::default(), LearnedShapes::default())?,
            learned_changed: false,
            pending: None,
        })
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

    /// What the last [`Engine::ingest`] asked and received, for auditing.
    #[must_use]
    pub fn trace(&self) -> &Trace {
        &self.trace
    }

    /// Run one signal to completion with the engine's own System One.
    pub fn ingest(&mut self, signal: &Signal) -> Result<Vec<Effect>, String> {
        let mut step = self.begin(signal)?;

        loop {
            let (state, questions) = match step {
                Step::Done(effects) => return Ok(effects),
                Step::Ask { state, questions } => (state, questions),
            };
            let started = std::time::Instant::now();
            let result = match self.system_one.as_mut() {
                Some(system_one) => system_one.ask(&state, &questions).map_err(|error| error.0),
                None => Err("no System One provider".to_string()),
            };
            let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            step = self.finish(result, elapsed);
        }
    }

    /// Start a signal. Hosts that call System One themselves ask the returned
    /// questions about the returned state, then call [`Engine::finish`].
    pub fn begin(&mut self, signal: &Signal) -> Result<Step, String> {
        self.trace = Trace::default();
        self.pending = None;
        signal.validate()?;
        self.record(signal);

        // Irreversible harm is vetoed before any capability or model call.
        if let Some(pattern) = self.backstop.veto(signal) {
            self.trace.veto = Some(pattern.to_string());

            return Ok(Step::Done(vec![Effect::Permission {
                agent_id: signal.agent_id.clone(),
                decision: PermissionDecision::Deny,
                message: Some(format!(
                    "Chauffeur blocked an irreversible action (matched \"{pattern}\")."
                )),
            }]));
        }

        // A request you chose to confirm yourself asks, whatever a contract says.
        if let Some(pattern) = self.backstop.confirm(signal) {
            self.trace.veto = Some(pattern.to_string());

            return Ok(Step::Done(vec![Effect::Permission {
                agent_id: signal.agent_id.clone(),
                decision: PermissionDecision::Ask,
                message: Some(format!(
                    "Chauffeur asks you to confirm this (matched \"{pattern}\")."
                )),
            }]));
        }

        let situation = self
            .situations
            .get(&signal.agent_id)
            .cloned()
            .unwrap_or_default();
        let (settled, asks) = self.plan(&situation, signal);
        let harm = learning::harm(signal);

        if asks.is_empty() && harm.is_none() {
            return Ok(Step::Done(settled));
        }

        let (command, harm) = harm.unzip();
        let mut questions = self.namespaced(&asks);

        questions.extend(harm);

        // One numbering across state and questions, so a value keeps its label.
        let mut masking = Masking::default();
        let state = self.redactor.redact(&situation.render(), &mut masking);
        let mut questions = self.redacted(questions, &mut masking);

        questions.extend(learning::secrets(&masking));
        self.trace.questions = questions
            .iter()
            .map(|question| TracedQuestion::new(question, 1))
            .collect();
        self.pending = Some(Pending {
            signal: signal.clone(),
            settled,
            asks,
            probe: Probe::new(command, &masking),
            round: 1,
            irreversible: false,
            questions: questions.clone(),
        });

        Ok(Step::Ask { state, questions })
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

    /// Finish one round; another may be ready when a capability depends on its answer.
    pub fn finish(&mut self, result: Result<Vec<Answer>, String>, elapsed_ms: u64) -> Step {
        let Some(mut pending) = self.pending.take() else {
            return Step::Done(Vec::new());
        };
        self.trace.elapsed_ms = self.trace.elapsed_ms.saturating_add(elapsed_ms);
        let result = result.and_then(|answers| {
            validate_answers(&pending.questions, &answers)
                .map_err(|error| error.0)
                .map(|()| answers)
        });
        let answers = match result {
            Ok(answers) => {
                self.trace.answers.extend(
                    answers
                        .iter()
                        .map(|answer| TracedAnswer::new(answer, pending.round)),
                );
                Some(answers)
            }
            Err(error) => {
                self.trace.error = Some(error);
                None
            }
        };
        if pending.round == 1 {
            pending.irreversible = answers
                .as_deref()
                .is_some_and(|answers| self.learn(&pending.probe, answers));
        }
        let (effects, next) = self.advance(
            &pending.signal,
            &pending.asks,
            answers.as_deref(),
            pending.round,
        );
        pending.settled.extend(effects);

        if !next.is_empty() && pending.round < MAX_ROUNDS && answers.is_some() {
            pending.asks = next;
            pending.round += 1;
            let mut masking = Masking::default();
            let situation = self
                .situations
                .get(&pending.signal.agent_id)
                .cloned()
                .unwrap_or_default();
            let state = self.redactor.redact(&situation.render(), &mut masking);
            let questions = self.redacted(self.namespaced(&pending.asks), &mut masking);
            self.trace.questions.extend(
                questions
                    .iter()
                    .map(|question| TracedQuestion::new(question, pending.round)),
            );
            pending.questions = questions.clone();
            self.pending = Some(pending);
            return Step::Ask { state, questions };
        }

        // A command judged irreversible is denied, whatever a contract said.
        if pending.irreversible {
            pending
                .settled
                .retain(|effect| !matches!(effect, Effect::Permission { .. }));
            pending.settled.push(Effect::Permission {
                agent_id: pending.signal.agent_id.clone(),
                decision: PermissionDecision::Deny,
                message: Some(
                    "Chauffeur blocked an irreversible action; it is now on the backstop list."
                        .into(),
                ),
            });
        }

        Step::Done(pending.settled)
    }

    /// Grow the safety lists from the core answers; `true` when the command
    /// was judged irreversible.
    fn learn(&mut self, probe: &Probe, answers: &[Answer]) -> bool {
        let (learned, irreversible) = probe.learn(answers, &mut self.backstop, &mut self.redactor);

        self.learned_changed |= !learned.is_empty();
        self.trace.learned = learned;

        irreversible
    }

    fn record(&mut self, signal: &Signal) {
        if !self.situations.contains_key(&signal.agent_id) && self.situations.len() >= MAX_AGENTS {
            self.evict_oldest();
        }

        self.situations
            .entry(signal.agent_id.clone())
            .or_default()
            .record(signal);
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

    /// Settled effects, plus each asking capability's index and questions.
    fn plan(
        &mut self,
        situation: &Situation,
        signal: &Signal,
    ) -> (Vec<Effect>, Vec<(usize, Vec<Question>)>) {
        let mut effects = Vec::new();
        let mut asks = Vec::new();

        for (index, capability) in self.capabilities.iter_mut().enumerate() {
            match capability.plan(situation, signal) {
                Plan::Skip => {}
                Plan::Settled(settled) => effects.extend(settled),
                Plan::Ask(questions) if questions.is_empty() => {}
                Plan::Ask(questions) => asks.push((index, questions)),
            }
        }

        (effects, asks)
    }

    /// Every capability's questions in one list, IDs namespaced by capability.
    fn namespaced(&self, asks: &[(usize, Vec<Question>)]) -> Vec<Question> {
        asks.iter()
            .flat_map(|(index, questions)| {
                let prefix = self.prefix(*index);

                questions.iter().map(move |question| Question {
                    id: format!("{prefix}{}", question.id),
                    ..question.clone()
                })
            })
            .collect()
    }

    /// Hand each asking capability its own answers, IDs restored.
    fn advance(
        &mut self,
        signal: &Signal,
        asks: &[(usize, Vec<Question>)],
        answers: Option<&[Answer]>,
        round: usize,
    ) -> (Vec<Effect>, Vec<(usize, Vec<Question>)>) {
        let mut effects = Vec::new();
        let mut next = Vec::new();

        for (index, _) in asks {
            let prefix = self.prefix(*index);
            let local: Option<Vec<Answer>> = answers.as_ref().map(|answers| {
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
            });

            if let Some(capability) = self.capabilities.get_mut(*index) {
                match capability.advance(signal, local.as_deref(), round) {
                    PipeStep::Done(done) => effects.extend(done),
                    PipeStep::Next(questions) if !questions.is_empty() => {
                        next.push((*index, questions))
                    }
                    PipeStep::Next(_) => {}
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
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

        fn plan(&mut self, _: &Situation, _: &Signal) -> Plan {
            if !self.interested {
                return Plan::Skip;
            }

            Plan::Ask(vec![
                Question {
                    id: "a".into(),
                    instructions: "?".into(),
                    kind: QuestionKind::Noul,
                },
                Question {
                    id: "b".into(),
                    instructions: "?".into(),
                    kind: QuestionKind::Noul,
                },
            ])
        }

        fn decide(&mut self, _: &Signal, answers: Option<&[Answer]>) -> Vec<Effect> {
            let ids = answers.map_or_else(
                || "none".to_string(),
                |answers| {
                    answers
                        .iter()
                        .map(|answer| answer.id.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                },
            );

            vec![keep(&format!("{}:{ids}", self.id))]
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
        assert_eq!(
            after.situations["agent"].render(),
            before.situations["agent"].render()
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

        let Step::Ask { state, questions } = engine.begin(&signal()).unwrap() else {
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

        fn plan(&mut self, _: &Situation, _: &Signal) -> Plan {
            Plan::Ask(vec![Question {
                id: "first".into(),
                instructions: "first?".into(),
                kind: QuestionKind::Noul,
            }])
        }

        fn decide(&mut self, _: &Signal, _: Option<&[Answer]>) -> Vec<Effect> {
            Vec::new()
        }

        fn advance(
            &mut self,
            signal: &Signal,
            answers: Option<&[Answer]>,
            round: usize,
        ) -> PipeStep {
            if round == 1 && answers.is_some_and(|answers| matches!(answers.first(), Some(Answer { value: AnswerValue::Noul(p), .. }) if *p > 0.7)) {
                return PipeStep::Next(vec![Question { id: "second".into(), instructions: "second?".into(), kind: QuestionKind::Noul }]);
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
