//! Code Mode surfacing. Tools in a Code Mode namespace reach the model through
//! `execute`, whose catalog shows each namespace only in part. When a request
//! needs a namespace, a note naming its best-matching tools joins the user's
//! message, once per session and one attachment per namespace, or answers the
//! agent's own request. A namespace the agent has already called gets no note:
//! it knows those tools.

use std::collections::{HashMap, HashSet};

use chauffeur_core::{CodeModeNamespace, Delivery, Effect, Facts, Question, QuestionKind};

use crate::config::ToolTexts;

const MAX_AGENTS: usize = 256;
/// Namespaces and tool IDs remembered per agent.
const MAX_KNOWN: usize = 256;
const MAX_NAME_CHARS: usize = 128;
/// Code Mode's own tool, whose input is the code it runs.
const CODE_MODE_TOOL: &str = "execute";
const MAX_DESCRIPTION_CHARS: usize = 160;
/// Listed tools a question describes; the rest are named only.
const DESCRIBED_TOOLS: usize = 4;
/// Characters of each described tool's first sentence.
const MAX_EXAMPLE_CHARS: usize = 80;

/// What each agent already knows of Code Mode in its session: the namespaces
/// whose notes it was sent, and the namespaces and tool IDs it has called. A
/// compaction keeps them, so a note is sent once per session.
#[derive(Default)]
pub struct Surfaced {
    agents: HashMap<String, HashSet<String>>,
}

impl Surfaced {
    /// The namespaces this agent was neither sent a note for nor has called.
    pub fn pending<'a>(
        &self,
        agent_id: &str,
        namespaces: &'a [CodeModeNamespace],
    ) -> Vec<&'a CodeModeNamespace> {
        let known = self.agents.get(agent_id);
        let knows = |namespace: &str| {
            known.is_some_and(|known| {
                known.iter().any(|name| {
                    name == namespace
                        || name
                            .strip_prefix(namespace)
                            .is_some_and(|rest| rest.starts_with('_'))
                })
            })
        };

        namespaces
            .iter()
            .filter(|namespace| !knows(&namespace.name))
            .collect()
    }

    /// Remember what a successful call reached: the namespaces `execute`'s
    /// code names, or the tool itself when the host reports a call made
    /// inside it, such as `patch` or `yab-browser_open`.
    pub fn called(&mut self, agent_id: &str, tool: &str, input: &str) {
        let names = if tool == CODE_MODE_TOOL {
            called_in(input)
        } else {
            vec![tool.to_string()]
        };

        self.record(agent_id, names.into_iter());
    }

    fn record(&mut self, agent_id: &str, names: impl Iterator<Item = String>) {
        let mut names = names.peekable();

        if names.peek().is_none() {
            return;
        }

        if !self.agents.contains_key(agent_id) && self.agents.len() >= MAX_AGENTS {
            self.agents.clear();
        }

        let known = self.agents.entry(agent_id.to_string()).or_default();

        for name in names {
            if known.len() >= MAX_KNOWN {
                break;
            }

            known.insert(name);
        }
    }

    pub fn save(&self) -> serde_json::Value {
        serde_json::json!(self.agents)
    }

    pub fn load(&mut self, state: serde_json::Value) {
        if let Ok(agents) = serde_json::from_value::<HashMap<String, HashSet<String>>>(state) {
            self.agents = agents.into_iter().take(MAX_AGENTS).collect();
        }
    }

    /// The notes for the chosen namespaces at `delivery`, recorded as
    /// surfaced: one per namespace on a prompt, where each becomes an
    /// attachment named after it, and one for all of them elsewhere.
    pub fn surface(
        &mut self,
        agent_id: &str,
        chosen: &[&CodeModeNamespace],
        delivery: Delivery,
        texts: &ToolTexts,
    ) -> Vec<Effect> {
        self.record(
            agent_id,
            chosen.iter().map(|namespace| namespace.name.clone()),
        );

        let groups: Vec<&[&CodeModeNamespace]> = match delivery {
            Delivery::Prompt => chosen.chunks(1).collect(),
            _ if chosen.is_empty() => Vec::new(),
            _ => vec![chosen],
        };

        groups
            .into_iter()
            .map(|namespaces| {
                let names: Vec<&str> = namespaces
                    .iter()
                    .map(|namespace| namespace.name.as_str())
                    .collect();

                Effect::Context {
                    agent_id: agent_id.to_string(),
                    delivery,
                    label: texts
                        .code_mode_label
                        .render(&[("namespace", &names.join(", "))]),
                    skills: Vec::new(),
                    text: Some(note(namespaces, texts)),
                }
            })
            .collect()
    }
}

/// The namespaces Code Mode code calls: `tools.patch(…)`,
/// `tools.opencode.session_move(…)`, `tools["yab-browser"].open(…)`. The host
/// reports the code as JSON, so a quote may be escaped.
fn called_in(code: &str) -> Vec<String> {
    let identifier = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$';
    let mut names: Vec<String> = Vec::new();

    for (index, _) in code.match_indices("tools") {
        if code[..index]
            .chars()
            .next_back()
            .is_some_and(|c| identifier(c) || c == '.')
        {
            continue;
        }

        let rest = &code[index + "tools".len()..];
        let name: String = if let Some(rest) = rest.strip_prefix('.') {
            rest.chars().take_while(|c| identifier(*c)).collect()
        } else if let Some(rest) = rest.strip_prefix('[') {
            let rest = rest.strip_prefix('\\').unwrap_or(rest);
            let mut chars = rest.chars();
            let Some(quote) = chars.next().filter(|c| matches!(c, '"' | '\'' | '`')) else {
                continue;
            };

            chars.take_while(|c| *c != quote && *c != '\\').collect()
        } else {
            continue;
        };

        if !name.is_empty() && name.chars().count() <= MAX_NAME_CHARS && !names.contains(&name) {
            names.push(name);
        }
    }

    names
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
        second_opinion: Default::default(),
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
    fn code_names_each_namespace_it_calls_once() {
        let code = r#"{"code":"const a=await tools.patch({x:1}); await tools.opencode.session_move({}); await tools[\"yab-browser\"].open({}); tools['iMCP'].x(); tools.patch(); mytools.no(); x.tools.no(); tools[name]"}"#;

        assert_eq!(
            called_in(code),
            ["patch", "opencode", "yab-browser", "iMCP"]
        );
    }

    #[test]
    fn a_called_namespace_is_never_pending() {
        let namespace = |name: &str| CodeModeNamespace {
            name: name.into(),
            size: 1,
            tools: Vec::new(),
        };
        let namespaces = [
            namespace("patch"),
            namespace("yab-browser"),
            namespace("yab"),
            namespace("write"),
        ];
        let mut surfaced = Surfaced::default();

        surfaced.called("ses", "execute", r#"{"code":"await tools.patch({})"}"#);
        surfaced.called("ses", "yab-browser_open", "{}");
        surfaced.called("ses", "execute", r#"{"code":"return 1"}"#);

        let pending: Vec<&str> = surfaced
            .pending("ses", &namespaces)
            .iter()
            .map(|namespace| namespace.name.as_str())
            .collect();
        assert_eq!(pending, ["yab", "write"]);

        let mut restored = Surfaced::default();
        restored.load(surfaced.save());
        assert_eq!(restored.pending("ses", &namespaces).len(), 2);
        assert_eq!(restored.pending("other", &namespaces).len(), 4);
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
