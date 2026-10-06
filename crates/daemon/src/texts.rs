//! The wording each host shows the agent: tool descriptions, replies and
//! notes. Shipped in `skills/config/hosts/<host>.json`, overlaid by your file
//! of the same name in the config folder, and reloaded when it changes. The
//! daemon checks only the shape; the host owns what each text means.

use std::collections::HashMap;
use std::path::Path;

use chauffeur_core::template::MAX_TEMPLATE_BYTES;
use chauffeur_core::{Reloading, Watch, load_layered};
use serde_json::Value;

/// Each host's shipped texts, compiled in.
const HOSTS: &[(&str, &str)] = &[(
    "opencode",
    include_str!("../../../skills/config/hosts/opencode.json"),
)];

/// Groups nest at most this deep.
const MAX_DEPTH: usize = 3;

/// Texts an open map (an empty object in the shipped file) may hold.
const MAX_OPEN_TEXTS: usize = 64;

/// Every known host's texts, each reloaded when your file changes.
pub struct HostTexts(HashMap<&'static str, Reloading<Value>>);

impl HostTexts {
    /// # Errors
    ///
    /// A shipped or overriding file that is unreadable or out of shape.
    pub fn load(config_dir: &Path) -> Result<Self, String> {
        let mut hosts = HashMap::new();

        for (host, shipped) in HOSTS {
            let path = config_dir.join("hosts").join(format!("{host}.json"));
            let texts = load(shipped, &path)?;
            let watched = path.clone();

            hosts.insert(
                *host,
                Reloading::new(
                    format!("{host} texts"),
                    texts,
                    Watch::new(vec![watched]),
                    Box::new(move |_| load(shipped, &path)),
                ),
            );
        }

        Ok(Self(hosts))
    }

    /// `host`'s texts as they stand now.
    ///
    /// # Errors
    ///
    /// An unknown host.
    pub fn current(&mut self, host: &str) -> Result<Value, String> {
        self.0
            .get_mut(host)
            .map(|texts| texts.current().clone())
            .ok_or_else(|| format!("no texts for host {host}"))
    }
}

/// Lists the groups that take texts under any name, such as tool
/// descriptions keyed by tool name; only the shipped file sets it.
const OPEN: &str = "$open";

/// The shipped texts overlaid by your file, if every key yours names exists
/// in the shipped file (or sits in an `$open` group) and every text is a
/// string within bounds.
fn load(shipped: &str, path: &Path) -> Result<Value, String> {
    let mut defaults: Value =
        serde_json::from_str(shipped).map_err(|error| format!("shipped host texts: {error}"))?;
    let mut merged: Value = load_layered(shipped, path)?;
    let open_groups = take_open(&mut defaults)?;
    let error = |error: String| format!("{}: {error}", path.display());

    if take_open(&mut merged).map_err(error)? != open_groups {
        return Err(error(format!("{OPEN} is set by the shipped texts only")));
    }

    same_shape(&defaults, &merged, "", 0, &open_groups).map_err(error)?;

    if let Value::Object(texts) = &mut merged {
        texts.insert(OPEN.into(), open_groups.into());
    }

    Ok(merged)
}

/// Remove and return the `$open` group names.
fn take_open(texts: &mut Value) -> Result<Vec<String>, String> {
    let Some(open) = texts.as_object_mut().and_then(|texts| texts.remove(OPEN)) else {
        return Ok(Vec::new());
    };

    serde_json::from_value(open).map_err(|_| format!("{OPEN} must list group names"))
}

fn same_shape(
    shipped: &Value,
    merged: &Value,
    at: &str,
    depth: usize,
    open_groups: &[String],
) -> Result<(), String> {
    match (shipped, merged) {
        (Value::Object(shipped), Value::Object(merged)) if depth < MAX_DEPTH => {
            let open = open_groups.iter().any(|group| group == at);
            let any_text = Value::String(String::new());

            if open && merged.len() > MAX_OPEN_TEXTS {
                return Err(format!("{at} holds more than {MAX_OPEN_TEXTS} texts"));
            }

            for (key, value) in merged {
                let here = if at.is_empty() {
                    key.clone()
                } else {
                    format!("{at}.{key}")
                };
                let expected = if open {
                    &any_text
                } else {
                    shipped
                        .get(key)
                        .ok_or_else(|| format!("unknown text {here}"))?
                };

                same_shape(expected, value, &here, depth + 1, open_groups)?;
            }

            Ok(())
        }
        (Value::String(_), Value::String(text)) => {
            if text.trim().is_empty() || text.len() > MAX_TEMPLATE_BYTES {
                return Err(format!("{at} must be 1-{MAX_TEMPLATE_BYTES} bytes"));
            }

            Ok(())
        }
        _ => Err(format!("{at} has the wrong shape")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("chauffeur-texts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("hosts")).unwrap();
        dir
    }

    #[test]
    fn your_file_overrides_one_text_and_typos_are_rejected() {
        let config = dir("override");
        std::fs::write(
            config.join("hosts/opencode.json"),
            r#"{ "todowrite": { "empty": "Nothing to do." } }"#,
        )
        .unwrap();

        let mut texts = HostTexts::load(&config).unwrap();
        let opencode = texts.current("opencode").unwrap();

        assert_eq!(opencode["todowrite"]["empty"], "Nothing to do.");
        assert!(
            opencode["todowrite"]["description"]
                .as_str()
                .unwrap()
                .contains("todo list")
        );
        assert!(texts.current("other").is_err());

        std::fs::write(
            config.join("hosts/opencode.json"),
            r#"{ "todowrite": { "emtpy": "typo" } }"#,
        )
        .unwrap();
        assert!(
            HostTexts::load(&config)
                .err()
                .unwrap()
                .contains("unknown text todowrite.emtpy")
        );

        std::fs::write(
            config.join("hosts/opencode.json"),
            r#"{ "todowrite": { "empty": 3 } }"#,
        )
        .unwrap();
        assert!(
            HostTexts::load(&config)
                .err()
                .unwrap()
                .contains("wrong shape")
        );

        let _ = std::fs::remove_dir_all(&config);
    }

    #[test]
    fn an_open_group_takes_texts_under_any_name() {
        let config = dir("open");
        let write = |text: &str| std::fs::write(config.join("hosts/opencode.json"), text).unwrap();

        write(r#"{ "tool_descriptions": { "shell": "Run a command.", "glob": "Find paths." } }"#);
        let opencode = HostTexts::load(&config)
            .unwrap()
            .current("opencode")
            .unwrap();
        assert_eq!(opencode["tool_descriptions"]["shell"], "Run a command.");
        // A tool the shipped file does not name, beside the shipped ones.
        assert_eq!(opencode["tool_descriptions"]["glob"], "Find paths.");
        assert!(opencode["tool_descriptions"]["grep"].is_string());
        assert_eq!(opencode["$open"], serde_json::json!(["tool_descriptions"]));

        write(r#"{ "$open": ["ask_chauffeur"] }"#);
        assert!(
            HostTexts::load(&config)
                .err()
                .unwrap()
                .contains("set by the shipped texts only")
        );

        write(r#"{ "tool_descriptions": { "shell": "" } }"#);
        assert!(
            HostTexts::load(&config)
                .err()
                .unwrap()
                .contains("must be 1-")
        );

        write(r#"{ "tool_descriptions": { "shell": { "nested": "no" } } }"#);
        assert!(
            HostTexts::load(&config)
                .err()
                .unwrap()
                .contains("wrong shape")
        );

        let _ = std::fs::remove_dir_all(&config);
    }
}
