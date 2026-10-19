//! Approval turns validated Skilld candidates into a source the user owns.
//! It decides the change without I/O; writing is a separate, explicit step.

use std::collections::BTreeSet;
use std::io::{ErrorKind, Read};
use std::path::Path;

use serde::Serialize;

use chauffeur_core::SkillSafety;

use super::model::{MAX_PATHS, MAX_SOURCES, valid_name};
use super::review::Review;
use super::skilld::{Candidate, DiscoveryError, Selector, Tier};
use super::{MAX_CONFIG, Source, SourcesConfig};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    /// A new source for a repository not yet followed.
    Added { source: String },
    /// A followed repository gains this path.
    Extended { source: String },
    /// A source already follows it; nothing to write.
    Covered { source: String },
}

/// What the user accepted beyond a clean approval.
#[derive(Clone, Debug, Default)]
pub struct Consent {
    /// `ask` behaviors Skilld flagged that the user allows.
    pub allow: Vec<String>,
    /// Add skills the judges found unsafe.
    pub accept_risk: bool,
    /// Add skills no judge reached a verdict on.
    pub unvetted: bool,
}

/// What approving a selector would do. Nothing is written until the caller
/// passes `config` to [`write_config`], and only when [`Approval::writes`].
#[derive(Clone, Debug, Serialize)]
pub struct Approval {
    pub selector: String,
    pub url: String,
    /// The folder the source follows: the skill's own, or the deepest one
    /// holding every skill in the repository, which includes future ones.
    pub dir: String,
    pub skills: Vec<Candidate>,
    /// Each skill's review, in the order of `skills`.
    pub reviews: Vec<Review>,
    /// `ask` behaviors the user has not allowed.
    pub blocked: Vec<String>,
    /// Why the judges' verdicts stop this approval, each naming its override.
    pub refused: Vec<String>,
    pub change: Change,
    #[serde(skip)]
    pub config: SourcesConfig,
}

impl Approval {
    pub fn writes(&self) -> bool {
        self.blocked.is_empty()
            && self.refused.is_empty()
            && !matches!(self.change, Change::Covered { .. })
    }
}

/// Selectors of the reviewed skills judged `safety`.
fn judged(candidates: &[Candidate], reviews: &[Review], safety: SkillSafety) -> Vec<String> {
    candidates
        .iter()
        .zip(reviews)
        .filter(|(_, review)| review.safety == safety)
        .map(|(candidate, _)| candidate.selector.clone())
        .collect()
}

/// Decide the change approving `candidates` makes to `config`, held to the
/// judges' `reviews` (in the same order; a missing review is no verdict).
///
/// # Errors
/// Returns mixed repositories, or a change that would exceed the source limits.
pub fn approve(
    selector: &Selector,
    candidates: Vec<Candidate>,
    config: &SourcesConfig,
    consent: &Consent,
    reviews: &[Review],
) -> Result<Approval, DiscoveryError> {
    let allow = &consent.allow;
    let Some(first) = candidates.first() else {
        return Err(DiscoveryError::Invalid(format!(
            "{selector} names no skills"
        )));
    };
    let url = first.url.clone();

    if candidates
        .iter()
        .any(|candidate| key(&candidate.url) != key(&url))
    {
        return Err(DiscoveryError::Invalid(format!(
            "the skills in {selector} come from different repositories"
        )));
    }

    let dir = candidates
        .iter()
        .map(|candidate| candidate.dir.clone())
        .reduce(|left, right| common_dir(&left, &right))
        .unwrap_or_else(|| ".".into());
    let blocked: BTreeSet<String> = candidates
        .iter()
        .flat_map(|candidate| &candidate.behaviors)
        .filter(|behavior| behavior.tier == Tier::Ask && !allow.contains(&behavior.id))
        .map(|behavior| behavior.id.clone())
        .collect();
    let mut config = config.clone();
    let change = merge(&mut config, &url, &dir, &selector.repository)?;

    config = config.check()?;

    let reviews: Vec<Review> = (0..candidates.len())
        .map(|index| {
            reviews
                .get(index)
                .cloned()
                .unwrap_or_else(|| Review::undetermined("not reviewed"))
        })
        .collect();
    let mut refused = Vec::new();
    let unsafe_ = judged(&candidates, &reviews, SkillSafety::Unsafe);
    let unvetted = judged(&candidates, &reviews, SkillSafety::Undetermined);

    if !unsafe_.is_empty() && !consent.accept_risk {
        refused.push(format!(
            "the judges found {} unsafe; --accept-risk adds it anyway",
            unsafe_.join(", ")
        ));
    }

    if !unvetted.is_empty() && !consent.unvetted {
        refused.push(format!(
            "no judge reached a verdict on {}; --unvetted adds it anyway",
            unvetted.join(", ")
        ));
    }

    Ok(Approval {
        selector: selector.to_string(),
        url,
        dir,
        skills: candidates,
        reviews,
        blocked: blocked.into_iter().collect(),
        refused,
        change,
        config,
    })
}

/// A repository's identity across HTTPS and SSH spellings.
fn key(url: &str) -> String {
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .split_once("://")
        .map_or(lower.as_str(), |(_, rest)| rest);
    let rest = match rest.split_once('@') {
        Some((user, host)) if !user.contains('/') => host,
        _ => rest,
    };
    let rest = rest.replacen(':', "/", 1);
    let rest = rest.trim_end_matches('/');

    rest.strip_suffix(".git").unwrap_or(rest).to_owned()
}

fn covers(path: &str, dir: &str) -> bool {
    path == "." || path == dir || dir.starts_with(&format!("{path}/"))
}

fn common_dir(left: &str, right: &str) -> String {
    let shared: Vec<&str> = left
        .split('/')
        .zip(right.split('/'))
        .take_while(|(a, b)| a == b && *a != ".")
        .map(|(a, _)| a)
        .collect();

    if shared.is_empty() {
        ".".into()
    } else {
        shared.join("/")
    }
}

fn merge(
    config: &mut SourcesConfig,
    url: &str,
    dir: &str,
    repository: &str,
) -> Result<Change, DiscoveryError> {
    let repository_key = key(url);
    let followed = config.sources.iter_mut().find_map(|source| match source {
        Source::Git { id, url, paths, .. } if key(url) == repository_key => Some((id, paths)),
        _ => None,
    });

    if let Some((id, paths)) = followed {
        if paths.iter().any(|path| covers(path, dir)) {
            return Ok(Change::Covered { source: id.clone() });
        }

        if dir == "." {
            *paths = vec![".".into()];
        } else {
            paths.retain(|path| !covers(dir, path));
            paths.push(dir.into());
        }

        if paths.len() > MAX_PATHS {
            return Err(DiscoveryError::Invalid(format!(
                "source {id} would follow more than {MAX_PATHS} paths; approve the repository instead"
            )));
        }

        return Ok(Change::Extended { source: id.clone() });
    }

    if config.sources.len() >= MAX_SOURCES {
        return Err(DiscoveryError::Invalid(format!(
            "skill-sources.json already has the maximum {MAX_SOURCES} sources"
        )));
    }

    let id = new_id(config, repository, url);

    config.sources.push(Source::Git {
        id: id.clone(),
        url: url.into(),
        reference: "HEAD".into(),
        paths: vec![dir.into()],
    });

    Ok(Change::Added { source: id })
}

fn slug(text: &str) -> String {
    let mut slug = String::new();

    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }

    slug.truncate(48);

    slug.trim_end_matches('-').to_owned()
}

fn new_id(config: &SourcesConfig, repository: &str, url: &str) -> String {
    let taken = |id: &str| config.sources.iter().any(|source| source.id() == id);
    let owner = key(url).rsplit('/').nth(1).map(slug).unwrap_or_default();
    let base = Some(slug(repository))
        .filter(|id| valid_name(id) && !taken(id))
        .or_else(|| Some(slug(&format!("{owner}-{repository}"))).filter(|id| valid_name(id)))
        .unwrap_or_else(|| "skilld".into());

    if !taken(&base) {
        return base;
    }

    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken(id))
        .unwrap_or(base)
}

/// The user's source configuration, or an empty one when there is none.
///
/// # Errors
/// Returns an unreadable, oversized, or invalid file: approval never replaces
/// a file it cannot understand.
pub fn read_config(path: &Path) -> Result<SourcesConfig, DiscoveryError> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(SourcesConfig::default()),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();

    file.take(MAX_CONFIG as u64 + 1).read_to_end(&mut bytes)?;

    if bytes.len() > MAX_CONFIG {
        return Err(DiscoveryError::Invalid(format!(
            "{} exceeds {MAX_CONFIG} bytes",
            path.display()
        )));
    }

    Ok(serde_json::from_slice::<SourcesConfig>(&bytes)
        .map_err(|error| DiscoveryError::Invalid(format!("{}: {error}", path.display())))?
        .check()?)
}

/// One source per line, as people write them by hand.
fn render(config: &SourcesConfig) -> Result<String, DiscoveryError> {
    let mut text = format!(
        "{{\n  \"refresh_seconds\": {},\n  \"sources\": [",
        config.refresh_seconds
    );

    for (index, source) in config.sources.iter().enumerate() {
        let separator = if index == 0 { "" } else { "," };

        text.push_str(&format!(
            "{separator}\n    {}",
            serde_json::to_string(source)?
        ));
    }

    text.push_str("\n  ]");

    if !config.descriptions.is_empty() {
        text.push_str(",\n  \"descriptions\": {");

        for (index, (name, description)) in config.descriptions.iter().enumerate() {
            let separator = if index == 0 { "" } else { "," };

            text.push_str(&format!(
                "{separator}\n    {}: {}",
                serde_json::to_string(name)?,
                serde_json::to_string(description)?
            ));
        }

        text.push_str("\n  }");
    }

    text.push_str("\n}\n");

    Ok(text)
}

/// Replace the configuration atomically; the daemon reloads it on change.
/// A symlinked file is written through to its target.
///
/// # Errors
/// Returns an invalid or oversized configuration, or a write failure.
pub fn write_config(path: &Path, config: &SourcesConfig) -> Result<(), DiscoveryError> {
    let text = render(&config.clone().check()?)?;

    if text.len() > MAX_CONFIG {
        return Err(DiscoveryError::Invalid(format!(
            "the sources would exceed {MAX_CONFIG} bytes"
        )));
    }

    let target = match std::fs::canonicalize(path) {
        Ok(target) => target,
        Err(error) if error.kind() == ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => return Err(error.into()),
    };
    let directory = target
        .parent()
        .ok_or_else(|| DiscoveryError::Invalid("the sources file has no folder".into()))?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("skill-sources.json");
    let temporary = directory.join(format!(".{name}.{}.tmp", std::process::id()));

    std::fs::create_dir_all(directory)?;
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, &target).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })?;

    Ok(())
}
