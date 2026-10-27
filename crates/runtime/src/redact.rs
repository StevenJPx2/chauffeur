//! Redacts evidence before any of it reaches a model. Known credential
//! formats and private-key blocks become `[REDACTED]`. Other
//! credential-like strings become a numbered description of their shape,
//! never their value. Paths, links and bare hashes stay readable.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use serde::Deserialize;

/// What a known secret is replaced with.
pub const REDACTED: &str = "[REDACTED]";

const MAX_PREFIXES: usize = 256;
/// Credential-like: at least this long with upper and lower case and digits…
const MIN_MIXED_LEN: usize = 24;
/// …or at least this long in hex.
const MIN_HEX_LEN: usize = 32;
/// How much of a word's start can be a readable prefix such as `sk_live_`.
const MAX_PREFIX_LEN: usize = 10;
/// Authorization schemes whose value follows a space, not `=` or `:`.
const SCHEMES: [&str; 2] = ["bearer", "basic"];
const PEM_BEGIN: &str = "-----BEGIN ";
const PEM_KEY_TAIL: &str = "PRIVATE KEY-----";
const PEM_END: &str = "-----END ";

/// A known credential format: a fixed prefix and a minimum body.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prefix {
    /// The prefix, such as `ghp_`.
    pub prefix: String,
    /// The fewest token characters after it.
    pub min_body: usize,
}

/// A redaction file: prefixes that add to the earlier files', or replace
/// them with `replace`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedactionConfig {
    /// Whether these prefixes replace the earlier files' instead of adding to them.
    #[serde(default)]
    pub replace: bool,
    /// The known credential formats.
    #[serde(default)]
    pub prefixes: Vec<Prefix>,
}

/// Why redaction could not be set up. Evidence is never sent without it.
#[derive(Debug, thiserror::Error)]
pub enum RedactError {
    /// A file could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// A file is not valid redaction configuration.
    #[error("{path} is not valid: {source}")]
    Parse {
        /// The file.
        path: PathBuf,
        /// Why.
        source: serde_json::Error,
    },
    /// Too many prefixes, or an empty one.
    #[error("redaction allows at most {MAX_PREFIXES} prefixes, none empty")]
    Prefixes,
}

/// What a credential-like word looks like, without its value.
struct Shape {
    prefix: String,
    class: &'static str,
    length: usize,
}

/// Numbers each described value within one navigation, so the same value
/// gets the same label in every block.
#[derive(Debug, Default)]
pub struct Masking {
    numbers: BTreeMap<String, usize>,
}

impl Masking {
    fn number(&mut self, value: &str) -> usize {
        let next = self.numbers.len().saturating_add(1);
        *self.numbers.entry(value.to_owned()).or_insert(next)
    }
}

/// Redacts text with the configured prefixes and the built-in shape rules.
#[derive(Clone, Debug)]
pub struct Redactor {
    prefixes: Vec<Prefix>,
}

impl Redactor {
    /// A redactor from `configs` in order, shipped first.
    ///
    /// # Errors
    ///
    /// [`RedactError::Prefixes`] with more than 256 prefixes or an empty one.
    pub fn new(configs: impl IntoIterator<Item = RedactionConfig>) -> Result<Self, RedactError> {
        let mut prefixes = Vec::new();
        for config in configs {
            if config.replace {
                prefixes.clear();
            }
            prefixes.extend(config.prefixes);
        }
        if prefixes.len() > MAX_PREFIXES || prefixes.iter().any(|prefix| prefix.prefix.is_empty()) {
            return Err(RedactError::Prefixes);
        }
        // Longer prefixes first, so `sk-ant-` wins over `sk-`.
        prefixes.sort_by_key(|prefix| std::cmp::Reverse(prefix.prefix.len()));
        Ok(Self { prefixes })
    }

    /// A redactor from the files at `paths`, shipped first.
    ///
    /// # Errors
    ///
    /// A file cannot be read or parsed, or its prefixes are refused.
    pub fn load(paths: &[PathBuf]) -> Result<Self, RedactError> {
        let configs = paths
            .iter()
            .map(|path| {
                let text = std::fs::read_to_string(path).map_err(|source| RedactError::Read {
                    path: path.clone(),
                    source,
                })?;
                serde_json::from_str(&text).map_err(|source| RedactError::Parse {
                    path: path.clone(),
                    source,
                })
            })
            .collect::<Result<Vec<RedactionConfig>, _>>()?;
        Self::new(configs)
    }

    /// `text` as it may be sent.
    #[must_use]
    pub fn redact(&self, text: &str, masking: &mut Masking) -> String {
        walk(text, |word, before| {
            if path_like(word, before) {
                return word
                    .split('/')
                    .map(|segment| self.replace_word(segment, before, masking))
                    .collect::<Vec<_>>()
                    .join("/");
            }
            self.replace_word(word, before, masking)
        })
    }

    /// A word as sent: redacted, described, or unchanged.
    fn replace_word(&self, word: &str, before: &str, masking: &mut Masking) -> String {
        if self.known(word) {
            return REDACTED.to_owned();
        }
        let Some(shape) = shape(word) else {
            return word.to_owned();
        };
        let key = key_before(before);
        // A bare hex string is a commit or content hash; after a key it may be a credential.
        if shape.class == "hex" && key.is_empty() {
            return word.to_owned();
        }
        describe(masking.number(word), &shape, key)
    }

    fn known(&self, word: &str) -> bool {
        self.prefixes.iter().any(|prefix| {
            word.strip_prefix(prefix.prefix.as_str())
                .is_some_and(|body| {
                    body.len() >= prefix.min_body && body.chars().all(is_token_char)
                })
        })
    }
}

/// `[string 2: 40 characters of hex, starts "sk_", after "api_key="]`
fn describe(number: usize, shape: &Shape, key: &str) -> String {
    let mut label = format!(
        "[string {number}: {} characters of {}",
        shape.length, shape.class
    );
    if !shape.prefix.is_empty() {
        let _ = write!(label, ", starts \"{}\"", shape.prefix);
    }
    if !key.is_empty() {
        let _ = write!(label, ", after \"{key}\"");
    }
    label.push(']');
    label
}

/// The shape of a credential-like word, or none.
fn shape(word: &str) -> Option<Shape> {
    let length = word.chars().count();
    let hex = word.chars().all(|character| character.is_ascii_hexdigit());
    let mixed = word.chars().any(|character| character.is_ascii_uppercase())
        && word.chars().any(|character| character.is_ascii_lowercase())
        && word.chars().any(|character| character.is_ascii_digit());
    if !((hex && length >= MIN_HEX_LEN) || (mixed && length >= MIN_MIXED_LEN)) {
        return None;
    }
    let class = if hex {
        "hex"
    } else if word.contains(['+', '/']) {
        "base64"
    } else {
        "mixed letters and digits"
    };
    let lead = word.get(..MAX_PREFIX_LEN.min(word.len())).unwrap_or("");
    let prefix = lead
        .rfind(['_', '-'])
        .map_or("", |end| lead.get(..=end).unwrap_or(""));
    Some(Shape {
        prefix: prefix.to_owned(),
        class,
        length,
    })
}

/// Whether `word` is a path whose segments are judged one by one: an
/// absolute or home path, a URL's path, or a relative path such as
/// `acme/web-app#42` (base64 has `/` but never `-` or `_`, and base64url has
/// those but never `/`). A value after `key=` or `key:` stays whole, as
/// base64 with `/` would.
fn path_like(word: &str, before: &str) -> bool {
    let in_url = before
        .rsplit(char::is_whitespace)
        .next()
        .is_some_and(|token| token.contains("://"));
    let absolute = word.starts_with('/') || before.ends_with('~');
    let not_base64 = word.contains(['-', '_']) && !word.contains('+');
    word.contains('/') && key_before(before).is_empty() && (in_url || absolute || not_base64)
}

/// The key a value follows, such as `api_key=` in `api_key=abc…`, or an
/// authorization scheme such as `Bearer` in `Bearer abc…`.
fn key_before(before: &str) -> &str {
    let trimmed = before.trim_end_matches(['"', '\'', ' ']);
    let scheme = SCHEMES.iter().find_map(|scheme| {
        let tail = trimmed.get(trimmed.len().checked_sub(scheme.len())?..)?;
        tail.eq_ignore_ascii_case(scheme).then_some(tail)
    });
    if let Some(scheme) = scheme {
        return scheme;
    }
    if !trimmed.ends_with(['=', ':']) {
        return "";
    }
    let start = trimmed
        .char_indices()
        .rev()
        .skip(1)
        .find(|(_, character)| {
            !(character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.'))
        })
        .map_or(0, |(index, character)| {
            index.saturating_add(character.len_utf8())
        });
    trimmed.get(start..).unwrap_or("")
}

/// Rebuilds `text` with private-key blocks redacted and each word, at a word
/// boundary, replaced by `word(word, text_so_far)`.
fn walk(text: &str, mut word: impl FnMut(&str, &str) -> String) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(next) = rest.chars().next() {
        if let Some(skip) = private_key_len(rest) {
            output.push_str(REDACTED);
            rest = rest.get(skip..).unwrap_or("");
            continue;
        }
        let at_boundary = output
            .chars()
            .next_back()
            .is_none_or(|last| !is_word_char(last));
        let length = rest
            .find(|character: char| !is_word_char(character))
            .unwrap_or(rest.len());
        if at_boundary && length > 0 {
            let replaced = word(rest.get(..length).unwrap_or(""), &output);
            output.push_str(&replaced);
            rest = rest.get(length..).unwrap_or("");
            continue;
        }
        output.push(next);
        rest = rest.get(next.len_utf8()..).unwrap_or("");
    }
    output
}

/// The byte length of a PEM private-key block starting `text`, through its
/// END line, or to the end of `text` when it never ends.
fn private_key_len(text: &str) -> Option<usize> {
    let header = text.strip_prefix(PEM_BEGIN)?;
    let header_end = header.find('\n').unwrap_or(header.len());
    if !header.get(..header_end)?.trim_end().ends_with(PEM_KEY_TAIL) {
        return None;
    }
    let Some(end) = text.find(PEM_END) else {
        return Some(text.len());
    };
    let after_end = text.get(end..)?;
    Some(end.saturating_add(after_end.find('\n').unwrap_or(after_end.len())))
}

/// Characters of a token body: letters, digits, `_` and `-`.
const fn is_token_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_' || character == '-'
}

/// Characters of a candidate word, which also covers base64. `=` is left
/// out, so `key=value` stays two words.
const fn is_word_char(character: char) -> bool {
    is_token_char(character) || matches!(character, '+' | '/')
}
