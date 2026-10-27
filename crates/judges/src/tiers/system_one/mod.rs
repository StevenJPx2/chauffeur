//! The System One tier: fast classifiers. Its `systemone` format speaks the
//! `/v1/systemone` API that Jev serves.

#[cfg(feature = "systemone")]
mod client;
#[cfg(feature = "systemone")]
mod wire;

#[cfg(feature = "systemone")]
pub use client::SystemOne;
use serde::Deserialize;

use crate::config::KeySource;
use crate::format::Format;

/// A model that speaks the `/v1/systemone` API.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemOneEntry {
    /// The API's base URL; requests go to `{endpoint}/v1/systemone`.
    pub endpoint: String,
    /// The model ID sent with each request.
    pub model: String,
    /// The key, sent as a bearer token.
    pub key: KeySource,
    /// How long one call may take, in milliseconds.
    pub timeout_ms: Option<u64>,
}

/// A client for `entry` that sends `key`.
#[cfg(feature = "systemone")]
#[expect(
    clippy::unnecessary_wraps,
    reason = "matches the build without the `systemone` feature"
)]
pub fn format(entry: &SystemOneEntry, key: String) -> Option<Box<dyn Format>> {
    Some(Box::new(SystemOne::new(entry, key)))
}

/// Built without HTTP, as for the browser: the model is left out.
#[cfg(not(feature = "systemone"))]
pub fn format(_: &SystemOneEntry, _: String) -> Option<Box<dyn Format>> {
    None
}
