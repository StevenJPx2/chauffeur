//! Rule evaluation and reminder production.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::context::{AgentContext, Brief};
use crate::rule::Rule;
use crate::skill::{
    Branch, DecisionAnswer, DecisionQuestion, Effect, EvaluationStatus, Skill, SkillContext,
    SkillResult, missing_evidence, resolve_branch,
};

/// Reminders produced per idle event, at most.
pub const MAX_REMINDERS_PER_IDLE: usize = 2;
const MAX_SKILL_COOLDOWN_ENTRIES: usize = 4_096;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "lowercase")]
pub enum Urgency {
    Routine,
    Important,
    Urgent,
}

impl Urgency {
    #[must_use]
    pub fn from_score(score: f32) -> Self {
        if score < 0.5 {
            Self::Routine
        } else if score < 1.5 {
            Self::Important
        } else {
            Self::Urgent
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Verdict {
    pub rule_id: String,
    pub applies: f32,
    pub urgency: Urgency,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct Reminder {
    pub agent_id: String,
    pub rule_id: String,
    pub urgency: Urgency,
    pub text: String,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Error {
    Judge(String),
    Delivery(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Judge(detail) => write!(formatter, "judge failed: {detail}"),
            Self::Delivery(detail) => write!(formatter, "delivery failed: {detail}"),
        }
    }
}

impl std::error::Error for Error {}

pub trait Judge: Send {
    fn judge(&mut self, brief: &Brief, rules: &[&Rule]) -> Result<Vec<Verdict>, Error>;

    fn decide_skill(
        &mut self,
        _state: &str,
        _question: &DecisionQuestion,
    ) -> Result<DecisionAnswer, Error> {
        Err(Error::Judge(
            "typed skill decisions are not supported by this judge".into(),
        ))
    }
}

impl<T: Judge + ?Sized> Judge for Box<T> {
    fn judge(&mut self, brief: &Brief, rules: &[&Rule]) -> Result<Vec<Verdict>, Error> {
        (**self).judge(brief, rules)
    }

    fn decide_skill(
        &mut self,
        state: &str,
        question: &DecisionQuestion,
    ) -> Result<DecisionAnswer, Error> {
        (**self).decide_skill(state, question)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Fired {
    count: u32,
    last_at: u64,
}

/// Stateful evaluator. Delivery is supplied per call so one evaluator can
/// serve many targets while only recording successfully queued reminders.
pub struct Steerer<J> {
    rules: Vec<Rule>,
    judge: J,
    fired: HashMap<(String, String), Fired>,
    skill_fired: HashMap<(String, String), Fired>,
}

impl<J: Judge> Steerer<J> {
    #[must_use]
    pub fn new(rules: Vec<Rule>, judge: J) -> Self {
        Self {
            rules,
            judge,
            fired: HashMap::new(),
            skill_fired: HashMap::new(),
        }
    }

    pub fn on_idle(
        &mut self,
        context: &AgentContext,
        mut deliver: impl FnMut(&Reminder) -> Result<(), Error>,
    ) -> Result<Vec<Reminder>, Error> {
        if context.is_blocked() {
            return Ok(Vec::new());
        }

        let candidates = candidates(&self.rules, &self.fired, context);

        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let verdicts = self.judge.judge(&context.brief(), &candidates)?;
        let reminders = render_reminders(context, select(&candidates, &verdicts));

        for reminder in &reminders {
            deliver(reminder)?;
            record(&mut self.fired, context, reminder);
        }

        Ok(reminders)
    }

    pub fn evaluate_skill(
        &mut self,
        skill: &Skill,
        context: &SkillContext,
    ) -> Result<SkillResult, String> {
        skill.validate()?;
        context.validate()?;

        if !skill.match_conditions.events.contains(&context.event)
            || !skill.match_conditions.actions.contains(&context.action)
        {
            return Ok(empty_skill_result(skill, EvaluationStatus::Unmatched));
        }

        let missing = missing_evidence(skill, context);

        if !missing.is_empty() {
            let result = outcome_result(
                skill,
                EvaluationStatus::MissingEvidence,
                &skill.outcomes.missing_evidence,
                missing,
                None,
                None,
            );
            record_skill(&mut self.skill_fired, skill, context);

            return Ok(result);
        }

        if cooldown_active(&self.skill_fired, skill, context) {
            return Ok(outcome_result(
                skill,
                EvaluationStatus::Cooldown,
                &skill.outcomes.cooldown,
                Vec::new(),
                None,
                None,
            ));
        }

        ensure_skill_cooldown_capacity(&self.skill_fired, skill, context)?;

        let result = self.judge_skill(skill, context);

        record_skill(&mut self.skill_fired, skill, context);

        Ok(result)
    }

    fn judge_skill(&mut self, skill: &Skill, context: &SkillContext) -> SkillResult {
        let decision = self.judge.decide_skill(&context.state, &skill.decision);

        match decision {
            Ok(answer) => self.resolve_skill_answer(skill, answer),
            Err(_) => outcome_result(
                skill,
                EvaluationStatus::JudgeFailure,
                &skill.outcomes.judge_failure,
                Vec::new(),
                None,
                None,
            ),
        }
    }

    fn resolve_skill_answer(&self, skill: &Skill, answer: DecisionAnswer) -> SkillResult {
        let confidence = Some(answer.confidence);
        let branch = match resolve_branch(skill, &answer) {
            Ok(branch) => branch,
            Err(_) => {
                return outcome_result(
                    skill,
                    EvaluationStatus::JudgeFailure,
                    &skill.outcomes.judge_failure,
                    Vec::new(),
                    confidence,
                    None,
                );
            }
        };
        let outcome = match branch {
            Branch::Positive => &skill.outcomes.positive,
            Branch::Negative => &skill.outcomes.negative,
            Branch::Uncertain => &skill.outcomes.uncertain,
        };

        outcome_result(
            skill,
            EvaluationStatus::Evaluated,
            outcome,
            Vec::new(),
            confidence,
            Some(branch),
        )
    }
}

fn empty_skill_result(skill: &Skill, status: EvaluationStatus) -> SkillResult {
    SkillResult {
        skill_id: skill.identity.id.clone(),
        status,
        effect: None,
        message: None,
        reminder: None,
        missing_evidence: Vec::new(),
        confidence: None,
        branch: None,
    }
}

fn outcome_result(
    skill: &Skill,
    status: EvaluationStatus,
    outcome: &crate::skill::Outcome,
    missing: Vec<String>,
    confidence: Option<f32>,
    branch: Option<Branch>,
) -> SkillResult {
    let reminder = match outcome.effect {
        Effect::Prompt | Effect::Remind => Some(
            outcome
                .reminder
                .clone()
                .unwrap_or_else(|| outcome.message.clone()),
        ),
        Effect::Allow | Effect::Deny | Effect::Ask => outcome.reminder.clone(),
    };

    SkillResult {
        skill_id: skill.identity.id.clone(),
        status,
        effect: Some(outcome.effect),
        message: Some(outcome.message.clone()),
        reminder,
        missing_evidence: missing,
        confidence,
        branch,
    }
}

fn cooldown_active(
    fired: &HashMap<(String, String), Fired>,
    skill: &Skill,
    context: &SkillContext,
) -> bool {
    let key = (context.agent_id.clone(), skill.identity.id.clone());

    fired.get(&key).is_some_and(|previous| {
        context.occurred_at.saturating_sub(previous.last_at) < skill.cooldown_seconds
    })
}

fn record_skill(
    fired: &mut HashMap<(String, String), Fired>,
    skill: &Skill,
    context: &SkillContext,
) {
    if skill.cooldown_seconds == 0 {
        return;
    }

    let key = (context.agent_id.clone(), skill.identity.id.clone());

    if fired.len() >= MAX_SKILL_COOLDOWN_ENTRIES && !fired.contains_key(&key) {
        return;
    }

    let entry = fired.entry(key).or_default();
    entry.count = entry.count.saturating_add(1);
    entry.last_at = context.occurred_at;
}

fn ensure_skill_cooldown_capacity(
    fired: &HashMap<(String, String), Fired>,
    skill: &Skill,
    context: &SkillContext,
) -> Result<(), String> {
    if skill.cooldown_seconds == 0 {
        return Ok(());
    }

    let key = (context.agent_id.clone(), skill.identity.id.clone());

    if !fired.contains_key(&key) && fired.len() >= MAX_SKILL_COOLDOWN_ENTRIES {
        return Err("skill cooldown tracking capacity reached".into());
    }

    Ok(())
}

fn candidates<'r>(
    rules: &'r [Rule],
    fired: &HashMap<(String, String), Fired>,
    context: &AgentContext,
) -> Vec<&'r Rule> {
    rules
        .iter()
        .filter(|rule| rule.gate.admits(context))
        .filter(|rule| guards_allow(fired, rule, context))
        .collect()
}

fn guards_allow(
    fired: &HashMap<(String, String), Fired>,
    rule: &Rule,
    context: &AgentContext,
) -> bool {
    let key = (context.agent_id.clone(), rule.id.clone());
    let Some(previous) = fired.get(&key) else {
        return true;
    };

    if rule.once && previous.count > 0 {
        return false;
    }

    context.idle_at.saturating_sub(previous.last_at) >= rule.cooldown_secs
}

fn select<'r>(candidates: &[&'r Rule], verdicts: &[Verdict]) -> Vec<(&'r Rule, Urgency)> {
    let mut chosen: Vec<_> = candidates
        .iter()
        .filter_map(|rule| {
            let verdict = verdicts.iter().find(|verdict| verdict.rule_id == rule.id)?;

            rule.threshold
                .accepts(verdict.applies)
                .then_some((*rule, verdict.urgency))
        })
        .collect();

    chosen.sort_by(|a, b| b.0.priority.cmp(&a.0.priority).then(b.1.cmp(&a.1)));
    chosen.truncate(MAX_REMINDERS_PER_IDLE);

    chosen
}

fn render_reminders(context: &AgentContext, chosen: Vec<(&Rule, Urgency)>) -> Vec<Reminder> {
    chosen
        .into_iter()
        .map(|(rule, urgency)| Reminder {
            agent_id: context.agent_id.clone(),
            rule_id: rule.id.clone(),
            urgency,
            text: format!("📋 Reminder ({}):\n\n{}", rule.name, rule.reminder),
        })
        .collect()
}

fn record(
    fired: &mut HashMap<(String, String), Fired>,
    context: &AgentContext,
    reminder: &Reminder,
) {
    let key = (context.agent_id.clone(), reminder.rule_id.clone());
    let entry = fired.entry(key).or_default();

    entry.count = entry.count.saturating_add(1);
    entry.last_at = context.idle_at;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::{Gate, Threshold};

    struct FakeJudge {
        answers: HashMap<String, (f32, Urgency)>,
        calls: usize,
    }

    impl FakeJudge {
        fn new(answers: &[(&str, f32, Urgency)]) -> Self {
            Self {
                answers: answers
                    .iter()
                    .map(|(id, probability, urgency)| (id.to_string(), (*probability, *urgency)))
                    .collect(),
                calls: 0,
            }
        }
    }

    impl Judge for FakeJudge {
        fn judge(&mut self, _brief: &Brief, rules: &[&Rule]) -> Result<Vec<Verdict>, Error> {
            self.calls = self.calls.saturating_add(1);

            Ok(rules
                .iter()
                .filter_map(|rule| self.answers.get(&rule.id).map(|answer| (rule, answer)))
                .map(|(rule, (applies, urgency))| Verdict {
                    rule_id: rule.id.clone(),
                    applies: *applies,
                    urgency: *urgency,
                })
                .collect())
        }
    }

    fn rule(id: &str, priority: u8) -> Rule {
        Rule {
            id: id.into(),
            name: id.to_uppercase(),
            situation: "situation".into(),
            gate: Gate::default(),
            reminder: format!("do {id}"),
            priority,
            once: false,
            cooldown_secs: 30,
            threshold: Threshold::new(0.6).expect("valid threshold"),
        }
    }

    fn idle(status: &str, at: u64) -> AgentContext {
        AgentContext {
            agent_id: "agent-1".into(),
            status: status.into(),
            source: "github".into(),
            tool_history: Vec::new(),
            notifications: Vec::new(),
            hooks: Vec::new(),
            idle_at: at,
        }
    }

    #[test]
    fn present_situation_is_delivered_and_absent_one_is_not() {
        let judge = FakeJudge::new(&[
            ("pr", 0.9, Urgency::Important),
            ("ci", 0.2, Urgency::Urgent),
        ]);
        let mut steerer = Steerer::new(vec![rule("pr", 1), rule("ci", 9)], judge);
        let mut delivered = Vec::new();

        let result = steerer
            .on_idle(&idle("implementing", 100), |reminder| {
                delivered.push(reminder.clone());
                Ok(())
            })
            .expect("evaluation succeeds");

        assert_eq!(result, delivered);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].rule_id, "pr");
    }

    #[test]
    fn blocked_and_gated_agents_are_not_judged() {
        let mut gated = rule("pr", 1);
        gated.gate.status = vec!["in_review".into()];
        let judge = FakeJudge::new(&[("pr", 1.0, Urgency::Urgent)]);
        let mut steerer = Steerer::new(vec![gated], judge);

        assert!(
            steerer
                .on_idle(&idle("blocked", 100), |_| Ok(()))
                .expect("ok")
                .is_empty()
        );
        assert!(
            steerer
                .on_idle(&idle("implementing", 100), |_| Ok(()))
                .expect("ok")
                .is_empty()
        );
        assert_eq!(steerer.judge.calls, 0);
    }

    #[test]
    fn once_is_recorded_only_after_delivery() {
        let mut once = rule("once", 1);
        once.once = true;
        let judge = FakeJudge::new(&[("once", 1.0, Urgency::Routine)]);
        let mut steerer = Steerer::new(vec![once], judge);

        assert!(
            steerer
                .on_idle(&idle("implementing", 100), |_| Err(Error::Delivery(
                    "full".into()
                )))
                .is_err()
        );
        assert_eq!(
            steerer
                .on_idle(&idle("implementing", 101), |_| Ok(()))
                .expect("retry")
                .len(),
            1
        );
        assert!(
            steerer
                .on_idle(&idle("implementing", 200), |_| Ok(()))
                .expect("once")
                .is_empty()
        );
    }

    #[test]
    fn output_is_bounded_and_priority_ordered() {
        let judge = FakeJudge::new(&[
            ("low", 1.0, Urgency::Urgent),
            ("mid", 1.0, Urgency::Routine),
            ("high", 1.0, Urgency::Routine),
        ]);
        let mut steerer =
            Steerer::new(vec![rule("low", 1), rule("mid", 5), rule("high", 9)], judge);

        let delivered = steerer
            .on_idle(&idle("implementing", 100), |_| Ok(()))
            .expect("ok");

        assert_eq!(delivered.len(), MAX_REMINDERS_PER_IDLE);
        assert_eq!(delivered[0].rule_id, "high");
        assert_eq!(delivered[1].rule_id, "mid");
    }
}
