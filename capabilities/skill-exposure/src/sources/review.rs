//! A candidate as a skill-review signal, and the verdict read back from the
//! daemon's effects. The judgment itself is the `skill-review` capability's.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use chauffeur_core::signal::MAX_SKILL_FILES;
use chauffeur_core::{Effect, MAX_PROMPT_BYTES, Signal, SignalKind, SkillFile, SkillSafety};
use serde::Serialize;

use super::git::Repository;
use super::skilld::{Candidate, DiscoveryError};

/// Larger files are listed as not shown.
const MAX_REVIEW_FILE: usize = 16 * 1024;
/// File contents one review reads, besides `SKILL.md`.
const MAX_REVIEW_FILES_BYTES: usize = 48 * 1024;

/// A skill's text as a review reads it, at the commit Skilld reviewed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gathered {
    pub instructions: String,
    pub files: Vec<SkillFile>,
}

/// A script or program, rather than a document.
fn runs(path: &str) -> bool {
    const CODE: &[&str] = &[
        "sh", "bash", "zsh", "fish", "ps1", "py", "js", "mjs", "cjs", "ts", "mts", "rb", "pl",
        "php", "lua", "go", "rs",
    ];
    let name = path.rsplit('/').next().unwrap_or(path);

    path.split('/')
        .next()
        .is_some_and(|top| matches!(top, "scripts" | "bin" | "hooks"))
        || name
            .rsplit_once('.')
            .is_some_and(|(_, extension)| CODE.contains(&extension))
        || !name.contains('.')
}

fn text(bytes: Vec<u8>) -> Option<String> {
    String::from_utf8(bytes)
        .ok()
        .filter(|text| !text.contains('\0'))
}

/// The skill's `SKILL.md` and readable text files at the commit Skilld
/// reviewed, fetched with Git as Chauffeur fetches every source, through a
/// partial clone at `scratch`. Scripts are read first; binary, oversized,
/// or over-budget files are listed without content.
///
/// # Errors
/// Returns a skill with no known commit, a `SKILL.md` that is missing,
/// binary, or too large to review, too many files, or a failed fetch.
pub async fn gather(candidate: &Candidate, scratch: &Path) -> Result<Gathered, DiscoveryError> {
    let named = |what: &str| DiscoveryError::Invalid(format!("{}: {what}", candidate.selector));
    let commit = candidate
        .commit
        .as_deref()
        .ok_or_else(|| named("Skilld gives no commit, so its files cannot be read"))?;
    let repository = Repository { path: scratch };
    let mut tree = repository
        .files_at(&candidate.url, commit, &candidate.dir)
        .await?;
    let main = tree
        .iter()
        .position(|(path, _)| path == "SKILL.md")
        .ok_or_else(|| named("its SKILL.md is missing at that commit"))?;
    let (_, main) = tree.remove(main);

    if tree.len() > MAX_SKILL_FILES {
        return Err(named("it carries too many files to review"));
    }

    // When not everything fits, what runs is read first.
    tree.sort_by_key(|(path, _)| !runs(path));

    let sizes: HashMap<&str, u64> = candidate
        .files
        .iter()
        .map(|file| (file.path.as_str(), file.size))
        .collect();
    let mut reserved = 0_usize;
    let wanted: Vec<bool> = tree
        .iter()
        .map(|(path, _)| {
            let size = sizes.get(path.as_str()).map_or(MAX_REVIEW_FILE, |size| {
                usize::try_from(*size).unwrap_or(usize::MAX)
            });
            let fits = size <= MAX_REVIEW_FILE && reserved + size <= MAX_REVIEW_FILES_BYTES;

            if fits {
                reserved += size;
            }

            fits
        })
        .collect();
    let mut objects = vec![main.as_str()];

    objects.extend(
        tree.iter()
            .zip(&wanted)
            .filter(|(_, wanted)| **wanted)
            .map(|((_, object), _)| object.as_str()),
    );

    let mut blobs = repository.blobs(&objects).await?.into_iter();
    let instructions = blobs
        .next()
        .filter(|bytes| bytes.len() <= MAX_PROMPT_BYTES)
        .and_then(text)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| named("its SKILL.md is empty, binary, or too large to review"))?;
    let mut total = 0;
    let files = tree
        .into_iter()
        .zip(wanted)
        .map(|((path, _), wanted)| {
            let content = wanted
                .then(|| blobs.next())
                .flatten()
                .filter(|bytes| bytes.len() <= MAX_REVIEW_FILE)
                .and_then(text)
                .filter(|text| total + text.len() <= MAX_REVIEW_FILES_BYTES);

            total += content.as_ref().map_or(0, String::len);

            SkillFile { path, content }
        })
        .collect();

    Ok(Gathered {
        instructions,
        files,
    })
}

/// What the judges decided about one skill.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Review {
    pub safety: SkillSafety,
    /// `system_one` or `system_two`; empty when no judge decided.
    pub judge: String,
    pub reason: String,
    pub concerns: Vec<String>,
}

impl Review {
    /// No verdict, and why: never read as safe.
    pub fn undetermined(reason: impl Into<String>) -> Self {
        Self {
            safety: SkillSafety::Undetermined,
            judge: String::new(),
            reason: reason.into(),
            concerns: Vec::new(),
        }
    }
}

/// The signal asking the daemon to review `candidate`, as `agent_id`.
#[must_use]
pub fn review_signal(
    candidate: &Candidate,
    gathered: Gathered,
    agent_id: String,
    at: u64,
) -> Signal {
    let mut provenance = format!(
        "Author: {}. License: {}. Commit: {}. Registry page: {}.",
        candidate.author.as_deref().unwrap_or("unknown"),
        candidate.license.as_deref().unwrap_or("unknown"),
        candidate.commit.as_deref().unwrap_or("unknown"),
        candidate.page,
    );

    if !candidate.behaviors.is_empty() {
        provenance.push_str("\nThe registry's text patterns flagged:");

        for behavior in &candidate.behaviors {
            let _ = write!(
                provenance,
                "\n- {} ({}), {} time(s): {}",
                behavior.id,
                behavior.label,
                behavior.total,
                behavior.locations.join(", ")
            );
        }
    }

    Signal {
        agent_id,
        at,
        kind: SignalKind::SkillReview {
            name: candidate.name.clone(),
            source: format!("{} {}", candidate.url, candidate.dir),
            provenance,
            instructions: gathered.instructions,
            files: gathered.files,
        },
    }
}

/// The verdict among a review's effects; none is undetermined.
#[must_use]
pub fn read_verdict(effects: &[Effect]) -> Review {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::SkillVerdict {
                safety,
                judge,
                reason,
                concerns,
                ..
            } => Some(Review {
                safety: *safety,
                judge: judge.clone(),
                reason: reason.clone(),
                concerns: concerns.clone(),
            }),
            _ => None,
        })
        .unwrap_or_else(|| {
            Review::undetermined("the daemon gave no verdict; is skill-review disabled?")
        })
}
