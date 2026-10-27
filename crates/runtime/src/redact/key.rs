/// Authorization schemes whose value follows a space, not `=` or `:`.
const SCHEMES: [&str; 2] = ["bearer", "basic"];

/// Whether `word` is a path whose segments are judged one by one: an
/// absolute or home path, a URL's path, or a relative path such as
/// `acme/web-app#42` (base64 has `/` but never `-` or `_`, and base64url has
/// those but never `/`). A value after `key=` or `key:` stays whole, as
/// base64 with `/` would.
pub(super) fn path_like(word: &str, before: &str) -> bool {
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
pub(super) fn key_before(before: &str) -> &str {
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
