//! Sense → Classify → Act: one System One call per signal across every
//! interested capability.

use std::collections::{HashMap, HashSet};

use crate::backstop::Backstop;
use crate::capability::{Capability, Plan};
use crate::effect::{Effect, PermissionDecision};
use crate::signal::Signal;
use crate::situation::Situation;
use crate::system_one::{Answer, Question, SystemOne};

pub const MAX_AGENTS: usize = 256;
/// Bumped when the saved state's shape changes; older state is ignored.
const STATE_VERSION: u32 = 1;

pub struct Engine {
    system_one: Box<dyn SystemOne>,
    capabilities: Vec<Box<dyn Capability>>,
    situations: HashMap<String, Situation>,
    backstop: Backstop,
}

impl Engine {
    pub fn new(
        system_one: Box<dyn SystemOne>,
        capabilities: Vec<Box<dyn Capability>>,
    ) -> Result<Self, String> {
        let mut ids = HashSet::new();

        for capability in &capabilities {
            if !ids.insert(capability.id().to_string()) {
                return Err(format!("duplicate capability id {}", capability.id()));
            }
        }

        Ok(Self {
            system_one,
            capabilities,
            situations: HashMap::new(),
            backstop: Backstop::new(Vec::new()),
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

    pub fn ingest(&mut self, signal: &Signal) -> Result<Vec<Effect>, String> {
        signal.validate()?;
        self.record(signal);

        // Irreversible harm is vetoed before any capability or model call.
        if let Some(pattern) = self.backstop.veto(signal) {
            return Ok(vec![Effect::Permission {
                agent_id: signal.agent_id.clone(),
                decision: PermissionDecision::Deny,
                message: Some(format!(
                    "Chauffeur blocked an irreversible action (matched \"{pattern}\")."
                )),
            }]);
        }

        let situation = self
            .situations
            .get(&signal.agent_id)
            .cloned()
            .unwrap_or_default();
        let (mut effects, asks) = self.plan(&situation, signal);

        if asks.is_empty() {
            return Ok(effects);
        }

        effects.extend(self.classify(&situation, signal, asks));

        Ok(effects)
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

    /// One System One call for every question. IDs are namespaced by
    /// capability and restored before each capability decides.
    fn classify(
        &mut self,
        situation: &Situation,
        signal: &Signal,
        asks: Vec<(usize, Vec<Question>)>,
    ) -> Vec<Effect> {
        let questions: Vec<Question> = asks
            .iter()
            .flat_map(|(index, questions)| {
                let prefix = self.prefix(*index);

                questions.iter().map(move |question| Question {
                    id: format!("{prefix}{}", question.id),
                    ..question.clone()
                })
            })
            .collect();
        let answers = self.system_one.ask(&situation.render(), &questions).ok();
        let mut effects = Vec::new();

        for (index, _) in &asks {
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
}
