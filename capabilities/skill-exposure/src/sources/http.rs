//! OpenCode-compatible HTTP catalogs. A change check is one conditional
//! request for `index.json`; a change fetches each `SKILL.md`; a skill's
//! other files are fetched when attached. Publishers bump an entry's
//! `version` when only references change, as OpenCode's catalogs require.

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hasher};
use std::path::Path;

use reqwest::StatusCode;
use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use serde::Deserialize;

use super::{
    Check, ListedFile, ListedSkill, Listing, MAX_BYTES, MAX_FILE, MAX_FILES, Source, SourceError,
    Validator, model::valid_name, parse, store, valid_path,
};

#[derive(Deserialize)]
struct Index {
    skills: Vec<IndexSkill>,
}

#[derive(Deserialize)]
struct IndexSkill {
    name: String,
    files: Vec<String>,
}

enum Fetched {
    NotModified,
    Body(Vec<u8>, Validator),
}

pub(super) async fn check(
    client: &reqwest::Client,
    source: &Source,
    current: Option<&Listing>,
    now: u64,
) -> Result<Check, SourceError> {
    let base = base(source.url())?;
    let index_url = join(&base, "index.json")?;
    let (bytes, validator) = match get(client, index_url, current.map(|l| &l.validator)).await? {
        Fetched::NotModified => return Ok(Check::Unchanged),
        Fetched::Body(bytes, validator) => (bytes, validator),
    };
    let index: Index = serde_json::from_slice(&bytes)?;

    if index.skills.len() > MAX_FILES {
        return Err(SourceError::Invalid("too many catalog entries"));
    }

    let mut revision = DefaultHasher::new();
    let mut names = HashSet::new();
    let mut skills = Vec::with_capacity(index.skills.len());

    revision.write(&bytes);

    for entry in index.skills {
        let main = format!("{}.md", entry.name);
        let main = ["SKILL.md", main.as_str()]
            .into_iter()
            .find(|name| entry.files.iter().any(|file| file == name))
            .map(str::to_owned);

        let (Some(main), true, true) = (
            main,
            valid_name(&entry.name) && names.insert(entry.name.clone()),
            entry.files.len() <= MAX_FILES && entry.files.iter().all(|file| valid_path(file)),
        ) else {
            return Err(SourceError::Invalid("invalid skill catalog entry"));
        };

        let Fetched::Body(text, _) = get(
            client,
            join(&base, &format!("{}/{main}", entry.name))?,
            None,
        )
        .await?
        else {
            return Err(SourceError::Invalid("unexpected not-modified reply"));
        };
        let parsed = parse::skill(&text)?;

        revision.write(&text);
        skills.push(ListedSkill {
            id: parsed.id,
            description: parsed.description,
            body: parsed.body,
            dir: entry.name,
            files: entry
                .files
                .into_iter()
                .filter(|file| file != &main)
                .map(|path| ListedFile {
                    path,
                    executable: false,
                    object: None,
                })
                .collect(),
        });
    }

    Ok(Check::Changed(Listing {
        source: source.clone(),
        revision: format!("h{:016x}", revision.finish()),
        checked_at: now,
        validator,
        skills,
    }))
}

/// Write one skill's files into `staging`.
pub(super) async fn materialize(
    client: &reqwest::Client,
    source: &Source,
    skill: &ListedSkill,
    staging: &Path,
) -> Result<(), SourceError> {
    let base = base(source.url())?;
    let mut total = 0_usize;

    for file in &skill.files {
        let Fetched::Body(bytes, _) = get(
            client,
            join(&base, &format!("{}/{}", skill.dir, file.path))?,
            None,
        )
        .await?
        else {
            return Err(SourceError::Invalid("unexpected not-modified reply"));
        };

        total = total.saturating_add(bytes.len());

        if total > MAX_BYTES {
            return Err(SourceError::Invalid("skill files exceed 16 MiB"));
        }

        store::write_file(staging, &file.path, &bytes, false).await?;
    }

    Ok(())
}

fn base(url: &str) -> Result<reqwest::Url, SourceError> {
    reqwest::Url::parse(&format!("{}/", url.trim_end_matches('/')))
        .map_err(|_| SourceError::Invalid("invalid catalog URL"))
}

/// A catalog file URL, which must stay under the configured base.
fn join(base: &reqwest::Url, path: &str) -> Result<reqwest::Url, SourceError> {
    let url = base
        .join(path)
        .map_err(|_| SourceError::Invalid("invalid catalog file URL"))?;

    if url.origin() != base.origin() || !url.path().starts_with(base.path()) {
        return Err(SourceError::Invalid("catalog file escaped its source"));
    }

    Ok(url)
}

async fn get(
    client: &reqwest::Client,
    url: reqwest::Url,
    validator: Option<&Validator>,
) -> Result<Fetched, SourceError> {
    let mut request = client.get(url);

    if let Some(validator) = validator {
        if let Some(etag) = &validator.etag {
            request = request.header(IF_NONE_MATCH, etag);
        }

        if let Some(modified) = &validator.last_modified {
            request = request.header(IF_MODIFIED_SINCE, modified);
        }
    }

    let mut response = request.send().await?;

    if response.status() == StatusCode::NOT_MODIFIED && validator.is_some() {
        return Ok(Fetched::NotModified);
    }

    if !response.status().is_success() {
        return Err(SourceError::Invalid(
            "skill source returned a non-success HTTP status",
        ));
    }

    let header = |name| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.len() <= 256)
            .map(str::to_owned)
    };
    let validator = Validator {
        etag: header(ETAG),
        last_modified: header(LAST_MODIFIED),
    };
    let mut bytes = Vec::new();

    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > MAX_FILE {
            return Err(SourceError::Invalid("source file exceeds 1 MiB"));
        }

        bytes.extend_from_slice(&chunk);
    }

    Ok(Fetched::Body(bytes, validator))
}
