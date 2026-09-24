//! Code Mode surfacing. Tools in a Code Mode namespace reach the model through
//! `execute`, whose catalog shows each namespace only in part. When a request
//! needs a namespace, a note naming its best-matching tools joins the user's
//! message, once per context.

use std::collections::{HashMap, HashSet};

use chauffeur_core::{CodeModeNamespace, Delivery, Effect, Question, QuestionKind};

const MAX_AGENTS: usize = 256;
const MAX_DESCRIPTION_CHARS: usize = 160;

/// Namespaces already surfaced in each agent's current context.
#[derive(Default)]
pub struct Surfaced {
    agents: HashMap<String, HashSet<String>>,
}

impl Surfaced {
    /// A new context starts with nothing surfaced.
    pub fn reset(&mut self, agent_id: &str) {
        self.agents.remove(agent_id);
    }

    /// The namespaces not yet surfaced for this agent.
    pub fn pending<'a>(
        &self,
        agent_id: &str,
        namespaces: &'a [CodeModeNamespace],
    ) -> Vec<&'a CodeModeNamespace> {
        let surfaced = self.agents.get(agent_id);

        namespaces
            .iter()
            .filter(|namespace| surfaced.is_none_or(|surfaced| !surfaced.contains(&namespace.name)))
            .collect()
    }

    fn record(&mut self, agent_id: &str, names: impl Iterator<Item = String>) {
        if !self.agents.contains_key(agent_id) && self.agents.len() >= MAX_AGENTS {
            self.agents.clear();
        }

        self.agents
            .entry(agent_id.to_string())
            .or_default()
            .extend(names);
    }

    pub fn save(&self) -> serde_json::Value {
        serde_json::json!(self.agents)
    }

    pub fn load(&mut self, state: serde_json::Value) {
        if let Ok(agents) = serde_json::from_value::<HashMap<String, HashSet<String>>>(state) {
            self.agents = agents.into_iter().take(MAX_AGENTS).collect();
        }
    }

    /// The note for the chosen namespaces, recorded as surfaced.
    pub fn surface(&mut self, agent_id: &str, chosen: &[&CodeModeNamespace]) -> Option<Effect> {
        if chosen.is_empty() {
            return None;
        }

        self.record(
            agent_id,
            chosen.iter().map(|namespace| namespace.name.clone()),
        );

        Some(Effect::Context {
            agent_id: agent_id.to_string(),
            delivery: Delivery::Prompt,
            label: "Code Mode tools".into(),
            skills: Vec::new(),
            text: Some(note(chosen)),
        })
    }
}

/// Code Mode questions are kept apart from tool-group questions of the same name.
pub fn question_id(namespace: &str) -> String {
    format!("code-mode:{namespace}")
}

pub fn question(namespace: &CodeModeNamespace) -> Question {
    let examples: Vec<&str> = namespace
        .tools
        .iter()
        .map(|tool| tool.id.as_str())
        .collect();

    Question {
        id: question_id(&namespace.name),
        instructions: format!(
            "Will the coding agent need the \"{}\" tools, reached through Code Mode's execute \
             tool, for the user's latest request? Its catalog shows only some of them. It has {} \
             tools, such as: {}",
            namespace.name,
            namespace.size,
            examples.join(", ")
        ),
        kind: QuestionKind::Noul,
    }
}

fn note(chosen: &[&CodeModeNamespace]) -> String {
    let sections: Vec<String> = chosen
        .iter()
        .map(|namespace| {
            let lines: Vec<String> = namespace
                .tools
                .iter()
                .map(|tool| {
                    let description: String = tool.description.chars().take(MAX_DESCRIPTION_CHARS).collect();

                    format!("- {}: {description}", tool.id)
                })
                .collect();

            format!(
                "## {} ({} tools)\n{}\nFind exact paths with `search({{ namespace: \"{}\", query: \"…\" }})` inside `execute`.",
                namespace.name,
                namespace.size,
                lines.join("\n"),
                namespace.name
            )
        })
        .collect();

    format!(
        "Chauffeur: Code Mode tools that fit this request. The catalog shows these namespaces only in part.\n\n{}\n",
        sections.join("\n\n")
    )
}
