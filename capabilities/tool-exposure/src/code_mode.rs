//! Code Mode surfacing. Tools in a Code Mode namespace reach the model through
//! `execute`, whose catalog shows each namespace only in part. When a request
//! needs a namespace, a note naming its best-matching tools joins the user's
//! message, once per context, or answers the agent's own request.

use std::collections::{HashMap, HashSet};

use chauffeur_core::{CodeModeNamespace, Delivery, Effect, Facts, Question, QuestionKind};

use crate::config::ToolTexts;

const MAX_AGENTS: usize = 256;
const MAX_DESCRIPTION_CHARS: usize = 160;
/// Listed tools a question describes; the rest are named only.
const DESCRIBED_TOOLS: usize = 4;
/// Characters of each described tool's first sentence.
const MAX_EXAMPLE_CHARS: usize = 80;

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

    /// The note for the chosen namespaces at `delivery`, recorded as surfaced.
    pub fn surface(
        &mut self,
        agent_id: &str,
        chosen: &[&CodeModeNamespace],
        delivery: Delivery,
        texts: &ToolTexts,
    ) -> Option<Effect> {
        if chosen.is_empty() {
            return None;
        }

        self.record(
            agent_id,
            chosen.iter().map(|namespace| namespace.name.clone()),
        );

        Some(Effect::Context {
            agent_id: agent_id.to_string(),
            delivery,
            label: texts.code_mode_label.as_str().to_string(),
            skills: Vec::new(),
            text: Some(note(chosen, texts)),
        })
    }
}

/// Code Mode questions are kept apart from tool-group questions of the same name.
pub fn question_id(namespace: &str) -> String {
    format!("code-mode:{namespace}")
}

/// The namespace's listed tools, the first few with what they do: a judge
/// shown only tool IDs could not tell what an unfamiliar namespace is for,
/// and was unsure of more than a third of them.
pub fn examples(namespace: &CodeModeNamespace) -> String {
    let examples: Vec<String> = namespace
        .tools
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            let summary = first_sentence(&tool.description);

            if index < DESCRIBED_TOOLS && !summary.is_empty() {
                format!("{} ({summary})", tool.id)
            } else {
                tool.id.clone()
            }
        })
        .collect();

    examples.join("; ")
}

/// A description's first sentence, whitespace collapsed and clipped.
fn first_sentence(description: &str) -> String {
    let words: Vec<&str> = description.split_whitespace().collect();
    let text = words.join(" ");
    let end = text.find(". ").unwrap_or(text.len());
    let sentence = text[..end].trim_end_matches('.');

    if sentence.chars().count() <= MAX_EXAMPLE_CHARS {
        sentence.to_string()
    } else {
        let clipped: String = sentence.chars().take(MAX_EXAMPLE_CHARS - 1).collect();
        format!("{}…", clipped.trim_end())
    }
}

pub fn question(namespace: &CodeModeNamespace, texts: &ToolTexts) -> Question {
    Question {
        id: question_id(&namespace.name),
        instructions: texts.namespace.render(&[
            ("namespace", &namespace.name),
            ("size", &namespace.size.to_string()),
            ("examples", &examples(namespace)),
        ]),
        kind: QuestionKind::Noul,
        facts: Facts::new(),
    }
}

fn note(chosen: &[&CodeModeNamespace], texts: &ToolTexts) -> String {
    let sections: Vec<String> = chosen
        .iter()
        .map(|namespace| {
            let lines: Vec<String> = namespace
                .tools
                .iter()
                .map(|tool| {
                    let description: String = tool
                        .description
                        .chars()
                        .take(MAX_DESCRIPTION_CHARS)
                        .collect();

                    texts
                        .code_mode_tool
                        .render(&[("tool", &tool.id), ("description", &description)])
                })
                .collect();

            texts.code_mode_section.render(&[
                ("namespace", &namespace.name),
                ("size", &namespace.size.to_string()),
                ("tools", &lines.join("\n")),
            ])
        })
        .collect();

    texts
        .code_mode_note
        .render(&[("sections", &sections.join("\n\n"))])
}

#[cfg(test)]
mod tests {
    use chauffeur_core::CatalogEntry;

    use super::*;

    fn tool(id: &str, description: &str) -> CatalogEntry {
        CatalogEntry {
            id: id.into(),
            description: description.into(),
            bytes: 0,
        }
    }

    #[test]
    fn examples_say_what_the_first_tools_do_in_one_short_sentence() {
        let namespace = CodeModeNamespace {
            name: "agents".into(),
            size: 6,
            tools: vec![
                tool(
                    "agents_spawn",
                    "\n   Start a new agent in its own session.  It appears in history.",
                ),
                tool("agents_send", &"Send a message to any session ".repeat(9)),
                tool("agents_read", ""),
                tool("agents_list", "List agents."),
                tool("agents_stop", "Interrupt an agent."),
            ],
        };

        let text = examples(&namespace);
        let parts: Vec<&str> = text.split("; ").collect();

        assert_eq!(
            parts[0],
            "agents_spawn (Start a new agent in its own session)"
        );
        assert!(parts[1].starts_with("agents_send (Send a message") && parts[1].ends_with("…)"));
        assert!(parts[1].chars().count() <= "agents_send ()".len() + MAX_EXAMPLE_CHARS);
        assert_eq!(parts[2], "agents_read");
        assert_eq!(parts[3], "agents_list (List agents)");
        assert_eq!(parts[4], "agents_stop");
    }
}
