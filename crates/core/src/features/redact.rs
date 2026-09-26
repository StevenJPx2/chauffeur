//! Secret redaction applied before any text leaves the machine.
//!
//! Known formats (listed prefixes, private-key blocks, learned secret shapes)
//! become [`REDACTED`]. Other credential-like strings are replaced with a
//! numbered description of their shape, never their value, so System One can
//! judge whether that shape is a secret; its answers are learned.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

pub const REDACTED: &str = "[REDACTED]";

/// The shipped defaults (`skills/safety/redaction.json`), compiled in.
const DEFAULTS: &str = include_str!("../../../../skills/safety/redaction.json");
const MAX_PREFIXES: usize = 256;
const MAX_LEARNED: usize = 1_024;
/// Credential-like: at least this long with upper, lower, and digits…
const MIN_MIXED_LEN: usize = 24;
/// …or at least this long in hex.
const MIN_HEX_LEN: usize = 32;
const MAX_PREFIX_LEN: usize = 10;
const HEX: &str = "hex";
/// Authorization schemes whose value follows a space, not `=` or `:`.
const SCHEMES: [&str; 2] = ["bearer", "basic"];

/// A known credential format: a fixed prefix and a minimum body.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Prefix {
    pub prefix: String,
    pub min_body: usize,
}

/// Strict JSON: `{"replace": false, "prefixes": [{"prefix": "sk_live_", "min_body": 20}]}`.
/// Your file adds to the defaults, or with `replace` stands in for them.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RedactionConfig {
    #[serde(default)]
    pub replace: bool,
    #[serde(default)]
    pub prefixes: Vec<Prefix>,
}

/// What a credential-like string looks like, without its value.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, Hash)]
pub struct Shape {
    /// A readable lead such as `sk_live_`, or empty.
    pub prefix: String,
    pub class: String,
    pub length: usize,
    /// The assignment key immediately preceding the string, if any. Safe
    /// judgments never generalize across unrelated keys.
    #[serde(default)]
    pub context: String,
}

/// Shapes System One judged: secrets stay redacted, safe ones pass through.
#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct LearnedShapes {
    #[serde(default)]
    pub secret: Vec<Shape>,
    #[serde(default)]
    pub safe: Vec<Shape>,
}

/// The unknown shapes found while redacting one signal's text, numbered so
/// the same value gets the same label everywhere.
#[derive(Debug, Default)]
pub struct Masking {
    values: HashMap<String, usize>,
    pub shapes: Vec<(Shape, String)>,
}

pub struct Redactor {
    prefixes: Vec<Prefix>,
    secret: HashSet<Shape>,
    safe: HashSet<Shape>,
}

impl Redactor {
    /// The shipped defaults, plus or replaced by `config`, plus `learned`.
    pub fn new(config: RedactionConfig, learned: LearnedShapes) -> Result<Self, String> {
        let defaults: RedactionConfig =
            serde_json::from_str(DEFAULTS).map_err(|error| error.to_string())?;
        let mut prefixes = if config.replace {
            Vec::new()
        } else {
            defaults.prefixes
        };

        prefixes.extend(config.prefixes);

        if prefixes.len() > MAX_PREFIXES || prefixes.iter().any(|prefix| prefix.prefix.is_empty()) {
            return Err(format!(
                "at most {MAX_PREFIXES} non-empty redaction prefixes"
            ));
        }

        // Longer prefixes first, so `sk-ant-` wins over `sk-`.
        prefixes.sort_by_key(|prefix| std::cmp::Reverse(prefix.prefix.len()));

        Ok(Self {
            prefixes,
            secret: learned.secret.into_iter().take(MAX_LEARNED).collect(),
            safe: learned.safe.into_iter().take(MAX_LEARNED).collect(),
        })
    }

    #[must_use]
    pub fn learned(&self) -> LearnedShapes {
        LearnedShapes {
            secret: self.secret.iter().cloned().collect(),
            safe: self.safe.iter().cloned().collect(),
        }
    }

    /// Record a judgment; `true` when the shape was new.
    pub fn learn(&mut self, shape: Shape, secret: bool) -> bool {
        if !secret && shape.context.is_empty() {
            return false;
        }
        let shape = if secret {
            Shape {
                context: String::new(),
                ..shape
            }
        } else {
            shape
        };
        let (into, other) = if secret {
            (&mut self.secret, &mut self.safe)
        } else {
            (&mut self.safe, &mut self.secret)
        };

        if into.len() >= MAX_LEARNED || into.contains(&shape) {
            return false;
        }

        other.remove(&shape);
        into.insert(shape)
    }

    /// Redact `text`, collecting unknown credential-like shapes in `masking`.
    pub fn redact(&self, text: &str, masking: &mut Masking) -> String {
        walk(text, |word, before| {
            if url_path(word, before) {
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
            return REDACTED.into();
        }

        let Some(mut shape) = shape(word) else {
            return word.into();
        };

        if self.secret.contains(&shape) {
            return REDACTED.into();
        }

        // A bare hex string is a commit or content hash; after a key it may
        // be a credential, and is judged.
        if shape.class == HEX && key_before(before).is_empty() {
            return word.into();
        }

        shape.context = key_before(before).chars().take(32).collect();
        if self.safe.contains(&shape) {
            return word.into();
        }

        let number = masking.number(word, &shape, key_before(before));

        describe(number, &shape, key_before(before))
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

impl Masking {
    fn number(&mut self, value: &str, shape: &Shape, key: &str) -> usize {
        if let Some(number) = self.values.get(value) {
            return *number;
        }

        self.shapes.push((shape.clone(), key.to_string()));
        self.values.insert(value.to_string(), self.shapes.len());
        self.shapes.len()
    }
}

/// `[string 2: 40 characters of hex, starts "sk_", after "api_key="]`
#[must_use]
pub fn describe(number: usize, shape: &Shape, key: &str) -> String {
    let mut label = format!(
        "[string {number}: {} characters of {}",
        shape.length, shape.class
    );

    if !shape.prefix.is_empty() {
        label.push_str(&format!(", starts \"{}\"", shape.prefix));
    }
    if !key.is_empty() {
        label.push_str(&format!(", after \"{key}\""));
    }

    label.push(']');
    label
}

/// The shape of a credential-like word, or `None`.
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
        HEX
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
        prefix: prefix.into(),
        class: class.into(),
        length,
        context: String::new(),
    })
}

/// Whether `word` is a path: an absolute or home path such as
/// `/Users/me/hpdp-overlay/ADEPT-45130`, or a URL's path such as
/// `com/archives/C08/p1790…` after `https://acme.slack.`. Its segments are
/// judged one by one, so the path survives while a token in it is still
/// caught. A value after `key=` or `key:` stays whole, as base64 with `/`
/// would.
fn url_path(word: &str, before: &str) -> bool {
    let in_url = before
        .rsplit(char::is_whitespace)
        .next()
        .is_some_and(|token| token.contains("://"));
    let absolute = word.starts_with('/') || before.ends_with('~');

    word.contains('/') && key_before(before).is_empty() && (in_url || absolute)
}

/// The key a value follows, such as `api_key=` in `api_key=abc…`, or an
/// authorization scheme such as `Bearer` in `Bearer abc…`.
fn key_before(before: &str) -> &str {
    let trimmed = before.trim_end_matches(['"', '\'', ' ']);
    let scheme = SCHEMES.iter().find_map(|scheme| {
        let start = trimmed.len().checked_sub(scheme.len())?;
        let tail = trimmed.get(start..)?;

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
        .map_or(0, |(index, character)| index + character.len_utf8());

    trimmed.get(start..).unwrap_or("")
}

/// Replace private-key blocks and known credential tokens with [`REDACTED`],
/// using the shipped defaults only. Unknown shapes are left as they are.
#[must_use]
pub fn redact_secrets(text: &str) -> String {
    let Ok(redactor) = Redactor::new(RedactionConfig::default(), LearnedShapes::default()) else {
        return text.to_string();
    };

    walk(text, |word, _| {
        if redactor.known(word) {
            REDACTED.into()
        } else {
            word.into()
        }
    })
}

/// Rebuild `text` with private-key blocks redacted and each word, at a word
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

const PEM_BEGIN: &str = "-----BEGIN ";
const PEM_KEY_TAIL: &str = "PRIVATE KEY-----";
const PEM_END: &str = "-----END ";

/// Byte length of a PEM private-key block starting at `text`, through its
/// END line, or to the end of `text` when unterminated.
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
    let line_end = after_end.find('\n').unwrap_or(after_end.len());

    Some(end.saturating_add(line_end))
}

/// Characters of a token body: letters, digits, `_`, and `-`.
fn is_token_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_' || character == '-'
}

/// Characters of a candidate word, which also covers base64. `=` is left out,
/// so `key=value` stays two words; base64 padding is not part of a word.
fn is_word_char(character: char) -> bool {
    is_token_char(character) || matches!(character, '+' | '/')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redactor() -> Redactor {
        Redactor::new(RedactionConfig::default(), LearnedShapes::default()).unwrap()
    }

    #[test]
    fn redacts_known_tokens_and_keeps_surrounding_text() {
        let text = "key AKIAABCDEFGHIJKLMNOP and ghp_abcdefghijklmnopqrstuvwxyz0123 done";

        assert_eq!(redact_secrets(text), "key [REDACTED] and [REDACTED] done");
    }

    #[test]
    fn a_url_path_survives_while_tokens_in_urls_are_still_caught() {
        let redactor = redactor();
        let mut masking = Masking::default();
        let slack =
            "https://adeptmind.slack.com/archives/C08ABCDEF12/p1790340000123456 Can you fix this?";
        let github = "https://github.com/StevenJPx2/chauffeur/pull/42";
        let token = "https://x.com/api/ghp_abcdefghijklmnopqrstuvwxyz0123/repos";
        let query = "https://x.com/cb?sig=Zx9kQ2mP7vL4/nR8sT1wY6uB3cD5fG0hJ+aa";

        let path = "cd /Users/stevenjohn/Documents/Adeptmind/Projects/hpdp-overlay/ADEPT-45130 && ls ~/Projects/ADEPT-45130/src";

        assert_eq!(redactor.redact(path, &mut masking), path);

        // A bare hash passes; a hex value after a key or scheme is judged.
        let sha = "3f786850e387550fdab836ed7e6dc881de23001b";

        assert_eq!(
            redactor.redact(&format!("git show {sha}"), &mut masking),
            format!("git show {sha}")
        );
        assert!(
            !redactor
                .redact(&format!("Authorization: Bearer {sha}"), &mut masking)
                .contains(sha)
        );
        assert!(
            !redactor
                .redact(&format!("X-Api-Key: {sha}"), &mut masking)
                .contains(sha)
        );
        assert_eq!(redactor.redact(slack, &mut masking), slack);
        assert_eq!(redactor.redact(github, &mut masking), github);
        assert_eq!(
            redactor.redact(token, &mut masking),
            "https://x.com/api/[REDACTED]/repos"
        );
        assert!(
            redactor
                .redact(query, &mut masking)
                .contains("sig=[string ")
        );
    }

    #[test]
    fn redacts_private_key_blocks() {
        let text = "before\n-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----\nafter";

        assert_eq!(redact_secrets(text), "before\n[REDACTED]\nafter");
    }

    #[test]
    fn leaves_short_or_embedded_lookalikes() {
        assert_eq!(
            redact_secrets("sk-short and task-ant-x"),
            "sk-short and task-ant-x"
        );
        assert_eq!(
            redact_secrets("mask-abcdefghijklmnopqrstuvwxyz"),
            "mask-abcdefghijklmnopqrstuvwxyz"
        );
        assert_eq!(redact_secrets("naïve ✓"), "naïve ✓");
    }

    #[test]
    fn unknown_credentials_are_described_never_sent_and_learned() {
        let mut redactor = redactor();
        let secret = "Zx9kQ2mP7vL4nR8sT1wY6uB3cD5fG0hJ";
        let sha = "3f786850e387550fdab836ed7e6dc881de23001b";
        let mut masking = Masking::default();
        let text = format!("curl -H api_key={secret} && git show commit={sha} && echo {secret}");
        let sent = redactor.redact(&text, &mut masking);

        assert!(!sent.contains(secret) && !sent.contains(sha));
        assert!(
            sent.contains(
                "[string 1: 32 characters of mixed letters and digits, after \"api_key=\"]"
            )
        );
        assert!(sent.contains("[string 2: 40 characters of hex, after \"commit=\"]"));
        assert_eq!(masking.shapes.len(), 2, "the same value keeps its number");

        assert!(redactor.learn(masking.shapes[0].0.clone(), true));
        assert!(redactor.learn(masking.shapes[1].0.clone(), false));

        let mut again = Masking::default();
        let sent = redactor.redact(&text, &mut again);

        assert_eq!(
            sent,
            format!("curl -H api_key=[REDACTED] && git show commit={sha} && echo [REDACTED]")
        );
        assert!(again.shapes.is_empty());

        // Calling a commit hash safe cannot release a same-shaped API token.
        let mut elsewhere = Masking::default();
        let sent = redactor.redact(&format!("token={sha}"), &mut elsewhere);
        assert!(!sent.contains(sha));
    }

    #[test]
    fn your_prefixes_add_to_or_replace_the_defaults() {
        let added = RedactionConfig {
            replace: false,
            prefixes: vec![Prefix {
                prefix: "acme_".into(),
                min_body: 8,
            }],
        };
        let replaced = RedactionConfig {
            replace: true,
            ..added.clone()
        };
        let mut masking = Masking::default();
        let added = Redactor::new(added, LearnedShapes::default()).unwrap();
        let replaced = Redactor::new(replaced, LearnedShapes::default()).unwrap();

        assert_eq!(
            added.redact(
                "acme_12345678 ghp_abcdefghijklmnopqrstuvwxyz0123",
                &mut masking
            ),
            "[REDACTED] [REDACTED]"
        );
        assert_eq!(
            replaced.redact("acme_12345678 ghp_abcdefghijklmnopqrstu", &mut masking),
            "[REDACTED] ghp_abcdefghijklmnopqrstu"
        );
    }
}
