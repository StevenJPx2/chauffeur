//! Integration surface for sourcefed: its daemon POSTs a copy of every
//! monitor event (`SOURCEFED_FORWARD_URL`), which becomes an
//! `integration_event` signal for the session the monitor belongs to.

use chauffeur_core::{MAX_TEXT_BYTES, Signal, SignalKind};
use serde::Deserialize;

/// The body sourcefed forwards. Unknown fields are ignored so sourcefed can
/// grow its event shape without breaking the surface.
#[derive(Debug, Deserialize)]
pub struct ForwardedEvent {
    target: Target,
    source: Source,
    event: Event,
}

#[derive(Debug, Deserialize)]
struct Target {
    id: String,
}

#[derive(Debug, Deserialize)]
struct Source {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct Event {
    kind: String,
    summary: String,
    #[serde(default)]
    body: Option<String>,
    actionable: bool,
}

impl ForwardedEvent {
    /// The signal for the monitor's session, received at `at`.
    #[must_use]
    pub fn into_signal(self, at: u64) -> Signal {
        Signal {
            agent_id: self.target.id,
            at,
            kind: SignalKind::IntegrationEvent {
                source: clip(&self.source.kind),
                kind: clip(&self.event.kind),
                summary: clip(&self.event.summary),
                body: clip(self.event.body.as_deref().unwrap_or_default()),
                actionable: self.event.actionable,
            },
        }
    }
}

/// Clip to the signal's text bound on a character boundary.
fn clip(text: &str) -> String {
    let mut end = text.len().min(MAX_TEXT_BYTES);

    while !text.is_char_boundary(end) {
        end -= 1;
    }

    text[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forwarded_event_becomes_an_integration_event_for_its_session() {
        let body = serde_json::json!({
            "target": { "kind": "opencode-session", "id": "ses_1" },
            "monitorID": "mon_1",
            "source": { "type": "github", "repo": "owner/repo", "prNumber": 42 },
            "event": { "kind": "merged", "at": "2026-09-23T10:00:00.000Z", "summary": "PR #42 merged",
                       "actionable": true, "terminal": true, "future": "ignored" },
        });
        let event: ForwardedEvent = serde_json::from_value(body).expect("sourcefed shape");
        let signal = event.into_signal(7);

        signal.validate().expect("within bounds");
        assert_eq!(signal.agent_id, "ses_1");
        assert_eq!(
            signal.kind,
            SignalKind::IntegrationEvent {
                source: "github".into(),
                kind: "merged".into(),
                summary: "PR #42 merged".into(),
                body: String::new(),
                actionable: true,
            }
        );
    }

    #[test]
    fn long_text_is_clipped_on_a_character_boundary() {
        let clipped = clip(&"é".repeat(MAX_TEXT_BYTES));

        assert!(clipped.len() <= MAX_TEXT_BYTES);
        assert!(clipped.chars().all(|c| c == 'é'));
    }
}
