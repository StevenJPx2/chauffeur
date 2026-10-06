//! The wording the rules capability adds around a rule's own text: questions
//! to System One and notices to the agent, kept in `skills/config/rules.json`.

use chauffeur_core::Template;
use serde::Deserialize;

/// Questions to System One and notices to the session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RulesTexts {
    /// A step's question about a tool call: `{tool}`, `{outcome}`, `{input}`,
    /// `{question}`, the step's own.
    pub tool_call: Template,
    /// `{outcome}` for a call that succeeded.
    pub outcome_succeeded: Template,
    /// `{outcome}` for a call that failed.
    pub outcome_failed: Template,
    /// Appended to a turn-end question when the user asked something:
    /// `{user_request}`.
    pub user_request_note: Template,
    /// Appended to a turn-end question when the agent said something:
    /// `{summary}`.
    pub closing_message_note: Template,
    /// Appended to a turn-end question when the agent called tools: `{tools}`.
    pub tools_note: Template,
    /// An open todo: `{content}`.
    pub todo_item: Template,
    /// An open todo in progress: `{content}`.
    pub todo_item_in_progress: Template,
    /// The todo list when none is open.
    pub todos_none: Template,
    /// A rulebook command for a book that is not running: `{id}`.
    pub not_running: Template,
    /// A running book's status: `{name}`, `{state}`, `{args}`, `{used}`,
    /// `{budget}`.
    pub status: Template,
    /// `{state}` for a book that is running.
    pub state_running: Template,
    /// `{state}` for a book that is paused.
    pub state_paused: Template,
    /// A book was paused: `{name}`, `{id}`.
    pub paused: Template,
    /// A book was cleared: `{name}`.
    pub cleared: Template,
    /// No such rulebook here: `{id}`, `{known}`, the offered ones.
    pub unknown_book: Template,
    /// The session already runs the most books: `{max}`.
    pub session_full: Template,
    /// Appended to a start when idle steering is off.
    pub idle_steering_off: Template,
    /// The project's rulebooks cannot be read: `{error}`.
    pub unreadable: Template,
}

impl RulesTexts {
    /// The texts, if each names only the placeholders it may.
    ///
    /// # Errors
    ///
    /// Names the first text that is empty, oversized, or uses an unknown
    /// placeholder.
    pub fn checked(self) -> Result<Self, String> {
        for (field, template, allowed) in [
            (
                "texts.tool_call",
                &self.tool_call,
                &["tool", "outcome", "input", "question"][..],
            ),
            ("texts.outcome_succeeded", &self.outcome_succeeded, &[][..]),
            ("texts.outcome_failed", &self.outcome_failed, &[][..]),
            (
                "texts.user_request_note",
                &self.user_request_note,
                &["user_request"][..],
            ),
            (
                "texts.closing_message_note",
                &self.closing_message_note,
                &["summary"][..],
            ),
            ("texts.tools_note", &self.tools_note, &["tools"][..]),
            ("texts.todo_item", &self.todo_item, &["content"][..]),
            (
                "texts.todo_item_in_progress",
                &self.todo_item_in_progress,
                &["content"][..],
            ),
            ("texts.todos_none", &self.todos_none, &[][..]),
            ("texts.not_running", &self.not_running, &["id"][..]),
            (
                "texts.status",
                &self.status,
                &["name", "state", "args", "used", "budget"][..],
            ),
            ("texts.state_running", &self.state_running, &[][..]),
            ("texts.state_paused", &self.state_paused, &[][..]),
            ("texts.paused", &self.paused, &["name", "id"][..]),
            ("texts.cleared", &self.cleared, &["name"][..]),
            (
                "texts.unknown_book",
                &self.unknown_book,
                &["id", "known"][..],
            ),
            ("texts.session_full", &self.session_full, &["max"][..]),
            ("texts.idle_steering_off", &self.idle_steering_off, &[][..]),
            ("texts.unreadable", &self.unreadable, &["error"][..]),
        ] {
            template.check(field, allowed)?;
        }

        Ok(self)
    }
}
