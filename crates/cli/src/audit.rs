//! `chauffeur audit [N] [--brief] [--follow]`: the last N decisions from the
//! daemon's audit log, one line each, and with `--follow` every new one as
//! it is written.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

const DEFAULT_LINES: usize = 20;
const MAX_LINES: usize = 1_000;
const MAX_AUDIT_BYTES: u64 = 9 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(250);
const USAGE: &str = "usage: chauffeur audit [N] [--brief] [--follow] (N up to 1000)";

struct Options {
    count: usize,
    brief: bool,
    follow: bool,
}

pub fn run(args: &[String], path: &Path) -> Result<(), String> {
    let options = parse(args)?;
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("no audit log at {}: {error}", path.display()))?;

    if metadata.len() > MAX_AUDIT_BYTES {
        return Err(format!(
            "{} exceeds {MAX_AUDIT_BYTES} bytes",
            path.display()
        ));
    }

    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    let lines: Vec<&str> = text.lines().collect();

    for line in &lines[lines.len().saturating_sub(options.count)..] {
        println!("{}", render_line(line, options.brief));
    }

    if options.follow {
        follow(path, metadata.len(), options.brief)?;
    }

    Ok(())
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        count: DEFAULT_LINES,
        brief: false,
        follow: false,
    };
    let mut counted = false;

    for arg in args {
        match arg.as_str() {
            "--brief" | "-b" => options.brief = true,
            "--follow" | "-f" => options.follow = true,
            value => {
                let count: usize = value.parse().map_err(|_| USAGE.to_string())?;
                options.count = count.min(MAX_LINES);
                counted = true;
            }
        }
    }

    // Following starts at the end unless a count was asked for.
    if options.follow && !counted {
        options.count = 0;
    }

    Ok(options)
}

/// Print each complete line appended after `offset`, until interrupted. A
/// file that shrinks was rotated or cleared, so reading starts over.
fn follow(path: &Path, mut offset: u64, brief: bool) -> Result<(), String> {
    let mut partial = String::new();

    loop {
        std::thread::sleep(POLL);

        let Ok(len) = std::fs::metadata(path).map(|metadata| metadata.len()) else {
            continue;
        };

        if len < offset {
            offset = 0;
            partial.clear();
        }
        if len == offset {
            continue;
        }

        let mut file = std::fs::File::open(path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        let mut chunk = String::new();

        file.seek(SeekFrom::Start(offset))
            .and_then(|_| file.take(len - offset).read_to_string(&mut chunk))
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        offset = len;
        partial.push_str(&chunk);

        while let Some(end) = partial.find('\n') {
            let line: String = partial.drain(..=end).collect();
            println!("{}", render_line(line.trim_end(), brief));
        }
    }
}

fn render_line(line: &str, brief: bool) -> String {
    let record = serde_json::from_str(line).unwrap_or(Value::Null);

    if brief {
        render_brief(&record)
    } else {
        render(&record)
    }
}

fn clock(record: &Value) -> String {
    let at = record["at"].as_u64().unwrap_or(0);

    format!("{:02}:{:02}:{:02}", at / 3_600 % 24, at / 60 % 60, at % 60)
}

fn effects(record: &Value) -> String {
    let effects: Vec<String> = record["effects"]
        .as_array()
        .map(|effects| effects.iter().map(effect).collect())
        .unwrap_or_default();

    if effects.is_empty() {
        "nothing".into()
    } else {
        effects.join(", ")
    }
}

/// `12:04:31 tool_result → context steer · 412 ms`, for a narrow pane.
fn render_brief(record: &Value) -> String {
    let veto = record["trace"]["veto"]
        .as_str()
        .map(|reason| format!(" · vetoed: {reason}"))
        .unwrap_or_default();

    format!(
        "{} {} → {} · {} ms{veto}",
        clock(record),
        record["signal"].as_str().unwrap_or("?"),
        effects(record),
        record["trace"]["elapsed_ms"].as_u64().unwrap_or(0)
    )
}

/// `12:04:31 ses_f32… tool_result shell {...} → nudge | 6 asked, 412 ms`
fn render(record: &Value) -> String {
    let agent: String = record["agent_id"]
        .as_str()
        .unwrap_or("?")
        .chars()
        .take(12)
        .collect();
    let trace = &record["trace"];
    let mut notes = vec![format!(
        "{} asked, {} ms",
        trace["questions"].as_array().map_or(0, Vec::len),
        trace["elapsed_ms"].as_u64().unwrap_or(0)
    )];

    for (key, label) in [("veto", "vetoed"), ("error", "Jev failed")] {
        if let Some(reason) = trace[key].as_str() {
            notes.push(format!("{label}: {reason}"));
        }
    }

    format!(
        "{} UTC {agent}… {} {} → {} | {}",
        clock(record),
        record["signal"].as_str().unwrap_or("?"),
        record["detail"].as_str().unwrap_or(""),
        effects(record),
        notes.join("; ")
    )
}

fn effect(effect: &Value) -> String {
    let name = effect["type"].as_str().unwrap_or("?");

    if name == "tools" {
        return tools(effect);
    }
    if name == "context" {
        return context(effect);
    }

    let about = [
        "skills",
        "tools",
        "namespaces",
        "decision",
        "rule_id",
        "model",
    ]
    .iter()
    .find_map(|key| match &effect[*key] {
        Value::Null => None,
        Value::Array(items) if items.is_empty() => None,
        Value::String(text) => Some(text.clone()),
        Value::Array(items) if items.len() > 3 => Some(format!("{} items", items.len())),
        other => Some(other.to_string()),
    });

    about.map_or_else(|| name.to_string(), |about| format!("{name} {about}"))
}

/// `attach jira-cli, gh` for skills; otherwise the delivery and its label,
/// such as `steer "Reading files through the shell"`.
fn context(effect: &Value) -> String {
    let skills: Vec<&str> = effect["skills"]
        .as_array()
        .map(|skills| skills.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    if !skills.is_empty() {
        return format!("attach {}", skills.join(", "));
    }

    let delivery = effect["delivery"].as_str().unwrap_or("context");

    match effect["label"].as_str() {
        Some(label) if !label.is_empty() => format!("{delivery} \"{label}\""),
        _ => delivery.to_string(),
    }
}

/// `hide skill, artifact_* (3)` and `reveal artifact_* (3)`.
fn tools(effect: &Value) -> String {
    let parts: Vec<String> = ["hide", "reveal"]
        .iter()
        .filter_map(|key| {
            let names: Vec<&str> = effect[*key]
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .collect();

            (!names.is_empty()).then(|| format!("{key} {}", groups(&names)))
        })
        .collect();

    if parts.is_empty() {
        "tools".into()
    } else {
        parts.join(" · ")
    }
}

/// Tool names grouped by the prefix before the first `_`, as Chauffeur
/// judges them: `artifact_publish, artifact_edit` reads `artifact_* (2)`.
fn groups(names: &[&str]) -> String {
    let mut groups: Vec<(&str, usize)> = Vec::new();

    for name in names {
        let group = name.split_once('_').map_or(*name, |(prefix, _)| prefix);

        match groups.iter_mut().find(|(seen, _)| *seen == group) {
            Some((_, count)) => *count += 1,
            None => groups.push((group, 1)),
        }
    }

    groups
        .iter()
        .map(|(group, count)| match count {
            1 => names
                .iter()
                .find(|name| name.starts_with(group))
                .map_or_else(|| (*group).to_string(), |name| (*name).to_string()),
            count => format!("{group}_* ({count})"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn hidden_and_revealed_tools_are_grouped() {
        let hide = json!({
            "type": "tools",
            "hide": ["skill", "artifact_publish", "artifact_edit", "artifact_delete"],
            "reveal": []
        });
        let reveal =
            json!({ "type": "tools", "hide": [], "reveal": ["browser_open", "browser_read"] });

        assert_eq!(effect(&hide), "hide skill, artifact_* (3)");
        assert_eq!(effect(&reveal), "reveal browser_* (2)");
    }

    #[test]
    fn context_reads_as_attach_or_its_label() {
        let steer = json!({
            "type": "context",
            "skills": [],
            "delivery": "steer",
            "label": "Reading files through the shell"
        });

        assert_eq!(effect(&steer), "steer \"Reading files through the shell\"");
        assert_eq!(
            effect(&json!({ "type": "context", "skills": ["jira-cli", "gh"] })),
            "attach jira-cli, gh"
        );
        assert_eq!(effect(&json!({ "type": "gate", "skills": [] })), "gate");
        assert_eq!(
            effect(&json!({ "type": "permission", "decision": "allow" })),
            "permission allow"
        );
    }

    #[test]
    fn brief_lines_show_time_signal_effects_and_timing() {
        let record = json!({
            "at": 3_661,
            "signal": "user_message",
            "effects": [{ "type": "tools", "hide": ["skill"], "reveal": [] }],
            "trace": { "elapsed_ms": 412 }
        });

        assert_eq!(
            render_brief(&record),
            "01:01:01 user_message → hide skill · 412 ms"
        );
    }

    #[test]
    fn follow_starts_at_the_end_unless_counted() {
        let follow = parse(&["-f".into()]).unwrap();
        let counted = parse(&["5".into(), "--follow".into(), "--brief".into()]).unwrap();

        assert_eq!((follow.count, follow.follow), (0, true));
        assert_eq!((counted.count, counted.brief), (5, true));
        assert!(parse(&["--nope".into()]).is_err());
    }
}
