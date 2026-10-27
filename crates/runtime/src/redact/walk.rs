use super::REDACTED;

const PEM_BEGIN: &str = "-----BEGIN ";
const PEM_KEY_TAIL: &str = "PRIVATE KEY-----";
const PEM_END: &str = "-----END ";

/// Rebuilds `text` with private-key blocks redacted and each word, at a word
/// boundary, replaced by `word(word, text_so_far)`.
pub(super) fn walk(text: &str, mut word: impl FnMut(&str, &str) -> String) -> String {
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
pub(super) const fn is_token_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_' || character == '-'
}

/// Characters of a candidate word, which also covers base64. `=` is left
/// out, so `key=value` stays two words.
const fn is_word_char(character: char) -> bool {
    is_token_char(character) || matches!(character, '+' | '/')
}
