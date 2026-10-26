//! Running a plan: validate it, then advance one round per answer.

use core::fmt;

use crate::event::{Event, Resolution, Verdict};
use crate::gate::{Change, Link, StepState, resolve};
use crate::judge::Panel;
use crate::plan::{Gate, Outcome, Plan};

/// Why a plan cannot start.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlanError {
    /// The plan has no steps.
    Empty,
    /// Two steps share this ID.
    DuplicateStep(String),
    /// A gate on `step` names `source`, which is not an earlier step.
    NotPreceding { step: String, source: String },
    /// A gate on `step` names no sources.
    EmptySources { step: String },
    /// A gate on `step` matches no outcomes.
    EmptyOutcomes { step: String },
    /// `step` names `judge`, which the panel does not have.
    UnknownJudge { step: String, judge: String },
}

/// Why a round rejected a reply. The round is unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AnswerError {
    /// No step has this ID.
    UnknownStep(String),
    /// The step exists but this round did not ask it.
    NotAsked(String),
    /// The reply answers this step more than once.
    Duplicate(String),
    /// The reply does not answer this asked step.
    Missing(String),
}

/// The result of a transition: what to do next, and what happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition<Q, O> {
    pub next: Next<Q, O>,
    pub events: Vec<Event<O>>,
}

/// What the caller does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next<Q, O> {
    /// Answer these steps.
    Ask(Round<Q, O>),
    /// The plan has finished.
    Done(Verdict<O>),
}

impl<Q, O> Next<Q, O> {
    /// The round to answer, if the plan has not finished.
    #[must_use]
    pub fn ask(self) -> Option<Round<Q, O>> {
        match self {
            Self::Ask(round) => Some(round),
            Self::Done(_) => None,
        }
    }

    /// The verdict, if the plan has finished.
    #[must_use]
    pub fn done(self) -> Option<Verdict<O>> {
        match self {
            Self::Done(verdict) => Some(verdict),
            Self::Ask(_) => None,
        }
    }
}

/// Steps awaiting answers. Answering consumes the round, so it is answered once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round<Q, O> {
    machine: Machine<Q, O>,
}

/// A rejected reply, with the round to answer again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected<Q, O> {
    pub round: Round<Q, O>,
    pub error: AnswerError,
}

/// One asked step in a round.
#[derive(Debug, PartialEq, Eq)]
pub struct Question<'a, Q> {
    /// The step's ID; a reply names the step by it.
    pub step: &'a str,
    /// The name of the judge the step asks.
    pub judge: &'a str,
    pub request: &'a Q,
}

/// A plan run to its end by a [`Panel`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished<O> {
    pub verdict: Verdict<O>,
    /// Every event of the run, in order.
    pub events: Vec<Event<O>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Node<Q> {
    id: String,
    judge: String,
    request: Q,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Machine<Q, O> {
    nodes: Vec<Node<Q>>,
    gates: Vec<Vec<Link<O>>>,
    states: Vec<StepState<O>>,
    round: u32,
}

impl<Q, O: Clone + PartialEq> Plan<Q, O> {
    /// Validate the plan and run it up to its first round.
    ///
    /// # Errors
    ///
    /// The plan is empty, repeats a step ID, or has a gate that names no
    /// sources, no outcomes, or a step that is not before its own.
    pub fn start(self) -> Result<Transition<Q, O>, PlanError> {
        let machine = Machine::compile(self)?;
        let steps = machine.nodes.len();
        Ok(machine.advance(vec![Event::Started { steps }]))
    }

    /// Run the plan to its end, asking each step's judge on `panel`. Each
    /// round, every judge answers its share of the round in one
    /// [`judge_all`](crate::Judge::judge_all) call.
    ///
    /// # Errors
    ///
    /// The plan is invalid, as for [`Plan::start`], or names a judge the panel
    /// does not have. Either is reported before any judge is asked.
    pub fn run(self, panel: &Panel<'_, Q, O>) -> Result<Finished<O>, PlanError> {
        let machine = Machine::compile(self)?;
        if let Some(node) = machine
            .nodes
            .iter()
            .find(|node| panel.get(&node.judge).is_none())
        {
            return Err(PlanError::UnknownJudge {
                step: node.id.clone(),
                judge: node.judge.clone(),
            });
        }
        let steps = machine.nodes.len();
        let mut transition = machine.advance(vec![Event::Started { steps }]);
        let mut events = Vec::new();
        // Ends within one round per step; `proofs` gives the argument.
        loop {
            events.append(&mut transition.events);
            match transition.next {
                Next::Done(verdict) => return Ok(Finished { verdict, events }),
                Next::Ask(Round { machine }) => {
                    let slots = machine.judge(panel);
                    transition = machine.settle(slots);
                }
            }
        }
    }
}

impl<Q, O: Clone + PartialEq> Round<Q, O> {
    /// This round's number, counting from 1.
    #[must_use]
    pub const fn number(&self) -> u32 {
        self.machine.round
    }

    /// The asked steps, in plan order.
    pub fn questions(&self) -> impl Iterator<Item = Question<'_, Q>> {
        let machine = &self.machine;
        machine
            .nodes
            .iter()
            .zip(&machine.states)
            .filter(|(_, state)| matches!(state, StepState::Asked))
            .map(|(node, _)| Question {
                step: &node.id,
                judge: &node.judge,
                request: &node.request,
            })
    }

    /// Settle every asked step from `reply`, which must answer each exactly once.
    ///
    /// # Errors
    ///
    /// The reply names an unknown or unasked step, repeats one, or misses one.
    /// The rejection returns the unchanged round.
    pub fn answer<K: AsRef<str>>(
        self,
        reply: impl IntoIterator<Item = (K, Outcome<O>)>,
    ) -> Result<Transition<Q, O>, Rejected<Q, O>> {
        match self.machine.collect(reply) {
            Ok(slots) => Ok(self.machine.settle(slots)),
            Err(error) => Err(Rejected { round: self, error }),
        }
    }

    /// Settle every asked step as failed, as when the provider call fails.
    #[must_use]
    pub fn fail(self) -> Transition<Q, O> {
        let slots = self
            .machine
            .states
            .iter()
            .map(|state| matches!(state, StepState::Asked).then_some(Outcome::Failed))
            .collect();
        self.machine.settle(slots)
    }
}

impl<Q, O: Clone + PartialEq> Machine<Q, O> {
    fn compile(plan: Plan<Q, O>) -> Result<Self, PlanError> {
        let steps = plan.into_steps();
        if steps.is_empty() {
            return Err(PlanError::Empty);
        }
        let mut nodes: Vec<Node<Q>> = Vec::with_capacity(steps.len());
        let mut gates = Vec::with_capacity(steps.len());
        for step in steps {
            if nodes.iter().any(|node| node.id == step.id) {
                return Err(PlanError::DuplicateStep(step.id));
            }
            let links = step
                .gates
                .into_iter()
                .map(|gate| link(&nodes, &step.id, gate))
                .collect::<Result<Vec<_>, _>>()?;
            gates.push(links);
            nodes.push(Node {
                id: step.id,
                judge: step.judge,
                request: step.request,
            });
        }
        let states = nodes.iter().map(|_| StepState::Pending).collect();
        Ok(Self {
            nodes,
            gates,
            states,
            round: 0,
        })
    }

    fn id(&self, index: usize) -> String {
        self.nodes
            .get(index)
            .map(|node| node.id.clone())
            .unwrap_or_default()
    }

    /// Resolve pending steps, then ask the next round or finish.
    fn advance(mut self, mut events: Vec<Event<O>>) -> Transition<Q, O> {
        let mut asked = Vec::new();
        for change in resolve(&self.gates, &mut self.states) {
            match change {
                Change::Skipped(index) => events.push(Event::StepSkipped {
                    step: self.id(index),
                }),
                Change::Asked(index) => asked.push(self.id(index)),
            }
        }
        if asked.is_empty() {
            events.push(Event::Finished { rounds: self.round });
            return Transition {
                next: Next::Done(self.verdict()),
                events,
            };
        }
        self.round = self.round.saturating_add(1);
        events.push(Event::RoundReady {
            round: self.round,
            steps: asked,
        });
        Transition {
            next: Next::Ask(Round { machine: self }),
            events,
        }
    }

    /// Match a reply to the asked steps: one outcome per asked step, by index.
    fn collect<K: AsRef<str>>(
        &self,
        reply: impl IntoIterator<Item = (K, Outcome<O>)>,
    ) -> Result<Vec<Option<Outcome<O>>>, AnswerError> {
        let mut slots: Vec<Option<Outcome<O>>> = self.states.iter().map(|_| None).collect();
        for (key, outcome) in reply {
            let name = key.as_ref();
            let index = self
                .nodes
                .iter()
                .position(|node| node.id == name)
                .ok_or_else(|| AnswerError::UnknownStep(name.to_owned()))?;
            if !matches!(self.states.get(index), Some(StepState::Asked)) {
                return Err(AnswerError::NotAsked(name.to_owned()));
            }
            match slots.get_mut(index) {
                Some(slot @ None) => *slot = Some(outcome),
                _ => return Err(AnswerError::Duplicate(name.to_owned())),
            }
        }
        let missing = self
            .states
            .iter()
            .zip(&slots)
            .position(|(state, slot)| matches!(state, StepState::Asked) && slot.is_none());
        missing.map_or(Ok(slots), |index| Err(AnswerError::Missing(self.id(index))))
    }

    /// Answer every asked step from `panel`, one batch per judge, in the order
    /// each judge first appears. A batch whose judge is missing, or which
    /// returns the wrong number of outcomes, settles as failed.
    fn judge(&self, panel: &Panel<'_, Q, O>) -> Vec<Option<Outcome<O>>> {
        let mut slots: Vec<Option<Outcome<O>>> = self.states.iter().map(|_| None).collect();
        let asked: Vec<&Node<Q>> = self
            .nodes
            .iter()
            .zip(&self.states)
            .filter(|(_, state)| matches!(state, StepState::Asked))
            .map(|(node, _)| node)
            .collect();
        for (first, lead) in asked.iter().enumerate() {
            if asked
                .iter()
                .take(first)
                .any(|node| node.judge == lead.judge)
            {
                continue;
            }
            let batch: Vec<&Node<Q>> = asked
                .iter()
                .copied()
                .filter(|node| node.judge == lead.judge)
                .collect();
            let requests: Vec<&Q> = batch.iter().map(|node| &node.request).collect();
            let outcomes = panel
                .get(&lead.judge)
                .map(|judge| judge.judge_all(&requests))
                .filter(|outcomes| outcomes.len() == batch.len())
                .unwrap_or_else(|| batch.iter().map(|_| Outcome::Failed).collect());
            for (node, outcome) in batch.into_iter().zip(outcomes) {
                let index = self.nodes.iter().position(|each| each.id == node.id);
                if let Some(slot) = index.and_then(|index| slots.get_mut(index)) {
                    *slot = Some(outcome);
                }
            }
        }
        slots
    }

    fn settle(mut self, slots: Vec<Option<Outcome<O>>>) -> Transition<Q, O> {
        let mut events = Vec::new();
        for ((node, state), slot) in self.nodes.iter().zip(&mut self.states).zip(slots) {
            if let Some(outcome) = slot {
                events.push(Event::StepSettled {
                    round: self.round,
                    step: node.id.clone(),
                    outcome: outcome.clone(),
                });
                *state = StepState::Settled(outcome);
            }
        }
        self.advance(events)
    }

    /// Every step is terminal once no step is asked (proven in `proofs`).
    fn verdict(self) -> Verdict<O> {
        let steps = self
            .nodes
            .into_iter()
            .zip(self.states)
            .filter_map(|(node, state)| {
                let resolution = match state {
                    StepState::Settled(outcome) => Resolution::Settled(outcome),
                    StepState::Skipped => Resolution::Skipped,
                    StepState::Pending | StepState::Asked => return None,
                };
                Some((node.id, resolution))
            })
            .collect();
        Verdict { steps }
    }
}

/// Resolve a gate's source names to indices of steps already in `nodes`.
fn link<Q, O>(nodes: &[Node<Q>], step: &str, gate: Gate<O>) -> Result<Link<O>, PlanError> {
    if gate.sources.is_empty() {
        return Err(PlanError::EmptySources {
            step: step.to_owned(),
        });
    }
    if gate.on.is_empty() {
        return Err(PlanError::EmptyOutcomes {
            step: step.to_owned(),
        });
    }
    let sources = gate
        .sources
        .into_iter()
        .map(|source| {
            nodes
                .iter()
                .position(|node| node.id == source)
                .ok_or_else(|| PlanError::NotPreceding {
                    step: step.to_owned(),
                    source,
                })
        })
        .collect::<Result<_, _>>()?;
    Ok(Link {
        mode: gate.mode,
        sources,
        on: gate.on,
    })
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("the plan has no steps"),
            Self::DuplicateStep(id) => write!(f, "step `{id}` appears more than once"),
            Self::NotPreceding { step, source } => {
                write!(
                    f,
                    "step `{step}` gates on `{source}`, which is not an earlier step"
                )
            }
            Self::EmptySources { step } => write!(f, "a gate on `{step}` names no sources"),
            Self::EmptyOutcomes { step } => write!(f, "a gate on `{step}` matches no outcomes"),
            Self::UnknownJudge { step, judge } => {
                write!(
                    f,
                    "step `{step}` asks judge `{judge}`, which the panel lacks"
                )
            }
        }
    }
}

impl fmt::Display for AnswerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownStep(id) => write!(f, "no step is called `{id}`"),
            Self::NotAsked(id) => write!(f, "step `{id}` is not asked in this round"),
            Self::Duplicate(id) => write!(f, "step `{id}` is answered more than once"),
            Self::Missing(id) => write!(f, "step `{id}` has no answer"),
        }
    }
}

impl core::error::Error for PlanError {}
impl core::error::Error for AnswerError {}
