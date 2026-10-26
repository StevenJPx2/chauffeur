use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use super::{SourceError, valid_path};

pub(super) const MAX_SOURCES: usize = 32;
/// Paths one Git source follows.
pub(super) const MAX_PATHS: usize = 16;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Source {
    Git {
        id: String,
        url: String,
        #[serde(default = "head", rename = "ref", skip_serializing_if = "is_head")]
        reference: String,
        #[serde(default = "roots", skip_serializing_if = "is_roots")]
        paths: Vec<String>,
    },
    Http {
        id: String,
        url: String,
    },
}

fn head() -> String {
    "HEAD".into()
}
fn roots() -> Vec<String> {
    vec![".".into()]
}
fn is_head(reference: &str) -> bool {
    reference == "HEAD"
}
fn is_roots(paths: &[String]) -> bool {
    paths == ["."]
}
/// Once a day; a new or edited source is checked right away regardless.
fn refresh() -> u64 {
    86_400
}

impl Source {
    pub fn id(&self) -> &str {
        match self {
            Self::Git { id, .. } | Self::Http { id, .. } => id,
        }
    }

    pub fn url(&self) -> &str {
        match self {
            Self::Git { url, .. } | Self::Http { url, .. } => url,
        }
    }

    fn check(&self) -> Result<(), SourceError> {
        if !valid_name(self.id()) {
            return Err(SourceError::Invalid("invalid source id"));
        }

        if let Self::Git {
            reference, paths, ..
        } = self
        {
            if reference.is_empty()
                || reference.starts_with('-')
                || reference.contains("..")
                || !reference
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_./-".contains(&b))
            {
                return Err(SourceError::Invalid("invalid Git ref"));
            }

            if paths.is_empty()
                || paths.len() > MAX_PATHS
                || !paths.iter().all(|p| p == "." || valid_path(p))
            {
                return Err(SourceError::Invalid(
                    "Git paths must stay within the repository",
                ));
            }

            if let Some((user_host, path)) = self.url().split_once(':') {
                if !self.url().contains("://")
                    && user_host.contains('@')
                    && !path.is_empty()
                    && valid_path(path)
                    && user_host
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"@.-_".contains(&b))
                {
                    return Ok(());
                }
            }
        }

        let url = reqwest::Url::parse(self.url())
            .map_err(|_| SourceError::Invalid("invalid source URL"))?;
        let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        let allowed = match self {
            Self::Http { .. } => url.scheme() == "https" || (local && url.scheme() == "http"),
            Self::Git { .. } => {
                matches!(url.scheme(), "https" | "ssh") || (local && url.scheme() == "git")
            }
        };

        if !allowed
            || url.password().is_some()
            || (url.scheme() != "ssh" && !url.username().is_empty())
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(SourceError::Invalid(
                "sources require HTTPS or Git SSH, without embedded secrets; loopback development servers may use HTTP or git",
            ));
        }

        Ok(())
    }
}

pub(super) fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourcesConfig {
    #[serde(default = "refresh")]
    pub refresh_seconds: u64,
    pub sources: Vec<Source>,
    /// Your wording for a remote skill's description, by skill name. Only
    /// selection reads it; the instructions and files still come from upstream.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub descriptions: BTreeMap<String, String>,
}

impl Default for SourcesConfig {
    fn default() -> Self {
        Self {
            refresh_seconds: refresh(),
            sources: Vec::new(),
            descriptions: BTreeMap::new(),
        }
    }
}

impl SourcesConfig {
    pub(super) fn check(self) -> Result<Self, SourceError> {
        if self.sources.len() > MAX_SOURCES || !(60..=604_800).contains(&self.refresh_seconds) {
            return Err(SourceError::Invalid(
                "at most 32 sources; refresh_seconds must be in 60..=604800",
            ));
        }

        if self.descriptions.len() > 256
            || !self.descriptions.iter().all(|(name, text)| {
                valid_name(name) && !text.trim().is_empty() && text.len() <= 4096
            })
        {
            return Err(SourceError::Invalid(
                "descriptions maps at most 256 skill names to nonempty texts of at most 4096 bytes",
            ));
        }

        let mut ids = HashSet::new();

        for source in &self.sources {
            source.check()?;

            if !ids.insert(source.id()) {
                return Err(SourceError::Invalid("duplicate source id"));
            }
        }

        Ok(self)
    }
}
