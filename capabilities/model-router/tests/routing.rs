use std::sync::Arc;

use chauffeur_capability_model_router::{ModelRouter, ModelRouterConfig, STAY};
use chauffeur_capability_model_router::{Provider, Tier, TierEntry};
use chauffeur_core::{
    Answer, AnswerValue, AvailableModel, CachePrices, Capability, Effect, Judging, ModelRef, Plan,
    QuestionKind, Signal, SignalKind, Situation,
};

struct Table(&'static str, Vec<TierEntry>);

impl Provider for Table {
    fn id(&self) -> &str {
        self.0
    }

    fn tiers(&self) -> &[TierEntry] {
        &self.1
    }
}

/// Rows as `(model, tier)` for every variant.
fn rows(entries: &[(&'static str, Tier)]) -> Vec<TierEntry> {
    entries
        .iter()
        .map(|(model, tier)| TierEntry {
            model: (*model).to_string(),
            variant: None,
            tier: *tier,
            recommended: false,
        })
        .collect()
}

fn model_ref(key: &str) -> ModelRef {
    model(key)
}

/// `provider/model` or `provider/model#variant`.
fn model(key: &str) -> ModelRef {
    let (provider, rest) = key.split_once('/').expect("provider/model");
    let (id, variant) = match rest.split_once('#') {
        Some((id, variant)) => (id, Some(variant.to_string())),
        None => (rest, None),
    };

    ModelRef {
        provider: provider.into(),
        model: id.into(),
        variant,
    }
}

fn defaults() -> ModelRouterConfig {
    ModelRouterConfig::default()
}

fn switch_back_after() -> u64 {
    defaults().switch_back.after_seconds
}

fn router(pins: &[&str]) -> Judging<ModelRouter> {
    router_with(ModelRouterConfig {
        pins: pins.iter().map(|pin| (*pin).to_string()).collect(),
        ..defaults()
    })
}

fn router_with(config: ModelRouterConfig) -> Judging<ModelRouter> {
    let providers: Vec<Arc<dyn Provider>> = vec![
        Arc::new(Table(
            "anthropic",
            rows(&[("opus", Tier::Frontier), ("haiku", Tier::Fast)]),
        )),
        Arc::new(Table(
            "openai",
            rows(&[
                ("sol", Tier::Frontier),
                ("luna", Tier::Frontier),
                ("spark", Tier::Fast),
            ]),
        )),
    ];

    Judging::new(ModelRouter::new(providers, config))
}

fn limit(tool_executed: bool) -> Signal {
    let available = [
        "anthropic/opus",
        "anthropic/haiku",
        "openai/sol",
        "openai/luna",
        "openai/spark",
    ]
    .into_iter()
    .map(|key| AvailableModel {
        model: model(key),
        usable: true,
    })
    .collect();

    Signal {
        agent_id: "session".into(),
        at: 1,
        kind: SignalKind::ModelError {
            model: model("anthropic/opus"),
            error_type: "rate_limit_error".into(),
            status: Some(429),
            message: "rate limit reached, retry shortly".into(),
            tool_executed,
            available,
        },
    }
}

fn options(plan: &Plan) -> Vec<String> {
    let Plan::Ask(questions) = plan else {
        panic!("expected a question, got {plan:?}")
    };
    let [question] = questions.as_slice() else {
        panic!("expected one question")
    };
    let QuestionKind::Choice { options } = &question.kind else {
        panic!("expected a choice")
    };

    options.iter().map(|option| option.value.clone()).collect()
}

fn choice(value: &str, confidence: f32) -> Answer {
    Answer {
        id: "choice".into(),
        value: AnswerValue::Choice(value.into()),
        confidence: Some(confidence),
    }
}

fn switch(key: &str) -> Vec<Effect> {
    vec![Effect::Model {
        agent_id: "session".into(),
        model: Some(model(key)),
    }]
}

#[test]
fn asks_among_same_tier_candidates_plus_stay() {
    let plan = router(&[]).plan(&Situation::default(), &limit(false));

    assert_eq!(options(&plan), vec!["openai/sol", "openai/luna", STAY]);
}

#[test]
fn pins_order_the_candidates() {
    let plan = router(&["openai/luna"]).plan(&Situation::default(), &limit(false));

    assert_eq!(options(&plan), vec!["openai/luna", "openai/sol", STAY]);
}

#[test]
fn a_blacklisted_rated_model_is_never_offered_even_when_pinned() {
    let config = ModelRouterConfig {
        pins: vec!["openai/sol".into()],
        blacklist: vec!["openai/sol".into()],
        ..defaults()
    };

    assert_eq!(
        options(&router_with(config).plan(&Situation::default(), &limit(false))),
        vec!["openai/luna", STAY]
    );
}

#[test]
fn no_eligible_model_keeps_the_current_model() {
    let config = ModelRouterConfig {
        blacklist: vec!["*/*".into()],
        ..defaults()
    };

    assert_eq!(
        router_with(config).plan(&Situation::default(), &limit(false)),
        keep()
    );
}

#[test]
fn applies_the_judged_choice_and_never_retries_it() {
    let mut router = router(&[]);
    let signal = limit(false);
    router.plan(&Situation::default(), &signal);

    assert_eq!(
        router.decide(&signal, Some(&[choice("openai/luna", 0.8)][..])),
        switch("openai/luna")
    );
    // Luna's limit covers all of openai, and anthropic is already exhausted.
    let mut failed = limit(false);
    if let SignalKind::ModelError { model, .. } = &mut failed.kind {
        *model = model_ref("openai/luna");
    }
    assert_eq!(router.plan(&Situation::default(), &failed), keep());
}

fn keep() -> Plan {
    Plan::Settled(vec![Effect::Model {
        agent_id: "session".into(),
        model: None,
    }])
}

#[test]
fn a_limit_covers_every_model_of_its_provider() {
    // Opus hits the anthropic limit: haiku is never offered, even pinned.
    let plan = router(&["anthropic/haiku"]).plan(&Situation::default(), &limit(false));

    assert_eq!(options(&plan), vec!["openai/sol", "openai/luna", STAY]);
}

#[test]
fn a_request_too_large_for_a_provider_moves_on_without_waiting() {
    // The context outgrew what the provider accepts: no model behind it can
    // take the request, and waiting will not shrink it.
    let mut signal = limit(false);
    if let SignalKind::ModelError {
        error_type,
        status,
        message,
        ..
    } = &mut signal.kind
    {
        *error_type = "unknown".into();
        *status = None;
        *message = "BodyLimitError: Anthropic request body exceeds 10485760 byte limit".into();
    }

    let plan = router(&["anthropic/haiku"]).plan(&Situation::default(), &signal);

    assert_eq!(options(&plan), vec!["openai/sol", "openai/luna"]);
}

#[test]
fn a_model_that_cannot_serve_leaves_the_rest_of_its_provider() {
    let mut router = switched_router();

    // sol does not exist for this account; luna, also openai, still can serve.
    assert_eq!(
        options(&router.plan(&Situation::default(), &auth_error("openai/sol"))),
        vec!["openai/luna", STAY]
    );
}

#[test]
fn stay_keeps_the_model() {
    let mut router = router(&[]);
    let signal = limit(false);
    router.plan(&Situation::default(), &signal);

    assert_eq!(
        router.decide(&signal, Some(&[choice(STAY, 0.9)][..])),
        vec![Effect::Model {
            agent_id: "session".into(),
            model: None
        }]
    );
}

#[test]
fn classifier_failure_or_low_confidence_switches_to_the_preferred_candidate() {
    let mut router = router(&["openai/luna"]);
    let signal = limit(false);
    router.plan(&Situation::default(), &signal);

    assert_eq!(
        router.decide(&signal, Some(&[choice(STAY, 0.05)][..])),
        switch("openai/luna")
    );
    // The switch lands on luna, which hits openai's limit: nothing is left.
    let mut failed = limit(false);
    if let SignalKind::ModelError { model, .. } = &mut failed.kind {
        *model = model_ref("openai/luna");
    }
    assert_eq!(router.plan(&Situation::default(), &failed), keep());
}

#[test]
fn a_tool_that_already_ran_blocks_failover_without_asking() {
    let plan = router(&[]).plan(&Situation::default(), &limit(true));

    assert_eq!(
        plan,
        Plan::Settled(vec![Effect::Model {
            agent_id: "session".into(),
            model: None
        }])
    );
}

#[test]
fn success_resets_attempts_and_non_limit_errors_are_ignored() {
    let mut router = router(&[]);
    let signal = limit(false);
    router.plan(&Situation::default(), &signal);
    router.decide(&signal, Some(&[choice("openai/sol", 0.9)][..]));

    let success = Signal {
        agent_id: "session".into(),
        at: 2,
        kind: SignalKind::ModelSucceeded {
            model: model("openai/sol"),
        },
    };

    assert_eq!(router.plan(&Situation::default(), &success), Plan::Skip);
    assert_eq!(
        options(&router.plan(&Situation::default(), &signal)),
        vec!["openai/sol", "openai/luna", STAY]
    );
    assert!(!defaults().is_limit_error("invalid_request", Some(400), "bad schema"));
    assert!(defaults().is_limit_error("Rate Limit", None, ""));
    // OpenCode's own classification of an exhausted provider balance.
    assert!(defaults().is_limit_error("provider.quota", None, ""));
    assert!(defaults().is_limit_error("provider.invalid-request", Some(402), ""));
    // A provider out of capacity.
    assert!(defaults().is_limit_error(
        "provider.internal",
        Some(503),
        "This model is currently experiencing high demand."
    ));
    assert!(!defaults().is_limit_error("provider.invalid-request", Some(400), "unknown field"));
    // Anthropic reports an empty balance as an invalid request.
    assert!(defaults().is_limit_error(
        "provider.invalid-request",
        Some(400),
        "Your credit balance is too low to access the Anthropic API."
    ));
}

#[test]
fn a_late_success_from_a_previous_model_does_not_reoffer_a_failed_candidate() {
    let mut router = switched_router();
    let error = auth_error("openai/sol");

    assert_eq!(
        options(&router.plan(&Situation::default(), &error)),
        vec!["openai/luna", STAY]
    );
    assert_eq!(
        router.decide(&error, Some(&[choice("openai/luna", 0.9)])),
        switch("openai/luna")
    );
    let late = Signal {
        agent_id: "session".into(),
        at: 3,
        kind: SignalKind::ModelSucceeded {
            model: model("openai/sol"),
        },
    };
    router.plan(&Situation::default(), &late);

    let mut limit = limit(false);
    if let SignalKind::ModelError { model: failed, .. } = &mut limit.kind {
        *failed = model("openai/luna");
    }
    // Luna's limit covers openai and anthropic is exhausted: the late success
    // from sol did not make either usable again.
    assert_eq!(router.plan(&Situation::default(), &limit), keep());
}

#[test]
fn a_proposed_switch_that_never_reached_the_host_can_be_retried() {
    let mut router = switched_router();

    assert_eq!(
        options(&router.plan(&Situation::default(), &limit(false))),
        vec!["openai/sol", "openai/luna", STAY]
    );
}

#[test]
fn an_unknown_tier_offers_pinned_then_tiered_models_only() {
    let mut router = router(&["google/pinned"]);
    let mut signal = limit(false);

    if let SignalKind::ModelError {
        model, available, ..
    } = &mut signal.kind
    {
        *model = model_ref("unknown/current");
        available.extend((0..40).map(|n| AvailableModel {
            model: model_ref(&format!("many/m{n}")),
            usable: true,
        }));
        available.push(AvailableModel {
            model: model_ref("google/pinned"),
            usable: true,
        });
    }

    let offered = options(&router.plan(&Situation::default(), &signal));

    // The 40 untiered models are left out; the pin and the five tiered ones stay.
    assert_eq!(
        offered,
        vec![
            "google/pinned",
            "anthropic/opus",
            "anthropic/haiku",
            "openai/sol",
            "openai/luna",
            "openai/spark",
            STAY
        ]
    );
}

#[test]
fn a_pinned_model_is_a_candidate_across_tiers() {
    let mut router = router(&["openai/spark"]);

    assert_eq!(
        options(&router.plan(&Situation::default(), &limit(false))),
        vec!["openai/spark", "openai/sol", "openai/luna", STAY]
    );
}

fn user_message(at: u64, current: &str) -> Signal {
    Signal {
        agent_id: "session".into(),
        at,
        kind: SignalKind::UserMessage {
            text: "keep going".into(),
            first_in_context: false,
            skills: Vec::new(),
            tools: Vec::new(),
            model: Some(model(current)),
            code_mode: Vec::new(),
            workspace: String::new(),
            ranking: Vec::new(),
        },
    }
}

fn switch_back(p: f32) -> Answer {
    Answer {
        id: "switch_back".into(),
        value: AnswerValue::Noul(p),
        confidence: None,
    }
}

/// Fail over from anthropic/opus to openai/sol at t=1.
fn switched_router() -> Judging<ModelRouter> {
    switched_router_with(defaults())
}

fn switched_router_with(config: ModelRouterConfig) -> Judging<ModelRouter> {
    let mut router = router_with(config);
    let signal = limit(false);
    let chosen = Answer {
        id: "choice".into(),
        value: AnswerValue::Choice("openai/sol".into()),
        confidence: Some(0.9),
    };

    router.plan(&Situation::default(), &signal);
    assert!(matches!(
        router.decide(&signal, Some(&[chosen])).as_slice(),
        [Effect::Model { model: Some(model), .. }] if model.key() == "openai/sol"
    ));

    router
}

#[test]
fn switching_back_is_judged_only_after_the_wait_and_on_the_switched_model() {
    let mut router = switched_router();
    let after = 1 + switch_back_after();

    assert_eq!(
        router.plan(&Situation::default(), &user_message(60, "openai/sol")),
        Plan::Skip
    );

    let Plan::Ask(questions) =
        router.plan(&Situation::default(), &user_message(after, "openai/sol"))
    else {
        panic!("expected a switch-back question")
    };
    assert!(
        questions[0]
            .instructions
            .contains("model anthropic/opus failed")
    );

    assert_eq!(
        router.decide(
            &user_message(after, "openai/sol"),
            Some(&[switch_back(0.9)])
        ),
        vec![Effect::Model {
            agent_id: "session".into(),
            model: Some(model("anthropic/opus"))
        }]
    );
    // Back on the original model: nothing more to judge.
    assert_eq!(
        router.plan(
            &Situation::default(),
            &user_message(after + 1, "anthropic/opus")
        ),
        Plan::Skip
    );
}

#[test]
fn an_unconvinced_judgment_stays_and_a_manual_model_change_forgets_the_origin() {
    let mut router = switched_router();
    let after = 1 + switch_back_after();

    assert!(matches!(
        router.plan(&Situation::default(), &user_message(after, "openai/sol")),
        Plan::Ask(_)
    ));
    assert!(
        router
            .decide(
                &user_message(after, "openai/sol"),
                Some(&[switch_back(0.6)])
            )
            .is_empty()
    );
    assert!(matches!(
        router.plan(&Situation::default(), &user_message(after, "openai/sol")),
        Plan::Ask(_)
    ));
    assert!(
        router
            .decide(&user_message(after, "openai/sol"), None)
            .is_empty()
    );

    // The user picked another model themselves.
    assert_eq!(
        router.plan(&Situation::default(), &user_message(after, "openai/luna")),
        Plan::Skip
    );
    assert_eq!(
        router.plan(
            &Situation::default(),
            &user_message(after + 1, "openai/sol")
        ),
        Plan::Skip
    );
}

#[test]
fn a_lower_switch_back_bar_returns_on_a_less_sure_answer() {
    let mut config = defaults();
    config.switch_back.bar =
        serde_json::from_str(r#"{ "at": 0.5, "confidence": 0.2 }"#).expect("a valid bar");
    let mut router = switched_router_with(config);
    let message = user_message(1 + switch_back_after(), "openai/sol");

    router.plan(&Situation::default(), &message);

    // P = 0.6 (confidence 0.2) stays at the shipped bar and returns at this one.
    assert_eq!(
        router.decide(&message, Some(&[switch_back(0.6)])),
        vec![Effect::Model {
            agent_id: "session".into(),
            model: Some(model("anthropic/opus"))
        }]
    );
}

#[test]
fn a_configured_limit_phrase_makes_an_error_a_limit() {
    let mut signal = limit(false);

    if let SignalKind::ModelError {
        error_type,
        status,
        message,
        ..
    } = &mut signal.kind
    {
        *error_type = "provider.internal".into();
        *status = Some(500);
        *message = "Too busy, come back tomorrow".into();
    }

    assert_eq!(router(&[]).plan(&Situation::default(), &signal), Plan::Skip);

    let mut config = defaults();
    config.limit.messages.push("too busy".into());

    assert_eq!(
        options(&router_with(config).plan(&Situation::default(), &signal)),
        vec!["openai/sol", "openai/luna", STAY]
    );
}

fn auth_error(on: &str) -> Signal {
    let mut signal = limit(false);

    if let SignalKind::ModelError {
        model,
        error_type,
        status,
        message,
        ..
    } = &mut signal.kind
    {
        *model = model_ref(on);
        *error_type = "provider.auth".into();
        *status = Some(401);
        *message = "Invalid API key".into();
    }

    signal.at = 2;
    signal
}

#[test]
fn a_switch_to_an_unusable_model_moves_on_to_the_next_candidate() {
    let mut router = switched_router();

    // openai/sol, which the router chose, rejects the credentials.
    let offered = options(&router.plan(&Situation::default(), &auth_error("openai/sol")));
    assert_eq!(offered, vec!["openai/luna", STAY]);

    // The same error on a model the user chose is not the router's to fix.
    assert_eq!(
        router.plan(&Situation::default(), &auth_error("anthropic/opus")),
        Plan::Skip
    );
    assert!(defaults().is_unusable_error("provider.auth", None, ""));
    assert!(!defaults().is_unusable_error("provider.quota", Some(402), "insufficient funds"));
}

#[test]
fn the_model_left_behind_survives_a_restart() {
    let before = switched_router();
    let mut after = router(&[]);

    after.load(before.save().expect("the router keeps memory"));

    let Plan::Ask(questions) = after.plan(
        &Situation::default(),
        &user_message(1 + switch_back_after(), "openai/sol"),
    ) else {
        panic!("expected a switch-back question after the restart")
    };
    assert!(
        questions[0]
            .instructions
            .contains("model anthropic/opus failed")
    );
}

#[test]
fn a_restored_blacklisted_origin_is_never_offered_for_switch_back() {
    let before = switched_router();
    let mut after = router_with(ModelRouterConfig {
        blacklist: vec!["anthropic/*".into()],
        ..defaults()
    });
    after.load(before.save().unwrap());

    let message = user_message(1 + switch_back_after(), "openai/sol");
    assert_eq!(after.plan(&Situation::default(), &message), Plan::Skip);
    assert_eq!(after.plan(&Situation::default(), &message), Plan::Skip);
}

#[test]
fn a_second_fallback_does_not_restart_the_original_limits_clock() {
    let mut router = switched_router();
    let mut error = auth_error("openai/sol");

    error.at = 240;
    router.plan(&Situation::default(), &error);
    assert_eq!(
        router.decide(&error, Some(&[choice("openai/luna", 0.9)])),
        switch("openai/luna")
    );

    assert!(matches!(
        router.plan(
            &Situation::default(),
            &user_message(1 + switch_back_after(), "openai/luna")
        ),
        Plan::Ask(_)
    ));
}

/// The shipped tables, with the host's models listed without variants.
fn shipped_router() -> Judging<ModelRouter> {
    let providers: Vec<Arc<dyn Provider>> = vec![
        Arc::new(chauffeur_plugin_anthropic::AnthropicProvider::default()),
        Arc::new(chauffeur_plugin_openai::OpenAiProvider::default()),
    ];

    Judging::new(ModelRouter::new(providers, ModelRouterConfig::default()))
}

fn limit_on(current: &str) -> Signal {
    let mut signal = limit(false);

    if let SignalKind::ModelError {
        model: failed,
        available,
        ..
    } = &mut signal.kind
    {
        *failed = model(current);
        *available = [
            "anthropic/claude-opus-5-5",
            "anthropic/claude-sonnet-5-5",
            "openai/gpt-6.1-sol",
            "openai/gpt-6-luna",
            "opencode/claude-sonnet-5-5",
            "opencode/fledge-alpha-free",
            "opencode/ling-3.1-flash-free",
        ]
        .into_iter()
        .map(|key| AvailableModel {
            model: model(key),
            usable: true,
        })
        .collect();
    }

    signal
}

#[test]
fn tiers_follow_thinking_variants_and_never_offer_the_same_model() {
    let mut router = shipped_router();

    // Opus at high thinking is frontier: only gpt-6.1-sol matches, at high.
    assert_eq!(
        options(&router.plan(
            &Situation::default(),
            &limit_on("anthropic/claude-opus-5-5#high")
        )),
        vec!["openai/gpt-6.1-sol#high", STAY]
    );
    // Opus at its default thinking is frontier too, by the family's any-variant row.
    assert_eq!(
        options(&shipped_router().plan(
            &Situation::default(),
            &limit_on("anthropic/claude-opus-5-5")
        )),
        vec!["openai/gpt-6.1-sol#high", STAY]
    );
    // Opus at low thinking is balanced: gpt-6-luna at max thinking. Sonnet
    // shares the anthropic limit, and the gateway copy is not used directly.
    assert_eq!(
        options(&shipped_router().plan(
            &Situation::default(),
            &limit_on("anthropic/claude-opus-5-5#low")
        )),
        vec!["openai/gpt-6-luna#max", STAY]
    );

    // A switch carries the variant.
    let signal = limit_on("anthropic/claude-opus-5-5#low");
    router.plan(&Situation::default(), &signal);
    let pick = Answer {
        id: "choice".into(),
        value: AnswerValue::Choice("openai/gpt-6-luna#max".into()),
        confidence: Some(0.9),
    };
    assert!(matches!(
        router.decide(&signal, Some(&[pick])).as_slice(),
        [Effect::Model { model: Some(model), .. }] if model.variant.as_deref() == Some("max") && model.model == "gpt-6-luna"
    ));
}

#[test]
fn an_agent_on_a_gateway_moves_to_its_tier_on_a_provider_used_directly() {
    // claude-sonnet-5-5 through OpenCode's gateway is balanced, like the original.
    assert_eq!(
        options(&shipped_router().plan(
            &Situation::default(),
            &limit_on("opencode/claude-sonnet-5-5")
        )),
        vec![
            "anthropic/claude-opus-5-5#low",
            "anthropic/claude-sonnet-5-5#high",
            "openai/gpt-6-luna#max",
            STAY
        ]
    );

    // An untiered current model may move to any tiered one, but never to
    // another untiered free model.
    let offered = options(&shipped_router().plan(
        &Situation::default(),
        &limit_on("opencode/fledge-alpha-free"),
    ));

    assert!(
        !offered.iter().any(|key| key.contains("free")),
        "{offered:?}"
    );
    assert!(
        offered.contains(&"openai/gpt-6.1-sol#high".to_string()),
        "{offered:?}"
    );
}

#[test]
fn recommended_models_come_first_and_fast_or_old_editions_are_never_offered() {
    let mut signal = limit_on("anthropic/claude-opus-5-5#high");

    if let SignalKind::ModelError { available, .. } = &mut signal.kind {
        // Listed first, as hosts list them, ahead of the recommended model.
        let older = [
            "openai/gpt-6-sol",
            "openai/gpt-6.1-sol-fast",
            "openai/gpt-5.6-sol",
            "anthropic/claude-opus-5",
            "anthropic/claude-opus-5-5-fast",
            "anthropic/claude-opus-4-8",
        ];

        available.splice(
            0..0,
            older.into_iter().map(|key| AvailableModel {
                model: model(key),
                usable: true,
            }),
        );
    }

    let mut router = shipped_router();
    let plan = router.plan(&Situation::default(), &signal);

    assert_eq!(
        options(&plan),
        vec!["openai/gpt-6.1-sol#high", "openai/gpt-6-sol#high", STAY]
    );

    let Plan::Ask(questions) = &plan else {
        unreachable!()
    };
    let QuestionKind::Choice { options } = &questions[0].kind else {
        unreachable!()
    };
    assert!(options[0].description.contains("recommended"));
    assert!(!options[1].description.contains("recommended"));
}

#[test]
fn with_no_same_tier_model_left_another_tier_comes_before_free_models() {
    // gpt-6-luna at default thinking is the only fast model available.
    let offered =
        options(&shipped_router().plan(&Situation::default(), &limit_on("openai/gpt-6-luna")));

    assert!(
        !offered.iter().any(|key| key.contains("free")),
        "{offered:?}"
    );
    assert_eq!(offered[0], "anthropic/claude-opus-5-5#high");
}

#[test]
fn free_gateway_models_are_offered_only_when_nothing_else_is_left() {
    let mut signal = limit_on("anthropic/claude-opus-5-5#high");

    // Only the gateway's models remain.
    if let SignalKind::ModelError { available, .. } = &mut signal.kind {
        available.retain(|entry| entry.model.provider == "opencode");
    }

    let offered = options(&shipped_router().plan(&Situation::default(), &signal));

    assert_eq!(
        offered,
        vec![
            "opencode/fledge-alpha-free",
            "opencode/ling-3.1-flash-free",
            STAY
        ]
    );
}

#[test]
fn go_is_blocked_even_when_pinned_or_last_resort_but_zen_free_models_remain() {
    let mut signal = limit_on("anthropic/claude-opus-5-5#high");
    if let SignalKind::ModelError { available, .. } = &mut signal.kind {
        available.retain(|entry| entry.model.provider == "opencode");
        available.splice(
            0..0,
            ["opencode-go/sol", "opencode-go/example-free"].map(|key| AvailableModel {
                model: model(key),
                usable: true,
            }),
        );
    }
    let config = ModelRouterConfig {
        pins: vec!["opencode-go/sol".into(), "opencode-go/example-free".into()],
        last_resort: vec!["opencode-go/*".into(), "opencode/*-free".into()],
        ..defaults()
    };

    assert_eq!(
        options(&router_with(config).plan(&Situation::default(), &signal)),
        vec![
            "opencode/fledge-alpha-free",
            "opencode/ling-3.1-flash-free",
            STAY
        ]
    );
}

#[test]
fn waiting_is_offered_once_then_the_next_limit_switches() {
    let mut router = router(&[]);
    let signal = limit(false);

    router.plan(&Situation::default(), &signal);
    assert_eq!(
        router.decide(&signal, Some(&[choice(STAY, 0.9)][..])),
        vec![Effect::Model {
            agent_id: "session".into(),
            model: None
        }]
    );

    // The same model hits its limit again: stay is no longer offered.
    let mut again = limit(false);
    again.at = 2;
    assert_eq!(
        options(&router.plan(&Situation::default(), &again)),
        vec!["openai/sol", "openai/luna"]
    );
}

#[test]
fn a_lasting_limit_never_offers_stay_and_one_candidate_switches_without_asking() {
    let mut router = router(&[]);
    let mut signal = limit(false);

    if let SignalKind::ModelError {
        error_type,
        status,
        message,
        available,
        ..
    } = &mut signal.kind
    {
        *error_type = "provider.quota".into();
        *status = None;
        *message = "You exceeded your current quota".into();
        available.retain(|entry| entry.model.model != "luna");
    }

    let Plan::Settled(effects) = router.plan(&Situation::default(), &signal) else {
        panic!("one candidate and no stay needs no question")
    };

    assert_eq!(effects, switch("openai/sol"));
}

/// A prompt resuming a 600k-token context on anthropic/opus after 20 idle
/// minutes, with `available` models.
fn resumed(available: &[&str], window: u64) -> Signal {
    Signal {
        agent_id: "session".into(),
        at: 1,
        kind: SignalKind::ContextResumed {
            model: model("anthropic/opus"),
            context_tokens: 600_000,
            context_window: window,
            idle_seconds: 1_200,
            available: available
                .iter()
                .map(|key| AvailableModel {
                    model: model(key),
                    usable: true,
                })
                .collect(),
        },
    }
}

fn renew(p: f32) -> Answer {
    Answer {
        id: "renew_context".into(),
        value: AnswerValue::Noul(p),
        confidence: Some(1.0),
    }
}

fn compact(key: &str) -> Vec<Effect> {
    vec![Effect::Compact {
        agent_id: "session".into(),
        model: model(key),
    }]
}

#[test]
fn a_resumed_context_asks_with_its_facts_and_a_yes_compacts_with_the_fast_model() {
    let mut router = router(&[]);
    let signal = resumed(
        &["anthropic/opus", "anthropic/haiku", "openai/spark"],
        1_000_000,
    );
    let Plan::Ask(questions) = router.plan(&Situation::default(), &signal) else {
        panic!("expected the renewal question")
    };
    let facts = &questions[0].facts;

    assert_eq!(questions[0].id, "renew_context");
    assert_eq!(facts["idle_seconds"], 1_200);
    assert_eq!(facts["cache_seconds"], 300);
    assert_eq!(facts["context_tokens"], 600_000);
    assert_eq!(facts["context_window"], 1_000_000);
    assert_eq!(facts["context_budget"], 100_000);
    assert!(questions[0].instructions.contains("idle 20 minutes"));
    assert!(questions[0].instructions.contains("at least 100000 tokens"));

    // The agent's own provider's fast model writes the summary.
    assert_eq!(
        router.decide(&signal, Some(&[renew(1.0)])),
        compact("anthropic/haiku")
    );

    router.plan(&Situation::default(), &signal);
    assert!(router.decide(&signal, Some(&[renew(0.0)])).is_empty());
    router.plan(&Situation::default(), &signal);
    assert!(router.decide(&signal, None).is_empty());
}

#[test]
fn an_unknown_window_is_left_out_of_the_facts() {
    let Plan::Ask(questions) =
        router(&[]).plan(&Situation::default(), &resumed(&["anthropic/haiku"], 0))
    else {
        panic!("expected the renewal question")
    };

    assert!(!questions[0].facts.contains_key("context_window"));
    assert_eq!(questions[0].facts["context_budget"], 100_000);
    assert!(questions[0].instructions.contains("unknown"));
}

#[test]
fn the_budget_is_a_share_of_the_window_between_a_floor_and_a_ceiling() {
    let budget = defaults().renewal.budget;

    assert_eq!(budget.tokens(2_000_000), 100_000);
    assert_eq!(budget.tokens(1_000_000), 100_000);
    assert_eq!(budget.tokens(350_000), 75_250);
    assert_eq!(budget.tokens(128_000), 50_000);
    assert_eq!(budget.tokens(32_000), 50_000);
    assert_eq!(budget.tokens(0), 100_000);

    for bad in [
        r#"{ "share": 0, "min_tokens": 1, "max_tokens": 2 }"#,
        r#"{ "share": 1.5, "min_tokens": 1, "max_tokens": 2 }"#,
        r#"{ "share": 0.2, "min_tokens": 3, "max_tokens": 2 }"#,
        r#"{ "share": 0.2, "min_tokens": 0, "max_tokens": 0 }"#,
    ] {
        let config = ModelRouterConfig {
            renewal: chauffeur_capability_model_router::Renewal {
                budget: serde_json::from_str(bad).expect("a budget"),
                ..defaults().renewal
            },
            ..defaults()
        };

        assert!(config.checked().is_err(), "{bad}");
    }
}

#[test]
fn the_summary_model_is_a_pin_then_fast_then_free_and_never_blacklisted() {
    let decide = |config: ModelRouterConfig, available: &[&str]| {
        let mut router = router_with(config);
        let signal = resumed(available, 1_000_000);

        router.plan(&Situation::default(), &signal);
        router.decide(&signal, Some(&[renew(1.0)]))
    };
    let pinned = ModelRouterConfig {
        renewal: chauffeur_capability_model_router::Renewal {
            pins: vec!["openai/spark".into()],
            ..defaults().renewal
        },
        ..defaults()
    };
    let everything = [
        "anthropic/opus",
        "anthropic/haiku",
        "openai/spark",
        "opencode/x-free",
    ];

    assert_eq!(decide(pinned, &everything), compact("openai/spark"));
    // Without the agent's own fast model, another provider's.
    assert_eq!(
        decide(
            defaults(),
            &["anthropic/opus", "openai/spark", "opencode/x-free"]
        ),
        compact("openai/spark")
    );
    // No fast model at all: a free one.
    assert_eq!(
        decide(defaults(), &["anthropic/opus", "opencode/x-free"]),
        compact("opencode/x-free")
    );
    let blocked = ModelRouterConfig {
        blacklist: vec!["anthropic/haiku".into()],
        ..defaults()
    };
    assert_eq!(
        decide(blocked, &["anthropic/haiku", "openai/spark"]),
        compact("openai/spark")
    );
}

/// Prices in millionths of a dollar per million tokens: input, cache read, cache write.
fn prices(input: u64, cache_read: u64, cache_write: u64) -> CachePrices {
    CachePrices {
        input,
        cache_read,
        cache_write,
    }
}

/// A session on `key` going idle with a 120k-token context.
fn idle(key: &str, prices: CachePrices) -> Signal {
    Signal {
        agent_id: "session".into(),
        at: 1,
        kind: SignalKind::ContextIdle {
            model: model(key),
            context_tokens: 120_000,
            prices,
        },
    }
}

fn warm(every_seconds: u64, times: u32) -> Plan {
    Plan::Settled(vec![Effect::Warm {
        agent_id: "session".into(),
        every_seconds,
        times,
    }])
}

#[test]
fn an_idle_context_is_warmed_before_its_cache_expires_for_at_most_an_hour() {
    let plan = |key: &str, prices: CachePrices| {
        router(&[]).plan(&Situation::default(), &idle(key, prices))
    };

    // Opus 5.5 reads its cache at 0.05x: 24 warms would still pay, so the
    // hour decides: every 4 minutes, the last at 56.
    assert_eq!(
        plan("anthropic/opus", prices(4_000_000, 200_000, 5_000_000)),
        warm(240, 14)
    );
    // At 0.1x, 11 warms cost as much as re-reading the context cold.
    assert_eq!(
        plan("anthropic/sonnet", prices(3_000_000, 300_000, 3_750_000)),
        warm(240, 11)
    );
    // OpenAI keeps its cache 30 minutes: warmed twice within the hour.
    assert_eq!(
        plan("openai/sol", prices(5_000_000, 250_000, 6_250_000)),
        warm(1_740, 2)
    );
    // No write price is the input price.
    assert_eq!(
        plan("openai/luna", prices(2_000_000, 200_000, 0)),
        warm(1_740, 2)
    );
}

#[test]
fn a_context_that_caching_cannot_save_or_a_limited_provider_is_not_warmed() {
    let opus = prices(4_000_000, 200_000, 5_000_000);
    let plan = |signal: &Signal| router(&[]).plan(&Situation::default(), signal);

    // No known cache price, or one a subscription makes free.
    assert_eq!(
        plan(&idle("anthropic/opus", prices(4_000_000, 0, 5_000_000))),
        Plan::Skip
    );
    assert_eq!(
        plan(&idle("anthropic/opus", CachePrices::default())),
        Plan::Skip
    );
    // A small context costs little to re-read.
    let mut small = idle("anthropic/opus", opus);
    if let SignalKind::ContextIdle { context_tokens, .. } = &mut small.kind {
        *context_tokens = 8_000;
    }
    assert_eq!(plan(&small), Plan::Skip);

    // Warming off.
    let off = ModelRouterConfig {
        renewal: chauffeur_capability_model_router::Renewal {
            warming: chauffeur_capability_model_router::Warming {
                max_seconds: 0,
                ..defaults().renewal.warming
            },
            ..defaults().renewal
        },
        ..defaults()
    };
    assert_eq!(
        router_with(off).plan(&Situation::default(), &idle("anthropic/opus", opus)),
        Plan::Skip
    );

    // A provider the agent hit a usage limit on.
    let mut limited = router(&[]);
    let failed = limit(false);
    limited.plan(&Situation::default(), &failed);
    limited.decide(&failed, Some(&[choice("openai/sol", 0.9)]));
    assert_eq!(
        limited.plan(&Situation::default(), &idle("anthropic/opus", opus)),
        Plan::Skip
    );
    assert_eq!(
        limited.plan(&Situation::default(), &idle("openai/sol", opus)),
        warm(1_740, 2)
    );
}

#[test]
fn a_session_already_on_a_cheap_model_is_left_to_compact_the_usual_way() {
    let on = |key: &str| {
        let mut signal = resumed(
            &["anthropic/opus", "anthropic/haiku", "opencode/x-free"],
            200_000,
        );

        if let SignalKind::ContextResumed { model, .. } = &mut signal.kind {
            *model = model_ref(key);
        }

        router(&[]).plan(&Situation::default(), &signal)
    };

    // A fast-tier model, a gateway copy of one, or a free last resort.
    assert_eq!(on("anthropic/haiku"), Plan::Skip);
    assert_eq!(on("opencode/haiku"), Plan::Skip);
    assert_eq!(on("opencode/x-free"), Plan::Skip);
    assert!(matches!(on("anthropic/opus"), Plan::Ask(_)));
}

#[test]
fn nothing_is_asked_when_no_model_can_write_the_summary() {
    assert_eq!(
        router(&[]).plan(
            &Situation::default(),
            &resumed(&["anthropic/opus"], 1_000_000)
        ),
        Plan::Skip
    );
}
