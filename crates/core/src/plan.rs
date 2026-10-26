//! Plans: named steps joined by gates into an ordered graph, and the helpers
//! that compose them.

/// What an asked step settled to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome<O> {
    /// An answer of the caller's outcome type.
    Value(O),
    /// The step was asked but no usable answer arrived.
    Failed,
}

/// How a gate combines its sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Run once one source matches; skip once all are terminal and none did.
    Any,
    /// Run once every source matches; skip once one is terminal and did not.
    All,
}

/// A condition on earlier steps that must pass before a step runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate<O> {
    /// How the sources combine.
    pub mode: Mode,
    /// IDs of steps that precede the gated step in the plan.
    pub sources: Vec<String>,
    /// The outcomes that count as a match.
    pub on: Vec<Outcome<O>>,
}

impl<O> Gate<O> {
    /// A gate that passes once any source settles to an outcome in `on`.
    #[must_use]
    pub fn any<S: Into<String>>(
        sources: impl IntoIterator<Item = S>,
        on: impl IntoIterator<Item = Outcome<O>>,
    ) -> Self {
        Self::new(Mode::Any, sources, on)
    }

    /// A gate that passes once every source settles to an outcome in `on`.
    #[must_use]
    pub fn all<S: Into<String>>(
        sources: impl IntoIterator<Item = S>,
        on: impl IntoIterator<Item = Outcome<O>>,
    ) -> Self {
        Self::new(Mode::All, sources, on)
    }

    fn new<S: Into<String>>(
        mode: Mode,
        sources: impl IntoIterator<Item = S>,
        on: impl IntoIterator<Item = Outcome<O>>,
    ) -> Self {
        Self {
            mode,
            sources: sources.into_iter().map(Into::into).collect(),
            on: on.into_iter().collect(),
        }
    }
}

/// One question in a plan. It runs at most once, and only when all its gates pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step<Q, O> {
    /// Unique within the plan; replies and gates name the step by it.
    pub id: String,
    /// The name of the [`Judge`](crate::Judge) that answers this step: a rule,
    /// a model or a person. A [`Panel`](crate::Panel) maps names to judges.
    pub judge: String,
    /// What the step asks, in the caller's terms.
    pub request: Q,
    /// All must pass for the step to run. An ungated step runs in round 1.
    pub gates: Vec<Gate<O>>,
}

impl<Q, O> Step<Q, O> {
    /// An ungated step `id`, asking `judge` this `request`.
    #[must_use]
    pub fn new(id: impl Into<String>, judge: impl Into<String>, request: Q) -> Self {
        Self {
            id: id.into(),
            judge: judge.into(),
            request,
            gates: Vec::new(),
        }
    }

    /// Add a gate; every gate on a step must pass.
    #[must_use]
    pub fn gated(mut self, gate: Gate<O>) -> Self {
        self.gates.push(gate);
        self
    }
}

/// The gate [`Plan::then_on`] puts on the following plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct When<O> {
    mode: Mode,
    on: Vec<Outcome<O>>,
}

impl<O> When<O> {
    /// Continue once any terminal step of the preceding plan matches.
    #[must_use]
    pub fn any(on: impl IntoIterator<Item = Outcome<O>>) -> Self {
        Self {
            mode: Mode::Any,
            on: on.into_iter().collect(),
        }
    }

    /// Continue once every terminal step of the preceding plan matches.
    #[must_use]
    pub fn all(on: impl IntoIterator<Item = Outcome<O>>) -> Self {
        Self {
            mode: Mode::All,
            on: on.into_iter().collect(),
        }
    }
}

/// Steps in order. A gate may only name steps before its own; [`Plan::start`]
/// checks this along with unique IDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan<Q, O> {
    steps: Vec<Step<Q, O>>,
}

/// A plan of one ungated step `id`, asking `judge` this `request`.
#[must_use]
pub fn ask<Q, O>(id: impl Into<String>, judge: impl Into<String>, request: Q) -> Plan<Q, O> {
    Plan::new([Step::new(id, judge, request)])
}

/// Independent plans side by side; their ready steps share rounds.
#[must_use]
pub fn all<Q, O>(plans: impl IntoIterator<Item = Plan<Q, O>>) -> Plan<Q, O> {
    Plan::new(plans.into_iter().flat_map(|plan| plan.steps))
}

impl<Q, O> Plan<Q, O> {
    /// A plan of these steps, in order.
    #[must_use]
    pub fn new(steps: impl IntoIterator<Item = Step<Q, O>>) -> Self {
        Self {
            steps: steps.into_iter().collect(),
        }
    }

    /// The steps, in order.
    #[must_use]
    pub fn steps(&self) -> &[Step<Q, O>] {
        &self.steps
    }

    /// The steps, in order.
    #[must_use]
    pub fn into_steps(self) -> Vec<Step<Q, O>> {
        self.steps
    }

    /// Append `next`, gating each of its ungated steps on this plan's terminal
    /// steps: those no other step's gate names.
    #[must_use]
    pub fn then_on(mut self, when: When<O>, next: Self) -> Self
    where
        O: Clone,
    {
        let When { mode, on } = when;
        let sources = self.terminals();
        self.steps.extend(next.steps.into_iter().map(|mut step| {
            if step.gates.is_empty() {
                step.gates.push(Gate {
                    mode,
                    sources: sources.clone(),
                    on: on.clone(),
                });
            }
            step
        }));
        self
    }

    /// Append `next`, run once any terminal step of this plan settles to `on`.
    #[must_use]
    pub fn then_if_any(self, on: Outcome<O>, next: Self) -> Self
    where
        O: Clone,
    {
        self.then_on(When::any([on]), next)
    }

    fn terminals(&self) -> Vec<String> {
        let named = |id: &String| {
            self.steps
                .iter()
                .flat_map(|step| &step.gates)
                .any(|gate| gate.sources.contains(id))
        };
        self.steps
            .iter()
            .map(|step| &step.id)
            .filter(|id| !named(id))
            .cloned()
            .collect()
    }
}
