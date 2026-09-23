//! Who a reminder is for. Mirrors SourceFed's `MonitorTarget`: a host kind
//! plus the host's own id (an OpenCode session, a CLI hostname, …).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, Hash)]
pub struct Target {
    pub kind: String,
    pub id: String,
}

impl Target {
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
        }
    }

    /// Stable map key.
    pub fn key(&self) -> String {
        format!("{}:{}", self.kind, self.id)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.kind.is_empty() || self.id.is_empty() {
            return Err("target needs a non-empty kind and id".to_string());
        }

        Ok(())
    }
}
