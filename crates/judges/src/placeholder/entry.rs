use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

/// Recorded or fixed answers that stand in for a real model.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceholderEntry {
    /// A file of answers matched by evidence, relative to the `models.json`
    /// that names it.
    pub answers: Option<PathBuf>,
    /// The probabilities given for any question no recorded answer matches.
    pub otherwise: Option<BTreeMap<String, f64>>,
}
