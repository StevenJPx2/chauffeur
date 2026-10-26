//! Which judge answers each ask, and the judges the engine runs in place,
//! from `judge.json`'s `routes` and `facts`. System One and System Two run on
//! the scheduler's workers; the facts judge decides from a question's facts
//! by its configured rule, inside the engine.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use chauffeur_core::{Routes, SYSTEM_ONE, SYSTEM_TWO, load_layered};
use chauffeur_judge_facts::{FactsConfig, FactsJudge, NAME as FACTS};
use serde::Deserialize;
use serde::de::IgnoredAny;

/// The shipped routes and rules, in the same file as System Two's profiles.
const SHIPPED: &str = include_str!("../../../skills/config/judge.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JudgeFile {
    #[serde(default)]
    routes: BTreeMap<String, String>,
    #[serde(default)]
    facts: FactsConfig,
    /// Read by [`crate::reasoning`].
    #[serde(default)]
    #[expect(dead_code, reason = "System Two's profiles are read where they run")]
    system_two: IgnoredAny,
}

/// The routes and the facts judge, from the shipped `judge.json` overlaid by
/// yours in `config_dir`.
///
/// # Errors
///
/// An unreadable or invalid file; a route to a judge that does not exist; or
/// a route to the facts judge for a question it has no rule for.
pub(crate) fn load(config_dir: &Path) -> Result<(Routes, FactsJudge), String> {
    let path = config_dir.join("judge.json");
    let file = load_layered::<JudgeFile>(SHIPPED, &path)?;
    let at = |error: String| format!("{}: {error}", path.display());
    let facts = file.facts.checked().map_err(at)?;

    for (id, judge) in &file.routes {
        if ![SYSTEM_ONE, SYSTEM_TWO, FACTS].contains(&judge.as_str()) {
            return Err(at(format!(
                "routes.{id} names {judge}; judges: {SYSTEM_ONE}, {SYSTEM_TWO}, {FACTS}"
            )));
        }

        if judge == FACTS && !facts.has(id) {
            return Err(at(format!(
                "routes.{id} goes to {FACTS}, which has no rule for it"
            )));
        }
    }

    Ok((
        Routes::new(file.routes.into_iter().collect::<HashMap<_, _>>()),
        FactsJudge::new(facts),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str, json: Option<&str>) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("chauffeur-judges-{}-{name}", std::process::id()));

        std::fs::create_dir_all(&dir).unwrap();
        if let Some(json) = json {
            std::fs::write(dir.join("judge.json"), json).unwrap();
        }
        dir
    }

    #[test]
    fn a_route_sends_a_question_to_the_facts_judge_and_the_rest_to_system_one() {
        let path = dir(
            "routed",
            Some(
                r#"{
                    "routes": { "x/y": "facts" },
                    "facts": { "x/y": { "all": [{ "fact": "n", "at_least": 1 }] } }
                }"#,
            ),
        );
        let (routes, _) = load(&path).unwrap();

        assert_eq!(routes.question("x/y"), FACTS);
        assert_eq!(routes.question("skill-exposure/x"), SYSTEM_ONE);
    }

    #[test]
    fn a_route_to_an_unknown_judge_or_a_missing_rule_is_an_error() {
        let unknown = dir("unknown", Some(r#"{ "routes": { "x/y": "oracle" } }"#));
        let ruleless = dir("ruleless", Some(r#"{ "routes": { "x/y": "facts" } }"#));

        assert!(load(&unknown).unwrap_err().contains("oracle"));
        assert!(load(&ruleless).unwrap_err().contains("no rule"));
    }
}
