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
    fn the_shipped_file_sends_context_renewal_to_the_facts_judge() {
        let (routes, _) = load(&dir("shipped", None)).unwrap();

        assert_eq!(routes.question("model-router/renew_context"), FACTS);
        assert_eq!(routes.question("skill-exposure/x"), SYSTEM_ONE);
    }

    #[test]
    fn your_route_can_hand_a_question_back_to_system_one() {
        let path = dir(
            "back",
            Some(r#"{ "routes": { "model-router/renew_context": "system_one" } }"#),
        );
        let (routes, _) = load(&path).unwrap();

        assert_eq!(routes.question("model-router/renew_context"), SYSTEM_ONE);
    }

    /// The shipped router and judges, with no System One at all: the
    /// renewal question never reaches a model.
    fn engine() -> chauffeur_core::Engine {
        use std::sync::Arc;

        use chauffeur_capability_model_router::{ModelRouter, ModelRouterConfig, Provider};
        use chauffeur_core::{Engine, Judging};

        let (routes, facts) = load(&dir("engine", None)).unwrap();
        let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(
            chauffeur_plugin_anthropic::AnthropicProvider::default(),
        )];

        Engine::hosted(vec![Box::new(Judging::new(ModelRouter::new(
            providers,
            ModelRouterConfig::default(),
        )))])
        .unwrap()
        .with_judge(Box::new(facts))
        .with_routes(routes)
    }

    fn resumed(idle_seconds: u64, context_tokens: u64) -> chauffeur_core::Signal {
        use chauffeur_core::{AvailableModel, ModelRef, Signal, SignalKind};

        let model = |id: &str| ModelRef {
            provider: "anthropic".into(),
            model: id.into(),
            variant: None,
        };

        Signal {
            agent_id: "ses".into(),
            at: 1,
            kind: SignalKind::ContextResumed {
                model: model("claude-opus-5-5"),
                context_tokens,
                context_window: 1_000_000,
                idle_seconds,
                available: ["claude-opus-5-5", "claude-haiku-5"]
                    .map(|id| AvailableModel {
                        model: model(id),
                        usable: true,
                    })
                    .to_vec(),
            },
        }
    }

    #[test]
    fn a_cold_long_context_is_compacted_by_the_fast_model_without_any_model_judging_it() {
        use chauffeur_core::Effect;

        let mut engine = engine();
        let effects = engine.ingest(&resumed(1_800, 600_000)).unwrap();

        assert!(matches!(
            effects.as_slice(),
            [Effect::Compact { model, .. }] if model.model == "claude-haiku-5"
        ));
        // A 1M window renews at the 100k ceiling, not at a share of it.
        assert!(!engine.ingest(&resumed(1_800, 105_000)).unwrap().is_empty());
        // Warm cache, or a context under the budget: left as it is.
        assert!(engine.ingest(&resumed(60, 600_000)).unwrap().is_empty());
        assert!(engine.ingest(&resumed(1_800, 90_000)).unwrap().is_empty());
    }

    #[test]
    fn a_route_to_an_unknown_judge_or_a_missing_rule_is_an_error() {
        let unknown = dir("unknown", Some(r#"{ "routes": { "x/y": "oracle" } }"#));
        let ruleless = dir("ruleless", Some(r#"{ "routes": { "x/y": "facts" } }"#));

        assert!(load(&unknown).unwrap_err().contains("oracle"));
        assert!(load(&ruleless).unwrap_err().contains("no rule"));
    }
}
