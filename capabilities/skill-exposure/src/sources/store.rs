use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use chauffeur_core::{
    HeldSkill, RemoteSkill, SkillAcceptResult, SkillSourceState, SkillSourceStatus, SkillsResult,
};
use tokio::fs;
use tokio::io::AsyncReadExt;

use super::review::{Review, ReviewRequest, chosen, instructions, reading_order, shown};
use super::vetting::{Approved, ReviewJob, Reviews, Shown, digest};
use super::{
    Check, ListedSkill, Listing, MAX_CONFIG, MAX_FILE, MAX_LISTED_FILES, MAX_REVISIONS, Source,
    SourceError, SourcesConfig, git::Repository, http, valid_path,
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A failed check is retried after this, or the refresh interval when shorter.
const RETRY_SECONDS: u64 = 900;
/// A review record: verdicts and approved listings for one source's skills.
const MAX_REVIEWS: usize = 8 * 1024 * 1024;

struct Entry {
    source: Source,
    listing: Option<Listing>,
    state: SkillSourceState,
    /// When the latest check failed; cleared by a successful one.
    failed_at: Option<u64>,
    /// Which version of each skill agents may see.
    reviews: Reviews,
}

impl Entry {
    fn due(&self, now: u64, refresh: u64) -> bool {
        let (last, wait) = match (self.failed_at, &self.listing) {
            (Some(failed), _) => (failed, refresh.min(RETRY_SECONDS)),
            (None, Some(listing)) => (listing.checked_at, refresh),
            (None, None) => return true,
        };

        now.saturating_sub(last) >= wait
    }
}

/// A host-independent catalog. The caller owns scheduling, cancellation,
/// publication, and exclusive writer access to this cache. Fetched skill
/// folders are never evicted beneath a session.
pub struct SourceStore {
    config_path: PathBuf,
    cache: PathBuf,
    config: SourcesConfig,
    entries: HashMap<String, Entry>,
    client: reqwest::Client,
    /// Show only approved versions; without a reviewer, every listed one.
    reviewed: bool,
}

impl SourceStore {
    /// Construct without network or filesystem work.
    ///
    /// # Errors
    /// Returns an HTTP client construction or path resolution error.
    pub fn new(config_path: PathBuf, cache: PathBuf) -> Result<Self, SourceError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()?;

        Ok(Self {
            config_path: std::path::absolute(config_path)?,
            cache: std::path::absolute(cache)?,
            config: SourcesConfig::default(),
            entries: HashMap::new(),
            client,
            reviewed: false,
        })
    }

    /// Agents see a skill's version only once the judges or you approve it;
    /// the caller runs the reviews from [`Self::due_reviews`].
    #[must_use]
    pub fn with_reviews(mut self) -> Self {
        self.reviewed = true;
        self
    }

    async fn configuration(&mut self) -> Result<(), SourceError> {
        let config = match read(&self.config_path, MAX_CONFIG).await {
            Ok(bytes) => serde_json::from_slice::<SourcesConfig>(&bytes)?.check()?,
            Err(SourceError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                SourcesConfig::default()
            }
            Err(error) => return Err(error),
        };

        // Parse before changing ownership. Invalid edits preserve the previous catalog.
        self.entries.retain(|id, entry| {
            config
                .sources
                .iter()
                .any(|source| source.id() == id && source == &entry.source)
        });
        self.config = config;

        Ok(())
    }

    fn root(&self, source: &Source) -> PathBuf {
        self.cache.join(source.id())
    }

    /// Restore cached descriptions without contacting any upstream source.
    ///
    /// # Errors
    /// Returns invalid or unreadable configuration errors. Individual cache errors become source status.
    pub async fn restore(&mut self) -> Result<SkillsResult, SourceError> {
        self.configuration().await?;

        for source in self.config.sources.clone() {
            if self.entries.contains_key(source.id()) {
                continue;
            }

            let listing = self.cached(&source).await;
            let state = match &listing {
                Ok(listing) => SkillSourceState::Ready {
                    revision: listing.revision.clone(),
                    checked_at: listing.checked_at,
                },
                Err(error) => SkillSourceState::Unavailable {
                    error: error.to_string(),
                },
            };
            let reviews = self.reviews(&source, listing.as_ref().ok()).await;

            self.entries.insert(
                source.id().into(),
                Entry {
                    source,
                    listing: listing.ok(),
                    state,
                    failed_at: None,
                    reviews,
                },
            );
        }

        Ok(self.catalog())
    }

    /// The source's review record. A source cached before reviews existed
    /// starts with what it already lists approved; a new source, or an
    /// unreadable record, with nothing approved.
    async fn reviews(&self, source: &Source, cached: Option<&Listing>) -> Reviews {
        let path = self.root(source).join("reviews.json");
        let empty = || Reviews {
            url: source.url().into(),
            ..Reviews::default()
        };

        if !self.reviewed {
            return empty();
        }

        match read(&path, MAX_REVIEWS).await {
            Ok(bytes) => serde_json::from_slice::<Reviews>(&bytes)
                .ok()
                .filter(|reviews| reviews.url == source.url())
                .unwrap_or_else(empty),
            Err(SourceError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                let reviews = cached.map_or_else(empty, |listing| {
                    Reviews::baseline(source, &listing.revision, &listing.skills)
                });

                if let Err(error) = self.save_reviews(source, &reviews).await {
                    eprintln!("chauffeur: skill reviews for {}: {error}", source.id());
                }

                reviews
            }
            Err(_) => empty(),
        }
    }

    async fn save_reviews(&self, source: &Source, reviews: &Reviews) -> Result<(), SourceError> {
        let root = self.root(source);
        let temporary = root.join(format!(
            "reviews.json.{}.tmp",
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));

        fs::create_dir_all(&root).await?;
        fs::write(&temporary, serde_json::to_vec(reviews)?).await?;
        fs::rename(&temporary, root.join("reviews.json")).await?;

        Ok(())
    }

    async fn cached(&self, source: &Source) -> Result<Listing, SourceError> {
        let listing: Listing = serde_json::from_slice(
            &read(&self.root(source).join("listing.json"), 4 * 1024 * 1024).await?,
        )?;

        if &listing.source != source || !valid_revision(&listing.revision) {
            return Err(SourceError::Invalid(
                "cached listing does not match its configured source",
            ));
        }

        Ok(listing)
    }

    /// Re-read the allowlist and check due sources, serially, within a
    /// 20-second cycle budget. A check that finds no change downloads nothing.
    /// `now` is Unix time in seconds, injected so expiry is deterministic in tests.
    ///
    /// # Errors
    /// Returns invalid configuration errors. A failed check keeps the last good listing.
    pub async fn refresh(&mut self, now: u64) -> Result<SkillsResult, SourceError> {
        self.restore().await?;
        let deadline = Instant::now() + Duration::from_secs(20);

        for source in self.config.sources.clone() {
            let Some(mut entry) = self.entries.remove(source.id()) else {
                continue;
            };
            if entry.due(now, self.config.refresh_seconds) && Instant::now() < deadline {
                let budget = deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(10));
                let Ok(checked) =
                    tokio::time::timeout(budget, self.update(&source, entry.listing.as_ref(), now))
                        .await
                else {
                    // Cut short by the shared cycle budget, not its own limit: still due next cycle.
                    if budget < Duration::from_secs(10) {
                        self.entries.insert(source.id().into(), entry);
                        continue;
                    }

                    entry.failed_at = Some(now);
                    entry.state =
                        stale_or_unavailable(entry.listing.as_ref(), &SourceError::Timeout);
                    self.entries.insert(source.id().into(), entry);
                    continue;
                };

                entry.failed_at = checked.is_err().then_some(now);
                entry.state = match (checked, &entry.listing) {
                    (Ok(listing), _) => {
                        let state = SkillSourceState::Ready {
                            revision: listing.revision.clone(),
                            checked_at: listing.checked_at,
                        };

                        entry.listing = Some(listing);
                        state
                    }
                    (Err(error), listing) => stale_or_unavailable(listing.as_ref(), &error),
                };
            }

            self.entries.insert(source.id().into(), entry);
        }

        Ok(self.catalog())
    }

    /// One cheap check; a new listing is written only when upstream changed.
    async fn update(
        &self,
        source: &Source,
        current: Option<&Listing>,
        now: u64,
    ) -> Result<Listing, SourceError> {
        let root = self.root(source);

        fs::create_dir_all(&root).await?;
        let checked = match source {
            Source::Git { .. } => {
                Repository {
                    path: &root.join("repo.git"),
                }
                .check(source, current, now)
                .await?
            }
            Source::Http { .. } => http::check(&self.client, source, current, now).await?,
        };
        let listing = match (checked, current) {
            (Check::Changed(listing), _) => listing,
            (Check::Unchanged, Some(current)) => Listing {
                checked_at: now,
                ..current.clone()
            },
            (Check::Unchanged, None) => {
                return Err(SourceError::Invalid(
                    "unchanged reply without a cached listing",
                ));
            }
        };
        let pointer = root.join(format!(
            "listing.json.{}.tmp",
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));

        fs::write(&pointer, serde_json::to_vec(&listing)?).await?;
        fs::rename(&pointer, root.join("listing.json")).await?;

        Ok(listing)
    }

    /// Skills in source order; the first source wins a name collision.
    #[must_use]
    pub fn catalog(&self) -> SkillsResult {
        let mut result = SkillsResult::default();
        let mut ids = HashSet::new();

        for source in &self.config.sources {
            let Some(entry) = self.entries.get(source.id()) else {
                continue;
            };

            result.sources.push(SkillSourceStatus {
                id: source.id().into(),
                state: entry.state.clone(),
            });

            let (visible, held) = self.visible(entry);

            for (revision, skill) in visible {
                // A signal carries at most this many skills; ranking picks which are judged.
                if result.skills.len() < chauffeur_core::signal::MAX_CATALOG_ENTRIES
                    && ids.insert(skill.id.as_str())
                {
                    result.skills.push(self.remote(source, revision, skill));
                }
            }

            result.held.extend(held);
        }

        result
    }

    /// The version of each listed skill agents may see, and those held back.
    fn visible<'a>(&self, entry: &'a Entry) -> (Vec<(&'a str, &'a ListedSkill)>, Vec<HeldSkill>) {
        let source = &entry.source;
        let Some(listing) = &entry.listing else {
            return (Vec::new(), Vec::new());
        };
        let mut visible = Vec::new();
        let mut held = Vec::new();

        if !self.reviewed {
            let all = listing
                .skills
                .iter()
                .map(|skill| (listing.revision.as_str(), skill));

            return (all.collect(), held);
        }

        for skill in &listing.skills {
            let digest = digest(source, &listing.revision, skill);
            let serving = match entry.reviews.shown(skill, &digest) {
                Shown::Current => {
                    visible.push((listing.revision.as_str(), skill));
                    continue;
                }
                Shown::Earlier(approved) if self.servable(source, approved) => {
                    visible.push((approved.revision.as_str(), &approved.skill));
                    Some(approved.revision.clone())
                }
                Shown::Earlier(_) | Shown::Withheld => None,
            };
            let verdict = entry.reviews.verdict(&skill.id, &digest);

            held.push(HeldSkill {
                id: skill.id.clone(),
                source_id: source.id().into(),
                revision: listing.revision.clone(),
                safety: verdict.map(|verdict| verdict.safety),
                reason: verdict.map_or_else(
                    || "awaiting review".into(),
                    |verdict| verdict.reason.clone(),
                ),
                concerns: verdict
                    .map(|verdict| verdict.concerns.clone())
                    .unwrap_or_default(),
                serving,
            });
        }

        (visible, held)
    }

    /// An earlier version can be served only from files of that version: Git
    /// keeps every listed revision; an HTTP catalog only serves its latest,
    /// so only a version already on disk.
    fn servable(&self, source: &Source, approved: &Approved) -> bool {
        match source {
            Source::Git { .. } => true,
            Source::Http { .. } => self
                .folder(source, &approved.revision, &approved.skill)
                .is_dir(),
        }
    }

    fn folder(&self, source: &Source, revision: &str, skill: &ListedSkill) -> PathBuf {
        self.root(source).join(revision).join(&skill.id)
    }

    fn remote(&self, source: &Source, revision: &str, skill: &ListedSkill) -> RemoteSkill {
        let base = self.folder(source, revision, skill);
        let mut content = format!(
            "Source: {} ({})\nRevision: {revision}\nBase directory: {}\n",
            source.id(),
            source.url(),
            base.display()
        );

        if !skill.files.is_empty() {
            content.push_str("Files:\n");

            for file in skill.files.iter().take(MAX_LISTED_FILES) {
                let _ = writeln!(
                    content,
                    "- {}{}",
                    base.join(&file.path).display(),
                    if file.executable { " (executable)" } else { "" }
                );
            }

            if skill.files.len() > MAX_LISTED_FILES {
                let _ = writeln!(
                    content,
                    "- and {} more in the base directory",
                    skill.files.len() - MAX_LISTED_FILES
                );
            }
        }

        let _ = write!(content, "\n{}", skill.body);

        RemoteSkill {
            id: skill.id.clone(),
            description: self
                .config
                .descriptions
                .get(&skill.id)
                .unwrap_or(&skill.description)
                .clone(),
            content,
            path: base.join("SKILL.md").to_string_lossy().into_owned(),
            source_id: source.id().into(),
            source_url: source.url().into(),
            revision: revision.into(),
        }
    }

    /// Skill versions the judges have not settled, excluding `busy` ones
    /// (by source and skill) already under review.
    #[must_use]
    pub fn due_reviews(&self, now: u64, busy: &HashSet<(String, String)>) -> Vec<ReviewJob> {
        if !self.reviewed {
            return Vec::new();
        }

        self.config
            .sources
            .iter()
            .filter_map(|source| self.entries.get(source.id()))
            .flat_map(|entry| {
                entry.listing.iter().flat_map(move |listing| {
                    listing.skills.iter().filter_map(move |skill| {
                        let digest = digest(&entry.source, &listing.revision, skill);

                        (entry.reviews.due(skill, &digest, now)
                            && !busy.contains(&(entry.source.id().to_owned(), skill.id.clone())))
                        .then(|| ReviewJob {
                            source: entry.source.id().into(),
                            skill: skill.id.clone(),
                            digest,
                            revision: listing.revision.clone(),
                        })
                    })
                })
            })
            .collect()
    }

    /// The listed skill `job` names, if upstream still lists that version.
    fn job_skill(&self, job: &ReviewJob) -> Option<(&Entry, &ListedSkill)> {
        let entry = self.entries.get(&job.source)?;
        let listing = entry.listing.as_ref()?;
        let skill = listing.skills.iter().find(|skill| skill.id == job.skill)?;

        (listing.revision == job.revision
            && digest(&entry.source, &listing.revision, skill) == job.digest)
            .then_some((entry, skill))
    }

    /// The review of `job`'s version: its files fetched as an agent would get
    /// them, then read within the review's limits.
    ///
    /// # Errors
    /// Returns a version upstream no longer lists, or a failed fetch or read.
    pub async fn review_request(&self, job: &ReviewJob) -> Result<ReviewRequest, SourceError> {
        let (entry, skill) = self.job_skill(job).ok_or(SourceError::Invalid(
            "upstream no longer lists that version",
        ))?;
        let source = &entry.source;

        self.fetch_skill(source, &job.revision, skill).await?;

        let folder = self.folder(source, &job.revision, skill);
        let instructions = instructions(read(&folder.join("SKILL.md"), MAX_FILE).await?)
            .ok_or(SourceError::Invalid("SKILL.md is empty or binary"))?;
        let mut files: Vec<(String, Option<u64>)> = Vec::with_capacity(skill.files.len());

        for file in &skill.files {
            let size = fs::metadata(folder.join(&file.path))
                .await
                .ok()
                .map(|metadata| metadata.len());

            files.push((file.path.clone(), size));
        }

        reading_order(&mut files);

        let wanted = chosen(files.iter().map(|(_, size)| *size));
        let mut read_files = Vec::with_capacity(files.len());

        for ((path, _), wanted) in files.into_iter().zip(wanted) {
            let bytes = if wanted {
                read(&folder.join(&path), MAX_FILE).await.ok()
            } else {
                None
            };

            read_files.push((path, bytes));
        }

        let approved = entry
            .reviews
            .skills
            .get(&skill.id)
            .and_then(|reviews| reviews.approved.as_ref());
        let history = approved.map_or_else(
            || "No earlier version of it was approved.".to_owned(),
            |approved| {
                format!(
                    "This is an update: the user approved its version at revision {}.",
                    approved.revision
                )
            },
        );
        let mut agent_id = format!("skill-review:{}:{}", source.id(), skill.id);

        agent_id.truncate(chauffeur_core::signal::MAX_AGENT_ID_BYTES);

        Ok(ReviewRequest {
            agent_id,
            name: skill.id.clone(),
            source: format!("{} ({}) {}", source.id(), source.url(), skill.dir),
            provenance: format!(
                "A source the user follows, at upstream revision {}. {history}",
                job.revision
            ),
            gathered: super::review::Gathered {
                instructions,
                files: shown(read_files),
            },
        })
    }

    /// Record the judges' answer for `job`. A version upstream has since
    /// replaced is ignored.
    ///
    /// # Errors
    /// Returns a failure to save the record.
    pub async fn record(
        &mut self,
        job: &ReviewJob,
        review: &Review,
        now: u64,
    ) -> Result<(), SourceError> {
        let Some((_, skill)) = self.job_skill(job) else {
            return Ok(());
        };
        let skill = skill.clone();
        let Some(entry) = self.entries.get_mut(&job.source) else {
            return Ok(());
        };

        entry.reviews.record(job, &skill, review, now);

        let (source, reviews) = (entry.source.clone(), entry.reviews.clone());

        self.save_reviews(&source, &reviews).await
    }

    /// Approve the latest listed version of `id`, whatever the judges said.
    ///
    /// # Errors
    /// Returns an unknown skill, one already approved, or a failure to save.
    pub async fn accept(&mut self, id: &str) -> Result<SkillAcceptResult, SourceError> {
        let found = self.config.sources.iter().find_map(|source| {
            let listing = self.entries.get(source.id())?.listing.as_ref()?;
            let skill = listing.skills.iter().find(|skill| skill.id == id)?;

            Some((source.clone(), listing.revision.clone(), skill.clone()))
        });
        let Some((source, revision, skill)) = found else {
            return Err(SourceError::Invalid("no remote skill has that name"));
        };
        let digest = digest(&source, &revision, &skill);
        let entry = self
            .entries
            .get_mut(source.id())
            .ok_or(SourceError::Invalid("no remote skill has that name"))?;

        if matches!(entry.reviews.shown(&skill, &digest), Shown::Current) {
            return Err(SourceError::Invalid(
                "its latest version is already approved",
            ));
        }

        entry.reviews.accept(&digest, &revision, &skill);

        let reviews = entry.reviews.clone();

        self.save_reviews(&source, &reviews).await?;

        Ok(SkillAcceptResult {
            id: id.into(),
            source_id: source.id().into(),
            revision,
        })
    }

    /// Fetch the files of each named remote skill at its listed revision,
    /// unless already on disk. Names that are not remote are ignored.
    ///
    /// # Errors
    /// Returns the first skill's failure after trying every skill.
    pub async fn materialize(&self, ids: &[String]) -> Result<(), SourceError> {
        let mut first = Ok(());

        for id in ids {
            // The version agents see, as the catalog lists it.
            let found = self.config.sources.iter().find_map(|source| {
                let entry = self.entries.get(source.id())?;

                self.visible(entry)
                    .0
                    .into_iter()
                    .find(|(_, skill)| &skill.id == id)
                    .map(|(revision, skill)| (source, revision, skill))
            });

            if let Some((source, revision, skill)) = found {
                if let Err(error) = self.fetch_skill(source, revision, skill).await {
                    first = first.and(Err(error));
                }
            }
        }

        first
    }

    async fn fetch_skill(
        &self,
        source: &Source,
        revision: &str,
        skill: &ListedSkill,
    ) -> Result<(), SourceError> {
        let folder = self.folder(source, revision, skill);

        if fs::try_exists(&folder).await? {
            return Ok(());
        }

        let revision = self.root(source).join(revision);

        if !fs::try_exists(&revision).await? {
            check_capacity(&self.root(source)).await?;
            fs::create_dir_all(&revision).await?;
        }

        let staging = revision.join(format!(
            ".staging-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));

        clean_staging(&revision).await?;
        fs::create_dir(&staging).await?;
        let fetched = self.fill(source, skill, &staging).await;
        let published = match fetched {
            Ok(()) => fs::rename(&staging, &folder)
                .await
                .map_err(SourceError::from),
            Err(error) => Err(error),
        };

        if published.is_err() {
            let _ = remove_tree(&staging).await;
        }

        published
    }

    async fn fill(
        &self,
        source: &Source,
        skill: &ListedSkill,
        staging: &Path,
    ) -> Result<(), SourceError> {
        // The registered path is this folder's SKILL.md; rebuild it from the listing.
        let main = format!(
            "---\nname: {}\ndescription: {}\n---\n\n{}\n",
            skill.id,
            serde_json::to_string(&skill.description)?,
            skill.body
        );

        write_file(staging, "SKILL.md", main.as_bytes(), false).await?;

        match source {
            Source::Git { .. } => {
                Repository {
                    path: &self.root(source).join("repo.git"),
                }
                .materialize(skill, staging)
                .await
            }
            Source::Http { .. } => http::materialize(&self.client, source, skill, staging).await,
        }
    }
}

fn stale_or_unavailable(listing: Option<&Listing>, error: &SourceError) -> SkillSourceState {
    match listing {
        Some(listing) => SkillSourceState::Stale {
            revision: listing.revision.clone(),
            checked_at: listing.checked_at,
            error: error.to_string(),
        },
        None => SkillSourceState::Unavailable {
            error: error.to_string(),
        },
    }
}

/// Write one fetched file read-only, keeping a script's executable bit.
pub(super) async fn write_file(
    root: &Path,
    path: &str,
    bytes: &[u8],
    executable: bool,
) -> Result<(), SourceError> {
    if !valid_path(path) {
        return Err(SourceError::Invalid("unsafe source file path"));
    }

    let destination = root.join(path);
    let parent = destination
        .parent()
        .ok_or(SourceError::Invalid("source file has no parent"))?;

    fs::create_dir_all(parent).await?;
    fs::write(&destination, bytes).await?;

    let mut permissions = fs::metadata(&destination).await?.permissions();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        permissions.set_mode(if executable { 0o555 } else { 0o444 });
    }

    #[cfg(not(unix))]
    {
        let _ = executable;

        permissions.set_readonly(true);
    }

    fs::set_permissions(&destination, permissions).await?;

    Ok(())
}

/// Read-only files sit in writable folders, so removal needs no permission changes.
async fn remove_tree(path: &Path) -> Result<(), SourceError> {
    match fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Staging folders left by an interrupted fetch; only this process writes the cache.
async fn clean_staging(revision: &Path) -> Result<(), SourceError> {
    let mut entries = fs::read_dir(revision).await?;

    while let Some(entry) = entries.next_entry().await? {
        if entry.file_name().to_string_lossy().starts_with(".staging-") {
            remove_tree(&entry.path()).await?;
        }
    }

    Ok(())
}

/// A Git object name, or `h` and 16 hex digits for an HTTP catalog.
fn valid_revision(revision: &str) -> bool {
    let hex = |value: &str| value.bytes().all(|b| b.is_ascii_hexdigit());

    (matches!(revision.len(), 40 | 64) && hex(revision))
        || revision
            .strip_prefix('h')
            .is_some_and(|rest| rest.len() == 16 && hex(rest))
}

async fn check_capacity(root: &Path) -> Result<(), SourceError> {
    let mut entries = fs::read_dir(root).await?;
    let mut count = 0_usize;

    while let Some(entry) = entries.next_entry().await? {
        if valid_revision(&entry.file_name().to_string_lossy()) {
            count = count.saturating_add(1);
        }

        if count >= MAX_REVISIONS {
            return Err(SourceError::Invalid(
                "skill cache holds 32 revisions of this source; remove old ones after sessions using them have closed",
            ));
        }
    }

    Ok(())
}

async fn read(path: &Path, limit: usize) -> Result<Vec<u8>, SourceError> {
    if !fs::symlink_metadata(path).await?.is_file() {
        return Err(SourceError::Invalid(
            "source cache or config must be a regular file",
        ));
    }

    let file = fs::File::open(path).await?;
    let mut bytes = Vec::new();
    let cap = u64::try_from(limit.saturating_add(1))
        .map_err(|_| SourceError::Invalid("invalid read limit"))?;

    file.take(cap).read_to_end(&mut bytes).await?;

    if bytes.len() > limit {
        return Err(SourceError::Invalid(
            "source cache or config exceeds its size limit",
        ));
    }

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(listing: bool, failed_at: Option<u64>) -> Entry {
        let source = Source::Http {
            id: "team".into(),
            url: "https://example.com/".into(),
        };

        Entry {
            listing: listing.then(|| Listing {
                source: source.clone(),
                revision: "h0000000000000000".into(),
                checked_at: 1_000,
                validator: super::super::Validator::default(),
                skills: Vec::new(),
            }),
            source,
            state: SkillSourceState::Unavailable {
                error: String::new(),
            },
            failed_at,
            reviews: Reviews::default(),
        }
    }

    #[test]
    fn a_failure_is_retried_within_fifteen_minutes_while_success_waits_the_interval() {
        let day = 86_400;

        assert!(entry(false, None).due(0, day));
        assert!(!entry(true, None).due(1_000 + day - 1, day));
        assert!(entry(true, None).due(1_000 + day, day));
        assert!(!entry(true, Some(5_000)).due(5_000 + 899, day));
        assert!(entry(true, Some(5_000)).due(5_000 + 900, day));
        assert!(entry(false, Some(5_000)).due(5_000 + 60, 60));
    }
}
