//! Bounded rolling view of one agent's recent activity.

use std::collections::VecDeque;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::load_layered;
use crate::signal::{Signal, SignalKind};
use crate::template::Template;

pub const MAX_ENTRIES: usize = 32;
/// The most characters one clipped line may keep, whatever is configured.
const MAX_KEPT_CHARS: usize = 2_048;
/// The most characters the whole state may hold, whatever is configured. A
/// provider counts the request's input against its own limit, and this is the
/// ceiling that keeps one long prompt from asking past it.
const MAX_STATE_CHARS: usize = 32_768;
/// The shipped wording (`skills/config/situation.json`), compiled in.
const SHIPPED: &str = include_str!("../../../../skills/config/situation.json");

/// How the state System One reads describes what the agent did.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SituationTexts {
    /// No activity is recorded yet.
    pub empty: Template,
    /// The line above the activity, oldest first.
    pub header: Template,
    /// A user message: `{text}`.
    pub user: Template,
    /// The agent asks Chauffeur for something: `{need}`.
    pub agent_request: Template,
    /// A permission request: `{action}`, `{targets}`, `{project}`, `{reason}`,
    /// `{asked}`.
    pub permission: Template,
    /// `{project}` when the session has a workspace: `{workspace}`.
    pub permission_project: Template,
    /// `{reason}` when the request gives none.
    pub permission_no_reason: Template,
    /// `{asked}` when no user request is recorded.
    pub permission_no_request: Template,
    /// A tool ran without input: `{tool}`.
    pub tool_ran: Template,
    /// A tool ran with input: `{tool}`, `{input}`.
    pub tool_ran_with_input: Template,
    /// A tool failed without input: `{tool}`.
    pub tool_failed: Template,
    /// A tool failed with input: `{tool}`, `{input}`.
    pub tool_failed_with_input: Template,
    /// Appended to a tool line that has an error: `{error}`.
    pub tool_error: Template,
    /// A model call failed: `{model}`, `{error_type}`, `{message}`.
    pub model_error: Template,
    /// An integration event: `{source}`, `{kind}`, `{summary}`.
    pub integration_event: Template,
    /// The agent ended its turn: `{summary}`.
    pub turn_end: Template,
    /// The user ran a rulebook command: `{rulebook}`, `{command}`, `{args}`.
    pub rulebook: Template,
    /// How much of each clipped line System One reads.
    pub clip: Clips,
    /// The characters the whole state may hold, oldest entries dropped first.
    pub max_chars: usize,
}

/// The characters of a whole line kept, its label included: the first `head`
/// and the last `tail`, joined by " … " when the middle is cut.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Keep {
    pub head: usize,
    pub tail: usize,
}

/// What each kind of clipped line keeps. Lines quoting the user keep both
/// ends, like a closing message: a request is 64 KiB at most, and the state is
/// one bounded window.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Clips {
    /// Tool, model, integration-event, and agent-request lines.
    pub line: Keep,
    /// The agent's closing message, which often ends on a question the
    /// user's next message answers.
    pub turn_end: Keep,
    /// User messages, permission requests, and rulebook commands, which carry
    /// the user's own words.
    pub quoted: Keep,
}

impl Keep {
    fn checked(self, field: &str) -> Result<Self, String> {
        let kept = self.head.saturating_add(self.tail);

        if (1..=MAX_KEPT_CHARS).contains(&kept) {
            Ok(self)
        } else {
            Err(format!(
                "{field}: head + tail must be 1-{MAX_KEPT_CHARS} characters"
            ))
        }
    }

    /// `line` within this bound.
    fn apply(self, line: &str) -> String {
        let count = line.chars().count();

        if count <= self.head + self.tail {
            return line.to_string();
        }

        let head: String = line.chars().take(self.head).collect();

        if self.tail == 0 {
            return head;
        }

        let tail: String = line.chars().skip(count - self.tail).collect();

        format!("{head} … {tail}")
    }
}

impl SituationTexts {
    /// The shipped wording overlaid by your `situation.json` at `path`.
    ///
    /// # Errors
    ///
    /// When your file is unreadable or invalid, or a text names a
    /// placeholder it may not.
    pub fn load(path: &Path) -> Result<Self, String> {
        load_layered::<Self>(SHIPPED, path)?
            .checked()
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    fn checked(mut self) -> Result<Self, String> {
        let tool = &["tool"][..];
        let tool_input = &["tool", "input"][..];

        for (field, template, allowed) in [
            ("texts.empty", &self.empty, &[][..]),
            ("texts.header", &self.header, &[][..]),
            ("texts.user", &self.user, &["text"][..]),
            ("texts.agent_request", &self.agent_request, &["need"][..]),
            (
                "texts.permission",
                &self.permission,
                &["action", "targets", "project", "reason", "asked"][..],
            ),
            (
                "texts.permission_project",
                &self.permission_project,
                &["workspace"][..],
            ),
            (
                "texts.permission_no_reason",
                &self.permission_no_reason,
                &[][..],
            ),
            (
                "texts.permission_no_request",
                &self.permission_no_request,
                &[][..],
            ),
            ("texts.tool_ran", &self.tool_ran, tool),
            (
                "texts.tool_ran_with_input",
                &self.tool_ran_with_input,
                tool_input,
            ),
            ("texts.tool_failed", &self.tool_failed, tool),
            (
                "texts.tool_failed_with_input",
                &self.tool_failed_with_input,
                tool_input,
            ),
            ("texts.tool_error", &self.tool_error, &["error"][..]),
            (
                "texts.model_error",
                &self.model_error,
                &["model", "error_type", "message"][..],
            ),
            (
                "texts.integration_event",
                &self.integration_event,
                &["source", "kind", "summary"][..],
            ),
            ("texts.turn_end", &self.turn_end, &["summary"][..]),
            (
                "texts.rulebook",
                &self.rulebook,
                &["rulebook", "command", "args"][..],
            ),
        ] {
            template.check(field, allowed)?;
        }

        self.clip.line = self.clip.line.checked("clip.line")?;
        self.clip.turn_end = self.clip.turn_end.checked("clip.turn_end")?;
        self.clip.quoted = self.clip.quoted.checked("clip.quoted")?;

        if !(1..=MAX_STATE_CHARS).contains(&self.max_chars) {
            return Err(format!(
                "max_chars: the state must hold 1-{MAX_STATE_CHARS} characters"
            ));
        }

        Ok(self)
    }
}

impl Default for SituationTexts {
    fn default() -> Self {
        serde_json::from_str::<Self>(SHIPPED)
            .map_err(|error| error.to_string())
            .and_then(Self::checked)
            .expect("shipped situation texts are valid")
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Situation {
    entries: VecDeque<String>,
    last_at: u64,
}

impl Situation {
    pub fn record(&mut self, signal: &Signal, texts: &SituationTexts) {
        self.last_at = self.last_at.max(signal.at);

        let Some(line) = describe(&signal.kind, texts) else {
            return;
        };

        if self.entries.len() == MAX_ENTRIES {
            self.entries.pop_front();
        }

        self.entries.push_back(if quotes_user(&signal.kind) {
            texts.clip.quoted.apply(&line)
        } else if matches!(signal.kind, SignalKind::TurnEnd { .. }) {
            texts.clip.turn_end.apply(&line)
        } else {
            texts.clip.line.apply(&line)
        });

        // A judgment is about the newest entries, so the oldest go first when
        // the state outgrows its budget.
        while self.entries.len() > 1 && self.held() > texts.max_chars {
            self.entries.pop_front();
        }
    }

    #[must_use]
    pub fn last_at(&self) -> u64 {
        self.last_at
    }

    /// The characters the entries hold, each with its bullet and newline.
    fn held(&self) -> usize {
        self.entries
            .iter()
            .map(|entry| entry.chars().count() + 3)
            .sum()
    }

    /// Prose rendering for System One: recent activity, oldest first, newest
    /// entries first within `budget`. The newest entry always survives.
    #[must_use]
    pub fn render(&self, texts: &SituationTexts, budget: usize) -> String {
        if self.entries.is_empty() {
            return texts.empty.render(&[]);
        }

        let header = texts.header.render(&[]);
        let mut used = header.chars().count() + 1;
        let mut kept: Vec<&String> = Vec::new();

        for entry in self.entries.iter().rev() {
            let cost = entry.chars().count() + 3;

            if !kept.is_empty() && used + cost > budget {
                break;
            }

            used += cost;
            kept.push(entry);
        }

        let mut state = header;

        state.push('\n');

        for entry in kept.into_iter().rev() {
            state.push_str("- ");
            state.push_str(entry);
            state.push('\n');
        }

        state
    }
}

fn describe(kind: &SignalKind, texts: &SituationTexts) -> Option<String> {
    match kind {
        SignalKind::UserMessage { text, .. } => Some(texts.user.render(&[("text", text)])),
        SignalKind::AgentRequest { need, .. } => {
            Some(texts.agent_request.render(&[("need", need)]))
        }
        SignalKind::PermissionRequest {
            action,
            resources,
            request,
            workspace,
            user_requests,
            ..
        } => {
            let targets: Vec<&str> = resources
                .iter()
                .map(|resource| resource.resolved.as_str())
                .collect();

            Some(describe_permission(
                texts,
                [action, &targets.join(", "), request, workspace],
                user_requests.last(),
            ))
        }
        SignalKind::ToolResult {
            tool,
            ok,
            input,
            error,
            ..
        } => Some(describe_tool(texts, tool, *ok, input, error)),
        SignalKind::ModelError {
            model,
            error_type,
            message,
            ..
        } => Some(texts.model_error.render(&[
            ("model", &model.key()),
            ("error_type", error_type),
            ("message", message),
        ])),
        SignalKind::IntegrationEvent {
            source,
            kind,
            summary,
            ..
        } => Some(texts.integration_event.render(&[
            ("source", source),
            ("kind", kind),
            ("summary", summary),
        ])),
        SignalKind::TurnEnd { summary, .. } if !summary.trim().is_empty() => {
            Some(texts.turn_end.render(&[("summary", summary)]))
        }
        SignalKind::Rulebook {
            command,
            rulebook,
            args,
            ..
        } => Some(texts.rulebook.render(&[
            ("rulebook", rulebook),
            ("command", &format!("{command:?}")),
            ("args", args),
        ])),
        SignalKind::ModelSucceeded { .. } | SignalKind::TurnEnd { .. } => None,
    }
}

/// A permission request, with the latest user request and the project, so
/// it can be judged against what the user asked and where the session works.
fn describe_permission(
    texts: &SituationTexts,
    [action, targets, request, workspace]: [&str; 4],
    latest_user_request: Option<&String>,
) -> String {
    let asked =
        latest_user_request.map_or_else(|| texts.permission_no_request.render(&[]), String::clone);
    let project = if workspace.is_empty() {
        String::new()
    } else {
        texts.permission_project.render(&[("workspace", workspace)])
    };
    let reason = if request.trim().is_empty() {
        texts.permission_no_reason.render(&[])
    } else {
        request.to_string()
    };

    texts.permission.render(&[
        ("action", action),
        ("targets", targets),
        ("project", &project),
        ("reason", &reason),
        ("asked", &asked),
    ])
}

fn describe_tool(texts: &SituationTexts, tool: &str, ok: bool, input: &str, error: &str) -> String {
    let template = match (ok, input.is_empty()) {
        (true, true) => &texts.tool_ran,
        (true, false) => &texts.tool_ran_with_input,
        (false, true) => &texts.tool_failed,
        (false, false) => &texts.tool_failed_with_input,
    };
    let mut line = template.render(&[("tool", tool), ("input", input)]);

    if !error.is_empty() {
        line.push_str(&texts.tool_error.render(&[("error", error)]));
    }

    line
}

/// Lines carrying the user's own words keep both ends, like a closing message:
/// a prompt is 64 KiB at most and the state is one bounded window. Lines about
/// tools and models keep the head only.
fn quotes_user(kind: &SignalKind) -> bool {
    matches!(
        kind,
        SignalKind::UserMessage { .. }
            | SignalKind::PermissionRequest { .. }
            | SignalKind::Rulebook { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(n: usize) -> Signal {
        Signal {
            agent_id: "a".into(),
            at: n as u64,
            kind: SignalKind::ToolResult {
                tool: format!("tool-{n}"),
                ok: true,
                workspace: String::new(),
                subagent: false,
                input: String::new(),
                error: String::new(),
                user_request: String::new(),
                evidence: String::new(),
                candidates: Vec::new(),
            },
        }
    }

    #[test]
    fn permission_requests_carry_the_latest_user_request() {
        let mut situation = Situation::default();

        situation.record(
            &Signal {
                agent_id: "a".into(),
                at: 1,
                kind: SignalKind::PermissionRequest {
                    action: "edit".into(),
                    resources: vec![crate::signal::Resource {
                        requested: "README.md".into(),
                        resolved: "/w/README.md".into(),
                    }],
                    request: "update install".into(),
                    workspace: "/w".into(),
                    user_requests: vec!["old".into(), "please update README.md".into()],
                    host_decision: crate::effect::PermissionDecision::Ask,
                },
            },
            &SituationTexts::default(),
        );

        assert!(situation
            .render(&SituationTexts::default(), usize::MAX)
            .contains(
            "Agent asks to edit /w/README.md from the project /w. Its reason: update install (latest user request: please update README.md)"
        ));
    }

    #[test]
    fn a_failed_tool_call_carries_its_error() {
        let mut situation = Situation::default();

        situation.record(
            &Signal {
                agent_id: "a".into(),
                at: 1,
                kind: SignalKind::ToolResult {
                    tool: "edit".into(),
                    ok: false,
                    workspace: String::new(),
                    subagent: false,
                    input: r#"{"path":"main.rs"}"#.into(),
                    error: "permission denied".into(),
                    user_request: String::new(),
                    evidence: String::new(),
                    candidates: Vec::new(),
                },
            },
            &SituationTexts::default(),
        );

        assert!(
            situation
                .render(&SituationTexts::default(), usize::MAX)
                .contains(r#"Tool failed: edit {"path":"main.rs"} (error: permission denied)"#)
        );
    }

    #[test]
    fn the_shipped_wording_renders_every_line_as_before() {
        let texts = SituationTexts::default();
        let mut situation = Situation::default();

        assert_eq!(
            situation.render(&texts, texts.max_chars),
            "No recent agent activity is recorded."
        );
        situation.record(&tool(1), &texts);
        situation.record(
            &Signal {
                agent_id: "a".into(),
                at: 2,
                kind: SignalKind::TurnEnd {
                    workspace: String::new(),
                    subagent: false,
                    user_request: String::new(),
                    summary: "done".into(),
                    todos: Vec::new(),
                },
            },
            &texts,
        );

        assert_eq!(
            situation.render(&texts, texts.max_chars),
            "Recent agent activity, oldest first:\n- Ran tool tool-1.\n- Agent ended its turn: done\n"
        );
    }

    #[test]
    fn a_text_naming_an_unknown_placeholder_is_an_error() {
        let path =
            std::env::temp_dir().join(format!("chauffeur-situation-{}.json", std::process::id()));

        std::fs::write(&path, r#"{ "user": "User said {words}" }"#).unwrap();

        let error = SituationTexts::load(&path).unwrap_err();

        assert!(error.contains("texts.user names {words}"), "{error}");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn keeps_only_the_newest_entries() {
        let mut situation = Situation::default();

        for n in 0..(MAX_ENTRIES + 5) {
            situation.record(&tool(n), &SituationTexts::default());
        }

        let state = situation.render(&SituationTexts::default(), usize::MAX);

        assert!(!state.contains("tool-4."));
        assert!(state.contains("tool-5."));
        assert!(state.contains(&format!("tool-{}.", MAX_ENTRIES + 4)));
        assert_eq!(situation.last_at(), (MAX_ENTRIES + 4) as u64);
    }

    #[test]
    fn user_words_keep_both_ends_while_tool_lines_keep_the_head() {
        let mut situation = Situation::default();
        let words = format!("{} the end", "requirement ".repeat(200));

        situation.record(
            &Signal {
                agent_id: "a".into(),
                at: 1,
                kind: SignalKind::UserMessage {
                    text: words,
                    first_in_context: true,
                    skills: Vec::new(),
                    tools: Vec::new(),
                    model: None,
                    code_mode: Vec::new(),
                    workspace: String::new(),
                },
            },
            &SituationTexts::default(),
        );
        let mut long_tool = tool(1);
        if let SignalKind::ToolResult { input, .. } = &mut long_tool.kind {
            *input = "x".repeat(1_000);
        }
        situation.record(&long_tool, &SituationTexts::default());

        let state = situation.render(&SituationTexts::default(), usize::MAX);

        assert!(state.contains("User: requirement requirement"), "{state}");
        assert!(state.contains(" … "), "{state}");
        assert!(state.contains("the end\n"), "{state}");
        assert!(!state.contains(&"x".repeat(240)));
    }

    /// A prompt is 64 KiB at most, and a permission request quotes it whole:
    /// the state would outgrow any provider's input limit on its own.
    #[test]
    fn a_long_prompt_cannot_fill_the_state() {
        let texts = SituationTexts::default();
        let prompt = format!("{} Shall I start on step 2?", "requirement ".repeat(6_000));
        let mut situation = Situation::default();

        for n in 0..MAX_ENTRIES {
            situation.record(
                &Signal {
                    agent_id: "a".into(),
                    at: n as u64,
                    kind: SignalKind::PermissionRequest {
                        action: "edit".into(),
                        resources: vec![crate::signal::Resource {
                            requested: "README.md".into(),
                            resolved: "/w/README.md".into(),
                        }],
                        request: String::new(),
                        workspace: "/w".into(),
                        user_requests: vec![prompt.clone()],
                        host_decision: crate::effect::PermissionDecision::Ask,
                    },
                },
                &texts,
            );
        }

        let state = situation.render(&texts, texts.max_chars);

        assert!(
            prompt.len() > texts.max_chars,
            "the prompt is the long part"
        );
        assert!(
            state.chars().count() <= texts.max_chars,
            "{} characters",
            state.chars().count()
        );
        assert!(state.contains("Shall I start on step 2?"), "{state}");
        assert_eq!(
            situation.entries.len(),
            texts.max_chars / 1_664,
            "only as many entries as fit"
        );
    }

    #[test]
    fn a_smaller_budget_drops_older_entries_and_keeps_the_newest() {
        let texts = SituationTexts::default();
        let mut situation = Situation::default();

        for n in 0..8 {
            situation.record(&tool(n), &texts);
        }

        let full = situation.render(&texts, texts.max_chars);
        let trimmed = situation.render(&texts, 60);

        assert!(full.contains("tool-0."));
        assert!(!trimmed.contains("tool-0."));
        assert!(trimmed.contains("tool-7."), "{trimmed}");
        assert!(trimmed.chars().count() <= 60, "{}", trimmed.chars().count());

        // The newest entry is the request being judged, so it survives any
        // budget a round leaves it.
        let alone = situation.render(&texts, 1);

        assert!(alone.contains("tool-7."), "{alone}");
        assert!(!alone.contains("tool-6."), "{alone}");
    }

    #[test]
    fn a_state_budget_out_of_range_is_an_error() {
        let path =
            std::env::temp_dir().join(format!("chauffeur-state-{}.json", std::process::id()));

        std::fs::write(&path, r#"{ "max_chars": 0 }"#).unwrap();

        let error = SituationTexts::load(&path).unwrap_err();

        assert!(error.contains("max_chars: the state must hold"), "{error}");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_closing_message_keeps_both_ends_when_configured() {
        let mut texts = SituationTexts::default();
        // The line's label counts: "Agent ended its turn: " is 22 characters.
        texts.clip.turn_end = Keep { head: 40, tail: 25 };
        let mut situation = Situation::default();
        let summary = format!(
            "Step 1 is done.{} Should I start on step 2?",
            " detail".repeat(80)
        );

        situation.record(
            &Signal {
                agent_id: "a".into(),
                at: 1,
                kind: SignalKind::TurnEnd {
                    workspace: String::new(),
                    subagent: false,
                    user_request: String::new(),
                    summary,
                    todos: Vec::new(),
                },
            },
            &texts,
        );

        let state = situation.render(&texts, usize::MAX);

        assert!(state.contains("Agent ended its turn: Step 1 is"), "{state}");
        assert!(state.contains(" … Should I start on step 2?"), "{state}");
        assert!(state.len() < 150, "{state}");
    }

    #[test]
    fn a_keep_of_nothing_or_too_much_is_rejected() {
        assert!(Keep { head: 0, tail: 0 }.checked("clip.line").is_err());
        assert!(
            Keep {
                head: 2_000,
                tail: 100
            }
            .checked("clip.line")
            .is_err()
        );
        assert_eq!(Keep { head: 3, tail: 2 }.apply("abcdefgh"), "abc … gh");
        assert_eq!(Keep { head: 3, tail: 0 }.apply("abcdefgh"), "abc");
        assert_eq!(Keep { head: 5, tail: 5 }.apply("short"), "short");
    }
}
