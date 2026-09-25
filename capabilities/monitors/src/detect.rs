//! Exact facts in a tool call that name something worth following: a pull
//! request the call created, a Jira issue a Jira call touched, or a Slack
//! thread it names. Jev then judges whether it is the agent's own work.

use crate::watch::Watch;

/// Candidates one call yields, at most.
const MAX_CANDIDATES: usize = 2;
const MAX_KEY_PREFIX: usize = 10;
const MAX_KEY_NUMBER: usize = 7;

/// What a call with this `tool`, `input`, and output `evidence` names.
#[must_use]
pub fn candidates(tool: &str, input: &str, evidence: &str) -> Vec<Watch> {
    let text = format!("{input}\n{evidence}");
    let mut found: Vec<Watch> = Vec::new();

    if creates_pr(tool, input) {
        found.extend(pull_requests(&text));
    }
    if touches_jira(tool, input) {
        found.extend(
            jira_keys(&text)
                .into_iter()
                .map(|key| Watch::JiraIssue { key }),
        );
    }

    found.extend(slack_threads(&text));

    let mut unique: Vec<Watch> = Vec::new();

    for watch in found {
        if !unique.contains(&watch) {
            unique.push(watch);
        }
    }

    unique.truncate(MAX_CANDIDATES);
    unique
}

/// A PR is followed from the call that opened it, not one that read it.
fn creates_pr(tool: &str, input: &str) -> bool {
    ["create_pr", "open_pr", "create_pull_request"]
        .iter()
        .any(|name| tool.contains(name))
        || input.contains("pr create")
}

fn touches_jira(tool: &str, input: &str) -> bool {
    tool.starts_with("jira")
        || input.contains("jira issue")
        || input.contains("atlassian.net/browse/")
}

/// `github.com/owner/repo/pull/42` links.
fn pull_requests(text: &str) -> Vec<Watch> {
    text.match_indices("github.com/")
        .filter_map(|(start, pattern)| {
            let rest = text.get(start + pattern.len()..)?;
            let mut parts = rest.splitn(4, '/');
            let (owner, repo, pull, tail) =
                (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
            let number: String = tail.chars().take_while(char::is_ascii_digit).collect();

            (pull == "pull" && name(owner) && name(repo) && !number.is_empty()).then(|| {
                Watch::GithubPr {
                    repo: format!("{owner}/{repo}"),
                    number: number.parse().unwrap_or_default(),
                }
            })
        })
        .filter(|watch| !matches!(watch, Watch::GithubPr { number: 0, .. }))
        .collect()
}

fn name(part: &str) -> bool {
    !part.is_empty()
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Jira keys such as `ADEPT-123`: an uppercase project key, a dash, digits.
fn jira_keys(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut keys = Vec::new();

    for (dash, _) in text.match_indices('-') {
        let start = (0..dash)
            .rev()
            .take_while(|&i| bytes[i].is_ascii_uppercase() || bytes[i].is_ascii_digit())
            .last();
        let Some(start) = start else { continue };
        let prefix = &text[start..dash];
        let digits: String = text[dash + 1..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let bounded = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();

        if bounded
            && prefix.as_bytes()[0].is_ascii_uppercase()
            && (2..=MAX_KEY_PREFIX).contains(&prefix.len())
            && (1..=MAX_KEY_NUMBER).contains(&digits.len())
        {
            let key = format!("{prefix}-{digits}");

            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }

    keys
}

/// `slack.com/archives/C123/p1700000000123456` links; `p` drops the dot of
/// the thread's timestamp, `1700000000.123456`.
fn slack_threads(text: &str) -> Vec<Watch> {
    text.match_indices("slack.com/archives/")
        .filter_map(|(start, pattern)| {
            let rest = text.get(start + pattern.len()..)?;
            let (channel, tail) = rest.split_once('/')?;
            let digits: String = tail
                .strip_prefix('p')?
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();

            (name(channel) && digits.len() > 6).then(|| {
                let (seconds, micros) = digits.split_at(digits.len() - 6);

                Watch::SlackThread {
                    channel: channel.to_string(),
                    thread: format!("{seconds}.{micros}"),
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_created_pull_request_is_a_candidate_and_a_viewed_one_is_not() {
        let created = "https://github.com/acme/app/pull/42\n";

        assert_eq!(
            candidates("shell", r#"{"command":"gh pr create --fill"}"#, created),
            vec![Watch::GithubPr {
                repo: "acme/app".into(),
                number: 42
            }]
        );
        assert!(candidates("shell", r#"{"command":"gh pr view 42"}"#, created).is_empty());
        assert!(candidates("github_create_pr", "{}", "no link here").is_empty());
    }

    #[test]
    fn jira_keys_come_only_from_jira_calls() {
        assert_eq!(
            candidates("shell", r#"{"command":"jira issue view ADEPT-45130"}"#, ""),
            vec![Watch::JiraIssue {
                key: "ADEPT-45130".into()
            }]
        );
        assert!(candidates("shell", r#"{"command":"echo UTF-8 ADEPT-1"}"#, "").is_empty());
        assert!(jira_keys("utf-8 X-1 abcDEF-12 QA-7").eq(&["QA-7".to_string()]));
    }

    #[test]
    fn a_slack_thread_link_names_its_thread() {
        assert_eq!(
            candidates(
                "webfetch",
                "https://acme.slack.com/archives/C024BE91L/p1700000000123456",
                ""
            ),
            vec![Watch::SlackThread {
                channel: "C024BE91L".into(),
                thread: "1700000000.123456".into()
            }]
        );
    }
}
