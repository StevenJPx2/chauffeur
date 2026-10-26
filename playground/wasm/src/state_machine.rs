//! The State machine tab: build a plan step by step from the page, then drive
//! the core state machine one round at a time. Steps for the `person` judge are
//! answered by hand; steps for a rule judge are answered by that rule.

use chauffeur_core::{
    Gate, Judge, Next, Outcome, Panel, Plan, PlanError, Question, Resolution, Round, Step,
    Transition, Verdict,
};
use wasm_bindgen::prelude::{JsError, wasm_bindgen};

/// The page's outcomes are strings; this one is the core's distinct failure.
const FAILED: &str = "failed";

/// The judge whose answers are picked on the page.
const PERSON: &str = "person";

fn outcome(value: String) -> Outcome<String> {
    if value == FAILED {
        Outcome::Failed
    } else {
        Outcome::Value(value)
    }
}

/// A deterministic judge: `yes` if the request mentions a TODO, `no` if it
/// says done, otherwise `unsure`.
struct Keywords;

impl Judge<String, String> for Keywords {
    fn judge(&self, request: &String) -> Outcome<String> {
        let lower = request.to_ascii_lowercase();
        let answer = if lower.contains("todo") {
            "yes"
        } else if lower.contains("done") {
            "no"
        } else {
            "unsure"
        };
        Outcome::Value(answer.to_owned())
    }
}

static KEYWORDS: Keywords = Keywords;

/// The rule judges the page seats beside `person`.
fn rules() -> Panel<'static, String, String> {
    Panel::new().with("keywords", &KEYWORDS)
}

/// What a rule judge answers to `question`, or `None` for a `person` step.
fn ruling(
    rules: &Panel<'_, String, String>,
    question: &Question<'_, String>,
) -> Option<Outcome<String>> {
    rules
        .get(question.judge)
        .map(|judge| judge.judge(question.request))
}

enum Stage {
    Building(Vec<Step<String, String>>),
    Asking(Round<String, String>),
    Done(Verdict<String>),
}

/// One run of one plan, as the page sees it.
#[wasm_bindgen]
pub struct Session {
    stage: Stage,
    events: Vec<String>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl Session {
    /// An empty plan, ready for steps.
    #[wasm_bindgen(constructor)]
    #[must_use]
    #[allow(
        clippy::missing_const_for_fn,
        reason = "wasm-bindgen cannot export a const fn"
    )]
    pub fn new() -> Self {
        Self {
            stage: Stage::Building(Vec::new()),
            events: Vec::new(),
        }
    }

    /// Append a step to the plan, answered by `judge`.
    ///
    /// # Errors
    ///
    /// The plan has already started.
    pub fn add_step(&mut self, id: String, judge: String, request: String) -> Result<(), JsError> {
        let Stage::Building(steps) = &mut self.stage else {
            return Err(JsError::new("the plan has already started"));
        };
        steps.push(Step::new(id, judge, request));
        Ok(())
    }

    /// Gate the most recently added step. `mode` is `any` or `all`.
    ///
    /// # Errors
    ///
    /// The mode is unknown, no step has been added, or the plan has started.
    pub fn add_gate(
        &mut self,
        mode: &str,
        sources: Vec<String>,
        on: Vec<String>,
    ) -> Result<(), JsError> {
        let on = on.into_iter().map(outcome);
        let gate = match mode {
            "any" => Gate::any(sources, on),
            "all" => Gate::all(sources, on),
            _ => return Err(JsError::new(&format!("unknown gate mode `{mode}`"))),
        };
        let Stage::Building(steps) = &mut self.stage else {
            return Err(JsError::new("the plan has already started"));
        };
        let step = steps
            .last_mut()
            .ok_or_else(|| JsError::new("add a step before its gates"))?;
        step.gates.push(gate);
        Ok(())
    }

    /// Validate the plan and run it to its first round.
    ///
    /// # Errors
    ///
    /// The plan is invalid, names a judge the page lacks, or has already
    /// started.
    pub fn start(&mut self) -> Result<(), JsError> {
        let Stage::Building(steps) = &mut self.stage else {
            return Err(JsError::new("the plan has already started"));
        };
        let rules = rules();
        if let Some(step) = steps
            .iter()
            .find(|step| step.judge != PERSON && rules.get(&step.judge).is_none())
        {
            let error = PlanError::UnknownJudge {
                step: step.id.clone(),
                judge: step.judge.clone(),
            };
            return Err(JsError::new(&error.to_string()));
        }
        let plan = Plan::new(core::mem::take(steps));
        let transition = plan
            .start()
            .map_err(|error| JsError::new(&error.to_string()))?;
        self.apply(transition);
        Ok(())
    }

    /// Answer the current round: `outcomes[i]` answers `steps[i]`, a `person`
    /// step; the rule judges answer the rest.
    ///
    /// # Errors
    ///
    /// No round is waiting, or the core rejected the reply; the round stays
    /// open to answer again.
    pub fn answer(&mut self, steps: Vec<String>, outcomes: Vec<String>) -> Result<(), JsError> {
        let round = self.take_round()?;
        let rules = rules();
        let by_rule: Vec<(String, Outcome<String>)> = round
            .questions()
            .filter_map(|question| {
                ruling(&rules, &question).map(|answer| (question.step.to_owned(), answer))
            })
            .collect();
        let by_hand = steps.into_iter().zip(outcomes.into_iter().map(outcome));
        match round.answer(by_hand.chain(by_rule)) {
            Ok(transition) => {
                self.apply(transition);
                Ok(())
            }
            Err(rejected) => {
                self.stage = Stage::Asking(rejected.round);
                Err(JsError::new(&rejected.error.to_string()))
            }
        }
    }

    /// Fail every step of the current round, as a failed provider call would.
    ///
    /// # Errors
    ///
    /// No round is waiting.
    pub fn fail_round(&mut self) -> Result<(), JsError> {
        let round = self.take_round()?;
        self.apply(round.fail());
        Ok(())
    }

    /// Every event so far, rendered for people.
    #[must_use]
    pub fn events(&self) -> Vec<String> {
        self.events.clone()
    }

    /// The current round's number, or 0 when no round is waiting.
    #[must_use]
    #[allow(
        clippy::missing_const_for_fn,
        reason = "wasm-bindgen cannot export a const fn"
    )]
    pub fn round(&self) -> u32 {
        match &self.stage {
            Stage::Asking(round) => round.number(),
            Stage::Building(_) | Stage::Done(_) => 0,
        }
    }

    /// The IDs of the steps the current round asks.
    #[must_use]
    pub fn asked(&self) -> Vec<String> {
        self.questions(|question| question.step.to_owned())
    }

    /// The judges of the steps the current round asks, matching [`Self::asked`].
    #[must_use]
    pub fn judges(&self) -> Vec<String> {
        self.questions(|question| question.judge.to_owned())
    }

    /// The requests of the steps the current round asks, matching [`Self::asked`].
    #[must_use]
    pub fn requests(&self) -> Vec<String> {
        self.questions(|question| question.request.clone())
    }

    /// What each rule judge answers in the current round, matching
    /// [`Self::asked`]; empty for a `person` step.
    #[must_use]
    pub fn rulings(&self) -> Vec<String> {
        let rules = rules();
        self.questions(|question| {
            ruling(&rules, &question).map_or_else(String::new, |answer| answer.to_string())
        })
    }

    /// Each step's ID once the plan has finished; empty before.
    #[must_use]
    pub fn verdict_steps(&self) -> Vec<String> {
        self.verdict(|(id, _)| id.to_owned())
    }

    /// Each step's resolution, matching [`Self::verdict_steps`].
    #[must_use]
    pub fn verdict_resolutions(&self) -> Vec<String> {
        self.verdict(|(_, resolution)| resolution.to_string())
    }
}

impl Session {
    fn apply(&mut self, transition: Transition<String, String>) {
        self.events
            .extend(transition.events.iter().map(ToString::to_string));
        self.stage = match transition.next {
            Next::Ask(round) => Stage::Asking(round),
            Next::Done(verdict) => Stage::Done(verdict),
        };
    }

    /// Take the waiting round out; the caller puts the next stage back.
    fn take_round(&mut self) -> Result<Round<String, String>, JsError> {
        match core::mem::replace(&mut self.stage, Stage::Building(Vec::new())) {
            Stage::Asking(round) => Ok(round),
            stage => {
                self.stage = stage;
                Err(JsError::new("no round is waiting for answers"))
            }
        }
    }

    fn questions(&self, field: impl Fn(Question<'_, String>) -> String) -> Vec<String> {
        match &self.stage {
            Stage::Asking(round) => round.questions().map(field).collect(),
            Stage::Building(_) | Stage::Done(_) => Vec::new(),
        }
    }

    fn verdict(&self, field: impl Fn((&str, &Resolution<String>)) -> String) -> Vec<String> {
        match &self.stage {
            Stage::Done(verdict) => verdict.iter().map(field).collect(),
            Stage::Building(_) | Stage::Asking(_) => Vec::new(),
        }
    }
}
