use std::fmt::Write as _;

/// Credential-like: at least this long with upper and lower case and digits…
const MIN_MIXED_LEN: usize = 24;
/// …or at least this long in hex.
const MIN_HEX_LEN: usize = 32;
/// How much of a word's start can be a readable prefix such as `sk_live_`.
const MAX_PREFIX_LEN: usize = 10;

/// What a credential-like word looks like, without its value.
pub(super) struct Shape {
    prefix: String,
    class: &'static str,
    length: usize,
}

impl Shape {
    /// Whether the word is all hex, as a commit or content hash is.
    pub(super) fn is_hex(&self) -> bool {
        self.class == "hex"
    }
}

/// `[string 2: 40 characters of hex, starts "sk_", after "api_key="]`
pub(super) fn describe(number: usize, shape: &Shape, key: &str) -> String {
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
pub(super) fn shape(word: &str) -> Option<Shape> {
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
