//! The Skilld registry (<https://skilld.dev>) as a read-only discovery index.
//! Nothing it returns runs or becomes available until approval writes a
//! source; Chauffeur then fetches the skill from GitHub like any other.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use reqwest::Url;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{SourceError, valid_path};

const DEFAULT_ORIGIN: &str = "https://skilld.dev";
const MAX_RESPONSE: usize = 1024 * 1024;
/// Skilld rejects longer queries.
const MAX_QUERY_CHARS: usize = 200;
pub const MAX_SEARCH: u8 = 50;
/// Skills one repository approval reads; each costs one detail request.
pub(super) const MAX_REPOSITORY_SKILLS: usize = 64;
/// Remote text is shown to the user, never followed; keep it short.
const MAX_TEXT_CHARS: usize = 500;

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("Skilld request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Skilld answered {status} {code}: {detail}")]
    Api {
        status: u16,
        code: String,
        detail: String,
    },
    #[error("unexpected Skilld answer: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("skill sources: {0}")]
    Source(#[from] SourceError),
    #[error("skill sources: {0}")]
    Io(#[from] std::io::Error),
}

fn invalid(message: impl Into<String>) -> DiscoveryError {
    DiscoveryError::Invalid(message.into())
}

/// `OWNER/REPOSITORY/SKILL` names one skill; `OWNER/REPOSITORY`, every
/// skill Skilld lists for that repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selector {
    pub owner: String,
    pub repository: String,
    pub name: Option<String>,
}

impl FromStr for Selector {
    type Err = DiscoveryError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = text.split('/').collect();
        let (owner, repository, name) = match parts.as_slice() {
            [owner, repository] => (*owner, *repository, None),
            [owner, repository, name] => (*owner, *repository, Some(*name)),
            _ => {
                return Err(invalid(format!(
                    "{text}: expected OWNER/REPOSITORY[/SKILL]"
                )));
            }
        };

        if !valid_owner(owner) || !valid_segment(repository) || !name.is_none_or(valid_segment) {
            return Err(invalid(format!(
                "{text}: not a GitHub OWNER/REPOSITORY[/SKILL]"
            )));
        }

        Ok(Self {
            owner: owner.into(),
            repository: repository.into(),
            name: name.map(Into::into),
        })
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.repository)?;

        if let Some(name) = &self.name {
            write!(f, "/{name}")?;
        }

        Ok(())
    }
}

fn valid_owner(owner: &str) -> bool {
    (1..=39).contains(&owner.len())
        && owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !owner.starts_with('-')
        && !owner.ends_with('-')
}

fn valid_segment(segment: &str) -> bool {
    (1..=100).contains(&segment.len())
        && !matches!(segment, "." | "..")
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

/// One search result: enough to choose, not enough to approve.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Found {
    pub selector: String,
    pub name: String,
    pub description: Option<String>,
    pub stars: u64,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Approval must name it with `--allow`.
    Ask,
    Show,
}

/// Something Skilld's fixed patterns found the skill's files asking an agent
/// to do. An empty list proves nothing.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Behavior {
    pub id: String,
    pub label: String,
    pub tier: Tier,
    pub total: u64,
    /// `path:line`, at most five.
    pub locations: Vec<String>,
}

/// A skill whose upstream identity is complete enough to become a source.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Candidate {
    pub selector: String,
    pub name: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub license: Option<String>,
    /// The Git URL a source follows.
    pub url: String,
    /// The skill's folder in the repository; `.` for its root.
    pub dir: String,
    /// The commit Skilld read; approval follows the branch, not this commit.
    pub commit: Option<String>,
    pub pushed_at: Option<String>,
    pub tags: Vec<String>,
    pub page: String,
    pub behaviors: Vec<Behavior>,
}

#[derive(Deserialize)]
struct SearchAnswer {
    items: Vec<SearchItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchItem {
    name: String,
    description: Option<String>,
    source: SearchSource,
    #[serde(default)]
    stargazer_count: u64,
}

#[derive(Deserialize)]
struct SearchSource {
    owner: String,
    repository: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillAnswer {
    owner: String,
    repository: String,
    name: String,
    description: Option<String>,
    author_name: Option<String>,
    license: Option<String>,
    repository_url: String,
    skill_path: Option<String>,
    source_commit: Option<String>,
    #[serde(default)]
    source_gone: bool,
    pushed_at: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    page_url: String,
    #[serde(default)]
    behaviors: Vec<BehaviorAnswer>,
}

#[derive(Deserialize)]
struct BehaviorAnswer {
    id: String,
    #[serde(default)]
    label: String,
    tier: String,
    #[serde(default)]
    total: u64,
    #[serde(default)]
    locations: Vec<LocationAnswer>,
}

#[derive(Deserialize)]
struct LocationAnswer {
    path: String,
    line: Option<u64>,
}

#[derive(Deserialize)]
struct RepositoryAnswer {
    #[serde(default)]
    skills: Vec<RepositorySkill>,
}

#[derive(Deserialize)]
struct RepositorySkill {
    name: String,
}

#[derive(Deserialize)]
struct Problem {
    code: Option<String>,
    detail: Option<String>,
    title: Option<String>,
}

/// Remote text, safe for a terminal: no control characters, bounded length.
fn clean(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control() && !matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}'))
        .collect();
    let words = flat.split_whitespace().collect::<Vec<_>>().join(" ");

    match words.char_indices().nth(MAX_TEXT_CHARS) {
        Some((end, _)) => format!("{}…", &words[..end]),
        None => words,
    }
}

fn clean_option(text: Option<&str>) -> Option<String> {
    text.map(clean).filter(|text| !text.is_empty())
}

impl Candidate {
    fn from_answer(answer: SkillAnswer) -> Result<Self, DiscoveryError> {
        let selector = Selector {
            owner: answer.owner,
            repository: answer.repository,
            name: Some(answer.name.clone()),
        };
        let shown = clean(&selector.to_string());

        if !valid_owner(&selector.owner)
            || !valid_segment(&selector.repository)
            || !valid_segment(&answer.name)
        {
            return Err(invalid(format!("{shown}: Skilld named an invalid skill")));
        }

        if answer.source_gone {
            return Err(invalid(format!("{selector}: its upstream source is gone")));
        }

        let url = github_url(&answer.repository_url).ok_or_else(|| {
            invalid(format!(
                "{selector}: Skilld gives no public GitHub repository for it"
            ))
        })?;
        let dir = answer
            .skill_path
            .as_deref()
            .and_then(skill_dir)
            .ok_or_else(|| {
                invalid(format!(
                    "{selector}: Skilld gives no usable SKILL.md path for it"
                ))
            })?;

        if let Some(commit) = &answer.source_commit
            && !(matches!(commit.len(), 40 | 64) && commit.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(invalid(format!(
                "{selector}: Skilld gives a malformed commit"
            )));
        }

        let behaviors = answer
            .behaviors
            .into_iter()
            .map(|behavior| Behavior {
                id: clean(&behavior.id),
                label: clean(&behavior.label),
                // Unknown tiers need approval too.
                tier: if behavior.tier == "show" {
                    Tier::Show
                } else {
                    Tier::Ask
                },
                total: behavior.total,
                locations: behavior
                    .locations
                    .iter()
                    .take(5)
                    .map(|location| match location.line {
                        Some(line) => format!("{}:{line}", clean(&location.path)),
                        None => clean(&location.path),
                    })
                    .collect(),
            })
            .collect();

        Ok(Self {
            selector: selector.to_string(),
            name: answer.name,
            description: clean_option(answer.description.as_deref()),
            author: clean_option(answer.author_name.as_deref()),
            license: clean_option(answer.license.as_deref()),
            url,
            dir,
            commit: answer
                .source_commit
                .map(|commit| commit.to_ascii_lowercase()),
            pushed_at: clean_option(answer.pushed_at.as_deref()),
            tags: answer.tags.iter().take(16).map(|tag| clean(tag)).collect(),
            page: clean(&answer.page_url),
            behaviors,
        })
    }
}

/// `https://github.com/OWNER/REPOSITORY` as a clone URL; nothing else.
fn github_url(text: &str) -> Option<String> {
    let url = Url::parse(text).ok()?;

    if url.scheme() != "https"
        || !url.host_str()?.eq_ignore_ascii_case("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }

    let segments: Vec<&str> = url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect();
    let [owner, repository] = segments.as_slice() else {
        return None;
    };
    let repository = repository.strip_suffix(".git").unwrap_or(repository);

    (valid_owner(owner) && valid_segment(repository))
        .then(|| format!("https://github.com/{owner}/{repository}.git"))
}

fn skill_dir(path: &str) -> Option<String> {
    if path == "SKILL.md" {
        return Some(".".into());
    }

    path.strip_suffix("/SKILL.md")
        .filter(|dir| valid_path(dir))
        .map(Into::into)
}

pub struct SkilldClient {
    origin: Url,
    http: reqwest::Client,
}

impl SkilldClient {
    /// The origin in `SKILLD_API_URL`, as for the Skilld CLI, or skilld.dev.
    ///
    /// # Errors
    /// Returns an invalid origin or HTTP client construction error.
    pub fn from_env() -> Result<Self, DiscoveryError> {
        Self::new(&std::env::var("SKILLD_API_URL").unwrap_or_else(|_| DEFAULT_ORIGIN.into()))
    }

    /// # Errors
    /// Returns an error unless `origin` is HTTPS, or HTTP on loopback.
    pub fn new(origin: &str) -> Result<Self, DiscoveryError> {
        let origin = Url::parse(origin).map_err(|_| invalid("invalid Skilld origin"))?;
        let loopback = matches!(origin.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));

        if !(origin.scheme() == "https" || (loopback && origin.scheme() == "http"))
            || origin.path() != "/"
            || origin.query().is_some()
            || !origin.username().is_empty()
            || origin.password().is_some()
        {
            return Err(invalid(
                "the Skilld origin must be an HTTPS origin, or HTTP on loopback",
            ));
        }

        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("chauffeur/", env!("CARGO_PKG_VERSION")))
            .build()?;

        Ok(Self { origin, http })
    }

    fn endpoint(&self, segments: &[&str]) -> Url {
        let mut url = self.origin.clone();

        if let Ok(mut path) = url.path_segments_mut() {
            path.clear().extend(["api", "v1"]).extend(segments);
        }

        url
    }

    async fn get(&self, url: Url) -> Result<Vec<u8>, DiscoveryError> {
        let mut response = self
            .http
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await?;
        let status = response.status();
        let mut bytes = Vec::new();

        while let Some(chunk) = response.chunk().await? {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE {
                return Err(invalid("Skilld answer exceeds 1 MiB"));
            }

            bytes.extend_from_slice(&chunk);
        }

        if status.is_success() {
            return Ok(bytes);
        }

        let problem = serde_json::from_slice::<Problem>(&bytes).ok();

        Err(DiscoveryError::Api {
            status: status.as_u16(),
            code: problem
                .as_ref()
                .and_then(|problem| problem.code.as_deref())
                .map_or_else(|| "HTTP_ERROR".into(), clean),
            detail: problem
                .as_ref()
                .and_then(|problem| problem.detail.as_deref().or(problem.title.as_deref()))
                .map_or_else(
                    || status.canonical_reason().unwrap_or("error").into(),
                    clean,
                ),
        })
    }

    /// Search by the user's own words; nothing else about the project is sent.
    ///
    /// # Errors
    /// Returns an empty or overlong query, or a failed request.
    pub async fn search(&self, query: &str, limit: u8) -> Result<Vec<Found>, DiscoveryError> {
        let query = query.split_whitespace().collect::<Vec<_>>().join(" ");

        if query.is_empty() || query.chars().count() > MAX_QUERY_CHARS {
            return Err(invalid(format!(
                "a search needs 1 to {MAX_QUERY_CHARS} characters"
            )));
        }

        let mut url = self.endpoint(&["skills"]);

        url.query_pairs_mut()
            .append_pair("q", &query)
            .append_pair("limit", &limit.clamp(1, MAX_SEARCH).to_string());

        let answer: SearchAnswer = serde_json::from_slice(&self.get(url).await?)?;

        Ok(answer
            .items
            .into_iter()
            .filter(|item| {
                valid_owner(&item.source.owner)
                    && valid_segment(&item.source.repository)
                    && valid_segment(&item.name)
            })
            .map(|item| Found {
                selector: format!(
                    "{}/{}/{}",
                    item.source.owner, item.source.repository, item.name
                ),
                name: item.name,
                description: clean_option(item.description.as_deref()),
                stars: item.stargazer_count,
            })
            .collect())
    }

    /// One skill's provenance, validated.
    ///
    /// # Errors
    /// Returns a failed request, or provenance too incomplete to follow.
    pub async fn skill(
        &self,
        owner: &str,
        repository: &str,
        name: &str,
    ) -> Result<Candidate, DiscoveryError> {
        let url = self.endpoint(&["skills", owner, repository, name]);

        Candidate::from_answer(serde_json::from_slice(&self.get(url).await?)?)
    }

    /// Every skill the selector names, each with its own provenance.
    ///
    /// # Errors
    /// Returns the first failed request or invalid skill; approval is all or nothing.
    pub async fn candidates(&self, selector: &Selector) -> Result<Vec<Candidate>, DiscoveryError> {
        if let Some(name) = &selector.name {
            return Ok(vec![
                self.skill(&selector.owner, &selector.repository, name)
                    .await?,
            ]);
        }

        let url = self.endpoint(&["repositories", &selector.owner, &selector.repository]);
        let answer: RepositoryAnswer = serde_json::from_slice(&self.get(url).await?)?;

        if answer.skills.is_empty() {
            return Err(invalid(format!("Skilld lists no skills in {selector}")));
        }

        if answer.skills.len() > MAX_REPOSITORY_SKILLS {
            return Err(invalid(format!(
                "{selector} lists {} skills; approve at most {MAX_REPOSITORY_SKILLS} at once, or name one",
                answer.skills.len()
            )));
        }

        let mut candidates = Vec::with_capacity(answer.skills.len());

        for skill in answer.skills {
            if !valid_segment(&skill.name) {
                return Err(invalid(format!(
                    "Skilld lists an invalid skill name in {selector}"
                )));
            }

            candidates.push(
                self.skill(&selector.owner, &selector.repository, &skill.name)
                    .await?,
            );
        }

        Ok(candidates)
    }
}
