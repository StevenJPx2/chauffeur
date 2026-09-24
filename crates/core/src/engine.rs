//! Sense → Classify → Act: one System One call per signal across every
//! interested capability.

use std::collections::{HashMap, HashSet};

use crate::backstop::Backstop;
use crate::capability::{Capability, Plan};
use crate::effect::{Effect, PermissionDecision};
use crate::signal::Signal;
use crate::situation::Situation;
use crate::system_one::{Answer, Question, SystemOne};
use crate::trace::{Trace, TracedAnswer, TracedQuestion};

pub const MAX_AGENTS: usize = 256;
/// Bumped when the saved state's shape changes; older state is ignored.
const STATE_VERSION: u32 = 1;

pub struct Engine {
    system_one: Option<Box<dyn SystemOne>>,
    capabilities: Vec<Box<dyn Capability>>,
    situations: HashMap<String, Situation>,
    backstop: Backstop,
    trace: Trace,
    pending: Option<Pending>,
}

/// What one signal needs next.
#[derive(Debug, PartialEq)]
pub enum Step {
    /// Decided without System One.
    Done(Vec<Effect>),
    /// Ask System One these questions about this state, then finish.
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
            pending: None,
        })
    }

    /// Replace the built-in-only backstop, for example with project patterns.
    #[must_use]
    pub fn with_backstop(mut self, backstop: Backstop) -> Self {
        self.backstop = backstop;
        self
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
        let (state, questions) = match self.begin(signal)? {
            Step::Done(effects) => return Ok(effects),
            Step::Ask { state, questions } => (state, questions),
        };
        let started = std::time::Instant::now();
        let result = match self.system_one.as_mut() {
            Some(system_one) => system_one.ask(&state, &questions).map_err(|error| error.0),
            None => Err("no System One provider".to_string()),
        };
        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        Ok(self.finish(result, elapsed))
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

        let situation = self
            .situations
            .get(&signal.agent_id)
            .cloned()
            .unwrap_or_default();
        let (settled, asks) = self.plan(&situation, signal);

        if asks.is_empty() {
            return Ok(Step::Done(settled));
        }

        let questions = self.namespaced(&asks);

        self.trace.questions = questions.iter().map(TracedQuestion::new).collect();
        self.pending = Some(Pending {
            signal: signal.clone(),
            settled,
            asks,
        });

        Ok(Step::Ask {
            state: situation.render(),
            questions,
        })
    }

    /// Finish the signal [`Engine::begin`] started, with System One's answers
    /// or its error; each capability applies its failure posture on error.
    pub fn finish(&mut self, result: Result<Vec<Answer>, String>, elapsed_ms: u64) -> Vec<Effect> {
        let Some(Pending {
            signal,
            mut settled,
            asks,
        }) = self.pending.take()
        else {
            return Vec::new();
        };

        self.trace.elapsed_ms = elapsed_ms;

        let answers = match result {
            Ok(answers) => {
                self.trace.answers = answers.iter().map(TracedAnswer::new).collect();
                Some(answers)
            }
            Err(error) => {
                self.trace.error = Some(error);
                None
            }
        };

        settled.extend(self.decide(&signal, &asks, answers.as_deref()));
        settled
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
    fn decide(
        &mut self,
        signal: &Signal,
        asks: &[(usize, Vec<Question>)],
        answers: Option<&[Answer]>,
    ) -> Vec<Effect> {
        let mut effects = Vec::new();

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
                effects.extend(capability.decide(signal, local.as_deref()));
            }
        }

        effects
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

            vec![Effect::KeepModel {
                agent_id: format!("{}:{ids}", self.id),
            }]
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
                input: String::new(),
                error: String::new(),
            },
        }
    }

    #[test]
    fn interested_capabilities_share_one_call() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let effects = engine(false, &calls).ingest(&signal()).unwrap();

        assert_eq!(*calls.lock().unwrap(), vec![4]);
        assert_eq!(
            effects,
            vec![
                Effect::KeepModel {
                    agent_id: "one:a,b".into()
                },
                Effect::KeepModel {
                    agent_id: "two:a,b".into()
                },
            ]
        );
    }

    #[test]
    fn classifier_failure_reaches_each_capability_posture() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let effects = engine(true, &calls).ingest(&signal()).unwrap();

        assert_eq!(effects.len(), 2);
        assert!(effects.contains(&Effect::KeepModel {
            agent_id: "one:none".into()
        }));
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
        let effects = engine.finish(Ok(answers), 42);

        assert_eq!(
            effects,
            vec![Effect::KeepModel {
                agent_id: "one:a,b".into()
            }]
        );
        assert_eq!(engine.trace().elapsed_ms, 42);
        // Nothing is pending once finished.
        assert!(engine.finish(Err("late".into()), 0).is_empty());
    }
}
