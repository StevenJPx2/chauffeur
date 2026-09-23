//! Built-in secret redaction applied before any state leaves the machine.

pub const REDACTED: &str = "[REDACTED]";

struct TokenPattern {
    prefix: &'static str,
    /// Minimum characters after the prefix.
    min_body: usize,
}

/// Longer prefixes first so `sk-ant-` wins over `sk-`.
const TOKENS: &[TokenPattern] = &[
    TokenPattern {
        prefix: "github_pat_",
        min_body: 20,
    },
    TokenPattern {
        prefix: "sk-ant-",
        min_body: 20,
    },
    TokenPattern {
        prefix: "ghp_",
        min_body: 20,
    },
    TokenPattern {
        prefix: "gho_",
        min_body: 20,
    },
    TokenPattern {
        prefix: "ghu_",
        min_body: 20,
    },
    TokenPattern {
        prefix: "ghs_",
        min_body: 20,
    },
    TokenPattern {
        prefix: "ghr_",
        min_body: 20,
    },
    TokenPattern {
        prefix: "xoxb-",
        min_body: 10,
    },
    TokenPattern {
        prefix: "xoxa-",
        min_body: 10,
    },
    TokenPattern {
        prefix: "xoxp-",
        min_body: 10,
    },
    TokenPattern {
        prefix: "xoxr-",
        min_body: 10,
    },
    TokenPattern {
        prefix: "xoxs-",
        min_body: 10,
    },
    TokenPattern {
        prefix: "AKIA",
        min_body: 16,
    },
    TokenPattern {
        prefix: "AIza",
        min_body: 35,
    },
    TokenPattern {
        prefix: "sk-",
        min_body: 20,
    },
];

const PEM_BEGIN: &str = "-----BEGIN ";
const PEM_KEY_TAIL: &str = "PRIVATE KEY-----";
const PEM_END: &str = "-----END ";

/// Replace private-key blocks and known credential tokens with [`REDACTED`].
#[must_use]
pub fn redact_secrets(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;

    while !rest.is_empty() {
        if let Some(skip) = private_key_len(rest) {
            output.push_str(REDACTED);
            rest = rest.get(skip..).unwrap_or("");
            continue;
        }

        let at_boundary = output
            .chars()
            .next_back()
            .is_none_or(|last| !is_token_char(last));

        if let Some(skip) = at_boundary.then(|| token_len(rest)).flatten() {
            output.push_str(REDACTED);
            rest = rest.get(skip..).unwrap_or("");
            continue;
        }

        let Some(next) = rest.chars().next() else {
            break;
        };

        output.push(next);
        rest = rest.get(next.len_utf8()..).unwrap_or("");
    }

    output
}

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

fn token_len(text: &str) -> Option<usize> {
    TOKENS.iter().find_map(|pattern| {
        let body = text.strip_prefix(pattern.prefix)?;
        let body_len = body
            .char_indices()
            .find(|(_, character)| !is_token_char(*character))
            .map_or(body.len(), |(index, _)| index);

        (body_len >= pattern.min_body).then(|| pattern.prefix.len().saturating_add(body_len))
    })
}

fn is_token_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_' || character == '-'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_known_tokens_and_keeps_surrounding_text() {
        let text = "key AKIAABCDEFGHIJKLMNOP and ghp_abcdefghijklmnopqrstuvwxyz0123 done";

        assert_eq!(redact_secrets(text), "key [REDACTED] and [REDACTED] done");
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
}
