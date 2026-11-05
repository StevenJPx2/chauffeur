use std::borrow::Cow;

use super::REDACTED;

const PEM_BEGIN: &str = "-----BEGIN ";
const PEM_KEY_TAIL: &str = "PRIVATE KEY-----";
const PEM_END: &str = "-----END ";

/// Rebuilds `text` with private-key blocks redacted and each word, at a word
/// boundary, replaced by `word(word, text_so_far)`. A word left as it was
/// comes back borrowed, so it costs no allocation.
///
/// Word characters are all ASCII, so words are found byte by byte: a
/// character outside ASCII is never part of one.
pub(super) fn walk<'a>(
    text: &'a str,
    mut word: impl FnMut(&'a str, &str) -> Cow<'a, str>,
) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(next) = rest.chars().next() {
        if let Some(skip) = private_key_len(rest) {
            output.push_str(REDACTED);
            rest = rest.get(skip..).unwrap_or("");
            continue;
        }
        let at_boundary = output
            .as_bytes()
            .last()
            .is_none_or(|&last| !is_word_byte(last));
        let length = rest
            .bytes()
            .position(|byte| !is_word_byte(byte))
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
    // Checked at every position, so the first byte rules most of them out.
    if text.as_bytes().first() != Some(&b'-') {
        return None;
    }
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

/// Bytes of a token body: letters, digits, `_` and `-`.
pub(super) const fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

/// Every byte, marked when it belongs in a word, so finding a word's end is
/// one lookup per byte.
#[expect(
    clippy::indexing_slicing,
    clippy::as_conversions,
    reason = "a u8 indexes a 256-entry table, and const code has no get_mut or usize::from"
)]
const WORD_BYTES: [bool; 256] = {
    let mut table = [false; 256];
    let mut byte: u8 = 0;
    loop {
        table[byte as usize] = is_token_byte(byte) || matches!(byte, b'+' | b'/');
        if byte == u8::MAX {
            break table;
        }
        byte += 1;
    }
};

/// Bytes of a candidate word, which also covers base64. `=` is left out, so
/// `key=value` stays two words.
fn is_word_byte(byte: u8) -> bool {
    WORD_BYTES.get(usize::from(byte)).copied().unwrap_or(false)
}
