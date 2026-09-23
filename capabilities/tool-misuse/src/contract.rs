//! Misuse contracts: strict JSON files, one per misuse, in a contracts
//! directory (`skills/misuse/` in this repo).

use std::collections::HashSet;
use std::path::Path;

use chauffeur_core::read_json_files;
use serde::Deserialize;

const MAX_CONTRACTS: usize = 64;
const MAX_CONTRACT_BYTES: u64 = 16_384;
const MAX_TOOLS: usize = 16;
const MAX_ID_BYTES: usize = 64;
const MAX_TEXT_BYTES: usize = 1_024;
const MAX_COOLDOWN_SECS: u64 = 86_400;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub id: String,
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Match {
    /// Tool IDs as the host names them, such as `shell`.
    pub tools: Vec<String>,
}

/// One misuse the agent should be steered away from.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MisuseContract {
    pub schema_version: u32,
    pub identity: Identity,
    #[serde(rename = "match")]
    pub matches: Match,
    /// Asked about the call, as a yes/no: "did the agent …?".
    pub question: String,
    /// Steer when P(yes) is at least this…
    pub nudge_at_or_above: f32,
    /// …and the answer is at least this confident.
    pub minimum_confidence: f32,
    /// Sent to the agent when the misuse is confirmed.
    pub steer: String,
    /// A skill delivered with the steer, such as `slack-cli`.
    #[serde(default)]
    pub handoff_skill: Option<String>,
    pub cooldown_seconds: u64,
}

impl MisuseContract {
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let contract: Self =
            serde_json::from_slice(bytes).map_err(|error| format!("invalid contract: {error}"))?;

        contract.validate()?;

        Ok(contract)
    }

    #[must_use]
    pub fn watches(&self, tool: &str) -> bool {
        self.matches.tools.iter().any(|watched| watched == tool)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err(format!(
                "unsupported schema_version {}",
                self.schema_version
            ));
        }

        let id = &self.identity.id;

        if id.is_empty()
            || id.len() > MAX_ID_BYTES
            || !id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(format!(
                "identity.id {id:?} must be 1-{MAX_ID_BYTES} of [a-z0-9-]"
            ));
        }
        if self.matches.tools.is_empty() || self.matches.tools.len() > MAX_TOOLS {
            return Err(format!("match.tools must list 1-{MAX_TOOLS} tools"));
        }
        for (field, text) in [("question", &self.question), ("steer", &self.steer)] {
            if text.trim().is_empty() || text.len() > MAX_TEXT_BYTES {
                return Err(format!("{field} must be 1-{MAX_TEXT_BYTES} bytes"));
            }
        }
        if !(0.5..=1.0).contains(&self.nudge_at_or_above)
            || !(0.0..=1.0).contains(&self.minimum_confidence)
        {
            return Err("nudge_at_or_above must be in 0.5-1 and minimum_confidence in 0-1".into());
        }
        if self.cooldown_seconds > MAX_COOLDOWN_SECS {
            return Err(format!("cooldown_seconds exceeds {MAX_COOLDOWN_SECS}"));
        }

        Ok(())
    }
}

/// Every contract in `directory`; a missing directory has none. IDs must be
/// unique.
pub fn load_contracts(directory: &Path) -> Result<Vec<MisuseContract>, String> {
    let mut ids = HashSet::new();

    read_json_files(directory, MAX_CONTRACTS, MAX_CONTRACT_BYTES)?
        .into_iter()
        .map(|(path, bytes)| {
            let contract = MisuseContract::from_json(&bytes)
                .map_err(|error| format!("{}: {error}", path.display()))?;

            if !ids.insert(contract.identity.id.clone()) {
                return Err(format!(
                    "{}: duplicate id {}",
                    path.display(),
                    contract.identity.id
                ));
            }

            Ok(contract)
        })
        .collect()
}
