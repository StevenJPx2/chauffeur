//! Complete in-process Chauffeur SDK runtime.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

use crate::context::AgentContext;
use crate::queue::{QueuedReminder, ReminderQueue};
use crate::rule::Rule;
use crate::skill::{MAX_SKILLS, Skill, SkillContext, SkillResult};
use crate::steer::{Error, Judge, Reminder, Steerer};
use crate::target::Target;

/// Owns the judge, rule state, durable queue, and live reminder fan-out.
pub struct Runtime {
    steerer: Mutex<Steerer<Box<dyn Judge>>>,
    skills: HashMap<String, Skill>,
    queue: Arc<dyn ReminderQueue>,
    events: broadcast::Sender<QueuedReminder>,
}

impl Runtime {
    #[must_use]
    pub fn new(rules: Vec<Rule>, judge: Box<dyn Judge>, queue: Arc<dyn ReminderQueue>) -> Self {
        let (events, _) = broadcast::channel(128);

        Self {
            steerer: Mutex::new(Steerer::new(rules, judge)),
            skills: HashMap::new(),
            queue,
            events,
        }
    }

    pub fn with_skills(
        rules: Vec<Rule>,
        skills: Vec<Skill>,
        judge: Box<dyn Judge>,
        queue: Arc<dyn ReminderQueue>,
    ) -> Result<Self, String> {
        if skills.len() > MAX_SKILLS {
            return Err(format!("runtime accepts at most {MAX_SKILLS} skills"));
        }

        let mut registry = HashMap::with_capacity(skills.len());

        for skill in skills {
            skill.validate()?;

            if registry.insert(skill.identity.id.clone(), skill).is_some() {
                return Err("duplicate skill id".into());
            }
        }

        let (events, _) = broadcast::channel(128);

        Ok(Self {
            steerer: Mutex::new(Steerer::new(rules, judge)),
            skills: registry,
            queue,
            events,
        })
    }

    pub fn steer(&self, target: &Target, context: &AgentContext) -> Result<Vec<Reminder>, String> {
        target.validate()?;

        let mut steerer = self.steerer.lock().map_err(|_| "steerer lock poisoned")?;
        let mut queued = Vec::new();

        let reminders = steerer
            .on_idle(context, |reminder| {
                let event = self
                    .queue
                    .enqueue(target, reminder.clone())
                    .map_err(Error::Delivery)?;
                queued.push(event);

                Ok(())
            })
            .map_err(|error| error.to_string())?;

        for event in queued {
            let _unused_receivers = self.events.send(event);
        }

        Ok(reminders)
    }

    pub fn reminders(&self, target: &Target) -> Result<Vec<QueuedReminder>, String> {
        target.validate()?;
        self.queue.read(target)
    }

    pub fn acknowledge(&self, target: &Target, ids: &[String]) -> Result<(), String> {
        target.validate()?;
        self.queue.acknowledge(target, ids)
    }

    pub fn evaluate_skill(
        &self,
        skill_id: &str,
        context: &SkillContext,
    ) -> Result<SkillResult, String> {
        let skill = self
            .skills
            .get(skill_id)
            .ok_or_else(|| format!("unknown skill {skill_id}"))?;
        let mut steerer = self.steerer.lock().map_err(|_| "steerer lock poisoned")?;

        steerer.evaluate_skill(skill, context)
    }

    #[must_use]
    pub fn skill_ids(&self) -> Vec<String> {
        let mut ids = self.skills.keys().cloned().collect::<Vec<_>>();
        ids.sort();

        ids
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<QueuedReminder> {
        self.events.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Brief;
    use crate::queue::InMemoryReminderQueue;
    use crate::steer::{Urgency, Verdict};

    struct Always;

    impl Judge for Always {
        fn judge(&mut self, _brief: &Brief, rules: &[&Rule]) -> Result<Vec<Verdict>, Error> {
            Ok(rules
                .iter()
                .map(|rule| Verdict {
                    rule_id: rule.id.clone(),
                    applies: 1.0,
                    urgency: Urgency::Important,
                })
                .collect())
        }
    }

    fn context() -> AgentContext {
        AgentContext {
            agent_id: "agent".into(),
            status: "implementing".into(),
            source: String::new(),
            tool_history: Vec::new(),
            notifications: Vec::new(),
            hooks: Vec::new(),
            idle_at: 1,
        }
    }

    #[test]
    fn runtime_routes_and_acknowledges_by_target() {
        let rules = vec![Rule::new("test:nudge", "Nudge", "true", "act")];
        let runtime = Runtime::new(
            rules,
            Box::new(Always),
            Arc::new(InMemoryReminderQueue::default()),
        );
        let one = Target::new("test", "one");
        let two = Target::new("test", "two");

        runtime.steer(&one, &context()).expect("steer");
        let queued = runtime.reminders(&one).expect("read");

        assert_eq!(queued.len(), 1);
        assert!(runtime.reminders(&two).expect("read").is_empty());

        runtime
            .acknowledge(&one, &[queued[0].id.clone()])
            .expect("ack");
        assert!(runtime.reminders(&one).expect("read").is_empty());
    }
}
