use serde::Deserialize;

use super::{SourceError, model::valid_name};

#[derive(Deserialize)]
struct Frontmatter {
    name: String,
    description: String,
}

/// A `SKILL.md`'s name, description, and body.
pub(super) struct Parsed {
    pub id: String,
    pub description: String,
    pub body: String,
}

/// Parse only metadata, not arbitrary objects or any filesystem include directives.
pub(super) fn skill(bytes: &[u8]) -> Result<Parsed, SourceError> {
    if bytes.len() > 65_536 {
        return Err(SourceError::Invalid("SKILL.md exceeds 64 KiB"));
    }

    let text =
        std::str::from_utf8(bytes).map_err(|_| SourceError::Invalid("skill is not UTF-8"))?;
    let normalized = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let rest = normalized
        .strip_prefix("---\n")
        .ok_or(SourceError::Invalid("skill needs YAML frontmatter"))?;
    let (header, body) = rest.split_once("\n---").ok_or(SourceError::Invalid(
        "skill needs closing frontmatter delimiter",
    ))?;

    if header.len() > 16_384 || !(body.is_empty() || body.starts_with('\n')) {
        return Err(SourceError::Invalid(
            "invalid or oversized skill frontmatter",
        ));
    }

    let metadata: Frontmatter = yaml_serde::from_str(header)?;

    if !valid_name(&metadata.name)
        || metadata.description.trim().is_empty()
        || metadata.description.len() > 4096
    {
        return Err(SourceError::Invalid(
            "skill needs a valid name and a nonempty description of at most 4096 bytes",
        ));
    }

    Ok(Parsed {
        id: metadata.name,
        description: metadata.description,
        body: body.trim().to_owned(),
    })
}
