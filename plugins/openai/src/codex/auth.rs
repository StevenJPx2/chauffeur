//! Your Codex CLI login, from `$CODEX_HOME/auth.json`, else
//! `~/.codex/auth.json`. The access token is refreshed shortly before it
//! expires, and the refreshed tokens are written back so the Codex CLI keeps
//! working: refresh tokens rotate, so a refresh kept only here would log it
//! out. Every other field of the file is kept as it was.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use reqwest::blocking::Client;
use serde_json::{Value, json};

pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// Refresh this long before the access token expires.
const MARGIN_SECS: u64 = 60;
/// Larger auth files are refused rather than read.
const MAX_AUTH_BYTES: u64 = 64 * 1024;

/// What a request needs from the login.
#[derive(Clone)]
pub struct Token {
    pub access: String,
    /// The ChatGPT account, sent as `chatgpt-account-id`; empty if unknown.
    pub account: String,
}

impl std::fmt::Debug for Token {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Token { .. }")
    }
}

/// One login, shared by every worker so only one refreshes at a time.
pub struct CodexAuth {
    path: PathBuf,
    token_url: String,
    cached: Mutex<Option<Token>>,
}

impl CodexAuth {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            token_url: TOKEN_URL.into(),
            cached: Mutex::new(None),
        }
    }

    /// `$CODEX_HOME/auth.json`, else `~/.codex/auth.json`.
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
            .map(|home| home.join("auth.json"))
    }

    /// Refresh against `url`, such as a test server.
    #[must_use]
    pub fn with_token_url(mut self, url: impl Into<String>) -> Self {
        self.token_url = url.into();
        self
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A token valid for at least another minute, refreshing it if needed.
    ///
    /// # Errors
    ///
    /// No ChatGPT login in the file, or a refresh that failed. Messages never
    /// include a token.
    pub fn token(&self, http: &Client) -> Result<Token, String> {
        let mut cached = self
            .cached
            .lock()
            .map_err(|_| "Codex login lock poisoned".to_string())?;

        if let Some(token) = cached.as_ref().filter(|token| fresh(&token.access)) {
            return Ok(token.clone());
        }

        // The file again: the Codex CLI may have refreshed it meanwhile.
        let mut file = self.read()?;

        if !fresh(field(&file, "access_token")) {
            self.refresh(http, &mut file)?;
            write(&self.path, &file)?;
        }

        let token = Token {
            access: field(&file, "access_token").to_string(),
            account: field(&file, "account_id").to_string(),
        };

        *cached = Some(token.clone());

        Ok(token)
    }

    /// Drop the cached token, such as after the provider rejected it, so the
    /// next call reads the file again.
    pub fn forget(&self) {
        if let Ok(mut cached) = self.cached.lock() {
            *cached = None;
        }
    }

    fn read(&self) -> Result<Value, String> {
        let shown = self.path.display();
        let size = std::fs::metadata(&self.path)
            .map_err(|error| format!("no Codex login at {shown} ({error}); run `codex login`"))?
            .len();

        if size > MAX_AUTH_BYTES {
            return Err(format!("{shown} exceeds {MAX_AUTH_BYTES} bytes"));
        }

        let text = std::fs::read_to_string(&self.path)
            .map_err(|error| format!("read {shown}: {error}"))?;
        let file: Value =
            serde_json::from_str(&text).map_err(|error| format!("{shown} is not JSON: {error}"))?;

        if field(&file, "access_token").is_empty() {
            return Err(format!(
                "{shown} has no ChatGPT login; run `codex login` with your ChatGPT account"
            ));
        }

        Ok(file)
    }

    fn refresh(&self, http: &Client, file: &mut Value) -> Result<(), String> {
        let refresh = field(file, "refresh_token");

        if refresh.is_empty() {
            return Err(
                "the Codex login expired and has no refresh token; run `codex login`".into(),
            );
        }

        let response = http
            .post(&self.token_url)
            .json(&json!({
                "client_id": CLIENT_ID,
                "grant_type": "refresh_token",
                "refresh_token": refresh,
                "scope": "openid profile email",
            }))
            .send()
            .map_err(|error| format!("Codex login refresh: {error}"))?;

        if !response.status().is_success() {
            return Err(format!(
                "Codex login refresh returned HTTP {}; run `codex login`",
                response.status()
            ));
        }

        let refreshed: Value = response
            .json()
            .map_err(|error| format!("Codex login refresh reply: {error}"))?;
        let access = refreshed["access_token"]
            .as_str()
            .filter(|access| !access.is_empty())
            .ok_or("Codex login refresh returned no access token")?;
        let tokens = file
            .get_mut("tokens")
            .and_then(Value::as_object_mut)
            .ok_or("the Codex login has no tokens")?;

        tokens.insert("access_token".into(), access.into());
        for name in ["refresh_token", "id_token"] {
            if let Some(value) = refreshed[name].as_str() {
                tokens.insert(name.into(), value.into());
            }
        }
        file["last_refresh"] = rfc3339(now()).into();

        Ok(())
    }
}

/// `tokens.<name>` in the auth file, or empty.
fn field<'a>(file: &'a Value, name: &str) -> &'a str {
    file["tokens"][name].as_str().unwrap_or_default()
}

/// Whether `token` is valid for longer than the margin.
fn fresh(token: &str) -> bool {
    expiry(token) > now() + MARGIN_SECS
}

/// A JWT's `exp`, or 0 when it has none.
fn expiry(token: &str) -> u64 {
    token
        .split('.')
        .nth(1)
        .and_then(|part| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(part)
                .ok()
        })
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|claims| claims["exp"].as_u64())
        .unwrap_or(0)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Write atomically, readable by you alone, as the Codex CLI keeps it.
fn write(path: &Path, file: &Value) -> Result<(), String> {
    let shown = path.display();
    let text = serde_json::to_string_pretty(file).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.chauffeur-tmp");
    let mut options = std::fs::OpenOptions::new();

    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);

    let written = options.open(&temporary).and_then(|mut out| {
        std::io::Write::write_all(&mut out, format!("{text}\n").as_bytes())?;
        out.sync_all()
    });

    written
        .and_then(|()| std::fs::rename(&temporary, path))
        .map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            format!("save the refreshed Codex login to {shown}: {error}")
        })
}

/// `secs` since the epoch as an RFC 3339 UTC timestamp.
fn rfc3339(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rest = secs % 86_400;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);

    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_rfc3339_utc() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_868_800), "2000-03-01T00:00:00Z");
        assert_eq!(rfc3339(1_791_158_399), "2026-10-04T23:59:59Z");
    }

    #[test]
    fn a_token_without_a_readable_expiry_is_stale() {
        assert!(!fresh("not-a-jwt"));
        assert!(!fresh("a.%%%.c"));
    }
}
