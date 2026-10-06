//! What the agent's requests were made of, from the probe plugin's
//! `requests.jsonl`: one line per request as sent to the provider.

use serde::{Deserialize, Serialize};

/// The probe plugin, written into each run and loaded by path.
pub const PROBE_JS: &str = include_str!("../probe/index.js");

/// One part of a request: a system section, a tool definition, the history.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Section {
    /// `system`, `tools`, `messages`, or `other`.
    pub part: String,
    /// A system section's first line, a tool's name, or a message count.
    pub name: String,
    pub chars: u64,
}

/// One request's size by part, with its system sections and tools.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Composition {
    pub total_chars: u64,
    /// Code Mode namespaces the catalog listed; differs between runs only
    /// when an MCP server failed to connect.
    #[serde(default)]
    pub namespaces: u64,
    pub system_chars: u64,
    pub tools_chars: u64,
    pub messages_chars: u64,
    pub other_chars: u64,
    pub sections: Vec<Section>,
}

/// The agent's requests in one run.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequestMetrics {
    pub requests: u64,
    /// Lines that were not probe records.
    pub bad_lines: u64,
    /// The first request: the fixed prompt plus the user's message.
    pub first: Option<Composition>,
}

#[derive(Deserialize)]
struct Line {
    total: u64,
    #[serde(default)]
    namespaces: u64,
    sections: Vec<Section>,
}

/// Summarize a probe file. Unreadable lines are counted, not fatal.
#[must_use]
pub fn parse(text: &str) -> RequestMetrics {
    let mut metrics = RequestMetrics::default();

    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(line) = serde_json::from_str::<Line>(line) else {
            metrics.bad_lines = metrics.bad_lines.saturating_add(1);
            continue;
        };

        metrics.requests = metrics.requests.saturating_add(1);

        if metrics.first.is_none() {
            metrics.first = Some(composition(line));
        }
    }

    metrics
}

fn composition(line: Line) -> Composition {
    let sum = |part: &str| {
        line.sections
            .iter()
            .filter(|section| section.part == part)
            .fold(0_u64, |total, section| total.saturating_add(section.chars))
    };

    Composition {
        total_chars: line.total,
        namespaces: line.namespaces,
        system_chars: sum("system"),
        tools_chars: sum("tools"),
        messages_chars: sum("messages"),
        other_chars: sum("other"),
        sections: line.sections,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_first_request_and_counts_the_rest() {
        let text = concat!(
            r##"{"at":1,"transport":"http","total":100,"sections":[{"part":"system","name":"# Code Mode","chars":60},{"part":"tools","name":"read","chars":30},{"part":"messages","name":"1 messages","chars":8},{"part":"other","name":"model","chars":2}]}"##,
            "\n",
            "not json\n",
            r#"{"at":2,"transport":"http","total":500,"sections":[]}"#,
            "\n",
        );

        let metrics = parse(text);
        let first = metrics.first.unwrap();

        assert_eq!((metrics.requests, metrics.bad_lines), (2, 1));
        assert_eq!(
            (
                first.total_chars,
                first.system_chars,
                first.tools_chars,
                first.messages_chars,
                first.other_chars
            ),
            (100, 60, 30, 8, 2)
        );
        assert_eq!(first.sections[0].name, "# Code Mode");
    }
}
