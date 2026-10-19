//! Which version of each remote skill agents may see. A skill's content is
//! identified by a digest; a version is shown once the judges find it safe,
//! or you accept it. Until then the last approved version stays, or none.

use std::collections::BTreeMap;

use chauffeur_core::SkillSafety;
use serde::{Deserialize, Serialize};

use super::review::Review;
use super::{ListedSkill, Source};

/// A failed review waits this long, doubling to 16 times, before retrying.
const RETRY_SECONDS: u64 = 900;

/// One source's review record, kept beside its listing.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Reviews {
    /// The source URL the record belongs to; another URL starts afresh.
    pub url: String,
    #[serde(default)]
    pub skills: BTreeMap<String, SkillReviews>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct SkillReviews {
    /// The version agents may see.
    pub approved: Option<Approved>,
    /// The latest version's verdict, while it is not approved.
    pub latest: Option<Verdict>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Approved {
    pub digest: String,
    pub revision: String,
    /// The listing to serve it from, whatever upstream lists now.
    pub skill: ListedSkill,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Verdict {
    pub digest: String,
    pub revision: String,
    pub safety: SkillSafety,
    pub judge: String,
    pub reason: String,
    pub concerns: Vec<String>,
    pub reviewed_at: u64,
    /// When a review that reached no judge is tried again.
    pub retry_at: Option<u64>,
    pub attempts: u32,
}

/// What agents see of one listed skill.
pub(super) enum Shown<'a> {
    Current,
    /// An earlier approved version.
    Earlier(&'a Approved),
    Withheld,
}

/// A skill version waiting for the judges.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ReviewJob {
    pub source: String,
    pub skill: String,
    pub digest: String,
    pub revision: String,
}

/// FNV-1a: stable across builds, so a stored digest stays comparable.
fn fnv(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes.iter().chain([&0xff]) {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
}

/// The skill's identity as agents would read it. Git files are named by
/// their blobs; HTTP files have no blob, so their listing revision stands in.
pub(super) fn digest(source: &Source, revision: &str, skill: &ListedSkill) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;

    fnv(&mut hash, skill.description.as_bytes());
    fnv(&mut hash, skill.body.as_bytes());

    for file in &skill.files {
        fnv(&mut hash, file.path.as_bytes());
        fnv(&mut hash, if file.executable { b"x" } else { b"-" });
        fnv(
            &mut hash,
            file.object.as_deref().unwrap_or_default().as_bytes(),
        );
    }

    if matches!(source, Source::Http { .. }) && !skill.files.is_empty() {
        fnv(&mut hash, revision.as_bytes());
    }

    format!("{hash:016x}")
}

impl Reviews {
    /// Every listed skill approved as it is: the versions already in use.
    pub(super) fn baseline(source: &Source, revision: &str, skills: &[ListedSkill]) -> Self {
        Self {
            url: source.url().into(),
            skills: skills
                .iter()
                .map(|skill| {
                    (
                        skill.id.clone(),
                        SkillReviews {
                            approved: Some(Approved {
                                digest: digest(source, revision, skill),
                                revision: revision.into(),
                                skill: skill.clone(),
                            }),
                            latest: None,
                        },
                    )
                })
                .collect(),
        }
    }

    pub(super) fn shown(&self, skill: &ListedSkill, digest: &str) -> Shown<'_> {
        match self
            .skills
            .get(&skill.id)
            .and_then(|reviews| reviews.approved.as_ref())
        {
            Some(approved) if approved.digest == digest => Shown::Current,
            Some(approved) => Shown::Earlier(approved),
            None => Shown::Withheld,
        }
    }

    /// The verdict on this version, if it was judged.
    pub(super) fn verdict(&self, skill: &str, digest: &str) -> Option<&Verdict> {
        self.skills
            .get(skill)
            .and_then(|reviews| reviews.latest.as_ref())
            .filter(|verdict| verdict.digest == digest)
    }

    /// Whether this version still needs the judges at `now`.
    pub(super) fn due(&self, skill: &ListedSkill, digest: &str, now: u64) -> bool {
        if matches!(self.shown(skill, digest), Shown::Current) {
            return false;
        }

        self.verdict(&skill.id, digest)
            .is_none_or(|verdict| verdict.retry_at.is_some_and(|at| at <= now))
    }

    /// Record the judges' answer for `job`; a safe version is approved.
    pub(super) fn record(
        &mut self,
        job: &ReviewJob,
        skill: &ListedSkill,
        review: &Review,
        now: u64,
    ) {
        let entry = self.skills.entry(job.skill.clone()).or_default();

        if review.safety == SkillSafety::Safe {
            entry.approved = Some(Approved {
                digest: job.digest.clone(),
                revision: job.revision.clone(),
                skill: skill.clone(),
            });
            entry.latest = None;

            return;
        }

        let attempts = entry
            .latest
            .as_ref()
            .filter(|verdict| verdict.digest == job.digest)
            .map_or(0, |verdict| verdict.attempts)
            .saturating_add(1);
        // No judge decided: try again later. A judge's verdict stands until upstream changes.
        let unjudged = review.safety == SkillSafety::Undetermined && review.judge.is_empty();

        entry.latest = Some(Verdict {
            digest: job.digest.clone(),
            revision: job.revision.clone(),
            safety: review.safety,
            judge: review.judge.clone(),
            reason: review.reason.clone(),
            concerns: review.concerns.clone(),
            reviewed_at: now,
            retry_at: unjudged.then(|| now + (RETRY_SECONDS << attempts.saturating_sub(1).min(4))),
            attempts,
        });
    }

    /// Approve this version, whatever the judges said.
    pub(super) fn accept(&mut self, digest: &str, revision: &str, skill: &ListedSkill) {
        let entry = self.skills.entry(skill.id.clone()).or_default();

        entry.approved = Some(Approved {
            digest: digest.into(),
            revision: revision.into(),
            skill: skill.clone(),
        });
        entry.latest = None;
    }
}
