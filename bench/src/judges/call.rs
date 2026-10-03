//! One decision-model endpoint speaking the `/v1/systemone` wire format.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Map, Value};

// Generous: a local model loads on its first call, and hosted ones queue.
const TIMEOUT: Duration = Duration::from_secs(60);

/// A judge as listed in `judges.json`.
#[derive(Clone, Debug, Deserialize)]
pub struct Judge {
    pub name: String,
    /// The endpoint; `{VAR}` is replaced with the environment variable `VAR`.
    pub url: String,
    pub model: String,
    /// The environment variable holding the bearer token, if any.
    #[serde(default)]
    pub key_env: Option<String>,
    /// A field wrapping the reply, such as Workers AI's `result`.
    #[serde(default)]
    pub unwrap: Option<String>,
    /// Send question IDs with only letters, digits, `_` and `-`, mapping the
    /// answers back: Workers AI rejects Chauffeur's `capability/name` IDs.
    #[serde(default)]
    pub safe_ids: bool,
}

pub struct Caller {
    http: reqwest::blocking::Client,
    url: String,
    key: Option<String>,
    model: String,
    unwrap: Option<String>,
    safe_ids: bool,
}

impl Caller {
    /// # Errors
    /// A missing environment variable or an HTTP client that cannot be built.
    pub fn new(judge: &Judge) -> Result<Self, String> {
        let key = judge
            .key_env
            .as_deref()
            .map(|name| env(name, &judge.name))
            .transpose()?;
        let http = reqwest::blocking::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .map_err(|error| format!("{}: build HTTP client: {error}", judge.name))?;

        Ok(Self {
            http,
            url: expand(&judge.url, &judge.name)?,
            key,
            model: judge.model.clone(),
            unwrap: judge.unwrap.clone(),
            safe_ids: judge.safe_ids,
        })
    }

    /// Ask the recorded request's questions with this judge's model.
    ///
    /// # Errors
    /// A transport failure, an HTTP error, or a reply without answers.
    pub fn ask(&self, request: &Value) -> Result<Map<String, Value>, String> {
        let mut body = request.clone();
        body["model"] = Value::String(self.model.clone());
        let renamed = if self.safe_ids {
            rename_ids(&mut body)
        } else {
            Vec::new()
        };

        let mut post = self.http.post(&self.url).json(&body);
        if let Some(key) = &self.key {
            post = post.bearer_auth(key);
        }
        let response = post.send().map_err(|error| format!("request: {error}"))?;
        let status = response.status();
        let reply: Value = response
            .json()
            .map_err(|error| format!("HTTP {status}: decode reply: {error}"))?;

        if !status.is_success() {
            return Err(format!("HTTP {status}: {}", clip(&reply.to_string())));
        }
        let reply = match &self.unwrap {
            Some(field) => &reply[field.as_str()],
            None => &reply,
        };

        let mut answers = reply["answers"]
            .as_object()
            .cloned()
            .ok_or_else(|| format!("no answers: {}", clip(&reply.to_string())))?;

        for (original, safe) in renamed {
            if let Some(answer) = answers.remove(&safe) {
                answers.insert(original, answer);
            }
        }

        Ok(answers)
    }
}

/// Rename each question to `q0`, `q1`, …; return (original, sent) pairs.
fn rename_ids(body: &mut Value) -> Vec<(String, String)> {
    let Some(questions) = body["questions"].as_object_mut() else {
        return Vec::new();
    };
    let original = std::mem::take(questions);
    let mut renamed = Vec::new();

    for (index, (id, question)) in original.into_iter().enumerate() {
        let safe = format!("q{index}");
        questions.insert(safe.clone(), question);
        renamed.push((id, safe));
    }

    renamed
}

fn env(name: &str, judge: &str) -> Result<String, String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{judge}: set {name}"))
}

/// Replace each `{VAR}` in `url` with the environment variable `VAR`.
fn expand(url: &str, judge: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = url;

    while let Some(start) = rest.find('{') {
        let end = rest[start..]
            .find('}')
            .ok_or_else(|| format!("{judge}: unclosed {{ in {url}"))?;
        out.push_str(&rest[..start]);
        out.push_str(&env(&rest[start + 1..start + end], judge)?);
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);

    Ok(out)
}

fn clip(text: &str) -> String {
    text.chars().take(300).collect()
}

#[cfg(test)]
mod tests {
    use super::{expand, rename_ids};

    #[test]
    fn renames_question_ids_and_keeps_the_mapping() {
        let mut body =
            serde_json::json!({ "questions": { "skill-exposure/drift": { "type": "noul" } } });
        let renamed = rename_ids(&mut body);

        assert_eq!(
            renamed,
            vec![("skill-exposure/drift".to_string(), "q0".to_string())]
        );
        assert_eq!(body["questions"]["q0"]["type"], "noul");
    }

    #[test]
    fn expands_variables_in_the_url() {
        // HOME is set in every test environment.
        let home = std::env::var("HOME").unwrap();

        assert_eq!(expand("a/{HOME}/b", "j").unwrap(), format!("a/{home}/b"));
        assert_eq!(expand("plain", "j").unwrap(), "plain");
        assert!(expand("a/{CHAUFFEUR_NO_SUCH_VAR}", "j").is_err());
        assert!(expand("a/{HOME", "j").is_err());
    }
}
